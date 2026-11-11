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
    include_str!("../browser_login.rs").replace("\r\n", "\n")
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
    assert!(
        settle.contains(
            "reauth::SignInKind::NeedsKeyProof | reauth::SignInKind::Unconfirmed => Ok(SignInSettlement::Unconfirmed),"
        ),
        "{settle}"
    );
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
    let slot_taken =
        at_in_place("let mut engine_slot = if handoff_key.is_some() { Some(acct.engine.lock().await) } else { None };");
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
        assert!(
            !above.contains(macos_only),
            "an ask gated to macOS:\n{above}\n{}",
            lines[at]
        );
    }
    for item in [
        "fn ask_to_resume_auth_paused(",
        "let token_confirmed = matches!(probe, StartupSessionCheck::Authorized { .. });",
    ] {
        let at = lines
            .iter()
            .position(|line| line.contains(item))
            .unwrap_or_else(|| panic!("{item}"));
        let above = lines[at.saturating_sub(4)..at].join("\n");
        assert!(!above.contains(macos_only), "{item} gated to macOS:\n{above}");
    }
    let swap = lines
        .iter()
        .position(|line| line.starts_with("fn reauth_swap_token("))
        .expect("fn reauth_swap_token(");
    assert_eq!(
        lines[swap - 1],
        "#[cfg(not(target_os = \"windows\"))]",
        "its own gate is kept"
    );
    let ask = squeeze("ask_to_resume_auth_paused(&write, acct);");
    let linux = squeeze(&super::finder_setup_command_tests::without_items(
        &production,
        &[macos_only, "#[cfg(target_os = \"windows\")]"],
    ));
    assert_eq!(linux.matches(&ask).count(), 4, "Linux compiles all four asks");
    let windows = squeeze(&super::finder_setup_command_tests::without_items(
        &production,
        &[
            macos_only,
            "#[cfg(not(target_os = \"windows\"))]",
            "#[cfg(target_os = \"linux\")]",
        ],
    ));
    assert_eq!(
        windows.matches(&ask).count(),
        3,
        "Windows compiles three: no reauth_swap_token"
    );
}
