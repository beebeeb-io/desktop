    use super::finder_setup_command_tests::{body_between, production_source};
    use super::*;

    /// The code of a function without its comment lines.
    fn code_of(production: &str, signature: &str) -> String {
        body_between(production, signature, "\n}\n")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The arm of `text` that contains `needle`, up to the closing brace at the arm's own indentation.
    fn arm<'a>(text: &'a str, needle: &str) -> &'a str {
        let at = text.find(needle).unwrap_or_else(|| panic!("{needle} exists"));
        let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let line = &text[line_start..];
        let indent = " ".repeat(line.len() - line.trim_start().len());
        let rest = &text[at..];
        &rest[..rest
            .find(&format!("\n{indent}}}"))
            .unwrap_or_else(|| panic!("the end of the arm {needle}"))]
    }

    fn browser_source() -> String {
        include_str!("browser_login.rs").replace("\r\n", "\n")
    }

    // ---- the wire contract with the frontend (lead ruling 4) ----

    /// `settledFrom` (the frontend's strict parser, `onboardingSignIn.ts`) reads `requires_2fa`, `reauthenticated`,
    /// `vault_unlocked`, `key_replaced` and `account_mismatch.pending_changes`. Every shape the commands can return always
    /// carries all five fields, never fewer. `key_replaced` is true only for a re-sign-in whose kept key the server no
    /// longer accepted (spec §5.6).
    #[test]
    fn login_outcome_json_is_the_frontends_contract() {
        let json = |outcome: LoginOutcome| serde_json::to_value(&outcome).unwrap();
        let cases = [
            (
                "a plain sign-in",
                LoginOutcome::signed_in(),
                r#"{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":false,"key_replaced":false,"account_mismatch":null}"#,
            ),
            (
                "a second factor is needed",
                LoginOutcome::needs_2fa(),
                r#"{"requires_2fa":true,"reauthenticated":false,"vault_unlocked":false,"key_replaced":false,"account_mismatch":null}"#,
            ),
            (
                "the same account, keys here",
                LoginOutcome::reauthenticated(true, false),
                r#"{"requires_2fa":false,"reauthenticated":true,"vault_unlocked":true,"key_replaced":false,"account_mismatch":null}"#,
            ),
            (
                "the same account, no keys here",
                LoginOutcome::reauthenticated(false, false),
                r#"{"requires_2fa":false,"reauthenticated":true,"vault_unlocked":false,"key_replaced":false,"account_mismatch":null}"#,
            ),
            (
                "the same account, the kept key was no longer the account's",
                LoginOutcome::reauthenticated(false, true),
                r#"{"requires_2fa":false,"reauthenticated":true,"vault_unlocked":false,"key_replaced":true,"account_mismatch":null}"#,
            ),
            (
                "another account",
                LoginOutcome::account_mismatch(3, None, None),
                r#"{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":false,"key_replaced":false,"account_mismatch":{"pending_changes":3}}"#,
            ),
            (
                "another account, nothing waiting",
                LoginOutcome::account_mismatch(0, None, None),
                r#"{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":false,"key_replaced":false,"account_mismatch":{"pending_changes":0}}"#,
            ),
        ];
        for (name, outcome, expected) in cases {
            let value = json(outcome);
            assert_eq!(
                value,
                serde_json::from_str::<serde_json::Value>(expected).unwrap(),
                "{name}"
            );
            let keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
            assert_eq!(keys.len(), 5, "{name}: all five fields are always sent: {keys:?}");
            // The types the parser insists on: four flags that are booleans, a null or an object with a whole count.
            assert!(
                value["requires_2fa"].is_boolean()
                    && value["reauthenticated"].is_boolean()
                    && value["vault_unlocked"].is_boolean()
                    && value["key_replaced"].is_boolean(),
                "{name}"
            );
            assert!(
                value["account_mismatch"].is_null() || value["account_mismatch"]["pending_changes"].is_u64(),
                "{name}"
            );
        }
    }

    /// Both password steps return the outcome (the second one returned nothing before R8), `LoginOutcome` never skips
    /// a field, and the window command the frontend opens exists and starts sign-in in place.
    #[test]
    fn the_commands_and_the_window_the_frontend_calls_exist() {
        let production = production_source();
        for signature in ["async fn desktop_login(", "async fn desktop_login_2fa("] {
            let head = body_between(&production, signature, "{\n");
            assert!(head.contains("-> Result<LoginOutcome, String>"), "{signature}:\n{head}");
        }
        let outcome = body_between(&production, "struct LoginOutcome {", "\n}\n");
        assert!(
            !outcome.contains("skip_serializing_if") && !outcome.contains("skip_serializing"),
            "{outcome}"
        );
        let handler = body_between(&production, "tauri::generate_handler![", "])");
        assert!(
            handler.contains("open_reauth_window,") && handler.contains("open_onboarding_window,"),
            "registered next to the other window command"
        );
        let window = body_between(&production, "async fn open_reauth_window(", "\n}\n");
        assert!(
            window.contains(r#"const URL: &str = "index.html?window=onboarding&mode=reauth";"#),
            "{window}"
        );
        assert!(
            window.contains("window.location.search = '?window=onboarding&mode=reauth'"),
            "an open window is reloaded into the same mode:\n{window}"
        );
        let windows_arm = arm(window, "#[cfg(target_os = \"windows\")]\n    {");
        assert!(
            windows_arm.contains("Err(") && !windows_arm.contains("WebviewWindowBuilder"),
            "Windows keeps its own flow:\n{windows_arm}"
        );
        // Nothing is cleared first: the window command touches no session state.
        for forbidden in ["clear_", "session", "keychain", "purge"] {
            assert!(
                !window.contains(forbidden),
                "open_reauth_window must not touch {forbidden}:\n{window}"
            );
        }
    }

    // ---- wiring: every sign-in method asks, before anything is stored; Windows keeps its refusal (lead ruling 10) ----

    #[test]
    fn every_sign_in_method_is_checked_before_it_stores_anything_and_windows_never_reaches_the_check() {
        let production = production_source();
        for signature in ["async fn desktop_login(", "async fn desktop_login_2fa("] {
            let body = body_between(&production, signature, "\n}\n");
            let check = body
                .find("settle_sign_in(")
                .unwrap_or_else(|| panic!("{signature} checks the account"));
            let store = body.find("store_first_sign_in(").expect("then stores the token");
            assert!(check < store, "{signature}: the check comes before anything is stored");
            let cfg_at = body[..check]
                .rfind("#[cfg(not(target_os = \"windows\"))]")
                .expect("a not-Windows cfg above the check");
            assert!(
                check - cfg_at < 120,
                "{signature}: the check sits directly under a not-Windows cfg"
            );
            assert!(
                body.contains("Sign out of the current account before signing in again."),
                "{signature}: Windows keeps its refusal"
            );
        }
        let browser = browser_source();
        let check = browser
            .find("crate::settle_sign_in(")
            .expect("the browser handoff checks the account");
        assert!(check < browser.find("crate::apply_session(").expect("then applies the session"));
        assert!(
            browser[..check]
                .rfind("#[cfg(not(target_os = \"windows\"))]")
                .is_some_and(|cfg_at| check - cfg_at < 1200),
            "the browser handoff's check is compiled out on Windows"
        );
        assert_eq!(
            production
                .matches("Sign out of the current account before signing in again.")
                .count(),
            4,
            "Windows keeps all four refusals"
        );
        for signature in [
            "async fn settle_sign_in(",
            "async fn reauth_in_place(",
            "fn reauth_swap_token(",
            "fn classify_sign_in(",
            "async fn decide_sign_in(",
            "async fn prove_stored_key(",
            "fn load_stored_vault_key(",
            "fn gather_local_facts(",
            "fn note_engine_start_failure(",
            "fn keychain_session_trace(",
            "fn keychain_email_trace(",
            "fn unowned_local_data(",
            "fn credentials_identity(",
        ] {
            let at = production.find(signature).unwrap_or_else(|| panic!("{signature}"));
            let above = production[..at].lines().rev().take(8).collect::<Vec<_>>().join("\n");
            assert!(
                above.contains("#[cfg(not(target_os = \"windows\"))]"),
                "{signature} does not exist on Windows:\n{above}"
            );
        }
    }

    /// A mismatch, an unconfirmed account and a failed re-sign-in change nothing local: each arm only revokes the
    /// session just minted (a failed one only before its Keychain write succeeded: after it, that token is the live
    /// one) and tells the frontend. In all three sign-in methods. (The behaviour is `a_mismatch_changes_nothing…`.)
    #[test]
    fn every_arm_that_does_not_sign_in_revokes_the_new_session_and_changes_nothing() {
        let production = production_source();
        let settle = code_of(&production, "async fn settle_sign_in(");
        assert!(
            settle.contains(concat!(
                "reauth::SignInKind::DifferentAccount => ",
                "Ok(switch_settlement(pending_changes, switch_snapshot(state))),"
            )),
            "{settle}"
        );
        assert!(settle.contains("reauth::SignInKind::NeedsKeyProof | reauth::SignInKind::Unconfirmed => Ok(SignInSettlement::Unconfirmed),"), "{settle}");
        let texts = [
            ("desktop_login", code_of(&production, "async fn desktop_login(")),
            ("desktop_login_2fa", code_of(&production, "async fn desktop_login_2fa(")),
            ("the browser handoff", browser_source()),
        ];
        // The switch's pattern names three fields, which rustfmt lays over several lines, so its arm is read from the
        // `=> {` that ends the pattern.
        fn switch_arm(text: &str) -> &str {
            let at = text
                .find("SignInSettlement::AccountMismatch {")
                .expect("the switch arm");
            arm(&text[at..], "=> {")
        }
        for (name, text) in &texts {
            for (needle, arm) in [
                ("SignInSettlement::AccountMismatch", switch_arm(text)),
                (
                    "SignInSettlement::Unconfirmed",
                    arm(text, "SignInSettlement::Unconfirmed"),
                ),
            ] {
                for forbidden in [
                    "persist_",
                    "record_local_data_owner",
                    "set_auth_",
                    "clear_",
                    "acct.session",
                    "cached_profile",
                    "apply_session",
                    "keychain",
                ] {
                    assert!(
                        !arm.contains(forbidden),
                        "{name}: the {needle} arm must not touch {forbidden}:\n{arm}"
                    );
                }
                assert!(
                    arm.contains("revoke_desktop_session("),
                    "{name}: the {needle} arm revokes the session just minted:\n{arm}"
                );
            }
            let unconfirmed = arm(text, "SignInSettlement::Unconfirmed");
            assert!(
                unconfirmed.contains("SIGN_IN_ACCOUNT_UNKNOWN"),
                "{name}: the retryable sentence:\n{unconfirmed}"
            );
            let failure = arm(text, "Err(failure) =>");
            let revoke = failure
                .find("revoke_desktop_session(")
                .unwrap_or_else(|| panic!("{name}: a failure before the Keychain write revokes:\n{failure}"));
            assert!(
                failure[..revoke].contains("if !failure.token_stored"),
                "{name}: only while its token is not stored:\n{failure}"
            );
            assert!(failure.contains("return Err(failure.message)"), "{name}:\n{failure}");
        }
        // The browser handoff also refuses a sign-in it could not identify on a Mac that holds an account: it revokes the
        // session and says so, and (like every arm above) changes nothing.
        let unidentified = arm(&texts[2].1, "None if crate::sign_in_leaves_account_traces(state) =>");
        assert!(
            unidentified.contains("revoke_desktop_session(") && unidentified.contains("SIGN_IN_ACCOUNT_UNKNOWN"),
            "{unidentified}"
        );
        for forbidden in [
            "persist_",
            "record_local_data_owner",
            "set_auth_",
            "clear_",
            "apply_session",
            "keychain",
        ] {
            assert!(
                !unidentified.contains(forbidden),
                "the unidentified arm must not touch {forbidden}:\n{unidentified}"
            );
        }
        // The two password methods return what the re-sign-in became, `key_replaced` included (spec §5.6), and the
        // second one drops the held second-factor challenge on every way out that spends it.
        for (name, text) in &texts[..2] {
            let at = text
                .find("SignInSettlement::Reauthenticated {")
                .unwrap_or_else(|| panic!("{name}: the re-sign-in arm"));
            let end = at
                + text[at..]
                    .find("SignInSettlement::AccountMismatch")
                    .expect("the next arm");
            let reauthenticated = &text[at..end];
            assert!(
                crate::source_pin::squeeze(reauthenticated).contains(&crate::source_pin::squeeze(
                    "return Ok(LoginOutcome::reauthenticated(vault_unlocked, key_replaced));"
                )),
                "{name}:\n{reauthenticated}"
            );
            if *name == "desktop_login_2fa" {
                assert!(
                    reauthenticated.contains("state.pending_2fa.lock()"),
                    "the spent challenge is dropped on a re-sign-in:\n{reauthenticated}"
                );
            }
        }
        let second_factor = &texts[1].1;
        for (needle, arm) in [
            ("SignInSettlement::AccountMismatch", switch_arm(second_factor)),
            (
                "SignInSettlement::Unconfirmed",
                arm(second_factor, "SignInSettlement::Unconfirmed"),
            ),
            ("Err(failure) =>", arm(second_factor, "Err(failure) =>")),
        ] {
            assert!(
                arm.contains("state.pending_2fa.lock()"),
                "the spent challenge is dropped on {needle}"
            );
        }
    }

    /// R8: "Finder, keys, cache and pending edits all stay". The re-sign-in path never purges, never removes the
    /// domain, never writes or clears keys, never signs out, and never creates an engine itself.
    #[test]
    fn a_re_sign_in_never_purges_removes_or_touches_keys() {
        let production = production_source();
        for signature in [
            "async fn settle_sign_in(",
            "async fn reauth_in_place(",
            "fn reauth_swap_token(",
            "fn replace_session_token_in_memory(",
            "fn name_session_in_memory(",
            "fn reauth_settle_flags(",
            "fn classify_sign_in(",
            "async fn decide_sign_in(",
            "fn gather_local_facts(",
            "fn unowned_local_data(",
            "fn credentials_identity(",
            "fn note_engine_start_failure(",
        ] {
            let body = code_of(&production, signature);
            for forbidden in [
                "purge_local_state_files",
                "purge_all_local_state",
                "purge_local_data_for_sign_out",
                "clear_account_data",
                "clear_keychain_session",
                "clear_session_impl",
                "remove_file_provider_domain",
                "finder_remove_for",
                "Trigger::SignOut",
                "Trigger::Repair",
                "persist_vault_key_to_keychain",
                "persist_session_to_keychain",
                "store_wrapped_master_key",
                "store_recovered_vault_key",
                "master_key =",
                "EngineRunner::",
                "spawn_bound_engine",
            ] {
                assert!(!body.contains(forbidden), "{signature} must not use {forbidden}");
            }
        }
        // The token a re-sign-in replaces is wiped, not only dropped.
        assert!(
            code_of(&production, "fn replace_session_token_in_memory(")
                .contains("zeroize::Zeroize::zeroize(&mut replaced);"),
            "the replaced token is wiped"
        );
        // The decision reads no key material: it only asks whether a vault key exists (the one server proof reads the
        // key inside `prove_stored_key`, which is pinned below).
        for signature in [
            "async fn settle_sign_in(",
            "fn classify_sign_in(",
            "async fn decide_sign_in(",
            "fn gather_local_facts(",
            "fn unowned_local_data(",
            "fn credentials_identity(",
        ] {
            let body = code_of(&production, signature);
            for forbidden in [
                "master_key",
                "load_session_from_keychain",
                ".unlock(",
                "AuthVault",
                "load_session_token_from_keychain",
                "load_stored_vault_key",
            ] {
                assert!(
                    !body.contains(forbidden),
                    "{signature} reads key material ({forbidden})"
                );
            }
        }
        // A re-sign-in starts an engine only through the one start function, which only ever reaches the gate.
        let in_place = code_of(&production, "async fn reauth_in_place(");
        assert_eq!(in_place.matches("start_engine_if_possible(").count(), 1);
        // An engine that does not start does not fail the sign-in (nothing in this function returns its error), so a
        // retry never mints a second server session for an account that is already signed in.
        assert!(
            !in_place.contains("started?") && !in_place.contains("return Err("),
            "the engine's error is not the sign-in's:\n{in_place}"
        );
        assert!(
            in_place.contains("note_engine_start_failure(acct, &error)"),
            "{in_place}"
        );
        assert!(in_place.contains("-> Result<bool, ReauthError>"), "{in_place}");
    }

    /// Lead ruling F9: on a Mac the operations paused for `auth` are resumed in the one engine start and nowhere else
    /// in this file (spec D11, task 1888: off macOS the engine resumes them after its reconcile pass). The start reads
    /// the ask before it reads the keys (so an ask is served only by a start that holds the session of the sign-in that
    /// asked, or a later one), and resumes after the binding and before the engine exists. Exactly four writers ask, in
    /// their turn and after their session is in memory: the re-sign-in in place (only when unlocked), the browser
    /// handoff's install, the recovery-phrase unlock and (F9 review I-2) the startup restore whose probe confirmed the
    /// stored token. The key-replaced re-sign-in and the Keychain unlock after a Lock never ask. After a key
    /// replacement (F9 review I-1, M2) the `auth` pauses are held as `key_replaced`, under the engine slot, before the
    /// kept key is removed or the slot is released. Compared with `squeeze`.
    #[test]
    fn the_queue_resumes_in_the_one_engine_start_after_a_sign_in_put_its_session_in_memory() {
        use crate::source_pin::squeeze;
        let production = production_source();
        let bound = squeeze(&code_of(&production, "fn start_engine_bound("));
        let at = |snippet: &str| {
            bound
                .find(&squeeze(snippet))
                .unwrap_or_else(|| panic!("{snippet}:\n{bound}"))
        };
        let asked = at("let resume_asked = acct.auth_resume_asked.load(Ordering::SeqCst);");
        let keys = at("authorize_engine_start(");
        let bound_or_returned = at("StartPermit::FinderRemovalOwed => return Ok(EngineStart::FinderRemovalOwed)");
        let resumed = at("resume_operations_paused_for_auth(acct, paths, resume_asked);");
        let spawned = at("spawn(paths.sync_root.clone()");
        assert!(
            asked < keys && keys < bound_or_returned && bound_or_returned < resumed && resumed < spawned,
            "the ask before the keys; the resume after the binding and before the engine:\n{bound}"
        );
        let squeezed = squeeze(&production);
        assert_eq!(
            squeezed.matches("resume_operations_paused_for_auth(").count(),
            3,
            "its definition, the start's one call, and the state database call inside it"
        );
        let resume = squeeze(&code_of(&production, "fn resume_operations_paused_for_auth("));
        assert!(
            resume.contains(&squeeze("if asked <= acct.auth_resume_done.load(Ordering::SeqCst) {"))
                && resume.contains(&squeeze("acct.auth_resume_done.store(asked, Ordering::SeqCst);")),
            "an ask is served once:\n{resume}"
        );

        let ask = squeeze("ask_to_resume_auth_paused(&write, acct);");
        // F9 re-review N-1: the re-sign-in in place decides "this key replaces the kept one" before it asks.
        let asks_when_unlocked = squeeze(
            "if vault_unlocked && !handoff_replaces_kept_key(acct, handoff_key) { \
             ask_to_resume_auth_paused(&write, acct); }",
        );
        assert_eq!(
            squeezed.matches(&ask).count(),
            4,
            "three sign-in writers and the startup check ask"
        );
        for (signature, in_memory) in [
            ("fn install_new_session(", "*guard = Some(Session {"),
            ("fn install_recovered_session(", "*guard = Some(Session {"),
            (
                "fn reauth_swap_token(",
                "install_unlocked_session(&write, acct, session)",
            ),
        ] {
            let body = squeeze(&code_of(&production, signature));
            let installed = body.find(&squeeze(in_memory)).expect(signature);
            let asks = body.find(&ask).unwrap_or_else(|| panic!("{signature} asks"));
            let released = body
                .rfind("drop(write)")
                .unwrap_or_else(|| panic!("{signature} ends its turn"));
            assert!(
                installed < asks && asks < released,
                "{signature}: in its turn, after the session is in memory:\n{body}"
            );
        }
        assert!(
            squeeze(&code_of(&production, "fn reauth_swap_token(")).contains(&asks_when_unlocked),
            "asks only when unlocked, and not when its handed-over key replaces the kept one"
        );
        for signature in [
            "fn reauth_swap_token_without_key(",
            "async fn reauth_without_the_kept_key(",
            "fn install_keychain_session(",
            "async fn unlock_vault(",
        ] {
            assert!(
                !code_of(&production, signature).contains("ask_to_resume_auth_paused"),
                "{signature} is not a sign-in that unlocks"
            );
        }
        // F9 review I-2: the startup restore asks in its own turn, after the session is in memory, and only when the
        // probe confirmed the token (not when it could not tell).
        let restore = squeeze(&code_of(&production, "async fn restore_stored_session("));
        let at_restore = |snippet: &str| {
            restore
                .find(&squeeze(snippet))
                .unwrap_or_else(|| panic!("{snippet}:\n{restore}"))
        };
        let confirmed = at_restore("let token_confirmed = matches!(probe, StartupSessionCheck::Authorized { .. });");
        let turn = at_restore("let Ok(write) = claim_session_write(acct, &mut turn) else {");
        let in_memory = at_restore("Ok(mut guard) => *guard = Some(session),");
        let asks = at_restore("if token_confirmed { ask_to_resume_auth_paused(&write, acct); }");
        let restored = at_restore("tracing::info!(\"vault auto-unlocked from credential store on startup\");");
        assert!(
            confirmed < turn && turn < in_memory && in_memory < asks && asks < restored,
            "the startup restore asks in its turn, after the session is in memory, only for a confirmed token:\n{restore}"
        );
        assert_eq!(restore.matches(&ask).count(), 1, "{restore}");

        // F9 review I-1: the key-replaced re-sign-in holds the `auth` pauses under the engine slot, in its turn, before
        // the kept key is removed; a hold that fails stops the sign-in before anything is stored.
        let without_key = squeeze(&code_of(&production, "fn reauth_swap_token_without_key("));
        let hold = squeeze("hold_auth_paused_for_the_replaced_key(sources, _engine_slot)");
        let held = without_key.find(&hold).expect("the key-replaced sign-in holds");
        assert!(
            without_key.find(&squeeze("claim_session_write(acct, turn)")).unwrap() < held
                && held < without_key.find("delete_vault_key_from_keychain(").unwrap(),
            "in the turn, before the key is removed:\n{without_key}"
        );
        assert!(
            without_key.contains(&squeeze(
                "return Err(ReauthError::before_store(SIGN_IN_LOCAL_DATA_UNREADABLE));"
            )),
            "{without_key}"
        );
        // F9 review M2: a browser sign-in on a Mac swaps the token holding the engine slot; when the handed-over key
        // replaces the kept one, the outgoing engine stops and the pauses are held before the slot is released.
        let in_place = squeeze(&code_of(&production, "async fn reauth_in_place("));
        let at_in_place = |snippet: &str| {
            in_place
                .find(&squeeze(snippet))
                .unwrap_or_else(|| panic!("{snippet}:\n{in_place}"))
        };
        let slot_taken = at_in_place(
            "let mut engine_slot = if handoff_key.is_some() { Some(acct.engine.lock().await) } else { None };",
        );
        let swapped = at_in_place("reauth_swap_token(state, acct, sources, token, profile, handoff_key, turn)?;");
        let replaced = at_in_place("if vault_unlocked && handoff_replaces_kept_key(acct, handoff_key) {");
        let held = at_in_place("stop_the_outgoing_engine_and_hold_auth_paused(acct, sources, slot)");
        let returned = at_in_place("return Ok(false);");
        let released = at_in_place("drop(engine_slot);");
        let started = at_in_place("start_engine_if_possible(");
        assert!(
            slot_taken < swapped
                && swapped < replaced
                && replaced < held
                && held < returned
                && returned < released
                && released < started,
            "the slot before the swap, the hold before the replace returns, the slot released before the start:\n{in_place}"
        );
        let stop_then_hold = squeeze(&code_of(
            &production,
            "async fn stop_the_outgoing_engine_and_hold_auth_paused(",
        ));
        assert!(
            stop_then_hold
                .find(&squeeze("stop_engine_in_slot(acct, engine)"))
                .expect("the outgoing engine stops")
                < stop_then_hold
                    .find(&squeeze("hold_auth_paused_for_the_replaced_key(sources, engine_slot)"))
                    .expect("and then the pauses are held"),
            "{stop_then_hold}"
        );
        let writer = code_of(&production, "fn ask_to_resume_auth_paused(");
        assert!(
            writer[..writer.find(") {").unwrap()].contains("write: &SessionWrite"),
            "the ask is made only inside a checked turn:\n{writer}"
        );

        // Ruling F9, round 2, as spec D11 (task 1888) amends it: every OS asks (see
        // `windows_and_linux_ask_for_the_resume_at_the_four_sign_in_points`); the holds under the engine slot stay a
        // Mac's (off macOS the engine's key check holds them, spec D12), and so does the resume in the engine start.
        let macos_only = squeeze("#[cfg(target_os = \"macos\")]");
        for (signature, gated) in [
            (
                "fn reauth_swap_token_without_key(",
                format!(
                    "{macos_only}{}",
                    squeeze("if let Err(error) = hold_auth_paused_for_the_replaced_key(sources, _engine_slot) {")
                ),
            ),
            (
                "async fn reauth_in_place(",
                format!(
                    "{macos_only}{}",
                    squeeze("let mut engine_slot = if handoff_key.is_some() {")
                ),
            ),
            (
                "async fn reauth_in_place(",
                format!("{macos_only}{}", squeeze("if let Some(slot) = engine_slot.as_mut() {")),
            ),
            (
                "async fn reauth_in_place(",
                format!("{macos_only}{}", squeeze("drop(engine_slot);")),
            ),
        ] {
            let body = squeeze(&code_of(&production, signature));
            assert!(
                body.contains(&gated),
                "{signature} asks or holds on macOS only:\n{body}"
            );
        }
        // What Windows and Linux compile names no hold under the slot and no resume in the engine start: the engine
        // resumes there, after its reconcile pass and the key check (spec D11, D12).
        let other_platforms =
            super::finder_setup_command_tests::without_items(&production, &["#[cfg(target_os = \"macos\")]"]);
        let named = other_platforms
            .lines()
            .filter(|line| {
                !line.trim_start().starts_with("//")
                    && [
                        "hold_auth_paused_for_the_replaced_key",
                        "stop_the_outgoing_engine_and_hold_auth_paused",
                        "resume_operations_paused_for_auth",
                        "let resume_asked",
                    ]
                    .iter()
                    .any(|name| line.contains(name))
            })
            .collect::<Vec<_>>();
        assert!(named.is_empty(), "Windows and Linux compile none of it: {named:?}");
        let other_in_place = code_of(&other_platforms, "async fn reauth_in_place(");
        assert!(
            !other_in_place.contains("engine_slot"),
            "Linux swaps without the slot, as before:\n{other_in_place}"
        );
        let mac = squeeze(&super::finder_setup_command_tests::macos_compiled(&production));
        assert_eq!(mac.matches(&ask).count(), 4, "a Mac compiles all four asks");
        assert_eq!(
            mac.matches(&squeeze("hold_auth_paused_for_the_replaced_key(sources,"))
                .count(),
            2,
            "and both holds"
        );
    }

    /// Spec D11 (task 1888; the lead lifted the F9/R8 macOS-only ruling): every sign-in point that asks for the resume
    /// asks on every OS that has it. No `#[cfg(target_os = "macos")]` stands in the four lines above any of the four
    /// asks, nor above the writer or the startup check's `token_confirmed`. `reauth_swap_token` keeps its own
    /// `not(windows)` gate (builder check Q2), so Windows has three asks and Linux four.
    #[test]
    fn windows_and_linux_ask_for_the_resume_at_the_four_sign_in_points() {
        use crate::source_pin::squeeze;
        let production = production_source();
        let lines: Vec<&str> = production.lines().collect();
        let macos_only = "#[cfg(target_os = \"macos\")]";
        let asks: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                line.contains("ask_to_resume_auth_paused(")
                    && !line.contains("fn ask_to_resume_auth_paused(")
                    && !line.trim_start().starts_with("//")
            })
            .map(|(at, _)| at)
            .collect();
        assert_eq!(asks.len(), 4, "four sign-in points ask");
        for at in asks {
            let above = lines[at.saturating_sub(4)..at].join("\n");
            assert!(!above.contains(macos_only), "an ask gated to macOS:\n{above}\n{}", lines[at]);
        }
        for item in [
            "fn ask_to_resume_auth_paused(",
            "let token_confirmed = matches!(probe, StartupSessionCheck::Authorized { .. });",
        ] {
            let at = lines.iter().position(|line| line.contains(item)).unwrap_or_else(|| panic!("{item}"));
            let above = lines[at.saturating_sub(4)..at].join("\n");
            assert!(!above.contains(macos_only), "{item} gated to macOS:\n{above}");
        }
        let swap = lines
            .iter()
            .position(|line| line.starts_with("fn reauth_swap_token("))
            .expect("fn reauth_swap_token(");
        assert_eq!(lines[swap - 1], "#[cfg(not(target_os = \"windows\"))]", "its own gate is kept");
        let ask = squeeze("ask_to_resume_auth_paused(&write, acct);");
        let linux = squeeze(&super::finder_setup_command_tests::without_items(
            &production,
            &[macos_only, "#[cfg(target_os = \"windows\")]"],
        ));
        assert_eq!(linux.matches(&ask).count(), 4, "Linux compiles all four asks");
        let windows = squeeze(&super::finder_setup_command_tests::without_items(
            &production,
            &[macos_only, "#[cfg(not(target_os = \"windows\"))]", "#[cfg(target_os = \"linux\")]"],
        ));
        assert_eq!(windows.matches(&ask).count(), 3, "Windows compiles three: no reauth_swap_token");
    }

    /// Spec D11: a Mac resumes in the engine start, before the engine exists; Windows and Linux do not resume there
    /// (the engine does, after its pass). The start reads the ask, and resumes, on macOS only.
    #[test]
    fn start_engine_bound_resumes_before_the_spawn_on_macos_only() {
        use crate::source_pin::squeeze;
        let production = production_source();
        let macos_only = squeeze("#[cfg(target_os = \"macos\")]");
        let bound = squeeze(&code_of(&production, "fn start_engine_bound("));
        for gated in [
            "let resume_asked = acct.auth_resume_asked.load(Ordering::SeqCst);",
            "resume_operations_paused_for_auth(acct, paths, resume_asked);",
        ] {
            assert!(bound.contains(&format!("{macos_only}{}", squeeze(gated))), "{gated} on macOS only:\n{bound}");
        }
        let resumed = bound.find("resume_operations_paused_for_auth(acct,paths,resume_asked);").unwrap();
        assert!(resumed < bound.find("spawn(paths.sync_root.clone()").unwrap(), "before the spawn:\n{bound}");
        let other_platforms =
            super::finder_setup_command_tests::without_items(&production, &["#[cfg(target_os = \"macos\")]"]);
        let other_bound = code_of(&other_platforms, "fn start_engine_bound(");
        assert!(
            !other_bound.contains("resume_operations_paused_for_auth") && !other_bound.contains("resume_asked"),
            "Windows and Linux do not resume in the start:\n{other_bound}"
        );
        assert!(
            !other_platforms.contains("fn resume_operations_paused_for_auth("),
            "the start's resume is a Mac's"
        );
        // The engine gets the account's resume requests on every OS.
        let spawn = squeeze(&code_of(&production, "fn spawn_bound_engine("));
        assert!(
            spawn.contains(&squeeze(
                "runner::ResumeTicket { asked: acct.auth_resume_asked.clone(), done: acct.auth_resume_done.clone(),
                 pass_wanted: acct.reconcile_pass_wanted.clone(), pass_served: acct.reconcile_pass_served.clone(), }"
            )),
            "{spawn}"
        );
    }

    /// Spec D11 and I6: off macOS the engine runs the reconcile pass, then the key check and the resume, inside the
    /// successful tick's arm (the pull proved the server accepts this session's token) and before that tick's queue
    /// step. D11b: the send phase's wake runs no queue step until this run's first pass has committed.
    #[test]
    fn the_engine_reconciles_then_resumes_in_the_successful_tick_before_its_queue_step() {
        use crate::source_pin::squeeze;
        let runner = include_str!("runner.rs").replace("\r\n", "\n");
        let runner = &runner[..runner.find("#[cfg(test)]\nmod tests").expect("runner's tests")];
        let tick = &runner[runner.find("Ok(tick) => {").expect("the successful tick's arm")..];
        let tick = squeeze(&tick[..tick.find("Err(e) => {").unwrap()]);
        let pass = tick
            .find(&squeeze(
                "#[cfg(not(target_os = \"macos\"))]
                 if tick.reached_server
                     && let Err(error) = bridge.reconcile_then_resume(&sync_root, &resume, &mut reconciled_this_run).await {",
            ))
            .unwrap_or_else(|| panic!("the pass, off macOS, in the arm, only when the pull came back:\n{tick}"));
        assert!(tick.find("auth_health.note_result(None);").unwrap() < pass, "after the pull succeeded");
        assert!(pass < tick.find("queue_step(").unwrap(), "before the tick's queue step:\n{tick}");
        assert_eq!(squeeze(runner).matches("reconcile_then_resume(").count(), 1, "one call");
        // Review P2: the engine's queue reads the same ticket, so it sends nothing while a release waits for a pass.
        assert!(squeeze(runner).contains(&squeeze(".with_resume(resume.clone())")), "the bridge shares the ticket");
        assert!(
            squeeze(runner).contains(&squeeze(
                "#[cfg(not(target_os = \"macos\"))]
                 let mut reconciled_this_run = false;"
            )),
            "per engine run"
        );
        // D11b.
        let phase = &runner[runner.find("Wake::Phase => {").expect("the phase's arm")..];
        let phase = squeeze(&phase[..phase.find("\n            }\n").unwrap()]);
        let waits = phase
            .find(&squeeze(
                "#[cfg(not(target_os = \"macos\"))]
                 if !reconciled_this_run { continue; }",
            ))
            .unwrap_or_else(|| panic!("the phase waits for the first pass:\n{phase}"));
        assert!(waits < phase.find("queue_step(").unwrap(), "{phase}");
    }

    /// Task 1 ruling (1888 ledger, on 2267's held changes; review P2): "Upload to my Beebeeb" makes a held ghost folder's
    /// create due again, so it asks for a pass before the release commits and again after it, on the pass counter (the
    /// engine's queue waits for that pass; a release resumes no `auth` op).
    #[test]
    fn a_released_held_change_asks_for_a_pass() {
        use crate::source_pin::squeeze;
        let production = production_source();
        let upload = squeeze(&code_of(&production, "async fn found_while_paused_upload("));
        let ask = squeeze("pass_wanted.fetch_add(1, Ordering::SeqCst);");
        let released = upload.find("db.release_found_while_paused(").expect("the release");
        let before = upload.find(&ask).unwrap_or_else(|| panic!("asks for a pass:\n{upload}"));
        let after = upload.rfind(&ask).unwrap();
        assert!(before < released && released < after, "before the release commits, and again after it:\n{upload}");
        assert!(
            upload.contains(&squeeze("let pass_wanted = state.active_account()")) && upload.contains(&squeeze(".reconcile_pass_wanted.clone();")),
            "the pass counter, not the sign-in counter:\n{upload}"
        );
        assert!(!upload.contains("auth_resume_asked"), "a release resumes no auth op:\n{upload}");
    }

    /// Spec §5.6: a re-sign-in whose kept key the server no longer accepts removes only that key. It never purges,
    /// never removes the domain, never signs out, never writes a key and never starts an engine; the session in memory
    /// ends the Lock way (the turn first, then the reconciler's hold, then the engine stop, all before the writes, which
    /// run under the slot); and memory is cleared before the key is removed from both layouts and the token stored.
    /// Compared with `squeeze`, so the pins do not depend on rustfmt's layout.
    #[test]
    fn a_kept_key_the_server_rejects_is_removed_the_lock_way_and_nothing_else_changes() {
        use crate::source_pin::squeeze;
        let production = production_source();
        let settle = squeeze(&code_of(&production, "async fn settle_sign_in("));
        let arm = settle
            .find(&squeeze("reauth::SignInKind::KeyOutdated =>"))
            .expect("the KeyOutdated arm");
        assert!(
            settle[arm..].starts_with(&squeeze(
                "reauth::SignInKind::KeyOutdated => { reauth_without_the_kept_key(state, &acct, &sources, token, profile, turn).await"
            )) || settle[arm..].starts_with(&squeeze(
                "reauth::SignInKind::KeyOutdated => reauth_without_the_kept_key(state, &acct, &sources, token, profile, turn).await"
            )),
            "{settle}"
        );
        let outer = squeeze(&code_of(&production, "async fn reauth_without_the_kept_key("));
        let inner = squeeze(&code_of(&production, "fn reauth_swap_token_without_key("));
        for (name, body) in [
            ("reauth_without_the_kept_key", &outer),
            ("reauth_swap_token_without_key", &inner),
        ] {
            for forbidden in [
                "purge_local_state_files",
                "purge_all_local_state",
                "purge_local_data_for_sign_out",
                "clear_account_data",
                "clear_keychain_session",
                "clear_session_impl",
                "remove_file_provider_domain",
                "finder_remove_for",
                "Trigger::SignOut",
                "Trigger::Repair",
                "persist_vault_key_to_keychain",
                "persist_session_to_keychain",
                "store_wrapped_master_key",
                "install_unlocked_session",
                "EngineRunner::",
                "spawn_bound_engine",
                "start_engine_if_possible",
                "keys_arrived",
            ] {
                assert!(!body.contains(forbidden), "{name} must not use {forbidden}:\n{body}");
            }
        }
        let order = |body: &str, steps: &[&str]| {
            let at: Vec<usize> = steps
                .iter()
                .map(|step| body.find(&squeeze(step)).unwrap_or_else(|| panic!("{step}:\n{body}")))
                .collect();
            assert!(at.windows(2).all(|pair| pair[0] < pair[1]), "{steps:?}:\n{body}");
        };
        order(
            &outer,
            &[
                "drop(claim_session_write(acct, turn)",
                "finder_lock_for(state)",
                "acct.engine.lock().await",
                "stop_engine_in_slot(acct, engine)",
                "reauth_swap_token_without_key(state, acct, sources, token, profile, turn, &engine_slot)",
                "drop(engine_slot);",
            ],
        );
        order(
            &inner,
            &[
                "claim_session_write(acct, turn)",
                "acct.session.lock()",
                "record_local_data_owner(",
                "hold_auth_paused_for_the_replaced_key(sources, _engine_slot)",
                "delete_vault_key_from_keychain(&write, acct.id.as_str())",
                "persist_session_token_to_keychain(&write, acct.id.as_str(), token, email.as_deref())",
                "drop(write);",
            ],
        );
        assert!(
            inner.contains(&squeeze(
                "_engine_slot: &tokio::sync::MutexGuard<'_, Option<EngineRunner>>"
            )),
            "the writes run while the caller holds the engine slot:\n{inner}"
        );
        let delete = squeeze(&code_of(&production, "fn delete_vault_key_from_keychain("));
        for needle in [
            "platform_keychain_store_for(account_id).delete_wrapped_master_key()",
            "keychain::legacy_platform_keychain_store().delete_wrapped_master_key()",
            "if keychain_vault_key_present(account_id) {",
        ] {
            assert!(
                delete.contains(&squeeze(needle)),
                "both layouts, then checked gone ({needle}):\n{delete}"
            );
        }
        for forbidden in ["delete_session_token", "delete_account_email", "clear_session"] {
            assert!(
                !delete.contains(forbidden),
                "only the key is removed ({forbidden}):\n{delete}"
            );
        }
    }

    /// Spec §5.6: the browser handoff carries the account's current key. `reauth_in_place` returns before it starts an
    /// engine when the kept key differs from it (so the handoff's own `apply_session` installs it), and the handoff
    /// returns early only when the vault is unlocked with the kept key. The comparison reads every byte.
    #[test]
    fn a_handed_over_key_that_differs_is_installed_and_the_comparison_reads_every_byte() {
        use crate::source_pin::squeeze;
        let production = production_source();
        let in_place = squeeze(&code_of(&production, "async fn reauth_in_place("));
        let check = in_place
            .find(&squeeze(
                "if vault_unlocked && handoff_replaces_kept_key(acct, handoff_key) {",
            ))
            .expect("the handoff key is compared");
        let start = in_place.find("start_engine_if_possible(").expect("the engine start");
        assert!(check < start, "compared before any engine starts:\n{in_place}");
        assert!(in_place[check..start].contains("returnOk(false);"), "{in_place}");
        let browser = squeeze(&browser_source());
        assert!(
            browser.contains(&squeeze("Some(&*master_key),")),
            "the handoff passes its key to the check"
        );
        assert!(
            browser.contains(&squeeze(
                "Some(crate::SignInSettlement::Reauthenticated { vault_unlocked: true, .. }) => {"
            )),
            "only an unlocked vault returns early"
        );
        let equal = squeeze(&code_of(&production, "fn vault_keys_equal("));
        assert!(
            equal.contains(&squeeze(".fold(0u8, |acc, (x, y)| acc | (x ^ y))"))
                && equal.contains(&squeeze("std::hint::black_box(difference) == 0")),
            "{equal}"
        );
        let compares = equal.replace(&squeeze("std::hint::black_box(difference) == 0"), "");
        for early in ["return", "==", "!=", "any(", "all(", "position("] {
            assert!(!compares.contains(early), "no early exit ({early}):\n{equal}");
        }
        assert!(
            squeeze(&code_of(&production, "fn session_holds_vault_key("))
                .contains(&squeeze("vault_keys_equal(&session.master_key, key)")),
            "the one comparison"
        );
    }

    /// The server proof is the one place R8 reads a vault key: into a wiped buffer that lives inside one function, from
    /// which a `MasterKey` (wiped on drop, not `Clone`) is built at once. Nothing else holds a plain copy: the proof
    /// receives the `MasterKey` and sends only its check. (The core takes the 32 bytes by value and has no
    /// reference-taking constructor; it wipes the argument it is given, and the buffer here is a `Zeroizing`.)
    #[test]
    fn the_server_proof_reads_the_key_only_to_send_its_check() {
        let production = production_source();
        assert_eq!(
            production.matches("load_stored_vault_key(").count(),
            2,
            "defined once, called by the proof only"
        );
        assert_eq!(production.matches("fn read_vault_key<").count(), 1);
        assert_eq!(
            production.matches("read_vault_key(").count(),
            2,
            "called for the two Keychain layouts, and only there"
        );
        let proof = code_of(&production, "async fn prove_stored_key(");
        let load = proof.find("load_stored_vault_key(").expect("the proof reads the key");
        let sent = proof.find("recovery_check_answer(").expect("then asks the server");
        assert!(load < sent, "read, then ask:\n{proof}");
        for plain_copy in ["from_bytes", "[u8; 32]", "drop(", "*key"] {
            assert!(
                !proof.contains(plain_copy),
                "the proof makes no plain copy of the key ({plain_copy}):\n{proof}"
            );
        }
        assert!(
            proof.contains("Err(_) => reauth::KeyProof::Unavailable"),
            "an error from the server settles nothing:\n{proof}"
        );
        let read = code_of(&production, "fn read_vault_key<");
        assert!(
            read.contains("-> Result<Option<beebeeb_core::kdf::MasterKey>, String>"),
            "the key leaves as a MasterKey:\n{read}"
        );
        assert!(
            read.contains("let mut raw = zeroize::Zeroizing::new([0u8; 32]);"),
            "the only plain buffer is a wiped one:\n{read}"
        );
        assert!(
            read.contains("beebeeb_core::kdf::MasterKey::from_bytes(*raw)"),
            "{read}"
        );
        assert!(
            code_of(&production, "fn load_stored_vault_key(")
                .contains("-> Result<Option<beebeeb_core::kdf::MasterKey>, String>")
        );
    }

    /// R10: a same-account re-sign-in records the owner before anything else changes.
    #[test]
    fn a_same_account_re_sign_in_records_the_owner_first() {
        let production = production_source();
        let body = code_of(&production, "fn reauth_swap_token(");
        let record = body.find("record_local_data_owner(").expect("the owner is recorded");
        assert!(record < body.find("persist_session_token_to_keychain(").expect("then the token"));
        assert!(record < body.find("replace_session_token_in_memory(").expect("then memory"));
    }

    /// A failure counts as "the token is not stored" up to and including the Keychain write, and as "stored" after it:
    /// the owner record and the Keychain write say `before_store`, every later step says `after_store`. (The Keychain
    /// write itself cannot be made to fail in a test, so this is pinned as source; the two ends are behaviour in
    /// `a_re_sign_in_that_fails_says_whether_its_token_was_stored`.)
    #[test]
    fn the_swap_calls_its_token_stored_exactly_when_the_keychain_write_has_succeeded() {
        let production = production_source();
        let swap = code_of(&production, "fn reauth_swap_token(");
        let keychain = swap
            .find("persist_session_token_to_keychain(")
            .expect("the Keychain write");
        let (before, after) = swap.split_at(keychain);
        let line_end = after
            .find(".map_err(ReauthError::before_store)?;")
            .expect("the write's failure is before_store");
        assert!(
            !after[..line_end].contains("after_store"),
            "the Keychain write's own failure is not 'stored':\n{after}"
        );
        assert!(
            before.contains("record_local_data_owner(") && before.contains(".map_err(ReauthError::before_store)?;"),
            "{before}"
        );
        assert!(
            !before.contains("after_store"),
            "nothing before the Keychain write is 'stored':\n{before}"
        );
        let later = &after[line_end + ".map_err(ReauthError::before_store)?;".len()..];
        assert!(
            !later.contains("before_store"),
            "nothing after the Keychain write is 'not stored':\n{later}"
        );
        assert!(
            later.matches("ReauthError::after_store").count() >= 3,
            "every later failure is 'stored':\n{later}"
        );
    }

    /// The account check looks at every retained trace, not only the token.
    #[test]
    fn the_account_check_counts_every_retained_trace() {
        let production = production_source();
        let facts = code_of(&production, "fn gather_local_facts(");
        for probe in [
            "acct.session.lock()",
            "state.auth_present.lock()",
            "keychain_session_trace(",
            "keychain_email_trace(",
            "keychain_vault_key_present(",
            "local_data_owner(",
            "acct.cached_profile.lock()",
            "queued_or_staged_present(",
        ] {
            assert!(facts.contains(probe), "gather_local_facts must use {probe}");
        }
        assert!(code_of(&production, "fn classify_sign_in_after(").contains("traces: facts.traces.any()"));
        assert!(
            code_of(&production, "fn classify_sign_in(")
                .contains("classify_sign_in_after(state, acct, sources, profile, None)"),
            "one place gathers the facts"
        );
        // An unreadable source is a trace, never an absence: each of the three locks, on its own line, so flipping any
        // ONE of them fails this pin (`a_poisoned_lock_counts_as_a_trace_in_each_of_its_three_places` is the behaviour).
        for lock in [
            "session_in_memory: acct.session.lock().map(|guard| guard.is_some()).unwrap_or(true),",
            "auth_present: state.auth_present.lock().map(|guard| *guard).unwrap_or(true),",
            "cached_profile: acct.cached_profile.lock().map(|guard| guard.is_some()).unwrap_or(true),",
        ] {
            assert!(facts.contains(lock), "a poisoned lock counts as a trace: {lock}");
        }
        assert!(
            !facts.contains("keychain_session_present(") && !facts.contains("keychain_account_email("),
            "the fail-open readers are not the probes"
        );
        let queued = code_of(&production, "fn queued_or_staged_present(");
        assert!(
            queued.contains("Err(_) => true"),
            "an unreadable state.db counts as present:\n{queued}"
        );
    }

    /// Item 3 is about unowned local DATA only: rows of `state.db`, read through `has_account_data`, in a database whose
    /// owner is not recorded. Credentials (the token, the Keychain email, the vault key, the cached profile) are not
    /// data: they name an account for the identity order. A source that cannot be read is neither data nor an absence:
    /// it is `unreadable`, and settles nothing.
    #[test]
    fn unowned_data_is_local_data_only_and_an_unreadable_source_settles_nothing() {
        let production = production_source();
        let data = code_of(&production, "fn unowned_local_data(");
        assert!(data.contains("-> Result<bool, String>"), "{data}");
        assert!(
            data.contains("match sources.state_db()? {"),
            "an unreadable database is an error, not an answer:\n{data}"
        );
        // Read through `squeeze`: rustfmt wraps this chain over lines.
        let squeezed_data = crate::source_pin::squeeze(&data);
        assert!(
            squeezed_data.contains(&crate::source_pin::squeeze(
                "Some(db) => db.has_account_data().map_err("
            )),
            "{data}"
        );
        assert!(
            data.contains("None => Ok(false),"),
            "no database yet is no data:\n{data}"
        );
        for credential in ["keychain", "session", "cached_profile", "auth_present", "AuthVault"] {
            assert!(
                !data.contains(credential),
                "unowned data does not look at credentials ({credential}):\n{data}"
            );
        }
        let facts = code_of(&production, "fn gather_local_facts(");
        assert!(
            facts.contains("let owner_unreadable = owner.is_err();"),
            "an owner record that cannot be read:\n{facts}"
        );
        assert!(
            crate::source_pin::squeeze(&facts).contains(&crate::source_pin::squeeze(
                "let data = if owner.is_some() { Ok(false) } else { unowned_local_data(sources) };"
            )),
            "{facts}"
        );
        assert!(facts.contains("unowned_data: matches!(data, Ok(true)),"), "{facts}");
        assert!(
            facts.contains("unreadable: owner_unreadable || data.is_err() || pending_changes.is_err(),"),
            "{facts}"
        );
        // Triage 25: the switch warning's count fails closed, never to 0.
        let count = code_of(&production, "fn pending_changes_count(");
        assert!(count.contains("-> Result<u64, String>"), "{count}");
        assert!(!count.contains("unwrap_or("), "an error is never a count:\n{count}");
        assert!(facts.contains("credentials: credentials_identity(acct)"), "{facts}");
        let classify = code_of(&production, "fn classify_sign_in_after(");
        assert!(
            classify.contains("unowned_data: facts.unowned_data,")
                && classify.contains("unreadable: facts.unreadable,"),
            "{classify}"
        );
        assert!(
            classify.contains("facts.owner.as_ref().unwrap_or(&facts.credentials)"),
            "an owner record wins; else the credentials name the account:\n{classify}"
        );
        let credentials = code_of(&production, "fn credentials_identity(");
        assert!(
            credentials.contains("acct.cached_profile.lock()") && credentials.contains("keychain_account_email("),
            "{credentials}"
        );
        assert!(
            !credentials.contains("master_key") && !credentials.contains("token"),
            "an identity is not a secret:\n{credentials}"
        );
    }

    /// The vault-key probe asks both Keychain layouts and never unlocks.
    #[test]
    fn the_vault_key_probe_asks_both_layouts_and_never_unlocks() {
        let production = production_source();
        let probe = code_of(&production, "fn keychain_vault_key_present(");
        assert!(
            probe.contains("platform_keychain_store_for(account_id)")
                && probe.contains("legacy_platform_keychain_store()"),
            "{probe}"
        );
        assert!(!probe.contains("unlock") && !probe.contains("AuthVault"), "{probe}");
        // The token and email probes: both layouts, through the fail-closed presence functions, never a read of the item.
        for (name, holds) in [
            ("fn keychain_session_trace(", "holds_session_token("),
            ("fn keychain_email_trace(", "holds_account_email("),
        ] {
            let body = code_of(&production, name);
            assert_eq!(
                body.matches(holds).count(),
                2,
                "{name} asks both layouts with {holds}:\n{body}"
            );
            assert!(
                body.contains("platform_keychain_store_for(account_id)")
                    && body.contains("legacy_platform_keychain_store()"),
                "{body}"
            );
            assert!(
                !body.contains("load_")
                    && !body.contains("AuthVault")
                    && !body.contains(".ok()")
                    && !body.contains("unwrap_or(false)"),
                "{name} must not fail open:\n{body}"
            );
        }
    }

    /// Every test that runs the real sign-out shares one scratch state directory, so the purge takes turns in test
    /// builds (a gate inside `clear_session_impl`, before the purge), and no test retries around a busy database. A test
    /// that works in that directory itself (the one that seeds it before it signs out) holds the same gate for its whole
    /// body through `with_the_shared_state_dir`, and its own sign-out knows it does (a task-local marker), so it does not
    /// wait for what it holds.
    #[test]
    fn the_sign_out_purge_takes_turns_in_tests_and_nothing_retries_around_it() {
        let source = crate::root_source::lib_rs().replace("\r\n", "\n");
        let clear = &source[source.find("async fn clear_session_impl(").unwrap()..];
        let clear = &clear[..clear.find("\n}\n").unwrap()];
        let held = "let _one_purge_at_a_time = if PURGE_GATE_HELD.try_with(|_| ()).is_ok() {\n            None\n        } else {\n            Some(SIGN_OUT_TEST_PURGE_GATE.lock().await)\n        };";
        let gate = clear
            .find(held)
            .expect("the purge takes turns, unless this test already holds the gate");
        assert!(
            clear[..gate].trim_end().ends_with("#[cfg(test)]"),
            "the gate is compiled into test builds only"
        );
        assert!(
            gate < clear.find("purge_local_data_for_sign_out(").expect("the purge"),
            "the gate is taken before the purge"
        );
        // The one test that seeds the shared database itself works under the gate, from its first write to its last check.
        let seeding_file = include_str!("finder_lock_and_sign_out_tests.rs").replace("\r\n", "\n");
        let seeding = &seeding_file[seeding_file
            .find(concat!("fn a_sign_out_leaves_no_account_row_", "and_no_owner() {"))
            .expect("the seeding test")..];
        let seeding = &seeding[..seeding.find("\n    }\n").unwrap()];
        let under_the_gate = seeding
            .find("with_the_shared_state_dir(")
            .expect("the seeding test holds the gate");
        assert!(
            under_the_gate < seeding.find("StateDb::open(").expect("it opens the shared database"),
            "the gate comes before its first write:\n{seeding}"
        );
        assert!(
            under_the_gate < seeding.find("has_account_data()").unwrap(),
            "{seeding}"
        );
        assert!(
            source.contains("static SIGN_OUT_TEST_PURGE_GATE: tokio::sync::Mutex<()>"),
            "the gate is one process-wide lock"
        );
        let source = super::finder_setup_command_tests::with_test_modules(&source);
        assert!(
            !source.contains(concat!("sign_out_", "retrying")),
            "no retry around a busy database is left"
        );
    }

    /// Ruling 3 (Task 11), carried by lead ruling 1 of Task 12: the unlock re-checks the account before it writes the
    /// key and the email, in the same checked turn as every other session write, and puts the session in memory before
    /// that turn ends. Every writer of a session takes the turn as a parameter, so none can write outside one; only the
    /// claim and the end of transitions make one.
    #[test]
    fn the_unlock_rechecks_the_account_and_every_session_writer_takes_the_lock() {
        let production = production_source();
        let unlock = code_of(&production, "async fn desktop_unlock_with_recovery_phrase(");
        let verified = unlock.find("verify_vault_key_from_phrase(").expect("the server check");
        let installed = unlock
            .find("install_recovered_session(&state, &acct, &mut turn,")
            .expect("the key goes through the checked install");
        assert!(
            verified < installed,
            "the key is written after the server check, through the checked install:\n{unlock}"
        );
        assert!(
            !unlock.contains("persist_vault_key_to_keychain(") && !unlock.contains("store_account_email("),
            "no unchecked write of the key or the email:\n{unlock}"
        );
        assert!(
            unlock.find("let mut turn = acct.session_generation();").unwrap()
                < unlock.find("load_session_token_from_keychain(").unwrap(),
            "captured before the Keychain is read"
        );
        let install = code_of(&production, "fn install_recovered_session(");
        let stored = install
            .find("let write = store_recovered_vault_key(acct, turn,")
            .expect("the checked writer");
        let in_memory = install
            .find("*guard = Some(Session {")
            .expect("the session goes into memory");
        let released = install.find("drop(write);").expect("and the turn ends");
        assert!(
            stored < in_memory && in_memory < released,
            "the install happens inside the unlock's turn:\n{install}"
        );
        let helper = code_of(&production, "fn store_recovered_vault_key(");
        let claim = helper
            .find("claim_session_write(acct, turn)")
            .expect("the writer takes the turn");
        let token = helper
            .find("load_session_token_from_keychain(")
            .expect("it re-reads the stored token");
        let write = helper
            .find("persist_vault_key_to_keychain(&write, ")
            .expect("then writes the key");
        assert!(claim < token && token < write, "turn, re-check, write:\n{helper}");
        assert!(
            helper.find("UNLOCK_ACCOUNT_CHANGED").unwrap() < write,
            "a changed account stops before the write:\n{helper}"
        );
        for signature in [
            "fn persist_session_to_keychain(",
            "fn persist_session_token_to_keychain(",
            "fn persist_vault_key_to_keychain(",
            "fn install_unlocked_session(",
            "fn clear_keychain_session_token(",
            "fn name_session_after_profile(",
            "fn backfill_local_data_owner(",
            "fn clear_keychain_session_holding(",
            "fn replace_session_token_in_memory(",
            "fn name_session_in_memory(",
            "fn delete_vault_key_from_keychain(",
        ] {
            let function = &production[production.find(signature).unwrap_or_else(|| panic!("{signature}"))..];
            let parameters = &function[..function.find(") ->").unwrap()];
            assert!(
                parameters.contains("write: &SessionWrite"),
                "{signature} can only write inside a checked turn:\n{parameters}"
            );
        }
        // The same for the one writer with no return value (triage 10, P3-C).
        let settle = &production[production.find("fn reauth_settle_flags(").expect("the flags writer")..];
        let parameters = &settle[..settle.find(") {").unwrap()];
        assert!(
            parameters.contains("write: &SessionWrite"),
            "reauth_settle_flags can only write inside a checked turn:\n{parameters}"
        );
        // The naming checks its turn and moves the generation on only when it named something (fix round 1, item 5).
        let naming = code_of(&production, "async fn establish_session_identity(");
        let checked = naming
            .find("check_session_turn(acct, &turn)")
            .expect("the naming checks its turn");
        let named = naming
            .find("let named = name_session_after_profile(&write,")
            .expect("then names");
        let moved = naming.find("if named {\n        acct.advance_session_generation_holding_session_write_lock(account::SessionTransition::Write);").expect("and moves the generation only then");
        assert!(checked < named && named < moved, "{naming}");
        assert!(
            !naming.contains("claim_session_write("),
            "a naming that names nothing moves nothing:\n{naming}"
        );
        assert!(
            naming.find("let turn = acct.session_generation();").unwrap() < naming.find("acct.session.lock()").unwrap(),
            "captured before the session is read"
        );
        // The Keychain email retry (fix round 2, item 3) writes in a turn captured before it read the session's email.
        let retry = code_of(&production, "async fn identify_unidentified_session(");
        let captured = retry
            .find("let turn = acct.session_generation();")
            .expect("the retry captures its turn");
        assert!(
            captured < retry.find("acct.session.lock()").unwrap(),
            "captured before the session is read:\n{retry}"
        );
        assert!(
            retry.contains("&& let Ok(write) = check_session_turn(&acct, &turn)"),
            "and checked with it:\n{retry}"
        );
        assert_eq!(
            retry.matches("session_generation()").count(),
            1,
            "never re-captured at the check:\n{retry}"
        );
        assert_eq!(
            production.matches("lock_session_keychain_writes()").count(),
            5,
            "the definition, the check, the end of transitions, the sign-out's last turn and the full Keychain clear"
        );
        // Struct literals only. Read through `squeeze`, since rustfmt wraps some of the literals over lines; the
        // definition (`struct SessionWrite { _writes: ..`) squeezes to the same text and is taken off.
        let squeezed = crate::source_pin::squeeze(&production);
        assert_eq!(
            squeezed.matches("SessionWrite{_writes:").count() - squeezed.matches("structSessionWrite{_writes:").count(),
            4,
            "only the check (the claim goes through it), the end of transitions, the sign-out's last turn and the full Keychain clear make a turn"
        );
        // The claim is the check plus the move, and every refusal names what moved the generation (item 8).
        let claim = code_of(&production, "fn claim_session_write(");
        assert!(
            claim.contains("let write = check_session_turn(acct, turn)?;"),
            "{claim}"
        );
        let check = code_of(&production, "fn check_session_turn(");
        assert!(check.contains("return Err(SessionChanged::of(acct));"), "{check}");
    }

    // ---- behaviour ----

    #[cfg(target_os = "macos")]
    mod on_a_mac {
        use super::super::*;
        use crate::account::{AccountId, AccountRuntime, synthesize_single_account};
        use crate::account_binding::Identity;
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        pub(super) fn profile(user_id: &str, email: &str) -> account_dto::AccountProfile {
            serde_json::from_str(&format!(
                r#"{{"user_id":"{user_id}","email":"{email}","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}}"#
            ))
            .unwrap()
        }

        pub(super) struct Fixture {
            pub state: AppState,
            pub acct: Arc<AccountRuntime>,
            pub dir: tempfile::TempDir,
        }

        impl Fixture {
            pub fn new() -> Self {
                let state = AppState::default();
                synthesize_single_account(&state, AccountId::new_v4());
                let acct = state.active_account().unwrap();
                Self {
                    state,
                    acct,
                    dir: tempfile::tempdir().unwrap(),
                }
            }

            pub fn id(&self) -> &str {
                self.acct.id.as_str()
            }

            /// Where this computer's local data is: a throwaway state dir.
            pub fn sources(&self) -> LocalSources {
                LocalSources::for_test(self.dir.path())
            }

            pub fn db(&self) -> state_db::StateDb {
                state_db::StateDb::open(self.dir.path().join("state.db")).unwrap()
            }

            /// The session in memory (and the flags that go with it).
            pub fn install_session(&self, token: &str, key: [u8; 32], email: Option<&str>) {
                *self.acct.session.lock().unwrap() = Some(Session {
                    token: token.into(),
                    master_key: key,
                    email: email.map(str::to_string),
                });
                set_auth_present(&self.state, true);
            }

            pub fn classify(&self, profile: &account_dto::AccountProfile) -> (reauth::SignInKind, u64) {
                classify_sign_in(&self.state, &self.acct, &self.sources(), profile)
            }

            /// The decision with the server's proof, against the server at `base_url`.
            pub async fn decide(
                &self,
                profile: &account_dto::AccountProfile,
                token: &str,
                base_url: &str,
            ) -> (reauth::SignInKind, u64) {
                decide_sign_in(&self.state, &self.acct, &self.sources(), profile, token, base_url).await
            }

            /// Everything a sign-in could change on this Mac. `state.db` is looked at only if it exists (looking creates it).
            pub fn snapshot(&self) -> Snapshot {
                let id = self.id();
                let db = self.dir.path().join("state.db").exists().then(|| self.db());
                Snapshot {
                    token: stored_token(id),
                    key: stored_key(id),
                    email: keychain_account_email(id),
                    owner: db.as_ref().and_then(|db| db.owner().unwrap()),
                    waiting: db.as_ref().map(|db| db.queued_or_staged_count().unwrap()),
                    session: self
                        .acct
                        .session
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|s| (s.token.clone(), s.master_key, s.email.clone())),
                    auth_present: *self.state.auth_present.lock().unwrap(),
                    auth_email: self.acct.auth_email.lock().unwrap().clone(),
                    cached: self
                        .acct
                        .cached_profile
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|p| p.user_id.clone()),
                    engine_state: self.acct.engine_state.lock().unwrap().clone(),
                }
            }
        }

        impl Fixture {
            /// [`Fixture::snapshot`] for a world whose `state.db` may be unreadable: the database is left out.
            pub fn snapshot_without_reading_state_db(&self) -> Snapshot {
                let id = self.id();
                Snapshot {
                    token: stored_token(id),
                    key: stored_key(id),
                    email: keychain_account_email(id),
                    owner: None,
                    waiting: None,
                    session: self
                        .acct
                        .session
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|s| (s.token.clone(), s.master_key, s.email.clone())),
                    auth_present: *self.state.auth_present.lock().unwrap(),
                    auth_email: self.acct.auth_email.lock().unwrap().clone(),
                    cached: self
                        .acct
                        .cached_profile
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|p| p.user_id.clone()),
                    engine_state: self.acct.engine_state.lock().unwrap().clone(),
                }
            }
        }

        /// A Mac's whole account state: the Keychain, the local database, memory and the flags.
        #[derive(Debug, PartialEq)]
        pub(super) struct Snapshot {
            token: Option<String>,
            key: Option<[u8; 32]>,
            email: Option<String>,
            owner: Option<Identity>,
            waiting: Option<u64>,
            session: Option<(String, [u8; 32], Option<String>)>,
            auth_present: bool,
            auth_email: Option<String>,
            cached: Option<String>,
            engine_state: String,
        }

        /// A local HTTP server that answers every request with `status_line` and `body` and records each request it
        /// received (request line, headers and body, as sent): a test sees what the server was asked, and with which token.
        fn serving(status_line: &'static str, body: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
            serving_by(move |_| (status_line, body))
        }

        /// A server that confirms exactly one key: `200 {valid:true}` for that key's check, `invalid_recovery_phrase`
        /// for any other request. Records each request, as [`serving`] does.
        fn confirming_only(key: [u8; 32]) -> (String, Arc<Mutex<Vec<String>>>) {
            let check = recovery_check_of(key);
            serving_by(move |request| {
                if request.contains(&check) {
                    KEY_IS_THIS_ACCOUNTS
                } else {
                    KEY_IS_ANOTHER_ACCOUNTS
                }
            })
        }

        /// [`serving`], with the status line and body chosen per request by `respond` (given the request as received).
        fn serving_by(
            respond: impl Fn(&str) -> (&'static str, &'static str) + Send + 'static,
        ) -> (String, Arc<Mutex<Vec<String>>>) {
            use std::io::{Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let seen: Arc<Mutex<Vec<String>>> = Arc::default();
            let record = seen.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { return };
                    let mut request = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        let n = stream.read(&mut buf).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        request.extend_from_slice(&buf[..n]);
                        if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&request[..end]).to_lowercase();
                            let wanted = head
                                .lines()
                                .find_map(|line| line.strip_prefix("content-length:"))
                                .and_then(|v| v.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            if request.len() >= end + 4 + wanted {
                                break;
                            }
                        }
                    }
                    let request = String::from_utf8_lossy(&request).into_owned();
                    let (status_line, body) = respond(&request);
                    record.lock().unwrap().push(request);
                    let response = format!(
                        "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
            });
            (base_url, seen)
        }

        const KEY_IS_THIS_ACCOUNTS: (&str, &str) = ("200 OK", r#"{"valid":true}"#);
        const KEY_IS_ANOTHER_ACCOUNTS: (&str, &str) = ("400 Bad Request", r#"{"error":"invalid_recovery_phrase"}"#);
        const SERVER_IS_BROKEN: (&str, &str) = ("500 Internal Server Error", r#"{"error":"boom"}"#);

        /// The `recovery_check` the server is sent for a key.
        fn recovery_check_of(key: [u8; 32]) -> String {
            encode_base64(&*beebeeb_core::opaque::compute_recovery_check(
                &beebeeb_core::kdf::MasterKey::from_bytes(key),
            ))
        }

        /// A base URL nothing listens on: the connection is refused at once.
        fn nobody_listens() -> String {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            drop(listener);
            url
        }

        /// Poison a lock the way a panic while holding it does.
        fn poison<T: Send>(mutex: &Mutex<T>) {
            std::thread::scope(|scope| {
                let _ = scope
                    .spawn(|| {
                        let _held = mutex.lock().unwrap();
                        panic!("poisoned on purpose");
                    })
                    .join();
            });
        }

        pub(super) fn some_operation(db: &state_db::StateDb, op_id: &str) {
            db.enqueue_operation(&state_db::PendingOperation {
                op_id: op_id.into(),
                kind: state_db::OperationKind::UploadVersion,
                file_id: Some("file-r8".into()),
                parent_id: None,
                target_path: Some("/R8.txt".into()),
                metadata_json: None,
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 5,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        }

        pub(super) fn stored_token(id: &str) -> Option<String> {
            load_session_token_from_keychain(id).unwrap()
        }

        pub(super) fn stored_key(id: &str) -> Option<[u8; 32]> {
            let mut vault = AuthVault::new(platform_keychain_store_for(id));
            vault.unlock().ok()?;
            let mut key = [0u8; 32];
            key.copy_from_slice(vault.master_key().ok()?);
            Some(key)
        }

        pub(super) fn put_email(id: &str, email: &str) {
            AuthVault::new(platform_keychain_store_for(id))
                .store_account_email(email)
                .unwrap();
        }

        pub(super) fn sam() -> account_dto::AccountProfile {
            profile("u-1", "sam@beebeeb.io")
        }

        pub(super) fn kim() -> account_dto::AccountProfile {
            profile("u-2", "kim@beebeeb.io")
        }

        // -- the token swap --

        #[test]
        fn a_new_token_replaces_only_the_token_in_memory() {
            let fx = Fixture::new();
            fx.install_session("old-token", [7u8; 32], Some("sam@beebeeb.io"));
            assert!(replace_session_token_in_memory(&SessionWrite::for_test(), &fx.acct, "new-token").unwrap());
            let guard = fx.acct.session.lock().unwrap();
            let session = guard.as_ref().unwrap();
            assert_eq!(session.token, "new-token");
            assert_eq!(session.master_key, [7u8; 32], "the keys stay");
            assert_eq!(session.email.as_deref(), Some("sam@beebeeb.io"));
        }

        #[test]
        fn without_keys_in_memory_nothing_is_invented() {
            let fx = Fixture::new();
            assert!(!replace_session_token_in_memory(&SessionWrite::for_test(), &fx.acct, "new-token").unwrap());
            assert!(fx.acct.session.lock().unwrap().is_none());
            assert!(
                !name_session_in_memory(&SessionWrite::for_test(), &fx.acct, "sam@beebeeb.io").unwrap(),
                "no session, nobody to name"
            );
            assert!(fx.acct.session.lock().unwrap().is_none());
        }

        #[test]
        fn a_re_sign_in_ends_the_revoked_streak_and_never_asks_finder_to_remove() {
            let fx = Fixture::new();
            fx.install_session("old-token", [7u8; 32], Some("sam@beebeeb.io"));
            for _ in 0..3 {
                fx.acct
                    .auth_health
                    .note_result(Some(&anyhow::anyhow!("HTTP 401 Unauthorized: session revoked")));
            }
            assert!(
                fx.acct.auth_health.is_expired(),
                "precondition: the session reads as revoked"
            );
            let (handle, mut rx) =
                finder_setup::driver::FinderSetupHandle::for_test(finder_setup::driver::FinderSetupView::initial(
                    finder_setup::launch_location::LaunchLocation::Applications,
                ));
            let _ = fx.state.finder_setup.set(handle);
            reauth_settle_flags(&SessionWrite::for_test(), &fx.state, &fx.acct, "sam@beebeeb.io");
            keys_arrived(&fx.state, KeysFrom::SignIn);
            assert!(!fx.acct.auth_health.is_expired());
            assert!(*fx.state.auth_present.lock().unwrap());
            let mut seen = 0;
            while let Ok(event) = rx.try_recv() {
                seen += 1;
                assert!(
                    matches!(
                        event,
                        finder_setup::driver::Event::Trigger(finder_setup::core::Trigger::KeysArrived)
                    ),
                    "a re-sign-in only ever tells Finder that keys are here"
                );
            }
            assert_eq!(seen, 1, "and tells it once");
        }

        /// R8: the same account signs in again, with keys in memory and in the Keychain. Only the token changes, in
        /// the Keychain and in memory; the key, the email, the queue, the owner and the cache all stay.
        #[test]
        fn a_same_account_re_sign_in_swaps_only_the_token_and_changes_nothing_else() {
            let fx = Fixture::new();
            let id = fx.id().to_string();
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                &id,
                "tok-old",
                &[7u8; 32],
                Some("sam@beebeeb.io"),
            )
            .unwrap();
            fx.install_session("tok-old", [7u8; 32], Some("sam@beebeeb.io"));
            let db = fx.db();
            db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            some_operation(&db, "op-r8");
            let waiting = db.queued_or_staged_count().unwrap();
            assert_eq!(waiting, 1);

            // The ids match and a key is here, so the server proves the key first (spec §5.6); here it confirms it.
            assert_eq!(fx.classify(&sam()), (reauth::SignInKind::NeedsKeyProof, 1));
            assert_eq!(
                classify_sign_in_after(
                    &fx.state,
                    &fx.acct,
                    &fx.sources(),
                    &sam(),
                    Some(reauth::KeyProof::Matches)
                ),
                (reauth::SignInKind::SameAccount, 1)
            );
            let unlocked = reauth_swap_token(
                &fx.state,
                &fx.acct,
                &fx.sources(),
                "tok-new",
                &sam(),
                None,
                &mut fx.acct.session_generation(),
            )
            .unwrap();

            assert!(unlocked, "the keys were here");
            assert_eq!(
                stored_token(&id).as_deref(),
                Some("tok-new"),
                "the Keychain holds the new token"
            );
            assert_eq!(stored_key(&id), Some([7u8; 32]), "and the same key");
            assert_eq!(keychain_account_email(&id).as_deref(), Some("sam@beebeeb.io"));
            {
                let guard = fx.acct.session.lock().unwrap();
                let session = guard.as_ref().unwrap();
                assert_eq!(
                    (session.token.as_str(), session.master_key),
                    ("tok-new", [7u8; 32]),
                    "memory: the new token, the same keys"
                );
                assert_eq!(session.email.as_deref(), Some("sam@beebeeb.io"));
            }
            assert_eq!(
                db.queued_or_staged_count().unwrap(),
                waiting,
                "pending edits stay and upload afterwards"
            );
            assert_eq!(
                db.owner().unwrap(),
                Some(Identity::new(Some("u-1"), Some("sam@beebeeb.io"))),
                "the same owner"
            );
            assert_eq!(
                fx.acct
                    .cached_profile
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|p| p.user_id.clone())
                    .as_deref(),
                Some("u-1")
            );
            assert!(*fx.state.auth_present.lock().unwrap());
            assert_eq!(fx.acct.auth_email.lock().unwrap().as_deref(), Some("sam@beebeeb.io"));
        }

        /// After a startup 401 (R9) the Keychain keeps the key and the email and no session is in memory: the same
        /// account signing in again reloads that key, and nothing asks for the recovery phrase.
        #[test]
        fn a_same_account_re_sign_in_after_a_revoked_token_reloads_the_key_this_mac_kept() {
            let fx = Fixture::new();
            let id = fx.id().to_string();
            persist_vault_key_to_keychain(&SessionWrite::for_test(), &id, &[5u8; 32]).unwrap();
            put_email(&id, "sam@beebeeb.io");
            fx.db()
                .set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            assert!(fx.acct.session.lock().unwrap().is_none());

            // The ids match and the key is kept, so the server proves it first (spec §5.6); here it confirms it.
            assert_eq!(fx.classify(&sam()).0, reauth::SignInKind::NeedsKeyProof);
            assert_eq!(
                classify_sign_in_after(
                    &fx.state,
                    &fx.acct,
                    &fx.sources(),
                    &sam(),
                    Some(reauth::KeyProof::Matches)
                )
                .0,
                reauth::SignInKind::SameAccount
            );
            let unlocked = reauth_swap_token(
                &fx.state,
                &fx.acct,
                &fx.sources(),
                "tok-new",
                &sam(),
                None,
                &mut fx.acct.session_generation(),
            )
            .unwrap();

            assert!(unlocked, "the kept key was loaded");
            let guard = fx.acct.session.lock().unwrap();
            let session = guard.as_ref().expect("the keys are in memory again");
            assert_eq!((session.token.as_str(), session.master_key), ("tok-new", [5u8; 32]));
            assert_eq!(session.email.as_deref(), Some("sam@beebeeb.io"));
            assert_eq!(stored_key(&id), Some([5u8; 32]), "the Keychain key is untouched");
        }

        /// Same account, no key on this Mac: the token is replaced and the person is sent to the recovery phrase.
        #[test]
        fn a_same_account_re_sign_in_without_a_key_here_asks_for_the_recovery_phrase() {
            let fx = Fixture::new();
            let id = fx.id().to_string();
            put_email(&id, "sam@beebeeb.io");
            fx.db()
                .set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();

            let unlocked = reauth_swap_token(
                &fx.state,
                &fx.acct,
                &fx.sources(),
                "tok-new",
                &sam(),
                None,
                &mut fx.acct.session_generation(),
            )
            .unwrap();

            assert!(!unlocked, "no key here: the recovery phrase step follows");
            assert!(fx.acct.session.lock().unwrap().is_none(), "no keys are invented");
            assert_eq!(stored_token(&id).as_deref(), Some("tok-new"));
            assert_eq!(stored_key(&id), None);
            assert!(*fx.state.auth_present.lock().unwrap());
        }

        /// The session in memory is named by the server's spelling of the email, so the next engine start compares
        /// the same identity as the owner record and keeps the data (a different spelling would read as another
        /// account and reset the local data).
        #[test]
        fn a_re_sign_in_gives_the_session_the_servers_email_so_the_binding_keeps_the_data() {
            let fx = Fixture::new();
            let owner = Identity::new(Some("u-1"), Some("Sam@beebeeb.io"));
            fx.db().set_owner(&owner).unwrap();
            for (name, session_email) in [
                ("an unidentified session", None),
                ("an older spelling", Some("sam@beebeeb.io")),
            ] {
                fx.install_session("tok-old", [7u8; 32], session_email);
                let canonical = profile("u-1", "Sam@beebeeb.io");
                reauth_swap_token(
                    &fx.state,
                    &fx.acct,
                    &fx.sources(),
                    "tok-new",
                    &canonical,
                    None,
                    &mut fx.acct.session_generation(),
                )
                .unwrap();
                let email = fx.acct.session.lock().unwrap().as_ref().unwrap().email.clone();
                assert_eq!(email.as_deref(), Some("Sam@beebeeb.io"), "{name}");
                let cached = fx.acct.cached_profile.lock().unwrap().clone();
                let identity = identity_of_session(email.as_deref(), cached.as_ref());
                assert_eq!(
                    account_binding::same_account(&owner, &identity),
                    Some(true),
                    "{name}: the engine start keeps the data"
                );
            }
        }

        // -- the account check --

        /// Against the real sources: each retained trace alone means the Mac is not fresh, and
        /// another account never gets `Fresh`. An empty Mac is `Fresh`.
        #[test]
        fn the_account_check_reads_every_retained_trace_from_the_real_sources() {
            use reauth::SignInKind::{DifferentAccount, NeedsKeyProof};
            type Set = fn(&Fixture);
            // What each retained trace alone makes of another account signing in: a switch, except a vault key, stored
            // or in a session in memory, which the server is asked about (it names no account itself; spec §5.6).
            let cases: [(&str, Set, reauth::SignInKind); 9] = [
                (
                    "a session in memory",
                    |fx| {
                        *fx.acct.session.lock().unwrap() = Some(Session {
                            token: "t".into(),
                            master_key: [1u8; 32],
                            email: None,
                        })
                    },
                    NeedsKeyProof,
                ),
                ("auth present", |fx| set_auth_present(&fx.state, true), DifferentAccount),
                (
                    "a Keychain token",
                    |fx| persist_session_token_to_keychain(&SessionWrite::for_test(), fx.id(), "tok-a", None).unwrap(),
                    DifferentAccount,
                ),
                (
                    "a Keychain email",
                    |fx| put_email(fx.id(), "a@beebeeb.io"),
                    DifferentAccount,
                ),
                (
                    "a vault key",
                    |fx| persist_vault_key_to_keychain(&SessionWrite::for_test(), fx.id(), &[3u8; 32]).unwrap(),
                    NeedsKeyProof,
                ),
                (
                    "a recorded owner",
                    |fx| fx.db().set_owner(&Identity::new(None, Some("a@beebeeb.io"))).unwrap(),
                    DifferentAccount,
                ),
                (
                    "a cached profile",
                    |fx| *fx.acct.cached_profile.lock().unwrap() = Some(profile("u-a", "a@beebeeb.io")),
                    DifferentAccount,
                ),
                (
                    "a queued change",
                    |fx| some_operation(&fx.db(), "op-trace"),
                    DifferentAccount,
                ),
                (
                    "a staged payload",
                    |fx| {
                        fx.db()
                            .track_staged_payload("/tmp/bb-trace-staged.bin", None, false)
                            .unwrap()
                    },
                    DifferentAccount,
                ),
            ];
            let empty = Fixture::new();
            assert_eq!(
                empty.classify(&kim()),
                (reauth::SignInKind::Fresh, 0),
                "nothing here: the first sign-in"
            );
            for (name, set, expected) in cases {
                let fx = Fixture::new();
                set(&fx);
                let (kind, _) = fx.classify(&kim());
                assert_ne!(
                    kind,
                    reauth::SignInKind::Fresh,
                    "{name} alone is a trace: another account must not get Fresh"
                );
                assert_eq!(kind, expected, "{name}");
            }
        }

        /// An unreadable local database is a trace, never an absence, and it settles nothing: the sign-in is
        /// `LocalDataUnreadable` (not `Fresh`, not a switch, not decided by the credentials; lead ruling 12 gave it its
        /// own outcome, where Task 11 had `Unconfirmed`).
        #[test]
        fn an_unreadable_state_db_is_a_trace_and_settles_nothing() {
            let fx = Fixture::new();
            std::fs::write(
                fx.dir.path().join("state.db"),
                b"this is not a database, and it is long enough to be read",
            )
            .unwrap();
            assert_eq!(fx.classify(&kim()).0, reauth::SignInKind::LocalDataUnreadable);
            assert!(gather_local_facts(&fx.state, &fx.acct, &fx.sources()).unreadable);
            assert!(
                queued_or_staged_present(&fx.sources()),
                "unreadable counts as waiting data"
            );
            assert!(local_data_owner(&fx.sources()).is_err());
            // Each fail-closed answer is its own trace, so neither depends on the other.
            let facts = gather_local_facts(&fx.state, &fx.acct, &fx.sources());
            assert!(
                facts.traces.recorded_owner,
                "an owner that cannot be read counts as recorded"
            );
            assert!(
                facts.traces.queued_or_staged,
                "data that cannot be read counts as waiting"
            );
            assert_eq!(facts.owner, None, "and names nobody");
        }

        /// The state database file of a fixture, opened directly: to break and repair it as a transient fault would.
        fn raw_state_db(fx: &Fixture) -> rusqlite::Connection {
            rusqlite::Connection::open(fx.dir.path().join("state.db")).unwrap()
        }

        /// A signed-in sam: credentials, a stored key, an owner record naming him, and one edit not uploaded yet.
        fn sam_with_a_pending_edit() -> Fixture {
            let fx = Fixture::new();
            after_a_first_sign_in(&fx, true, true);
            let db = fx.db();
            db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            some_operation(&db, "op-pending");
            fx
        }

        /// Lead ruling 12 (Task 12), end to end: a Mac whose owner record names sam by email only (another letter case)
        /// and that stores a vault key asks the server to prove the key. When the server answers with exactly
        /// `recovery_check_missing` (sam has no check on file), the email decides: sam signs in again in place, kim is a
        /// switch. When it answers `invalid_recovery_phrase`, it is the switch, whatever the email says.
        #[tokio::test]
        async fn no_recovery_check_on_file_lets_the_email_decide_and_a_mismatch_is_still_the_switch() {
            use reauth::SignInKind::{DifferentAccount, NeedsKeyProof, SameAccount};
            let fx = Fixture::new();
            after_a_first_sign_in(&fx, false, true);
            fx.db().set_owner(&Identity::new(None, Some("Sam@Beebeeb.io"))).unwrap();
            assert_eq!(
                fx.classify(&sam()).0,
                NeedsKeyProof,
                "the premise: the server's proof is asked first"
            );
            let (missing, asked) = serving("400 Bad Request", r#"{"error":"recovery_check_missing"}"#);
            assert_eq!(
                fx.decide(&sam(), "tok-2", &missing).await.0,
                SameAccount,
                "no check on file: the same address decides"
            );
            assert_eq!(
                fx.decide(&kim(), "tok-2", &missing).await.0,
                DifferentAccount,
                "and another address is a switch"
            );
            assert_eq!(asked.lock().unwrap().len(), 2, "the server was asked both times");
            let (mismatch, _) = serving(KEY_IS_ANOTHER_ACCOUNTS.0, KEY_IS_ANOTHER_ACCOUNTS.1);
            assert_eq!(
                fx.decide(&sam(), "tok-2", &mismatch).await.0,
                DifferentAccount,
                "a key that is not sam's is the switch"
            );
        }

        /// Spec §5.6: a `400` decides only with one of the server's two exact codes. A `400` with any other body (no
        /// JSON, another code, a near miss, a body the server's extractor rejected, an intermediary's page) says nothing
        /// about the kept key, so a same-account sign-in with a stored key settles nothing, as for a server error: the
        /// key is not called outdated, and nothing on this Mac changes. The same holds without a user id (rule 2).
        #[tokio::test]
        async fn a_400_without_one_of_the_two_codes_settles_nothing_and_the_kept_key_stays() {
            let not_an_answer: [&'static str; 5] = [
                "not json",
                r#"{"error":"bad_request"}"#,
                r#"{"error":"Invalid_Recovery_Phrase"}"#,
                "Failed to deserialize the JSON body into the target type: missing field `recovery_check`",
                "<html><body><h1>400 Bad Request</h1></body></html>",
            ];
            for body in not_an_answer {
                let fx = sam_with_a_pending_edit();
                let before = fx.snapshot();
                let (server, seen) = serving("400 Bad Request", body);
                assert_eq!(
                    fx.decide(&sam(), "tok-new", &server).await,
                    (reauth::SignInKind::Unconfirmed, 1),
                    "the same id and a stored key: {body:?} is not a mismatch"
                );
                assert_eq!(seen.lock().unwrap().len(), 1, "{body:?}: the server was asked once");
                assert_eq!(fx.snapshot(), before, "{body:?}: nothing changed, the kept key stays");
                assert_eq!(
                    stored_key(fx.id()),
                    Some([5u8; 32]),
                    "{body:?}: the key is still stored"
                );

                let fx = Fixture::new();
                persist_vault_key_to_keychain(&SessionWrite::for_test(), fx.id(), &[5u8; 32]).unwrap();
                fx.db().set_owner(&Identity::new(None, Some("sam@beebeeb.io"))).unwrap();
                let (server, _) = serving("400 Bad Request", body);
                assert_eq!(
                    fx.decide(&sam(), "tok-new", &server).await.0,
                    reauth::SignInKind::Unconfirmed,
                    "no user id here (rule 2): {body:?} is not a switch either"
                );
            }
        }

        /// An owner record or a `state.db` that cannot be read is neither "no owner" nor a switch: nothing is decided,
        /// nothing changes (the pending edit is not purged, the credentials do not decide), the server is not asked, and
        /// once the read works again the same account is signed in again in place.
        #[tokio::test]
        async fn an_unreadable_owner_record_or_state_db_settles_nothing_and_the_same_account_then_stays_in_place() {
            use reauth::SignInKind::{LocalDataUnreadable, SameAccount};
            let (server, seen) = serving(KEY_IS_THIS_ACCOUNTS.0, KEY_IS_THIS_ACCOUNTS.1);
            let pending_edits = |fx: &Fixture| {
                raw_state_db(fx)
                    .query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get::<_, i64>(0))
                    .unwrap()
            };

            // (A) The owner record alone cannot be read: its value is not text. Everything else in the database is fine.
            let fx = sam_with_a_pending_edit();
            raw_state_db(&fx)
                .execute("UPDATE sync_state SET value = x'ff00' WHERE key = 'owner_user_id'", [])
                .unwrap();
            assert!(
                local_data_owner(&fx.sources()).is_err(),
                "the premise: the owner cannot be read, the database opens"
            );
            let before = fx.snapshot_without_reading_state_db();
            assert_eq!(
                fx.decide(&sam(), "tok-2", &server).await,
                (LocalDataUnreadable, 1),
                "the credentials name sam and a key is stored: still nothing is decided"
            );
            assert_eq!(
                fx.decide(&kim(), "tok-2", &server).await.0,
                LocalDataUnreadable,
                "and another account is not switched either"
            );
            assert!(seen.lock().unwrap().is_empty(), "the server is not asked");
            assert_eq!(
                fx.snapshot_without_reading_state_db(),
                before,
                "nothing changed in the Keychain, memory or the flags"
            );
            assert_eq!(pending_edits(&fx), 1, "the pending edit is still there");
            // The read works again: the same account stays in place.
            raw_state_db(&fx)
                .execute("UPDATE sync_state SET value = 'u-1' WHERE key = 'owner_user_id'", [])
                .unwrap();
            assert_eq!(fx.decide(&sam(), "tok-2", &server).await, (SameAccount, 1));
            assert_eq!(
                seen.lock().unwrap().len(),
                1,
                "the ids match and a key is stored: the server proves it (spec §5.6)"
            );
            seen.lock().unwrap().clear();
            reauth_swap_token(
                &fx.state,
                &fx.acct,
                &fx.sources(),
                "tok-2",
                &sam(),
                None,
                &mut fx.acct.session_generation(),
            )
            .unwrap();
            assert_eq!(stored_token(fx.id()).as_deref(), Some("tok-2"));
            assert_eq!(stored_key(fx.id()), Some([5u8; 32]));
            assert_eq!(
                fx.db().owner().unwrap(),
                Some(Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
            );
            assert_eq!(pending_edits(&fx), 1, "and still the edit");

            // (B) The whole database cannot be read: moved aside, a file that is not a database in its place.
            let fx = sam_with_a_pending_edit();
            let aside = |suffix: &str| {
                (
                    fx.dir.path().join(format!("state.db{suffix}")),
                    fx.dir.path().join(format!("state.db{suffix}.valid")),
                )
            };
            for suffix in ["", "-wal", "-shm"] {
                let (from, to) = aside(suffix);
                if from.exists() {
                    std::fs::rename(from, to).unwrap();
                }
            }
            let garbage = b"this is not a database, and it is long enough to be read".to_vec();
            std::fs::write(fx.dir.path().join("state.db"), &garbage).unwrap();
            let before = fx.snapshot_without_reading_state_db();
            assert_eq!(fx.decide(&sam(), "tok-2", &server).await.0, LocalDataUnreadable);
            assert_eq!(fx.decide(&kim(), "tok-2", &server).await.0, LocalDataUnreadable);
            assert!(seen.lock().unwrap().is_empty(), "the server is not asked");
            assert_eq!(fx.snapshot_without_reading_state_db(), before);
            assert_eq!(
                std::fs::read(fx.dir.path().join("state.db")).unwrap(),
                garbage,
                "the unreadable file is not touched"
            );
            // The read works again: the real database is back, and nothing of it was purged.
            std::fs::remove_file(fx.dir.path().join("state.db")).unwrap();
            for suffix in ["", "-wal", "-shm"] {
                let (back, valid) = aside(suffix);
                if valid.exists() {
                    std::fs::rename(valid, back).unwrap();
                }
            }
            assert_eq!(fx.decide(&sam(), "tok-2", &server).await, (SameAccount, 1));
            assert_eq!(pending_edits(&fx), 1, "the pending edit survived the whole episode");
        }

        /// A test that works in the shared scratch state directory holds the gate the sign-out purge takes, and the marker
        /// that tells that sign-out so (otherwise its own sign-out would wait for the gate its test holds, for ever).
        #[tokio::test]
        async fn a_test_in_the_shared_state_dir_holds_the_purge_gate_and_says_so() {
            assert!(PURGE_GATE_HELD.try_with(|_| ()).is_err(), "no gate is held outside");
            with_the_shared_state_dir(async {
                assert!(
                    SIGN_OUT_TEST_PURGE_GATE.try_lock().is_err(),
                    "the gate is held for the whole body"
                );
                assert!(PURGE_GATE_HELD.try_with(|_| ()).is_ok(), "and the sign-out can tell");
            })
            .await;
            assert!(
                PURGE_GATE_HELD.try_with(|_| ()).is_err(),
                "the marker goes with the body"
            );
        }

        // -- the identity rule: ids, then the server's key proof, then the email in its canonical form (ruling A″) --

        /// Rule 2, the legacy owner: an older build recorded the lowercase text of the address, the server's own spelling
        /// has a capital, and the key is still stored on this Mac. The server proves the key is this account's, and the
        /// sign-in is in place: nothing is purged, the key stays, and the record is completed with the user id and the
        /// server's spelling. The proof is asked as the account that is signing in, about the key this Mac stores.
        #[tokio::test]
        async fn a_key_the_server_proves_signs_a_legacy_email_only_owner_in_again_in_place_whatever_the_letter_case() {
            let fx = Fixture::new();
            let id = fx.id().to_string();
            persist_vault_key_to_keychain(&SessionWrite::for_test(), &id, &[5u8; 32]).unwrap();
            put_email(&id, "sam@beebeeb.io");
            let db = fx.db();
            db.set_owner(&Identity::new(None, Some("sam@beebeeb.io"))).unwrap();
            some_operation(&db, "op-legacy");
            let (server, seen) = serving(KEY_IS_THIS_ACCOUNTS.0, KEY_IS_THIS_ACCOUNTS.1);
            let signing_in = profile("u-1", "Sam@beebeeb.io");

            assert_eq!(
                fx.decide(&signing_in, "tok-new", &server).await,
                (reauth::SignInKind::SameAccount, 1)
            );
            {
                let requests = seen.lock().unwrap();
                assert_eq!(requests.len(), 1, "one question to the server");
                assert!(
                    requests[0].starts_with("POST /api/v1/auth/verify-recovery-check "),
                    "{}",
                    requests[0]
                );
                assert!(
                    requests[0].to_lowercase().contains("authorization: bearer tok-new"),
                    "asked as the account that is signing in"
                );
                assert!(
                    requests[0].contains(&recovery_check_of([5u8; 32])),
                    "about the key this Mac stores"
                );
            }

            let unlocked = reauth_swap_token(
                &fx.state,
                &fx.acct,
                &fx.sources(),
                "tok-new",
                &signing_in,
                None,
                &mut fx.acct.session_generation(),
            )
            .unwrap();
            assert!(unlocked, "the stored key is loaded: no recovery phrase");
            assert_eq!(
                db.owner().unwrap(),
                Some(Identity::new(Some("u-1"), Some("Sam@beebeeb.io"))),
                "the record gets the id and the server's spelling"
            );
            assert_eq!(
                db.queued_or_staged_count().unwrap(),
                1,
                "no purge: the pending edit stays"
            );
            assert_eq!(stored_key(&id), Some([5u8; 32]), "the key stays");
            assert_eq!(stored_token(&id).as_deref(), Some("tok-new"));
            assert_eq!(keychain_account_email(&id).as_deref(), Some("Sam@beebeeb.io"));
            let session = fx.acct.session.lock().unwrap();
            let session = session.as_ref().expect("the keys are in memory again");
            assert_eq!((session.token.as_str(), session.master_key), ("tok-new", [5u8; 32]));
        }

        /// Rule 2, a key that is not this account's: a different account, and nothing local changes.
        #[tokio::test]
        async fn a_key_the_server_says_is_another_accounts_makes_the_sign_in_a_switch() {
            let fx = Fixture::new();
            persist_vault_key_to_keychain(&SessionWrite::for_test(), fx.id(), &[5u8; 32]).unwrap();
            put_email(fx.id(), "sam@beebeeb.io");
            let db = fx.db();
            db.set_owner(&Identity::new(None, Some("sam@beebeeb.io"))).unwrap();
            some_operation(&db, "op-legacy");
            let before = fx.snapshot();
            let (server, seen) = serving(KEY_IS_ANOTHER_ACCOUNTS.0, KEY_IS_ANOTHER_ACCOUNTS.1);

            let decision = fx.decide(&profile("u-1", "Sam@beebeeb.io"), "tok-new", &server).await;

            assert_eq!(
                decision,
                (reauth::SignInKind::DifferentAccount, 1),
                "the same letters, but not this account's key"
            );
            assert_eq!(seen.lock().unwrap().len(), 1);
            assert_eq!(fx.snapshot(), before, "nothing changed on this Mac");
        }

        /// Rule 2, no answer: nothing is swapped and nothing is switched. A broken server, a refused connection and a
        /// stored key that cannot be read are all "cannot confirm", and nothing local changes.
        #[tokio::test]
        async fn a_proof_that_cannot_complete_changes_nothing_and_decides_nothing() {
            let (broken, _) = serving(SERVER_IS_BROKEN.0, SERVER_IS_BROKEN.1);
            for (name, server) in [("a broken server", broken), ("a refused connection", nobody_listens())] {
                let fx = Fixture::new();
                persist_vault_key_to_keychain(&SessionWrite::for_test(), fx.id(), &[5u8; 32]).unwrap();
                put_email(fx.id(), "sam@beebeeb.io");
                let db = fx.db();
                db.set_owner(&Identity::new(None, Some("sam@beebeeb.io"))).unwrap();
                some_operation(&db, "op-legacy");
                let before = fx.snapshot();
                let decision = fx.decide(&profile("u-1", "Sam@beebeeb.io"), "tok-new", &server).await;
                assert_eq!(decision.0, reauth::SignInKind::Unconfirmed, "{name}");
                assert_eq!(fx.snapshot(), before, "{name}: nothing changed on this Mac");
            }
            // A key item that cannot be read (here: not 32 bytes) cannot be proven either, and the server is not asked.
            let fx = Fixture::new();
            AuthVault::new(platform_keychain_store_for(fx.id()))
                .store_wrapped_master_key(keychain::SecretBytes::new(vec![1u8, 2, 3]).unwrap())
                .unwrap();
            fx.db().set_owner(&Identity::new(None, Some("sam@beebeeb.io"))).unwrap();
            let (server, seen) = serving(KEY_IS_THIS_ACCOUNTS.0, KEY_IS_THIS_ACCOUNTS.1);
            assert_eq!(
                fx.decide(&profile("u-1", "sam@beebeeb.io"), "tok-new", &server).await.0,
                reauth::SignInKind::Unconfirmed
            );
            assert!(seen.lock().unwrap().is_empty(), "an unreadable key is never sent");
        }

        /// Rule 1 (spec §5.6): another user id is a switch and the server is never asked. The same id with a vault key
        /// here is one question to the server, about that key, asked as the account that is signing in: a key it
        /// confirms is in place, one it no longer accepts is `KeyOutdated`. The same id with no key here asks nothing.
        #[tokio::test]
        async fn with_the_same_id_the_server_proves_the_kept_key_and_another_id_asks_nothing() {
            use reauth::SignInKind::{DifferentAccount, KeyOutdated, SameAccount};
            let cases = [
                (
                    "the same id, a key it confirms",
                    "u-1",
                    true,
                    KEY_IS_THIS_ACCOUNTS,
                    SameAccount,
                    1,
                ),
                (
                    "the same id, a key it no longer accepts",
                    "u-1",
                    true,
                    KEY_IS_ANOTHER_ACCOUNTS,
                    KeyOutdated,
                    1,
                ),
                (
                    "the same id, no key here",
                    "u-1",
                    false,
                    KEY_IS_ANOTHER_ACCOUNTS,
                    SameAccount,
                    0,
                ),
                (
                    "another id, a key here",
                    "u-9",
                    true,
                    KEY_IS_THIS_ACCOUNTS,
                    DifferentAccount,
                    0,
                ),
            ];
            for (name, owner_id, key_here, answer, expected, questions) in cases {
                let fx = Fixture::new();
                if key_here {
                    persist_vault_key_to_keychain(&SessionWrite::for_test(), fx.id(), &[5u8; 32]).unwrap();
                }
                fx.db()
                    .set_owner(&Identity::new(Some(owner_id), Some("sam@beebeeb.io")))
                    .unwrap();
                let (server, seen) = serving(answer.0, answer.1);
                assert_eq!(
                    fx.decide(&profile("u-1", "sam@beebeeb.io"), "tok-new", &server).await.0,
                    expected,
                    "{name}"
                );
                let asked = seen.lock().unwrap();
                assert_eq!(asked.len(), questions, "{name}: questions to the server");
                if let Some(request) = asked.first() {
                    assert!(
                        request.contains(&recovery_check_of([5u8; 32])),
                        "{name}: about the kept key"
                    );
                    assert!(
                        request.to_lowercase().contains("authorization: bearer tok-new"),
                        "{name}: asked as the account that is signing in"
                    );
                }
            }
        }

        /// A stand-in reconciler that records what it is told (and whether the engine was still in its slot then), and
        /// acknowledges every Lock and every removal at once.
        fn recording_reconciler(fx: &Fixture) -> Arc<Mutex<Vec<String>>> {
            use finder_setup::driver::{Event, FinderSetupHandle, FinderSetupView};
            let (handle, mut rx) = FinderSetupHandle::for_test(FinderSetupView::initial(
                finder_setup::launch_location::LaunchLocation::Applications,
            ));
            let _ = fx.state.finder_setup.set(handle);
            let seen: Arc<Mutex<Vec<String>>> = Arc::default();
            let (record, acct) = (seen.clone(), fx.acct.clone());
            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    let engine = match acct.engine.try_lock() {
                        Ok(slot) if slot.is_some() => "engine_tracked",
                        Ok(_) => "no_engine",
                        Err(_) => "slot_busy",
                    };
                    let line = match event {
                        Event::Lock { ack } => {
                            let _ = ack.send(());
                            format!("lock:{engine}")
                        }
                        Event::Remove { trigger, ack } => {
                            let _ = ack.send((Ok(()), finder_removal::KeptFolder::default()));
                            format!("remove:{}", trigger.as_str())
                        }
                        Event::Trigger(trigger) => format!("trigger:{}", trigger.as_str()),
                        Event::AppActivated => "activated".to_string(),
                    };
                    record.lock().unwrap().push(line);
                }
            });
            seen
        }

        const KEY_REPLACED: SignInSettlement = SignInSettlement::Reauthenticated {
            vault_unlocked: false,
            key_replaced: true,
        };

        /// Spec §5.6, a stored key: after a startup 401 this Mac kept sam's key, and meanwhile sam's key was changed on
        /// another device. Sam signs in again: the server says the kept key is no longer his, so the new token is stored
        /// WITHOUT it, the key is gone from both Keychain layouts and is not in memory, the recovery phrase is asked
        /// (`key_replaced`), and the pending edit and the owner stay. Nothing asks Finder to remove anything.
        #[tokio::test]
        async fn a_stored_key_the_server_no_longer_accepts_is_removed_and_never_installed() {
            let fx = sam_with_a_pending_edit();
            let id = fx.id().to_string();
            let reconciler = recording_reconciler(&fx);
            let (server, seen) = serving(KEY_IS_ANOTHER_ACCOUNTS.0, KEY_IS_ANOTHER_ACCOUNTS.1);
            let mut turn = fx.acct.session_generation();

            let (kind, pending_changes) = fx.decide(&sam(), "tok-new", &server).await;
            assert_eq!((kind, pending_changes), (reauth::SignInKind::KeyOutdated, 1));
            assert_eq!(seen.lock().unwrap().len(), 1, "one question, about the kept key");
            let settled = reauth_without_the_kept_key(&fx.state, &fx.acct, &fx.sources(), "tok-new", &sam(), &mut turn)
                .await
                .unwrap();

            assert_eq!(settled, KEY_REPLACED, "the recovery phrase follows, and says why");
            assert_eq!(stored_key(&id), None, "the outdated key is gone");
            assert!(!keychain_vault_key_present(&id), "from both Keychain layouts");
            assert!(fx.acct.session.lock().unwrap().is_none(), "and no key is in memory");
            assert_eq!(
                stored_token(&id).as_deref(),
                Some("tok-new"),
                "the new token is the stored one"
            );
            assert_eq!(keychain_account_email(&id).as_deref(), Some("sam@beebeeb.io"));
            let db = fx.db();
            assert_eq!(db.queued_or_staged_count().unwrap(), 1, "the pending edit stays");
            assert_eq!(
                db.owner().unwrap(),
                Some(Identity::new(Some("u-1"), Some("sam@beebeeb.io"))),
                "the same owner"
            );
            assert!(*fx.state.auth_present.lock().unwrap(), "signed in");
            tokio::task::yield_now().await;
            assert!(
                reconciler
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|event| !event.starts_with("remove")),
                "Finder is never asked to remove: {:?}",
                reconciler.lock().unwrap()
            );
        }

        /// Spec §5.6, a key in memory: sam is unlocked, an engine runs with his key, and the Keychain kept no key. His key
        /// was changed on another device. The proof is asked about the key in memory; the server no longer accepts it, so
        /// the session ends the way Lock ends it: the reconciler is told to hold while the engine is still in its slot
        /// (never to remove), then the engine stops, and no key is left in memory or in the Keychain.
        #[tokio::test]
        async fn a_key_in_memory_the_server_no_longer_accepts_ends_the_way_lock_ends_it() {
            let fx = Fixture::new();
            let id = fx.id().to_string();
            persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-old", Some("sam@beebeeb.io"))
                .unwrap();
            fx.install_session("tok-old", [7u8; 32], Some("sam@beebeeb.io"));
            let db = fx.db();
            db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            some_operation(&db, "op-pending");
            let task = tokio::spawn(async {});
            tokio::task::yield_now().await;
            *fx.acct.engine.lock().await = Some(EngineRunner::for_test_with_task(task));
            let reconciler = recording_reconciler(&fx);
            let (server, seen) = serving(KEY_IS_ANOTHER_ACCOUNTS.0, KEY_IS_ANOTHER_ACCOUNTS.1);
            let mut turn = fx.acct.session_generation();

            let (kind, _) = fx.decide(&sam(), "tok-new", &server).await;
            assert_eq!(kind, reauth::SignInKind::KeyOutdated);
            {
                let asked = seen.lock().unwrap();
                assert_eq!(asked.len(), 1);
                assert!(
                    asked[0].contains(&recovery_check_of([7u8; 32])),
                    "the key in memory is the one proved"
                );
            }
            let settled = reauth_without_the_kept_key(&fx.state, &fx.acct, &fx.sources(), "tok-new", &sam(), &mut turn)
                .await
                .unwrap();

            assert_eq!(settled, KEY_REPLACED);
            assert!(
                fx.acct.session.lock().unwrap().is_none(),
                "the outdated key left memory"
            );
            assert!(fx.acct.engine.lock().await.is_none(), "the engine stopped");
            assert_eq!(fx.acct.engine_state.lock().unwrap().as_str(), "stopped");
            assert!(!keychain_vault_key_present(&id), "no key in the Keychain");
            assert_eq!(stored_token(&id).as_deref(), Some("tok-new"));
            assert_eq!(db.queued_or_staged_count().unwrap(), 1, "the pending edit stays");
            tokio::task::yield_now().await;
            assert_eq!(
                *reconciler.lock().unwrap(),
                vec!["lock:engine_tracked".to_string()],
                "held before the engine stopped, and nothing removed"
            );
        }

        /// Spec §5.6: the proof is about the key a same-account sign-in keeps. With a session in memory that is the key
        /// in memory (`reauth_swap_token` keeps it), even when the Keychain holds another one. Here memory holds one key,
        /// the Keychain another, and the server accepts only the Keychain's: the key in memory is the one proved, the
        /// server does not accept it, so the sign-in is `KeyOutdated` (never `SameAccount`), and the key in memory is
        /// never used with the new session.
        #[tokio::test]
        async fn with_a_session_in_memory_the_key_in_memory_is_the_one_proved() {
            let fx = Fixture::new();
            let id = fx.id().to_string();
            persist_vault_key_to_keychain(&SessionWrite::for_test(), &id, &[5u8; 32]).unwrap();
            fx.install_session("tok-old", [7u8; 32], Some("sam@beebeeb.io"));
            fx.db()
                .set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            let _reconciler = recording_reconciler(&fx);
            let (server, seen) = confirming_only([5u8; 32]);
            let mut turn = fx.acct.session_generation();

            let (kind, _) = fx.decide(&sam(), "tok-new", &server).await;
            {
                let asked = seen.lock().unwrap();
                assert_eq!(asked.len(), 1, "one question to the server");
                assert!(
                    asked[0].contains(&recovery_check_of([7u8; 32])),
                    "about the key in memory, the one the sign-in keeps"
                );
                assert!(
                    !asked[0].contains(&recovery_check_of([5u8; 32])),
                    "not about the Keychain's"
                );
            }
            assert_eq!(
                kind,
                reauth::SignInKind::KeyOutdated,
                "the server does not accept the key that would be kept"
            );
            let settled = reauth_without_the_kept_key(&fx.state, &fx.acct, &fx.sources(), "tok-new", &sam(), &mut turn)
                .await
                .unwrap();
            assert_eq!(settled, KEY_REPLACED);
            assert!(
                fx.acct.session.lock().unwrap().is_none(),
                "the key in memory is not used with the new session"
            );
            assert_eq!(stored_token(&id).as_deref(), Some("tok-new"));
            assert!(!keychain_vault_key_present(&id), "and no key is left in the Keychain");
        }

        /// Rule 2 (spec §5.6): with no user id on this Mac's side, a key in memory is proved like a stored one, never
        /// left to the email. An email-only owner, an unlocked session and no key in the Keychain: the server is asked
        /// about the key in memory. A key it confirms is in place; one it says is not this account's is a switch, though
        /// the email is the same; with no check on file, the email decides.
        #[tokio::test]
        async fn without_a_user_id_a_key_in_memory_is_proved_not_left_to_the_email() {
            use reauth::SignInKind::{DifferentAccount, NeedsKeyProof, SameAccount};
            let cases = [
                ("a key the server confirms", KEY_IS_THIS_ACCOUNTS, SameAccount),
                (
                    "a key the server says is another account's",
                    KEY_IS_ANOTHER_ACCOUNTS,
                    DifferentAccount,
                ),
                (
                    "no check on file: the same address decides",
                    ("400 Bad Request", r#"{"error":"recovery_check_missing"}"#),
                    SameAccount,
                ),
            ];
            for (name, answer, expected) in cases {
                let fx = Fixture::new();
                fx.install_session("tok-old", [7u8; 32], Some("sam@beebeeb.io"));
                fx.db().set_owner(&Identity::new(None, Some("sam@beebeeb.io"))).unwrap();
                assert!(
                    !keychain_vault_key_present(fx.id()),
                    "the premise: no key in the Keychain"
                );
                assert_eq!(
                    fx.classify(&sam()).0,
                    NeedsKeyProof,
                    "{name}: rule 2 asks the server, the email rule does not decide"
                );
                let (server, seen) = serving(answer.0, answer.1);
                assert_eq!(fx.decide(&sam(), "tok-new", &server).await.0, expected, "{name}");
                let asked = seen.lock().unwrap();
                assert_eq!(asked.len(), 1, "{name}: one question to the server");
                assert!(
                    asked[0].contains(&recovery_check_of([7u8; 32])),
                    "{name}: about the key in memory"
                );
            }
        }

        /// A browser sign-in hands over the account's current key. When the key this Mac kept (and the server
        /// confirmed, or could not check) is not that key, the kept one is not used: `reauth_in_place` returns before it
        /// starts an engine, and the handoff installs its own key (`apply_session`). The same key, or a password
        /// sign-in (no key handed over), keeps the kept one.
        #[tokio::test]
        async fn a_handed_over_key_that_differs_from_the_kept_one_replaces_it() {
            let fx = Fixture::new();
            let id = fx.id().to_string();
            persist_vault_key_to_keychain(&SessionWrite::for_test(), &id, &[5u8; 32]).unwrap();
            put_email(&id, "sam@beebeeb.io");
            fx.db()
                .set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            let (server, _) = serving(KEY_IS_THIS_ACCOUNTS.0, KEY_IS_THIS_ACCOUNTS.1);
            assert_eq!(
                fx.decide(&sam(), "tok-new", &server).await.0,
                reauth::SignInKind::SameAccount
            );
            let unlocked = reauth_swap_token(
                &fx.state,
                &fx.acct,
                &fx.sources(),
                "tok-new",
                &sam(),
                None,
                &mut fx.acct.session_generation(),
            )
            .unwrap();
            assert!(unlocked, "the kept key is in memory after the swap");
            assert!(
                handoff_replaces_kept_key(&fx.acct, Some(&[6u8; 32])),
                "another key handed over replaces the kept one"
            );
            assert!(
                !handoff_replaces_kept_key(&fx.acct, Some(&[5u8; 32])),
                "the same key handed over keeps it"
            );
            assert!(
                !handoff_replaces_kept_key(&fx.acct, None),
                "a password sign-in hands over no key"
            );
            let mut one_bit_off = [5u8; 32];
            one_bit_off[31] ^= 1;
            assert!(
                handoff_replaces_kept_key(&fx.acct, Some(&one_bit_off)),
                "every byte is compared"
            );
        }

        /// Rule 3: no id and no stored key. The emails are compared in their canonical form (ruling A″; ASCII-only before):
        /// a letter-case variant, ASCII or not, is the same account (and the record is completed); another address, or
        /// an accent that is not a letter case, is not. The server is never asked.
        #[tokio::test]
        async fn without_an_id_or_a_key_the_email_decides_in_its_canonical_form() {
            use reauth::SignInKind::*;
            let cases = [
                ("the same address", "sam@beebeeb.io", "sam@beebeeb.io", SameAccount),
                (
                    "an ASCII letter-case variant",
                    "sam@beebeeb.io",
                    "Sam@Beebeeb.IO",
                    SameAccount,
                ),
                ("another address", "kim@beebeeb.io", "sam@beebeeb.io", DifferentAccount),
                (
                    "a non-ASCII case variant is the same account",
                    "josé@beebeeb.io",
                    "JOSÉ@beebeeb.io",
                    SameAccount,
                ),
                (
                    "an accent is not a letter case",
                    "josé@beebeeb.io",
                    "jose@beebeeb.io",
                    DifferentAccount,
                ),
            ];
            for (name, stored, server_spelling, expected) in cases {
                let fx = Fixture::new();
                fx.db().set_owner(&Identity::new(None, Some(stored))).unwrap();
                let (server, seen) = serving(KEY_IS_THIS_ACCOUNTS.0, KEY_IS_THIS_ACCOUNTS.1);
                let signing_in = profile("u-1", server_spelling);
                assert_eq!(fx.decide(&signing_in, "tok-new", &server).await.0, expected, "{name}");
                assert!(
                    seen.lock().unwrap().is_empty(),
                    "{name}: without a stored key the server is not asked"
                );
                if expected == SameAccount {
                    reauth_swap_token(
                        &fx.state,
                        &fx.acct,
                        &fx.sources(),
                        "tok-new",
                        &signing_in,
                        None,
                        &mut fx.acct.session_generation(),
                    )
                    .unwrap();
                    assert_eq!(
                        fx.db().owner().unwrap(),
                        Some(Identity::new(Some("u-1"), Some(server_spelling))),
                        "{name}: the record is completed"
                    );
                }
            }
        }

        /// What a first sign-in leaves behind before its engine has started: a token and an email in the Keychain, the
        /// auth flags and (until a relaunch) the cached profile. No owner record, no local data.
        fn after_a_first_sign_in(fx: &Fixture, cached_profile: bool, vault_key: bool) {
            persist_session_token_to_keychain(&SessionWrite::for_test(), fx.id(), "tok-1", Some("sam@beebeeb.io"))
                .unwrap();
            set_auth_present(&fx.state, true);
            set_auth_email(&fx.state, Some("sam@beebeeb.io".into()));
            if cached_profile {
                *fx.acct.cached_profile.lock().unwrap() = Some(sam());
            }
            if vault_key {
                persist_vault_key_to_keychain(&SessionWrite::for_test(), fx.id(), &[5u8; 32]).unwrap();
            }
        }

        /// Signing in twice as the same account before the first engine has started is NOT a switch: the credentials of
        /// the first sign-in name the account, and the same order decides (ids, then the server's proof, then the
        /// email). No warning, no sign-out, nothing created: the token is replaced and the recovery phrase step goes on.
        #[tokio::test]
        async fn signing_in_twice_as_the_same_account_before_the_first_engine_starts_is_in_place() {
            use reauth::SignInKind::SameAccount;
            // (name, a cached profile is held, a vault key is stored, the sign-in's email, requests the server must get)
            let worlds = [
                ("straight after the first sign-in", true, false, "sam@beebeeb.io", 0),
                (
                    "straight after, the account's email has changed since: the ids decide",
                    true,
                    false,
                    "renamed@beebeeb.io",
                    0,
                ),
                (
                    "after a relaunch (no cached profile): the email decides",
                    false,
                    false,
                    "sam@beebeeb.io",
                    0,
                ),
                (
                    "after a relaunch, another letter case: the email decides",
                    false,
                    false,
                    "Sam@Beebeeb.IO",
                    0,
                ),
                (
                    "the recovery phrase is entered, profile cached: the ids match and the server proves the key",
                    true,
                    true,
                    "sam@beebeeb.io",
                    1,
                ),
                (
                    "the recovery phrase is entered, after a relaunch: the server proves the key",
                    false,
                    true,
                    "sam@beebeeb.io",
                    1,
                ),
            ];
            for (name, cached_profile, vault_key, email, requests) in worlds {
                let fx = Fixture::new();
                after_a_first_sign_in(&fx, cached_profile, vault_key);
                // An empty `state.db` (created, no rows, no owner) is not data either.
                let _ = fx.db();
                let (server, seen) = serving(KEY_IS_THIS_ACCOUNTS.0, KEY_IS_THIS_ACCOUNTS.1);
                let signing_in = profile("u-1", email);
                let before = fx.snapshot();
                assert_eq!(
                    fx.decide(&signing_in, "tok-2", &server).await,
                    (SameAccount, 0),
                    "{name}"
                );
                assert_eq!(seen.lock().unwrap().len(), requests, "{name}: requests to the server");
                assert_eq!(fx.snapshot(), before, "{name}: deciding changes nothing");

                let unlocked = reauth_swap_token(
                    &fx.state,
                    &fx.acct,
                    &fx.sources(),
                    "tok-2",
                    &signing_in,
                    None,
                    &mut fx.acct.session_generation(),
                )
                .unwrap();
                assert_eq!(
                    unlocked, vault_key,
                    "{name}: keys come back only if this Mac holds the key"
                );
                assert_eq!(
                    stored_token(fx.id()).as_deref(),
                    Some("tok-2"),
                    "{name}: only the token is replaced"
                );
                assert_eq!(
                    stored_key(fx.id()),
                    vault_key.then_some([5u8; 32]),
                    "{name}: the key is as it was"
                );
                assert!(*fx.state.auth_present.lock().unwrap(), "{name}");
            }
            // No state.db at all: the swap does not create one (the engine's first start records the owner).
            let fx = Fixture::new();
            after_a_first_sign_in(&fx, true, false);
            reauth_swap_token(
                &fx.state,
                &fx.acct,
                &fx.sources(),
                "tok-2",
                &sam(),
                None,
                &mut fx.acct.session_generation(),
            )
            .unwrap();
            assert!(!fx.dir.path().join("state.db").exists(), "nothing is invented");
        }

        /// A session in memory names its account when nothing else does: the Keychain holds no email (that write is
        /// allowed to fail) and no profile is cached. Its vault key is proved first (rule 2, spec §5.6): the key the
        /// server confirms is in place, one it says is another account's is a switch. When the account has no check on
        /// file, the session's email decides: the same account signs in in place, another one is a switch.
        #[tokio::test]
        async fn a_session_in_memory_names_the_account_when_the_keychain_holds_no_email() {
            use reauth::SignInKind::{DifferentAccount, SameAccount};
            let fx = Fixture::new();
            fx.install_session("tok-1", [1u8; 32], Some("sam@beebeeb.io"));
            assert_eq!(keychain_account_email(fx.id()), None);
            let sam_spelled = profile("u-1", "Sam@beebeeb.io");
            let (confirms, seen) = serving(KEY_IS_THIS_ACCOUNTS.0, KEY_IS_THIS_ACCOUNTS.1);
            assert_eq!(fx.decide(&sam_spelled, "tok-2", &confirms).await.0, SameAccount);
            let (rejects, _) = serving(KEY_IS_ANOTHER_ACCOUNTS.0, KEY_IS_ANOTHER_ACCOUNTS.1);
            assert_eq!(fx.decide(&kim(), "tok-2", &rejects).await.0, DifferentAccount);
            let (missing, _) = serving("400 Bad Request", r#"{"error":"recovery_check_missing"}"#);
            assert_eq!(
                fx.decide(&sam_spelled, "tok-2", &missing).await.0,
                SameAccount,
                "no check on file: the session's email names sam"
            );
            assert_eq!(
                fx.decide(&kim(), "tok-2", &missing).await.0,
                DifferentAccount,
                "and kim is a switch"
            );
            let asked = seen.lock().unwrap();
            assert_eq!(asked.len(), 1);
            assert!(
                asked[0].contains(&recovery_check_of([1u8; 32])),
                "the proof is about the key in memory"
            );
        }

        /// Another account in that same window is a switch, by the same order: another id, another email, or a key the
        /// server says is not this account's. Nothing changes.
        #[tokio::test]
        async fn another_account_before_the_first_engine_starts_is_a_switch() {
            use reauth::SignInKind::DifferentAccount;
            let worlds = [
                (
                    "straight after the first sign-in: another id",
                    true,
                    false,
                    KEY_IS_THIS_ACCOUNTS,
                    0,
                ),
                ("after a relaunch: another email", false, false, KEY_IS_THIS_ACCOUNTS, 0),
                (
                    "the phrase is entered, profile cached: another id, the server is not asked",
                    true,
                    true,
                    KEY_IS_THIS_ACCOUNTS,
                    0,
                ),
                (
                    "the phrase is entered, after a relaunch: the server says it is not this account's key",
                    false,
                    true,
                    KEY_IS_ANOTHER_ACCOUNTS,
                    1,
                ),
            ];
            for (name, cached_profile, vault_key, answer, requests) in worlds {
                let fx = Fixture::new();
                after_a_first_sign_in(&fx, cached_profile, vault_key);
                let (server, seen) = serving(answer.0, answer.1);
                let before = fx.snapshot();
                assert_eq!(fx.decide(&kim(), "tok-2", &server).await.0, DifferentAccount, "{name}");
                assert_eq!(seen.lock().unwrap().len(), requests, "{name}: requests to the server");
                assert_eq!(fx.snapshot(), before, "{name}: nothing changed on this Mac");
            }
        }

        /// Unowned local DATA is still a switch, whatever the credentials say: rows of `state.db` (the queue, a file
        /// row, a staged payload). The server is not asked, and nothing changes. (A database that cannot be read is not
        /// this: see `an_unreadable_owner_record_or_state_db_settles_nothing…`.)
        #[tokio::test]
        async fn unowned_local_data_is_a_switch_whatever_the_credentials_say() {
            use reauth::SignInKind::DifferentAccount;
            type Plant = fn(&Fixture);
            let data: [(&str, Plant); 3] = [
                ("a queued change", |fx| some_operation(&fx.db(), "op-unowned")),
                ("a file row", |fx| {
                    fx.db()
                        .upsert_file(&state_db::FileEntry {
                            file_id: "file-unowned".into(),
                            path: "/Unowned.txt".into(),
                            status: state_db::FileStatus::Local,
                            size_bytes: 1,
                            modified_at: 0,
                            content_hash: None,
                            remote_updated_at: 0,
                            parent_id: None,
                            item_kind: state_db::ItemKind::File,
                        })
                        .unwrap()
                }),
                ("a staged payload", |fx| {
                    fx.db()
                        .track_staged_payload("/tmp/bb-unowned-staged.bin", None, false)
                        .unwrap()
                }),
            ];
            for (name, plant) in data {
                // The credentials name the very account that is signing in, and a key is stored: the strongest case for
                // "the same account", and still a switch.
                let fx = Fixture::new();
                after_a_first_sign_in(&fx, true, true);
                plant(&fx);
                let (server, seen) = serving(KEY_IS_THIS_ACCOUNTS.0, KEY_IS_THIS_ACCOUNTS.1);
                let before = fx.snapshot_without_reading_state_db();
                assert_eq!(fx.decide(&sam(), "tok-2", &server).await.0, DifferentAccount, "{name}");
                assert!(
                    seen.lock().unwrap().is_empty(),
                    "{name}: unowned data is not for the server to settle"
                );
                assert_eq!(
                    fx.snapshot_without_reading_state_db(),
                    before,
                    "{name}: nothing changed"
                );
            }
            // A recorded owner makes the same rows owned: the ids decide.
            let fx = Fixture::new();
            after_a_first_sign_in(&fx, true, false);
            fx.db()
                .set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            some_operation(&fx.db(), "op-owned");
            assert_eq!(fx.classify(&sam()).0, reauth::SignInKind::SameAccount);
        }

        /// A lock that a panic poisoned counts as a trace, in each of the three places that read one: flipping any ONE of
        /// them to "absent" lets another account in as a first sign-in.
        #[test]
        fn a_poisoned_lock_counts_as_a_trace_in_each_of_its_three_places() {
            type Spoil = fn(&Fixture);
            type Trace = fn(&reauth::LocalTraces) -> bool;
            let places: [(&str, Spoil, Trace); 3] = [
                (
                    "the session in memory",
                    |fx| poison(&fx.acct.session),
                    |t| t.session_in_memory,
                ),
                ("auth present", |fx| poison(&fx.state.auth_present), |t| t.auth_present),
                (
                    "the cached profile",
                    |fx| poison(&fx.acct.cached_profile),
                    |t| t.cached_profile,
                ),
            ];
            for (name, spoil, trace) in places {
                let fx = Fixture::new();
                assert!(
                    !trace(&gather_local_facts(&fx.state, &fx.acct, &fx.sources()).traces),
                    "{name}: a clean world holds nothing"
                );
                spoil(&fx);
                assert!(
                    trace(&gather_local_facts(&fx.state, &fx.acct, &fx.sources()).traces),
                    "{name}: a poisoned lock is a trace"
                );
                assert_ne!(
                    fx.classify(&kim()).0,
                    reauth::SignInKind::Fresh,
                    "{name}: another account is never a first sign-in"
                );
            }
        }

        // -- a re-sign-in that fails, and an engine that does not start --

        /// A re-sign-in that fails says whether its token was already written: before that nothing of the new session
        /// is stored (the caller revokes it), after that the new token is the live one.
        #[test]
        fn a_re_sign_in_that_fails_says_whether_its_token_was_stored() {
            // The owner record cannot be written (an unreadable database): nothing was stored.
            let fx = Fixture::new();
            std::fs::write(
                fx.dir.path().join("state.db"),
                b"this is not a database, and it is long enough to be read",
            )
            .unwrap();
            let error = reauth_swap_token(
                &fx.state,
                &fx.acct,
                &fx.sources(),
                "tok-new",
                &sam(),
                None,
                &mut fx.acct.session_generation(),
            )
            .unwrap_err();
            assert!(!error.token_stored, "{error:?}");
            assert_eq!(
                stored_token(fx.id()),
                None,
                "nothing of the new session reached the Keychain"
            );

            // The Keychain write worked and memory cannot be updated (a poisoned lock): the token is stored.
            let fx = Fixture::new();
            poison(&fx.acct.session);
            let error = reauth_swap_token(
                &fx.state,
                &fx.acct,
                &fx.sources(),
                "tok-new",
                &sam(),
                None,
                &mut fx.acct.session_generation(),
            )
            .unwrap_err();
            assert!(error.token_stored, "{error:?}");
            assert_eq!(
                stored_token(fx.id()).as_deref(),
                Some("tok-new"),
                "the new token is the stored one"
            );
        }

        /// An engine that does not start after a successful re-sign-in is reported through the engine status; it does
        /// not turn the sign-in into an error.
        #[test]
        fn an_engine_that_does_not_start_after_a_re_sign_in_is_reported_through_the_engine_status() {
            let fx = Fixture::new();
            assert_eq!(
                engine_status::sync_status_engine(&fx.acct.engine_state.lock().unwrap().clone()),
                "stopped"
            );
            note_engine_start_failure(
                &fx.acct,
                "Beebeeb couldn’t confirm which account this computer’s local files belong to",
            );
            assert_eq!(
                engine_status::sync_status_engine(&fx.acct.engine_state.lock().unwrap().clone()),
                "error"
            );
            assert!(
                fx.acct.session.lock().unwrap().is_none() && !*fx.state.auth_present.lock().unwrap(),
                "and nothing else moves"
            );
        }

        // -- the email that names an unidentified session --

        /// A session named after the fact is named in ONE turn of the session-write lock (Task 12, lead ruling 1): the
        /// session in memory, the cached profile and the Keychain email. While the lock is held none of them changes,
        /// even after the server has answered; all of them land when the lock is free.
        #[test]
        fn the_email_that_names_an_unidentified_session_is_written_under_the_session_lock() {
            let fx = Fixture::new();
            fx.install_session("tok-x", [1u8; 32], None);
            let (server, seen) = serving(
                "200 OK",
                r#"{"user_id":"u-1","email":"sam@beebeeb.io","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}"#,
            );
            let held = lock_session_keychain_writes();
            std::thread::scope(|scope| {
                let naming = scope.spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap()
                        .block_on(establish_session_identity(&fx.state, &fx.acct, &server, None))
                });
                // Wait until the server has the request, then give the naming time to happen if nothing stops it.
                // Polling, not a fixed pause: the first request of a process is slow.
                let asked = (0..200).any(|_| {
                    std::thread::sleep(Duration::from_millis(50));
                    !seen.lock().unwrap().is_empty()
                });
                assert!(asked, "the server was asked while the lock is held");
                std::thread::sleep(Duration::from_millis(400));
                assert_eq!(
                    fx.acct
                        .session
                        .lock()
                        .unwrap()
                        .as_ref()
                        .and_then(|session| session.email.clone()),
                    None,
                    "memory waits for the lock"
                );
                assert!(fx.acct.cached_profile.lock().unwrap().is_none(), "so does the profile");
                assert_eq!(keychain_account_email(fx.id()), None, "and the Keychain email");
                assert!(!naming.is_finished(), "the naming is waiting on the lock, not done");
                drop(held);
                assert!(naming.join().unwrap(), "and it names the session once the lock is free");
            });
            assert_eq!(
                fx.acct
                    .session
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|session| session.email.clone())
                    .as_deref(),
                Some("sam@beebeeb.io")
            );
            assert_eq!(keychain_account_email(fx.id()).as_deref(), Some("sam@beebeeb.io"));
        }

        /// The switch warning's count (triage 25): every change that has not uploaded, the queue AND the staged copies
        /// that outlive their queue row (a staged copy can be the only unsynced copy of an edit, and the sign-out deletes
        /// it), each counted once. A count that cannot be read settles nothing: the sign-in is `LocalDataUnreadable`,
        /// never a warning that says 0.
        #[test]
        fn the_switch_warning_counts_staged_copies_once_and_an_unreadable_count_settles_nothing() {
            use reauth::SignInKind::{DifferentAccount, LocalDataUnreadable};
            let queued_with_payload = |db: &state_db::StateDb, op_id: &str, payload: &str| {
                db.enqueue_operation(&state_db::PendingOperation {
                    op_id: op_id.into(),
                    kind: state_db::OperationKind::UploadVersion,
                    file_id: Some(format!("file-{op_id}")),
                    parent_id: None,
                    target_path: Some(format!("/{op_id}.txt")),
                    metadata_json: None,
                    payload_path: Some(payload.into()),
                    base_version: None,
                    base_object_version_id: None,
                    attempts: 0,
                    max_attempts: 5,
                    next_retry_at: 0,
                    last_error: None,
                    backup_source_key: None,
                    created_at: 0,
                    updated_at: 0,
                })
                .unwrap();
            };
            let fx = Fixture::new();
            let db = fx.db();
            db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            db.track_staged_payload("/staging/only-copy", None, false).unwrap();
            assert_eq!(
                fx.classify(&kim()),
                (DifferentAccount, 1),
                "a staged copy whose queue row is gone is a change that has not uploaded"
            );
            queued_with_payload(&db, "op-queued", "/staging/queued-copy");
            db.track_staged_payload("/staging/queued-copy", None, false).unwrap();
            assert_eq!(
                fx.classify(&kim()),
                (DifferentAccount, 2),
                "a queued change and its staged copy count once"
            );

            // The count cannot be read (the queue lost the column it is counted by; opening the database recreates a
            // dropped table, but not a dropped column), while the owner record can.
            let fx = Fixture::new();
            fx.db()
                .set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            raw_state_db(&fx)
                .execute("ALTER TABLE operation_queue DROP COLUMN payload_path", [])
                .unwrap();
            assert!(local_data_owner(&fx.sources()).is_ok(), "the premise: the owner reads");
            assert!(
                fx.db().pending_changes_count().is_err(),
                "the premise: the count does not"
            );
            assert_eq!(
                fx.classify(&kim()).0,
                LocalDataUnreadable,
                "never a warning that says 0"
            );
            assert_eq!(
                fx.classify(&sam()).0,
                LocalDataUnreadable,
                "and nothing is decided for sam either"
            );
        }

        /// A mismatch changes nothing on this Mac (lead ruling 6): the check only reads. The world, before and
        /// after, is identical in the Keychain, the state database, memory and the flags.
        #[test]
        fn a_mismatch_changes_nothing_on_this_mac() {
            let fx = Fixture::new();
            let id = fx.id().to_string();
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                &id,
                "tok-a",
                &[7u8; 32],
                Some("sam@beebeeb.io"),
            )
            .unwrap();
            fx.install_session("tok-a", [7u8; 32], Some("sam@beebeeb.io"));
            *fx.acct.cached_profile.lock().unwrap() = Some(sam());
            set_auth_email(&fx.state, Some("sam@beebeeb.io".into()));
            let db = fx.db();
            db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            some_operation(&db, "op-a");

            let snapshot = || {
                (
                    stored_token(&id),
                    stored_key(&id),
                    keychain_account_email(&id),
                    db.owner().unwrap(),
                    db.queued_or_staged_count().unwrap(),
                    fx.acct
                        .session
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|s| (s.token.clone(), s.master_key, s.email.clone())),
                    *fx.state.auth_present.lock().unwrap(),
                    fx.acct.auth_email.lock().unwrap().clone(),
                    fx.acct
                        .cached_profile
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|p| p.user_id.clone()),
                )
            };
            let before = snapshot();
            assert_eq!(
                fx.classify(&kim()),
                (reauth::SignInKind::DifferentAccount, 1),
                "the warning's count is the waiting change"
            );
            assert_eq!(snapshot(), before, "nothing on this Mac changed");
        }

        // -- ruling 2: another account over a retained vault key --

        /// A password, second-factor or browser sign-in as another account while an older account's vault key is
        /// stored takes the switch path, writes nothing, and the confirmed switch removes the old key BEFORE the
        /// new account's own key is written: a token is never paired with another account's key.
        #[tokio::test]
        async fn another_account_over_a_retained_vault_key_is_a_switch_and_the_switch_removes_the_key_first() {
            crate::state_paths::init_for_test();
            type World = fn(&Fixture);
            let worlds: [(&str, World); 3] = [
                ("after a startup 401: the key and the email are kept", |fx| {
                    persist_vault_key_to_keychain(&SessionWrite::for_test(), fx.id(), &[1u8; 32]).unwrap();
                    put_email(fx.id(), "a@beebeeb.io");
                }),
                ("only the vault key is left", |fx| {
                    persist_vault_key_to_keychain(&SessionWrite::for_test(), fx.id(), &[1u8; 32]).unwrap()
                }),
                ("signed in: token, key, email and the session in memory", |fx| {
                    persist_session_to_keychain(
                        &SessionWrite::for_test(),
                        fx.id(),
                        "tok-a",
                        &[1u8; 32],
                        Some("a@beebeeb.io"),
                    )
                    .unwrap();
                    fx.install_session("tok-a", [1u8; 32], Some("a@beebeeb.io"));
                }),
            ];
            for (name, world) in worlds {
                // Its own `desktop.toml`, holding A's address for the sign-in prefill (a startup 401 saves it).
                let config_dir = tempfile::tempdir().unwrap();
                let config = config_dir.path().to_path_buf();
                crate::config::CONFIG_DIR_OVERRIDE
                    .scope(config, async {
                        DesktopConfig::update(|cfg| {
                            cfg.last_signed_in_email = Some("a@beebeeb.io".into());
                            (true, ())
                        })
                        .unwrap();
                        let fx = Fixture::new();
                        let id = fx.id().to_string();
                        let (handle, rx) = finder_setup::driver::FinderSetupHandle::for_test(
                            finder_setup::driver::FinderSetupView::initial(
                                finder_setup::launch_location::LaunchLocation::Applications,
                            ),
                        );
                        let _ = fx.state.finder_setup.set(handle);
                        tokio::spawn(confirm_every_finder_event(rx));
                        world(&fx);
                        let token_before = stored_token(&id);
                        let bob = profile("u-b", "b@beebeeb.io");
                        // The server says A's key is not B's.
                        let (server, _) = serving(KEY_IS_ANOTHER_ACCOUNTS.0, KEY_IS_ANOTHER_ACCOUNTS.1);

                        // B signs in: the account check says "another account" and writes nothing.
                        assert_eq!(
                            fx.decide(&bob, "tok-b", &server).await.0,
                            reauth::SignInKind::DifferentAccount,
                            "{name}"
                        );
                        assert_eq!(
                            stored_key(&id),
                            Some([1u8; 32]),
                            "{name}: the check leaves A's key where it is"
                        );
                        assert_eq!(stored_token(&id), token_before, "{name}: and stores no token of B's");

                        // The confirmed switch: the full sign-out, which forgets A's email too (spec §5.6).
                        clear_session_impl(&fx.state, true, None, crate::UnknownFilesArg::Keep, None)
                            .await
                            .unwrap_or_else(|e| panic!("{name}: sign-out: {e}"));
                        assert!(!keychain_vault_key_present(&id), "{name}: A's vault key is gone");
                        assert_eq!(
                            (stored_token(&id), keychain_account_email(&id)),
                            (None, None),
                            "{name}: and A's token and email"
                        );
                        assert_eq!(
                            DesktopConfig::load().unwrap().last_signed_in_email,
                            None,
                            "{name}: and A's address for the sign-in prefill"
                        );

                        // B signs in again: a first sign-in. A's key is gone BEFORE B's token is stored and before B's key is.
                        assert_eq!(fx.classify(&bob).0, reauth::SignInKind::Fresh, "{name}");
                        let key_present_when_b_signs_in = keychain_vault_key_present(&id);
                        persist_session_token_to_keychain(
                            &SessionWrite::for_test(),
                            &id,
                            "tok-b",
                            Some("b@beebeeb.io"),
                        )
                        .unwrap();
                        set_auth_email(&fx.state, Some("b@beebeeb.io".into()));
                        assert!(
                            !key_present_when_b_signs_in,
                            "{name}: a token is never stored next to another account's key"
                        );
                        store_recovered_vault_key(
                            &fx.acct,
                            &mut fx.acct.session_generation(),
                            "tok-b",
                            Some("b@beebeeb.io"),
                            &[2u8; 32],
                        )
                        .unwrap();
                        assert_eq!(stored_key(&id), Some([2u8; 32]), "{name}: B's key, and only B's");
                    })
                    .await;
            }
        }

        /// Spec §5.6: the switch's sign-out (`forget_email`) leaves none of A's email in `desktop.toml`, even the address
        /// an earlier startup 401 saved for the prefill, and saves none again; a plain sign-out keeps the prefill. Each
        /// world has its own config directory, so no other test's sign-out writes it meanwhile.
        #[tokio::test]
        async fn an_account_switch_forgets_the_previous_email_and_a_plain_sign_out_keeps_it() {
            crate::state_paths::init_for_test();
            let worlds = [
                ("the switch", true, Some("a@beebeeb.io"), None),
                ("the switch, nothing saved before", true, None, None),
                ("a plain sign-out", false, None, Some("a@beebeeb.io")),
            ];
            for (name, forget_email, saved_before, expected) in worlds {
                let config_dir = tempfile::tempdir().unwrap();
                let config = config_dir.path().to_path_buf();
                crate::config::CONFIG_DIR_OVERRIDE
                    .scope(config, async {
                        DesktopConfig::update(|cfg| {
                            cfg.last_signed_in_email = saved_before.map(str::to_string);
                            (true, ())
                        })
                        .unwrap();
                        let fx = Fixture::new();
                        let _reconciler = recording_reconciler(&fx);
                        persist_session_to_keychain(
                            &SessionWrite::for_test(),
                            fx.id(),
                            "tok-a",
                            &[1u8; 32],
                            Some("a@beebeeb.io"),
                        )
                        .unwrap();
                        fx.install_session("tok-a", [1u8; 32], Some("a@beebeeb.io"));
                        clear_session_impl(&fx.state, forget_email, None, crate::UnknownFilesArg::Keep, None)
                            .await
                            .unwrap_or_else(|e| panic!("{name}: sign-out: {e}"));
                        assert_eq!(
                            DesktopConfig::load().unwrap().last_signed_in_email.as_deref(),
                            expected,
                            "{name}"
                        );
                    })
                    .await;
            }
        }

        /// Spec §5.6 ("or it stops"): when `desktop.toml` cannot be written, the switch's sign-out stops before anything
        /// is torn down: the session, the Keychain, the generation and Finder are as they were, and it says why.
        #[tokio::test]
        async fn a_switch_that_cannot_forget_the_email_stops_before_anything_is_torn_down() {
            crate::state_paths::init_for_test();
            let config_dir = tempfile::tempdir().unwrap();
            let config = config_dir.path().to_path_buf();
            crate::config::CONFIG_DIR_OVERRIDE
                .scope(config, async {
                    DesktopConfig::update(|cfg| {
                        cfg.last_signed_in_email = Some("a@beebeeb.io".into());
                        (true, ())
                    })
                    .unwrap();
                    // A save writes a temporary file next to `desktop.toml` first: a directory in its place fails it.
                    std::fs::create_dir(DesktopConfig::path().unwrap().with_extension("toml.tmp")).unwrap();
                    let fx = Fixture::new();
                    let reconciler = recording_reconciler(&fx);
                    persist_session_to_keychain(
                        &SessionWrite::for_test(),
                        fx.id(),
                        "tok-a",
                        &[1u8; 32],
                        Some("a@beebeeb.io"),
                    )
                    .unwrap();
                    fx.install_session("tok-a", [1u8; 32], Some("a@beebeeb.io"));
                    let generation = fx.acct.session_generation();
                    let before = fx.snapshot();

                    assert_eq!(
                        clear_session_impl(&fx.state, true, None, crate::UnknownFilesArg::Keep, None).await,
                        Err(SIGN_OUT_EMAIL_NOT_FORGOTTEN.into())
                    );
                    assert_eq!(fx.snapshot(), before, "nothing was torn down");
                    assert!(
                        fx.acct.session_unchanged_since(generation),
                        "no session transition happened"
                    );
                    assert_eq!(
                        DesktopConfig::load().unwrap().last_signed_in_email.as_deref(),
                        Some("a@beebeeb.io")
                    );
                    tokio::task::yield_now().await;
                    assert!(reconciler.lock().unwrap().is_empty(), "Finder was not asked to remove");
                })
                .await;
        }

        // -- ruling 3: a sign-in while a recovery-phrase unlock is in its server check --

        /// The unlock read A's token and email, then waited for the server. While it waited, A signed out and B signed
        /// in. The unlock must not write A's key next to B's token, nor A's email over B's.
        #[test]
        fn an_unlock_started_for_one_account_never_writes_its_key_next_to_another_accounts_token() {
            let fx = Fixture::new();
            let id = fx.id().to_string();
            persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-a", Some("a@beebeeb.io")).unwrap();
            set_auth_email(&fx.state, Some("a@beebeeb.io".into()));
            clear_keychain_session(&id).unwrap();
            persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-b", Some("b@beebeeb.io")).unwrap();

            let result = store_recovered_vault_key(
                &fx.acct,
                &mut fx.acct.session_generation(),
                "tok-a",
                Some("a@beebeeb.io"),
                &[7u8; 32],
            )
            .map(drop);

            assert_eq!(result, Err(UNLOCK_ACCOUNT_CHANGED.to_string()));
            assert_eq!(stored_key(&id), None, "A's key is not written next to B's token");
            assert_eq!(stored_token(&id).as_deref(), Some("tok-b"));
            assert_eq!(
                keychain_account_email(&id).as_deref(),
                Some("b@beebeeb.io"),
                "and A's email does not replace B's"
            );
            assert!(fx.acct.session.lock().unwrap().is_none());
        }

        /// Every other way the world can change while the phrase is being checked stops the write too, and a world
        /// that did not change is written as before.
        #[test]
        fn an_unlock_writes_only_while_the_account_it_started_with_is_still_stored() {
            let started = |fx: &Fixture| {
                persist_session_token_to_keychain(&SessionWrite::for_test(), fx.id(), "tok-a", Some("a@beebeeb.io"))
                    .unwrap();
                set_auth_email(&fx.state, Some("a@beebeeb.io".into()));
            };
            let unlock = |fx: &Fixture| {
                store_recovered_vault_key(
                    &fx.acct,
                    &mut fx.acct.session_generation(),
                    "tok-a",
                    Some("a@beebeeb.io"),
                    &[7u8; 32],
                )
                .map(drop)
            };
            let refused = Err(UNLOCK_ACCOUNT_CHANGED.to_string());

            let fx = Fixture::new();
            started(&fx);
            clear_keychain_session(fx.id()).unwrap();
            assert_eq!(unlock(&fx), refused, "signed out meanwhile: no token is stored");
            assert_eq!(stored_key(fx.id()), None, "and no key is left behind by a sign-out");

            let fx = Fixture::new();
            started(&fx);
            persist_session_token_to_keychain(&SessionWrite::for_test(), fx.id(), "tok-a2", Some("a@beebeeb.io"))
                .unwrap();
            assert_eq!(
                unlock(&fx),
                refused,
                "the same account signed in again meanwhile: another token is stored"
            );
            assert_eq!(stored_key(fx.id()), None);
            assert_eq!(stored_token(fx.id()).as_deref(), Some("tok-a2"));

            let fx = Fixture::new();
            started(&fx);
            fx.install_session("tok-a", [9u8; 32], Some("a@beebeeb.io"));
            assert_eq!(unlock(&fx), refused, "keys were installed in memory meanwhile");
            assert_eq!(stored_key(fx.id()), None);

            let fx = Fixture::new();
            started(&fx);
            set_auth_email(&fx.state, Some("someone-else@beebeeb.io".into()));
            assert_eq!(unlock(&fx), refused, "the identity the unlock started with changed");
            assert_eq!(stored_key(fx.id()), None);

            let fx = Fixture::new();
            started(&fx);
            assert_eq!(unlock(&fx), Ok(()), "nothing changed: the key is written");
            assert_eq!(stored_key(fx.id()), Some([7u8; 32]));
            assert_eq!(keychain_account_email(fx.id()).as_deref(), Some("a@beebeeb.io"));
            assert_eq!(stored_token(fx.id()).as_deref(), Some("tok-a"), "next to its own token");
        }

        /// The other half of the interleaving: B signs in while A's unlock is in its server check. A's token is still
        /// stored, so B is another account and the check writes nothing, not even an email.
        #[test]
        fn a_sign_in_as_another_account_during_an_unlock_writes_nothing() {
            let fx = Fixture::new();
            let id = fx.id().to_string();
            persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-a", Some("a@beebeeb.io")).unwrap();
            set_auth_email(&fx.state, Some("a@beebeeb.io".into()));
            assert_eq!(
                fx.classify(&profile("u-b", "b@beebeeb.io")).0,
                reauth::SignInKind::DifferentAccount
            );
            assert_eq!(stored_token(&id).as_deref(), Some("tok-a"));
            assert_eq!(keychain_account_email(&id).as_deref(), Some("a@beebeeb.io"));
            assert_eq!(stored_key(&id), None);
            assert_eq!(
                store_recovered_vault_key(
                    &fx.acct,
                    &mut fx.acct.session_generation(),
                    "tok-a",
                    Some("a@beebeeb.io"),
                    &[7u8; 32]
                )
                .map(drop),
                Ok(()),
                "A's unlock still completes"
            );
        }

        /// The writers of a session and the unlock's check-and-write never run at the same time: nothing runs while
        /// the lock is held, and every one of them finishes once it is free.
        #[test]
        fn the_session_writers_and_the_unlock_check_never_run_at_the_same_time() {
            let fx = Fixture::new();
            let id = fx.id().to_string();
            persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-a", Some("a@beebeeb.io")).unwrap();
            set_auth_email(&fx.state, Some("a@beebeeb.io".into()));
            let held = lock_session_keychain_writes();
            let (tx, rx) = std::sync::mpsc::channel::<&'static str>();
            type Job = Box<dyn FnOnce() + Send>;
            let acct = fx.acct.clone();
            let jobs: Vec<(&'static str, Job)> = vec![
                ("a token write", {
                    let id = id.clone();
                    Box::new(move || {
                        let _ = persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-b", None);
                    })
                }),
                ("a token and key write", {
                    let id = id.clone();
                    Box::new(move || {
                        let _ = persist_session_to_keychain(&SessionWrite::for_test(), &id, "tok-b", &[1u8; 32], None);
                    })
                }),
                ("a sign-out clear", {
                    let id = id.clone();
                    Box::new(move || {
                        let _ = clear_keychain_session(&id);
                    })
                }),
                ("the unlock's check and key write", {
                    Box::new(move || {
                        let _ = store_recovered_vault_key(
                            &acct,
                            &mut acct.session_generation(),
                            "tok-a",
                            Some("a@beebeeb.io"),
                            &[2u8; 32],
                        );
                    })
                }),
            ];
            let total = jobs.len();
            for (name, job) in jobs {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    job();
                    let _ = tx.send(name);
                });
            }
            if let Ok(name) = rx.recv_timeout(Duration::from_millis(400)) {
                panic!("{name} ran while the session lock was held");
            }
            assert_eq!(
                stored_token(&id).as_deref(),
                Some("tok-a"),
                "nothing was written while the lock was held"
            );
            drop(held);
            for _ in 0..total {
                rx.recv_timeout(Duration::from_secs(20))
                    .expect("a writer finishes once the lock is free");
            }
        }

        // -- lead ruling F9: the queue kept across a re-sign-in resumes --

        /// What the device run left in the queue: an edit the server refused while the session was revoked (`auth`,
        /// one earlier attempt, its retry far away), and one held back by the quota.
        fn edits_paused_while_revoked(db: &state_db::StateDb) {
            some_operation(db, "op-auth");
            db.record_operation_attempt("op-auth", 1, 4_000_000_000, Some("timed out"))
                .unwrap();
            db.record_operation_pause(
                "op-auth",
                state_db::OperationPauseReason::Auth,
                Some("HTTP 401 Unauthorized"),
                1,
            )
            .unwrap();
            some_operation(db, "op-quota");
            db.record_operation_pause(
                "op-quota",
                state_db::OperationPauseReason::Quota,
                Some("quota exceeded"),
                1,
            )
            .unwrap();
        }

        /// The operations that are due now, with their attempts.
        fn due_now(db: &state_db::StateDb) -> Vec<(String, i64)> {
            let mut due: Vec<_> = db
                .list_due_operations(now_unix_seconds())
                .unwrap()
                .into_iter()
                .map(|op| (op.op_id, op.attempts))
                .collect();
            due.sort();
            due
        }

        /// The one engine start (`start_engine_bound`, as `spawn_bound_engine` makes it) onto this Mac's throwaway state
        /// dir. Returns what was due at the moment the engine would have come to exist, before its first tick; `None`
        /// when no engine started. The slot is left empty.
        async fn due_when_the_engine_starts(fx: &Fixture) -> Option<Vec<(String, i64)>> {
            let root = tempfile::tempdir().unwrap();
            let paths = LocalDataPaths::for_test(fx.dir.path(), root.path());
            let mut slot = fx.acct.engine.lock().await;
            let seen = std::cell::RefCell::new(None);
            let started = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, |_root, _token, _key| {
                *seen.borrow_mut() = Some(due_now(&fx.db()));
                EngineRunner::for_test_with_task(tokio::spawn(async {}))
            });
            drop(slot.take());
            match started {
                Ok(EngineStart::Started) => Some(seen.into_inner().expect("the engine was constructed")),
                Ok(_) => None,
                Err(error) => panic!("the engine start failed: {error}"),
            }
        }

        /// Ruling F9 (spec R8: "sync resumes"): the same account signs in again, with its keys in memory (revoked from
        /// the web while Beebeeb ran) or kept only in the Keychain (after a relaunch while revoked). The edit the server
        /// refused while revoked is due by the time the engine exists, with its attempt kept; the one held back by the
        /// quota stays paused.
        #[tokio::test]
        async fn a_same_account_re_sign_in_makes_the_edits_paused_for_auth_due_before_the_engine_starts() {
            for keys_in_memory in [true, false] {
                let fx = Fixture::new();
                let id = fx.id().to_string();
                persist_session_to_keychain(
                    &SessionWrite::for_test(),
                    &id,
                    "tok-old",
                    &[7u8; 32],
                    Some("sam@beebeeb.io"),
                )
                .unwrap();
                if keys_in_memory {
                    fx.install_session("tok-old", [7u8; 32], Some("sam@beebeeb.io"));
                }
                let db = fx.db();
                db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                    .unwrap();
                edits_paused_while_revoked(&db);
                assert_eq!(due_now(&db), vec![], "precondition: nothing is due");

                let unlocked = reauth_swap_token(
                    &fx.state,
                    &fx.acct,
                    &fx.sources(),
                    "tok-new",
                    &sam(),
                    None,
                    &mut fx.acct.session_generation(),
                )
                .unwrap();
                assert!(unlocked, "keys in memory: {keys_in_memory}");

                assert_eq!(
                    due_when_the_engine_starts(&fx).await,
                    Some(vec![("op-auth".to_string(), 1)]),
                    "keys in memory: {keys_in_memory}"
                );
                assert_eq!(
                    db.queue_diagnostics(now_unix_seconds()).unwrap().paused_by_reason,
                    std::collections::BTreeMap::from([("quota".to_string(), 1)]),
                    "the quota pause stays (keys in memory: {keys_in_memory})"
                );
            }
        }

        /// The pause reasons the support bundle counts.
        fn paused_by_reason(db: &state_db::StateDb) -> std::collections::BTreeMap<String, i64> {
            db.queue_diagnostics(now_unix_seconds()).unwrap().paused_by_reason
        }

        fn reasons(pairs: &[(&str, i64)]) -> std::collections::BTreeMap<String, i64> {
            pairs.iter().map(|(reason, n)| (reason.to_string(), *n)).collect()
        }

        /// What the account record endpoint answers for sam, as the startup probe reads it.
        const SAM_PROFILE: &str =
            r#"{"user_id":"u-1","email":"sam@beebeeb.io","email_verified":true,"created_at":"2026-01-01T00:00:00Z"}"#;

        /// Lead ruling, F9 review I-1 (spec R8, §5.6): the server says the key this Mac kept was replaced. The edit the
        /// server refused while revoked was queued with its name encrypted under that key, so the key-replaced sign-in
        /// re-marks it `key_replaced`: kept, never sent. No resume makes it due: not the recovery-phrase unlock, not a
        /// later same-account sign-in, not the startup check (ruling 2). The quota pause is untouched throughout.
        #[tokio::test]
        async fn after_a_replaced_key_the_edits_paused_for_auth_are_kept_and_never_sent() {
            let fx = Fixture::new();
            after_a_first_sign_in(&fx, true, true);
            let db = fx.db();
            db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            edits_paused_while_revoked(&db);
            let settled = reauth_without_the_kept_key(
                &fx.state,
                &fx.acct,
                &fx.sources(),
                "tok-new",
                &sam(),
                &mut fx.acct.session_generation(),
            )
            .await
            .unwrap();
            assert_eq!(settled, KEY_REPLACED);
            assert_eq!(
                paused_by_reason(&db),
                reasons(&[("key_replaced", 1), ("quota", 1)]),
                "the key-replaced sign-in re-marked the edit"
            );
            assert_eq!(due_when_the_engine_starts(&fx).await, None, "no keys, no engine");

            install_recovered_session(
                &fx.state,
                &fx.acct,
                &mut fx.acct.session_generation(),
                "tok-new",
                Some("sam@beebeeb.io".to_string()),
                &[9u8; 32],
            )
            .unwrap();
            assert_eq!(
                due_when_the_engine_starts(&fx).await,
                Some(vec![]),
                "(1) unlocked with the phrase: the edit is not sent"
            );

            let unlocked = reauth_swap_token(
                &fx.state,
                &fx.acct,
                &fx.sources(),
                "tok-newer",
                &sam(),
                None,
                &mut fx.acct.session_generation(),
            )
            .unwrap();
            assert!(unlocked, "the premise: a later sign-in that asks");
            assert_eq!(
                due_when_the_engine_starts(&fx).await,
                Some(vec![]),
                "(2) a later same-account sign-in: still not sent"
            );

            // A relaunch: nothing in memory, the server confirms the stored token.
            drop(fx.acct.session.lock().unwrap().take());
            let (base_url, _seen) = serving("200 OK", SAM_PROFILE);
            assert!(with_the_shared_state_dir(restore_stored_session(&fx.state, &fx.acct, &base_url)).await);
            assert_eq!(
                due_when_the_engine_starts(&fx).await,
                Some(vec![]),
                "(3) the startup check: still not sent"
            );
            assert_eq!(
                paused_by_reason(&db),
                reasons(&[("key_replaced", 1), ("quota", 1)]),
                "kept and still counted under its own reason"
            );
            assert_eq!(db.queued_or_staged_count().unwrap(), 2, "nothing was purged");
        }

        /// Lead ruling, F9 review I-2: on a Mac, the startup check that confirms the stored token (the vault unlocked
        /// from the Keychain) makes the edits paused for `auth` due before the engine's first tick. That is the relaunch
        /// after a deferred start ("Quit and reopen Beebeeb…"). An answer that confirms nothing (the server cannot be
        /// reached) resumes nothing; the quota pause stays either way.
        #[tokio::test]
        async fn the_startup_check_that_confirms_the_token_makes_the_edits_paused_for_auth_due_before_the_first_tick() {
            for confirmed in [true, false] {
                let fx = Fixture::new();
                persist_session_to_keychain(
                    &SessionWrite::for_test(),
                    fx.id(),
                    "tok-stored",
                    &[7u8; 32],
                    Some("sam@beebeeb.io"),
                )
                .unwrap();
                let db = fx.db();
                db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                    .unwrap();
                edits_paused_while_revoked(&db);
                let base_url = if confirmed {
                    serving("200 OK", SAM_PROFILE).0
                } else {
                    nobody_listens()
                };
                assert!(
                    with_the_shared_state_dir(restore_stored_session(&fx.state, &fx.acct, &base_url)).await,
                    "the session is restored either way (confirmed: {confirmed})"
                );
                let expected = if confirmed {
                    vec![("op-auth".to_string(), 1)]
                } else {
                    vec![]
                };
                assert_eq!(
                    due_when_the_engine_starts(&fx).await,
                    Some(expected),
                    "confirmed: {confirmed}"
                );
                assert_eq!(
                    paused_by_reason(&db),
                    if confirmed {
                        reasons(&[("quota", 1)])
                    } else {
                        reasons(&[("auth", 1), ("quota", 1)])
                    },
                    "confirmed: {confirmed}"
                );
            }
        }

        /// Lead ruling, F9 review M3: a resume the database refuses leaves the ask open, so the engine starts with the
        /// edit still paused and the next engine start tries again (and then sends it).
        #[tokio::test]
        async fn a_resume_that_fails_is_tried_again_at_the_next_engine_start() {
            let fx = Fixture::new();
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                fx.id(),
                "tok-old",
                &[7u8; 32],
                Some("sam@beebeeb.io"),
            )
            .unwrap();
            fx.install_session("tok-old", [7u8; 32], Some("sam@beebeeb.io"));
            let db = fx.db();
            db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            edits_paused_while_revoked(&db);
            // The database refuses exactly the resume: an update that clears a pause.
            db.hold_lock_for_test()
                .execute_batch(
                    "CREATE TRIGGER refuse_the_resume BEFORE UPDATE OF paused_reason ON operation_queue \
                     WHEN NEW.paused_reason IS NULL BEGIN SELECT RAISE(ABORT, 'refused by the test'); END;",
                )
                .unwrap();
            let unlocked = reauth_swap_token(
                &fx.state,
                &fx.acct,
                &fx.sources(),
                "tok-new",
                &sam(),
                None,
                &mut fx.acct.session_generation(),
            )
            .unwrap();
            assert!(unlocked, "the premise: the sign-in asks");
            assert_eq!(
                due_when_the_engine_starts(&fx).await,
                Some(vec![]),
                "the resume failed: the engine starts with the edit still paused"
            );
            db.hold_lock_for_test()
                .execute_batch("DROP TRIGGER refuse_the_resume;")
                .unwrap();
            assert_eq!(
                due_when_the_engine_starts(&fx).await,
                Some(vec![("op-auth".to_string(), 1)]),
                "the next engine start tries again"
            );
        }

        /// F9 review M2 with ruling I-1: a browser sign-in whose handed-over key is not the one this Mac kept. The
        /// re-sign-in in place swaps the token under the engine slot (so no start runs in between) and, because the
        /// handed-over key replaces the kept one, does not ask (re-review N-1); it stops the outgoing engine (which kept
        /// pausing edits for `auth` with its revoked token) and only then re-marks every `auth` pause `key_replaced`. A
        /// start that still holds the outgoing key finds nothing to resume, and neither does the handoff's own install
        /// with the new key (which asks).
        #[tokio::test]
        async fn a_browser_key_that_replaces_the_kept_one_holds_the_edits_once_the_outgoing_engine_stopped() {
            let fx = Fixture::new();
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                fx.id(),
                "tok-old",
                &[7u8; 32],
                Some("sam@beebeeb.io"),
            )
            .unwrap();
            fx.install_session("tok-old", [7u8; 32], Some("sam@beebeeb.io"));
            *fx.acct.cached_profile.lock().unwrap() = Some(sam());
            let db = fx.db();
            db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            edits_paused_while_revoked(&db);
            some_operation(&db, "op-late");
            // The outgoing engine: its token was revoked, so every edit it tries is paused for `auth`, until it stops.
            let state_db_path = fx.dir.path().join("state.db");
            let outgoing = tokio::spawn(async move {
                let db = state_db::StateDb::open(state_db_path).unwrap();
                loop {
                    let _ = db.record_operation_pause(
                        "op-late",
                        state_db::OperationPauseReason::Auth,
                        Some("HTTP 401 Unauthorized"),
                        1,
                    );
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            });
            *fx.acct.engine.lock().await = Some(EngineRunner::for_test_with_task(outgoing));
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert_eq!(
                paused_by_reason(&db),
                reasons(&[("auth", 2), ("quota", 1)]),
                "the premise: the outgoing engine has refused the late edit too"
            );

            let mut slot = fx.acct.engine.lock().await;
            let unlocked = reauth_swap_token(
                &fx.state,
                &fx.acct,
                &fx.sources(),
                "tok-new",
                &sam(),
                Some(&[9u8; 32]),
                &mut fx.acct.session_generation(),
            )
            .unwrap();
            assert!(
                unlocked && handoff_replaces_kept_key(&fx.acct, Some(&[9u8; 32])),
                "the premise: unlocked with the kept key, which the handed-over one replaces"
            );
            assert_eq!(
                stop_the_outgoing_engine_and_hold_auth_paused(&fx.acct, &fx.sources(), &mut slot).await,
                Ok(2),
                "the edit refused while revoked and the one the outgoing engine refused last"
            );
            assert!(slot.is_none(), "the outgoing engine was stopped");
            drop(slot);
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(
                paused_by_reason(&db),
                reasons(&[("key_replaced", 2), ("quota", 1)]),
                "nothing is paused for auth any more"
            );

            assert_eq!(
                due_when_the_engine_starts(&fx).await,
                Some(vec![]),
                "no ask is open, so a start with the outgoing key finds nothing to send"
            );
            assert_eq!(
                install_new_session(
                    &fx.state,
                    &fx.acct,
                    &mut fx.acct.session_generation(),
                    "tok-new",
                    &[9u8; 32],
                    Some("sam@beebeeb.io".to_string()),
                    Some(sam()),
                ),
                Ok(NewSession::Installed)
            );
            assert_eq!(
                due_when_the_engine_starts(&fx).await,
                Some(vec![]),
                "the handed-over key's engine: nothing queued under the replaced key is sent"
            );
            assert_eq!(db.queued_or_staged_count().unwrap(), 3, "kept, not purged");
        }

        /// A signed-in Mac whose session was revoked: keys in memory (`[7; 32]`), the Keychain holding them, the account's
        /// owner recorded, and the edits the server refused while revoked paused for `auth`.
        fn signed_in_with_edits_paused_while_revoked() -> Fixture {
            let fx = Fixture::new();
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                fx.id(),
                "tok-old",
                &[7u8; 32],
                Some("sam@beebeeb.io"),
            )
            .unwrap();
            fx.install_session("tok-old", [7u8; 32], Some("sam@beebeeb.io"));
            *fx.acct.cached_profile.lock().unwrap() = Some(sam());
            let db = fx.db();
            db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            edits_paused_while_revoked(&db);
            fx
        }

        /// Is a resume request open: asked, and not yet served by an engine start?
        fn a_resume_request_is_open(acct: &AccountRuntime) -> bool {
            use std::sync::atomic::Ordering::SeqCst;
            acct.auth_resume_asked.load(SeqCst) > acct.auth_resume_done.load(SeqCst)
        }

        /// F9 re-review N-1 (round 4): whether a re-sign-in in place opens a resume request is decided before it is
        /// made. A browser sign-in whose handed-over key replaces the kept one never opens one (the edits queued under
        /// the kept key must not be sent); one that hands over the key this Mac holds, and a password sign-in, which hands
        /// over none, still do.
        #[tokio::test]
        async fn a_browser_sign_in_asks_to_resume_unless_its_key_replaces_the_kept_one() {
            for (handed_over, replaces, asks) in [
                (Some([9u8; 32]), true, false),
                (Some([7u8; 32]), false, true),
                (None, false, true),
            ] {
                let fx = signed_in_with_edits_paused_while_revoked();
                let unlocked = reauth_swap_token(
                    &fx.state,
                    &fx.acct,
                    &fx.sources(),
                    "tok-new",
                    &sam(),
                    handed_over.as_ref(),
                    &mut fx.acct.session_generation(),
                )
                .unwrap();
                assert!(
                    unlocked,
                    "the premise: the vault is unlocked with the kept key ({handed_over:?})"
                );
                assert_eq!(
                    handoff_replaces_kept_key(&fx.acct, handed_over.as_ref()),
                    replaces,
                    "the premise ({handed_over:?})"
                );
                assert_eq!(
                    a_resume_request_is_open(&fx.acct),
                    asks,
                    "a resume request is open after the swap ({handed_over:?})"
                );
            }
        }

        /// F9 re-review N-1 (round 4): the browser path whose hold fails. The swap has stored the new token and put it in
        /// memory beside the kept key, the outgoing engine stopped, and the database refused the hold, so the edits the
        /// server refused while revoked are still paused for `auth` and the sign-in fails. No resume request is open, so
        /// the next engine start (a Lock and unlock, Finder "Try again") resumes nothing: the edits stay paused and are not
        /// sent with the replaced key. (The composition is `reauth_in_place`'s own, minus the `AppHandle` it needs.)
        #[tokio::test]
        async fn a_browser_sign_in_whose_hold_fails_leaves_no_resume_request_for_a_later_start_to_serve() {
            let fx = signed_in_with_edits_paused_while_revoked();
            let db = fx.db();
            // The database refuses exactly the hold: an update that marks a pause `key_replaced`.
            db.hold_lock_for_test()
                .execute_batch(
                    "CREATE TRIGGER refuse_the_hold BEFORE UPDATE OF paused_reason ON operation_queue \
                     WHEN NEW.paused_reason = 'key_replaced' BEGIN SELECT RAISE(ABORT, 'refused by the test'); END;",
                )
                .unwrap();
            let handed_over = [9u8; 32];
            let mut slot = fx.acct.engine.lock().await;
            let unlocked = reauth_swap_token(
                &fx.state,
                &fx.acct,
                &fx.sources(),
                "tok-new",
                &sam(),
                Some(&handed_over),
                &mut fx.acct.session_generation(),
            )
            .unwrap();
            assert!(
                unlocked && handoff_replaces_kept_key(&fx.acct, Some(&handed_over)),
                "the premise: unlocked with the kept key, which the handed-over one replaces"
            );
            let held = stop_the_outgoing_engine_and_hold_auth_paused(&fx.acct, &fx.sources(), &mut slot).await;
            assert!(held.is_err(), "the premise: the database refused the hold: {held:?}");
            drop(slot);
            assert_eq!(
                paused_by_reason(&db),
                reasons(&[("auth", 1), ("quota", 1)]),
                "the premise: the edit refused while revoked is still paused for auth"
            );

            assert!(
                !a_resume_request_is_open(&fx.acct),
                "the failed sign-in left no resume request open"
            );
            assert_eq!(
                due_when_the_engine_starts(&fx).await,
                Some(vec![]),
                "a later start resumes nothing: the edit is not sent with the replaced key"
            );
            assert_eq!(
                paused_by_reason(&db),
                reasons(&[("auth", 1), ("quota", 1)]),
                "and the edit is still paused"
            );
        }

        /// Lead ruling F9, Minor 4: the ordering race as behaviour. A start reads the ask, then the keys, and is held in
        /// the binding (another connection holds `state.db`). Meanwhile a sign-in puts its new session in memory and asks.
        /// The held start read the outgoing token, so it must not serve that ask: its engine finds the edit still paused.
        /// The sign-in's own start (the next one) then serves it, with the new token.
        #[tokio::test]
        async fn a_start_that_read_the_keys_before_a_sign_in_asked_never_serves_that_ask() {
            use std::sync::atomic::{AtomicBool, Ordering};
            let fx = Fixture::new();
            fx.install_session("tok-old", [7u8; 32], Some("sam@beebeeb.io"));
            let db = fx.db();
            db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            edits_paused_while_revoked(&db);
            drop(db);
            let blocker = rusqlite::Connection::open(fx.dir.path().join("state.db")).unwrap();
            blocker
                .execute_batch("PRAGMA locking_mode = EXCLUSIVE; BEGIN EXCLUSIVE;")
                .unwrap();
            let released = AtomicBool::new(false);
            let runtime = tokio::runtime::Handle::current();
            let root = tempfile::tempdir().unwrap();
            let paths = LocalDataPaths::for_test(fx.dir.path(), root.path());
            let (seen, runner) = std::thread::scope(|scope| {
                let start = scope.spawn(|| {
                    let mut slot = fx.acct.engine.blocking_lock();
                    let seen = std::cell::RefCell::new(None);
                    let started = start_engine_bound(&fx.state, &fx.acct, &mut slot, &paths, |_root, token, _key| {
                        *seen.borrow_mut() =
                            Some((released.load(Ordering::SeqCst), token.to_string(), due_now(&fx.db())));
                        EngineRunner::for_test_with_task(runtime.spawn(async {}))
                    });
                    assert!(matches!(started, Ok(EngineStart::Started)), "{started:?}");
                    (seen.into_inner(), slot.take())
                });
                std::thread::sleep(Duration::from_millis(500));
                {
                    // The sign-in, as every sign-in writer does it: the session in memory, then the ask, in its turn.
                    let write = SessionWrite::for_test();
                    fx.acct.session.lock().unwrap().as_mut().unwrap().token = "tok-new".into();
                    ask_to_resume_auth_paused(&write, &fx.acct);
                }
                released.store(true, Ordering::SeqCst);
                drop(blocker);
                start.join().unwrap()
            });
            drop(runner);
            assert_eq!(
                seen,
                Some((true, "tok-old".to_string(), vec![])),
                "(held in the binding until released, the outgoing token, the edit still paused)"
            );
            assert_eq!(
                due_when_the_engine_starts(&fx).await,
                Some(vec![("op-auth".to_string(), 1)]),
                "the sign-in's own start serves its ask"
            );
        }

        /// Ruling F9: a re-sign-in that fails asks for nothing, and an account switch resumes nothing of the account
        /// before. (1) Another sign-in wrote first, so this one is refused and stores nothing: the engine that starts
        /// with the session already here finds the edit still paused. (2) kim's session arrives over sam's local data:
        /// the binding resets that data before anything is resumed, so kim's engine finds none of sam's edits.
        #[tokio::test]
        async fn a_sign_in_that_fails_and_an_account_switch_make_nothing_due() {
            let fx = Fixture::new();
            let id = fx.id().to_string();
            persist_session_to_keychain(
                &SessionWrite::for_test(),
                &id,
                "tok-old",
                &[7u8; 32],
                Some("sam@beebeeb.io"),
            )
            .unwrap();
            fx.install_session("tok-old", [7u8; 32], Some("sam@beebeeb.io"));
            *fx.acct.cached_profile.lock().unwrap() = Some(sam());
            let db = fx.db();
            db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            edits_paused_while_revoked(&db);
            let mut turn = fx.acct.session_generation();
            drop(claim_session_write(&fx.acct, &mut fx.acct.session_generation()).unwrap());
            let refused = reauth_swap_token(&fx.state, &fx.acct, &fx.sources(), "tok-new", &sam(), None, &mut turn);
            assert!(matches!(&refused, Err(error) if !error.token_stored), "{refused:?}");
            assert_eq!(
                due_when_the_engine_starts(&fx).await,
                Some(vec![]),
                "(1) the failed sign-in resumed nothing"
            );

            let other = Fixture::new();
            let db = other.db();
            db.set_owner(&Identity::new(Some("u-1"), Some("sam@beebeeb.io")))
                .unwrap();
            edits_paused_while_revoked(&db);
            assert_eq!(
                install_new_session(
                    &other.state,
                    &other.acct,
                    &mut other.acct.session_generation(),
                    "tok-kim",
                    &[3u8; 32],
                    Some("kim@beebeeb.io".to_string()),
                    Some(kim()),
                ),
                Ok(NewSession::Installed)
            );
            assert_eq!(
                due_when_the_engine_starts(&other).await,
                None,
                "the reset owes the Finder removal first"
            );
            clear_finder_removal_owed_in(other.dir.path());
            assert_eq!(
                due_when_the_engine_starts(&other).await,
                Some(vec![]),
                "(2) none of sam's edits is due for kim"
            );
            assert_eq!(db.queued_or_staged_count().unwrap(), 0, "they were reset");
        }
    }
