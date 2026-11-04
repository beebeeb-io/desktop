    use super::{
        AppState, StartupSessionCheck, discard_unusable_startup_session, probe_startup_session, set_auth_email,
        set_auth_present,
    };
    use crate::account::{AccountId, synthesize_single_account};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Minimal loop-accepting HTTP/1.1 mock that answers EVERY request with
    /// one fixed status line and one fixed JSON body.
    struct AuthMeMockServer {
        base_url: String,
    }

    impl AuthMeMockServer {
        fn start(status_line: &'static str) -> Self {
            Self::start_with_body(status_line, "{}")
        }

        fn start_with_body(status_line: &'static str, body: &'static str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { return };
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf);
                    let response = format!(
                        "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
            });
            Self { base_url }
        }
    }

    #[tokio::test]
    async fn probe_startup_session_classifies_http_401_as_unauthorized() {
        let server = AuthMeMockServer::start("401 Unauthorized");
        assert_eq!(
            probe_startup_session(&server.base_url, "stored-token").await,
            StartupSessionCheck::Unauthorized,
            "exactly HTTP 401 must classify as Unauthorized — the ONLY auto-sign-out trigger"
        );
    }

    #[tokio::test]
    async fn probe_startup_session_classifies_2xx_as_authorized() {
        let server = AuthMeMockServer::start("200 OK");
        assert_eq!(
            probe_startup_session(&server.base_url, "stored-token").await,
            StartupSessionCheck::Authorized { profile: None },
            "a healthy token must classify as Authorized and restore normally"
        );
    }

    #[tokio::test]
    async fn probe_startup_session_fails_open_on_5xx_and_connection_refused() {
        let server = AuthMeMockServer::start("500 Internal Server Error");
        assert!(
            matches!(
                probe_startup_session(&server.base_url, "stored-token").await,
                StartupSessionCheck::Inconclusive(_)
            ),
            "a 5xx must be inconclusive (fail open — keep the session), never a sign-out trigger"
        );

        // Unreachable server: bind then drop a listener so the port refuses.
        let dead = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_url = format!("http://{}", dead.local_addr().unwrap());
        drop(dead);
        assert!(
            matches!(
                probe_startup_session(&dead_url, "stored-token").await,
                StartupSessionCheck::Inconclusive(_)
            ),
            "a network error must be inconclusive (fail open — offline users are never logged out)"
        );
    }

    /// The 401 action: boot signed-out. auth_present false (SignedOutGate
    /// routes to onboarding/sign-in), no in-memory session, mirrored email
    /// cleared. The keychain trio clear itself is the same idempotent call a
    /// real sign-out makes; on the Linux stub store it errors and must be
    /// tolerated (best-effort), which this test also proves on Linux.
    #[tokio::test]
    async fn discard_unusable_startup_session_boots_signed_out() {
        let state = AppState::default();
        synthesize_single_account(&state, AccountId("startup-discard-acct".to_string()));
        let acct = state.active_account().unwrap();
        // setup() seeds auth_present from keychain presence — a stored
        // session means true at this point.
        set_auth_present(&state, true);
        set_auth_email(&state, Some("user@example.com".into()));

        // None: no session email to preserve — also proves the helper makes
        // no config write when it has nothing to remember.
        #[cfg(not(target_os = "windows"))]
        discard_unusable_startup_session(&state, &acct, None, &mut acct.session_generation());
        #[cfg(target_os = "windows")]
        discard_unusable_startup_session(&state, &acct, None);

        assert!(
            !*state.auth_present.lock().unwrap(),
            "auth_present must be false so the app boots signed out"
        );
        assert!(
            acct.session.lock().unwrap().is_none(),
            "the rejected session must never be installed in memory"
        );
        assert!(
            acct.auth_email.lock().unwrap().is_none(),
            "the mirrored signed-in email must be cleared"
        );
    }

    #[test]
    fn a_startup_401_drops_only_the_token_on_macos_and_linux_and_everything_on_windows() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let production = &source[..source.find("#[cfg(test)]\nmod tests {").unwrap()];
        let non_windows = production
            .find("#[cfg(not(target_os = \"windows\"))]\nfn discard_unusable_startup_session(")
            .expect("a non-Windows discard");
        let body = &production[non_windows..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(
            body.contains("clear_keychain_session_token("),
            "R8: only the token goes"
        );
        assert!(!body.contains("clear_keychain_session("), "the keys and email stay");
        let windows = production
            .find("#[cfg(target_os = \"windows\")]\nfn discard_unusable_startup_session(")
            .expect("Windows keeps its discard");
        let body = &production[windows..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(
            body.contains("clear_keychain_session(acct.id.as_str())"),
            "Windows is unchanged"
        );
    }

    #[tokio::test]
    async fn an_authorized_probe_returns_the_account_profile() {
        let server = AuthMeMockServer::start_with_body(
            "200 OK",
            r#"{"user_id":"u-1","email":"sam@beebeeb.io","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}"#,
        );
        match probe_startup_session(&server.base_url, "stored-token").await {
            StartupSessionCheck::Authorized { profile: Some(profile) } => assert_eq!(profile.user_id, "u-1"),
            other => panic!("expected an authorized probe with a profile, got {other:?}"),
        }
        let unparseable = AuthMeMockServer::start_with_body("200 OK", "{}");
        assert_eq!(
            probe_startup_session(&unparseable.base_url, "stored-token").await,
            StartupSessionCheck::Authorized { profile: None },
            "an unreadable body still means authorized; only the profile is missing"
        );
    }

    /// "Already signed out" can still hold the vault key and the email (R9). Both sign-out branches verify that nothing
    /// of the account is left before they report success.
    #[test]
    fn a_switch_after_a_revoked_token_leaves_no_vault_key_behind() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let production = &source[..source.find("#[cfg(test)]\nmod tests {").unwrap()];
        let clear = &production[production.find("async fn clear_session_impl(").unwrap()..];
        let clear = &clear[..clear.find("\n}\n").unwrap()];
        let already = &clear[clear
            .find("if already_signed_out {\n            // Already signed out")
            .expect("the already-signed-out branch")..];
        let already = &already[..already.find("outcome: SignOutOutcome::NotSignedIn,").unwrap()];
        // Task 12 fix round 1, item 2: the Keychain clear and its check run in the sign-out's last turn on macOS/Linux.
        assert!(
            already.contains("finish_sign_out_turn(&acct, &engine_slot, true, &mut dropped).map_err(&kept_with)?;"),
            "the already-signed-out branch verifies"
        );
        let tail = &clear[clear.rfind("if already_signed_out {").unwrap()..];
        let tail = &tail[tail.find("outcome: SignOutOutcome::NotSignedIn,").unwrap()..];
        assert!(
            tail.contains("finish_sign_out_turn(&acct, &engine_slot, false, &mut dropped).map_err(&kept_with)?;"),
            "the signed-in branch verifies"
        );
        let last = &production[production.find("fn finish_sign_out_turn(").unwrap()..];
        let last = &last[..last.find("\n}\n").unwrap()];
        let cleared = last
            .find("clear_keychain_session_holding(&write, acct.id.as_str(), dropped)")
            .expect("the Keychain clear");
        let checked = last
            .find("ensure_keychain_holds_no_account(acct.id.as_str())")
            .expect("the check");
        let moved = last
            .find("acct.advance_session_generation_holding_session_write_lock(account::SessionTransition::SignOut);")
            .expect("the move");
        let released = last.find("drop(write);").expect("the turn ends");
        assert!(
            cleared < checked && checked < moved && moved < released,
            "clear, check, then move the generation, then release:\n{last}"
        );
    }

    /// Spec §5.6: the server's answer to the key proof. A `400` decides only with one of two codes, each matched
    /// EXACTLY: `invalid_recovery_phrase` is a mismatch, `recovery_check_missing` is "no check on file". Any other
    /// `400` (another code, a near miss, a longer code, no code, no JSON, a body the server's extractor rejected, an
    /// intermediary's page) says nothing about the key: it is an error, like a `5xx`, never a mismatch. The
    /// recovery-phrase unlock reads "no check on file" as "not this account's", as before, and any other `400` as "could
    /// not verify".
    #[tokio::test]
    async fn the_key_proof_decides_only_on_the_two_exact_codes_and_any_other_400_cannot_verify() {
        let key = beebeeb_core::kdf::MasterKey::from_bytes([6u8; 32]);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap();
        let answer = |status, body| {
            let server = AuthMeMockServer::start_with_body(status, body);
            let client = client.clone();
            let key = &key;
            async move { super::recovery_check_answer(&client, &server.base_url, "tok", key).await }
        };
        use super::RecoveryCheckAnswer::{Matches, Mismatch, NoCheckOnFile};
        let answers: [(&'static str, &'static str, super::RecoveryCheckAnswer); 3] = [
            (
                "400 Bad Request",
                r#"{"error":"recovery_check_missing"}"#,
                NoCheckOnFile,
            ),
            ("400 Bad Request", r#"{"error":"invalid_recovery_phrase"}"#, Mismatch),
            ("200 OK", r#"{"valid":true}"#, Matches),
        ];
        for (status, body, expected) in answers {
            assert_eq!(answer(status, body).await, Ok(expected), "{status} {body}");
        }
        let not_an_answer: [&'static str; 11] = [
            r#"{"error":"recovery_check_missing_or_other"}"#,
            r#"{"error":"Recovery_Check_Missing"}"#,
            r#"{"error":"Invalid_Recovery_Phrase"}"#,
            r#"{"error":"invalid_recovery_phrase "}"#,
            r#"{"error":"bad_request"}"#,
            r#"{"error":42}"#,
            r#"{}"#,
            "not json",
            "",
            "Failed to deserialize the JSON body into the target type: missing field `recovery_check`",
            "<html><body><h1>400 Bad Request</h1></body></html>",
        ];
        for body in not_an_answer {
            assert!(
                answer("400 Bad Request", body).await.is_err(),
                "a 400 without one of the two codes cannot verify, never a mismatch: {body:?}"
            );
        }
        let missing = AuthMeMockServer::start_with_body("400 Bad Request", r#"{"error":"recovery_check_missing"}"#);
        assert_eq!(
            super::recovered_key_matches_account(&client, &missing.base_url, "tok", &key).await,
            Ok(false),
            "the unlock: no check on file proves nothing, so the phrase is not accepted"
        );
        let mismatch = AuthMeMockServer::start_with_body("400 Bad Request", r#"{"error":"invalid_recovery_phrase"}"#);
        assert_eq!(
            super::recovered_key_matches_account(&client, &mismatch.base_url, "tok", &key).await,
            Ok(false),
            "the unlock: a key that is not the account's is not accepted"
        );
        let other = AuthMeMockServer::start_with_body("400 Bad Request", "not json");
        assert!(
            super::recovered_key_matches_account(&client, &other.base_url, "tok", &key)
                .await
                .is_err(),
            "the unlock: any other 400 cannot verify, so it is not called a wrong phrase"
        );
    }

    /// The production part of `lib.rs` (the test modules start at `mod tests`).
    fn production() -> String {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        source[..source.find("#[cfg(test)]\nmod tests {").unwrap()].to_string()
    }

    /// The body of the function whose signature starts with `signature`.
    fn body_of(production: &str, signature: &str) -> String {
        let at = production
            .find(signature)
            .unwrap_or_else(|| panic!("{signature} exists"));
        let body = &production[at..];
        body[..body.find("\n}\n").unwrap()].to_string()
    }

    /// Fix round 1, item 3 (review M1): the 2FA challenge store's generation check is macOS and Linux only (Windows
    /// orders that step with its transition lock and attempt fence, as before), and it comes before the challenge is
    /// stored.
    #[test]
    fn the_2fa_challenge_check_is_macos_and_linux_only_and_comes_before_the_store() {
        let login = body_of(&production(), "async fn desktop_login(");
        let check = login.find("if !acct.session_unchanged_since(turn) {\n                    return Err(SessionChanged::of(&acct).into());").expect("the check");
        let block = login[..check]
            .rfind("#[cfg(not(target_os = \"windows\"))]\n            {")
            .expect("a macOS/Linux block");
        assert!(
            login[block..check].lines().count() <= 4,
            "the check sits in that block:\n{}",
            &login[block..check]
        );
        assert!(
            check < login.find("*guard = Some(Pending2fa {").expect("the store"),
            "checked before it is stored"
        );
    }

    /// Fix round 1, item 6 (review M5): the `account_profile` command's cache is written only while no session
    /// transition happened since its fetch began, and the command captures before it fetches. That turn check is
    /// macOS and Linux only (`check_session_turn`): on Windows a Lock or a Sign-out drains every `session_command!`
    /// (`close_session_commands`) before it clears the cached profile, so a fetch in flight finishes first (pinned by
    /// `every_credential_command_has_admission_before_credential_reads` in `tests/windows_session_wiring.rs`).
    #[test]
    fn the_account_profile_cache_is_written_only_in_its_turn() {
        let state = AppState::default();
        synthesize_single_account(&state, AccountId("t12f1-profile-cache".to_string()));
        let acct = state.active_account().unwrap();
        let profile: crate::account_dto::AccountProfile = serde_json::from_str(
            r#"{"user_id":"u-1","email":"sam@beebeeb.io","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        #[cfg(not(target_os = "windows"))]
        {
            let turn = acct.session_generation();
            drop(super::end_sessions_in_flight(
                &acct,
                crate::account::SessionTransition::SignOut,
            ));
            assert!(
                !super::cache_profile_in_turn(&acct, &turn, profile.clone()),
                "a sign-out landed while the fetch ran"
            );
            assert!(
                acct.cached_profile.lock().unwrap().is_none(),
                "nothing is cached for the old session"
            );
        }
        let turn = acct.session_generation();
        assert!(super::cache_profile_in_turn(&acct, &turn, profile.clone()));
        assert_eq!(
            acct.cached_profile
                .lock()
                .unwrap()
                .as_ref()
                .map(|p| p.user_id.clone())
                .as_deref(),
            Some("u-1")
        );
        assert!(acct.session_unchanged_since(turn), "caching moves no generation");
        let command = body_of(&production(), "async fn account_profile(");
        let captured = command.find("let turn = acct.session_generation();").expect("captured");
        assert!(
            captured < command.find("api.account_profile().await").unwrap(),
            "before the fetch:\n{command}"
        );
        assert!(
            command.contains("cache_profile_in_turn(&acct, &turn, profile.clone());"),
            "{command}"
        );
        assert!(
            !command.contains("acct.cached_profile.lock() {"),
            "no unchecked write:\n{command}"
        );
    }

    /// PR #140 delta review, P2-B: Settings › Security says whether the authenticator is on from `account_profile_fresh`,
    /// which asks the server every time. A read of the session's cache first would be `account_profile` again, and say On
    /// for hours after the authenticator was turned off on the web. And (follow-up P3-6) it only reads: a profile cached
    /// after an email change made on the web carries an address that is not the session's, and `identity_of_session` would
    /// drop the user id the session knows.
    #[test]
    fn the_fresh_profile_asks_the_server_every_time_and_touches_neither_the_cache_nor_the_session() {
        let production = production();
        let command = body_of(&production, "async fn account_profile_fresh(");
        assert!(command.contains("api.account_profile().await"), "it fetches:\n{command}");
        for touches in ["cached_profile", ".as_ref()", "return Ok(", "cache_profile_in_turn", "session_generation", ".lock()", "acct"] {
            assert!(!command.contains(touches), "{touches}: it must read the server and nothing else:\n{command}");
        }
        // And Security is the one that asks it: its page does not read the cached command.
        let security = include_str!("../../src/MacSecurity.tsx").replace("\r\n", "\n");
        assert!(security.contains("accountProfileFresh()"), "Security reads the fresh profile");
        assert!(!security.contains("accountProfile()"), "and never the cached one");
    }

    /// Fix round 1, item 8 (review M7): every refusal says what fits its path, each in its own sentence.
    #[test]
    fn every_refusal_names_its_path_in_its_own_sentence() {
        use super::{
            KEYCHAIN_UNLOCK_INTERRUPTED, RECOVERY_UNLOCK_LOCKED_MEANWHILE, SESSION_CHANGED_WHILE_WAITING,
            SIGN_IN_LOCKED_MEANWHILE, SessionChanged, UNLOCK_ACCOUNT_CHANGED,
        };
        assert_eq!(
            SESSION_CHANGED_WHILE_WAITING,
            "Something changed on this computer while Beebeeb was working; nothing was saved. Try again."
        );
        assert_eq!(
            SIGN_IN_LOCKED_MEANWHILE,
            "Beebeeb was locked while signing in. Sign in again."
        );
        assert_eq!(
            RECOVERY_UNLOCK_LOCKED_MEANWHILE,
            "Beebeeb was locked while the recovery phrase was being checked, so nothing was saved. Enter the phrase once more."
        );
        assert_eq!(
            KEYCHAIN_UNLOCK_INTERRUPTED,
            "Beebeeb was signed out or locked while unlocking. Try again."
        );
        assert_eq!(
            UNLOCK_ACCOUNT_CHANGED,
            "You were signed out, or another account signed in, while Beebeeb was unlocking. Try again."
        );
        let (other, lock) = (SessionChanged { by_lock: false }, SessionChanged { by_lock: true });
        assert_eq!(
            (other.sign_in_sentence(), lock.sign_in_sentence()),
            (SESSION_CHANGED_WHILE_WAITING, SIGN_IN_LOCKED_MEANWHILE)
        );
        assert_eq!(
            (other.recovery_unlock_sentence(), lock.recovery_unlock_sentence()),
            (UNLOCK_ACCOUNT_CHANGED, RECOVERY_UNLOCK_LOCKED_MEANWHILE)
        );
        assert_eq!(
            (other.keychain_unlock_sentence(), lock.keychain_unlock_sentence()),
            (KEYCHAIN_UNLOCK_INTERRUPTED, KEYCHAIN_UNLOCK_INTERRUPTED)
        );
        assert_eq!(
            String::from(lock),
            SIGN_IN_LOCKED_MEANWHILE,
            "a sign-in's refusal by default"
        );
        let all = [
            SESSION_CHANGED_WHILE_WAITING,
            SIGN_IN_LOCKED_MEANWHILE,
            RECOVERY_UNLOCK_LOCKED_MEANWHILE,
            KEYCHAIN_UNLOCK_INTERRUPTED,
            UNLOCK_ACCOUNT_CHANGED,
        ];
        assert_eq!(
            all.iter().collect::<std::collections::HashSet<_>>().len(),
            all.len(),
            "one distinct sentence per path"
        );
        // What moved the generation last decides.
        let state = AppState::default();
        synthesize_single_account(&state, AccountId("t12f1-what-moved".to_string()));
        let acct = state.active_account().unwrap();
        for (by, by_lock) in [
            (crate::account::SessionTransition::Lock, true),
            (crate::account::SessionTransition::SignOut, false),
            (crate::account::SessionTransition::Write, false),
        ] {
            drop(super::end_sessions_in_flight(&acct, by));
            assert_eq!(SessionChanged::of(&acct), SessionChanged { by_lock }, "{by:?}");
        }
    }

    /// Fix round 1, item 9 (review M9): each command reaches the seam its behaviour is tested at, and the window focus and
    /// "Try again" ask an unidentified session again (the report of round 0 claimed these pins; now they exist).
    #[test]
    fn every_command_reaches_its_tested_seam() {
        let production = production();
        for (command, seam) in [
            (
                "async fn unlock_vault(",
                "install_keychain_session(&state, &acct, &mut turn, session)?;",
            ),
            (
                "async fn desktop_unlock_with_recovery_phrase(",
                "install_recovered_session(&state, &acct, &mut turn, &token, email, &verified_key)?;",
            ),
            (
                "pub(crate) async fn apply_session(",
                "install_new_session_or_revoke(state, &acct, turn, &token, master_key, email, profile, &runner::api_base_url()).await?;",
            ),
            (
                "async fn settle_sign_in(",
                "decide_sign_in_in_turn(state, &acct, &sources, profile, token, &runner::api_base_url(), turn).await?",
            ),
            (
                "async fn restore_session_inner(",
                "if !restore_stored_session(&state, &acct, &runner::api_base_url()).await {",
            ),
            (
                "async fn account_profile(",
                "cache_profile_in_turn(&acct, &turn, profile.clone());",
            ),
            (
                "async fn on_app_activated(",
                "identify_then_start(app, IdentifyPace::Debounced).await;",
            ),
            (
                "async fn finder_setup_retry(",
                "finder_setup_try_again(&state, &runner::api_base_url()).await",
            ),
            // Fix round 2, item 2: a naming in the background starts only an empty slot.
            (
                "async fn identify_then_start(",
                "start_engine_unless_running(app.clone(), &state).await",
            ),
            (
                "async fn start_engine_unless_running(",
                "keep_or_rebind_after_naming(&acct, &mut engine_slot, owner.as_ref(), &named, |slot| {",
            ),
            (
                "async fn keep_or_rebind_after_naming<",
                "keep_or_start_after_naming(engine_slot, start)",
            ),
        ] {
            // Read through `squeeze`: rustfmt wraps the longer of these calls over lines.
            assert!(
                crate::source_pin::squeeze(&body_of(&production, command)).contains(&crate::source_pin::squeeze(seam)),
                "{command} reaches {seam}"
            );
        }
        let try_again = body_of(&production, "async fn finder_setup_try_again(");
        let only_without_an_email = try_again
            .find("if session_in_memory_has_no_email(state) {")
            .expect("Try again asks only for a session with no email (area A M8)");
        let asked = try_again
            .find("identify_unidentified_session(state, base_url, std::time::Instant::now(), IdentifyPace::Now).await;")
            .expect("Try again asks first, not debounced");
        assert!(
            only_without_an_email < asked
                && asked < try_again.find("finder_setup_retry_impl(state)").expect("then retries"),
            "{try_again}"
        );
        let then_start = crate::source_pin::squeeze(&body_of(&production, "async fn identify_then_start("));
        let started = then_start
            .find(&crate::source_pin::squeeze(
                "start_engine_unless_running(app.clone(), &state).await",
            ))
            .expect("a naming starts the engine");
        let notified = then_start
            .find(&crate::source_pin::squeeze(
                "notify_finder(&state, finder_setup::core::Trigger::KeysArrived);",
            ))
            .expect("and tells the reconciler keys are here (area A M7)");
        assert!(
            then_start.find("ifnamed{").is_some_and(|named| named < started) && started < notified,
            "{then_start}"
        );
        let activation = body_of(&production, "async fn identify_on_app_activation(");
        assert!(
            activation.contains("identify_unidentified_session(state, base_url, now, IdentifyPace::Debounced).await"),
            "{activation}"
        );
        let then_start = body_of(&production, "async fn identify_then_start(");
        assert!(
            crate::source_pin::squeeze(&then_start).contains(&crate::source_pin::squeeze(
                "IdentifyPace::Debounced => { identify_on_app_activation(&state, &runner::api_base_url(), std::time::Instant::now()).await }"
            )),
            "{then_start}"
        );
        let run = body_of(&production, "pub fn run(");
        let focus = run.find("#[cfg(not(target_os = \"windows\"))]\n            if let tauri::WindowEvent::Focused(true) = event {\n                tauri::async_runtime::spawn(on_app_activated(window.app_handle().clone()));").expect("the window focus asks again");
        assert!(
            focus > run.find("handle.app_activated();").unwrap(),
            "after the reconciler's own activation check"
        );
    }

    /// Fix round 2, item 6: the `account_profile` cache's doc says what is true: the cached profile IS a sign-in input.
    #[test]
    fn the_profile_cache_doc_says_the_profile_is_a_sign_in_input() {
        let production = production();
        let at = production.find("fn cache_profile_in_turn(").expect("the cache");
        let doc = &production[production[..at]
            .rfind("/// The `account_profile` command's cache")
            .expect("its doc")..at];
        assert!(
            doc.contains("The cached profile IS an input to a\n/// sign-in's decision"),
            "{doc}"
        );
        assert!(
            !doc.contains("changes\n/// nothing a sign-in decides on"),
            "the old claim is gone:\n{doc}"
        );
    }

    /// Fix round 1, item 10 (P3-C, part): the two older copies of the key and of its check are wiped.
    #[test]
    fn the_older_key_and_check_copies_are_wiped() {
        let production = production();
        let load = body_of(&production, "fn load_session_from_vault<");
        assert!(
            load.contains("let mut master_key = zeroize::Zeroizing::new([0u8; 32]);"),
            "{load}"
        );
        assert!(
            load.contains("master_key: *master_key,"),
            "the session's copy is the only one that outlives it:\n{load}"
        );
        let check = body_of(&production, "async fn recovery_check_answer(");
        assert!(
            check.contains("let recovery_check = zeroize::Zeroizing::new(encode_base64("),
            "{check}"
        );
        assert!(
            crate::source_pin::squeeze(&check).contains(&crate::source_pin::squeeze(
                ".json(&RecoveryCheckBody { recovery_check: &recovery_check })"
            )),
            "{check}"
        );
        assert!(
            !check.contains("serde_json::json!({ \"recovery_check\""),
            "no second copy in a JSON value:\n{check}"
        );
    }

    /// Lead ruling 13 (Task 12): the Keychain writers borrow the vault key; none takes it by value (a by-value argument
    /// is a copy in a frame nobody wipes).
    #[test]
    fn the_keychain_writers_borrow_the_vault_key() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let production = &source[..source.find("#[cfg(test)]\nmod tests {").unwrap()];
        for signature in [
            "fn persist_vault_key_to_keychain(",
            "fn persist_session_to_keychain(",
            "fn install_new_session(",
            "fn install_recovered_session(",
            "fn store_recovered_vault_key(",
        ] {
            let function = &production[production.find(signature).unwrap_or_else(|| panic!("{signature}"))..];
            let parameters = &function[..function.find(") ->").unwrap()];
            assert!(
                parameters.contains(": &[u8; 32]"),
                "{signature} borrows the key:\n{parameters}"
            );
            assert!(
                !parameters.contains(": [u8; 32]"),
                "{signature} takes no key by value:\n{parameters}"
            );
        }
        assert!(
            !production.contains("SecretBytes::new_master_key(*"),
            "no caller copies the key out to build the stored one"
        );
    }

    /// Lead ruling 12 (Task 12): unreadable local account data at sign-in has its own outcome and its own sentence (the
    /// one Lane T renders), never the "connect to the internet" one; nothing is stored, so the new session is revoked.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn unreadable_local_account_data_has_its_own_sentence() {
        assert_eq!(
            super::SIGN_IN_LOCAL_DATA_UNREADABLE,
            "Beebeeb couldn’t read this Mac’s account data. Try again."
        );
        assert_ne!(super::SIGN_IN_LOCAL_DATA_UNREADABLE, super::SIGN_IN_ACCOUNT_UNKNOWN);
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let production = &source[..source.find("#[cfg(test)]\nmod tests {").unwrap()];
        let settle = &production[production.find("async fn settle_sign_in(").unwrap()..];
        let settle = &settle[..settle.find("\n}\n").unwrap()];
        assert!(
            settle.contains("reauth::SignInKind::LocalDataUnreadable => Err(ReauthError::before_store(SIGN_IN_LOCAL_DATA_UNREADABLE)),"),
            "its own sentence, and nothing stored:\n{settle}"
        );
    }
