/// U19, from the count to the window (W11): the switch settles with the snapshot it counted (a Mac) or none
/// (elsewhere); both password sign-ins hand the settlement's snapshot and E3 on to `LoginOutcome`; and the browser
/// sign-in hands them on in the same call (spec C C-S1, task 1747 Part 1): it returns the `LoginOutcome` the password
/// paths return, where it used to put each in an `account_mismatch` event and fail with a raw code. The sign-ins need
/// a server, so their text is read.
#[test]
fn the_switch_count_reaches_the_sign_in_window() {
    let counts = sign_out::snapshot::AskCounts {
        ask_id: "ask-1".to_string(),
        files: 1,
        copied_files: 1,
        others: 1,
        missing_files: 0,
        earlier_folders: Vec::new(),
    };
    assert_eq!(
        switch_settlement(2, Ok(Some(counts.clone()))),
        SignInSettlement::AccountMismatch {
            pending_changes: 2,
            unsent: Some(counts),
            unsent_error: None,
        },
        "a Mac: the snapshot rides the settlement"
    );
    assert_eq!(
        switch_settlement(2, Ok(None)),
        SignInSettlement::AccountMismatch {
            pending_changes: 2,
            unsent: None,
            unsent_error: None,
        },
        "elsewhere: the count alone"
    );
    let squeeze = crate::source_pin::squeeze;
    let production = super::finder_setup_command_tests::production_source();
    for login in ["async fn desktop_login(", "async fn desktop_login_2fa("] {
        let body = squeeze(super::finder_setup_command_tests::body_between(
            &production,
            login,
            "\n}\n",
        ));
        assert!(
            body.contains(&squeeze(
                "return Ok(LoginOutcome::account_mismatch(pending_changes, unsent, unsent_error));"
            )),
            "{login} hands both on"
        );
    }
    // The browser handoff holds the same hand-off as the two password sign-ins, in the same call, through its returned
    // outcome, and no raw `account_mismatch` event or error string is left in it (C-S1).
    let browser_source = include_str!("../browser_login.rs").replace("\r\n", "\n");
    // The handoff's own code, not its tests (which hold the words of what must not be there).
    let browser = squeeze(&browser_source[..browser_source.find("#[cfg(test)]").expect("the tests module")]);
    assert!(
        browser.contains(&squeeze(
            "return Ok(crate::LoginOutcome::account_mismatch(pending_changes, unsent, unsent_error));"
        )),
        "the browser handoff hands both on"
    );
    assert!(
        !browser.contains(concat!("\"account", "_mismatch\"")),
        "no raw code is left in the handoff, as an event or as an error"
    );
}

/// Task 1747 Part 1 (review P3-5 on #143): the switch snapshot is owned by the window the switch step runs in. On a Mac
/// that is `main`, which the app destroys on a close, so a close of `main` drops it, and another window's close leaves
/// it. It used to be owned by the label `onboarding`, a window a Mac no longer builds, so nothing dropped it.
#[tokio::test]
async fn the_switch_snapshot_is_dropped_by_the_window_it_runs_in_and_by_no_other() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        queue_write(&db, "w1", "f1", b"bytes");
        let state = AppState::default();
        let owner = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
        #[cfg(target_os = "macos")]
        assert_eq!(owner, "main", "the sign-in window of a Mac is the Beebeeb window");
        if switch_snapshot(&state).unwrap().is_some() {
            for other in ["settings", "onboarding", "popover", "conflict-7f3a"] {
                assert!(
                    !flow_window_closed(&state.sign_out, other).snapshot_dropped,
                    "{other}: another window's close leaves the switch snapshot"
                );
            }
            assert!(state.sign_out.snapshots.newest_ask_id().is_some());
            assert!(
                flow_window_closed(&state.sign_out, owner).snapshot_dropped,
                "the window the step runs in drops it"
            );
            assert_eq!(state.sign_out.snapshots.newest_ask_id(), None);
        }
    })
    .await;
}

#[test]
fn the_switch_snapshot_is_counted_for_the_window_label_not_a_literal() {
    let production = super::finder_setup_command_tests::production_source();
    let snapshot = super::finder_setup_command_tests::body_between(&production, "fn switch_snapshot(", "\n}\n");
    assert!(
        // Squeezed: the call is one line or three, as rustfmt lays the file out.
        crate::source_pin::squeeze(snapshot)
            .contains("count_ask(surfaces::policy::main_window_label(surfaces::policy::Platform::current()))?"),
        "{snapshot}"
    );
    assert!(
        !snapshot.contains("\"onboarding\""),
        "no window label as a literal:\n{snapshot}"
    );
}

// ---- the fix round after the /code-review of the Mac sign-in PR (task 1683 slice 2) ----

/// Review P2-3: ⌘W destroys the window and its flow with it, and a window opened again must be able to resume “Set up
/// this Mac”. Rust keeps the step for the process, answers it only while it is still true, and does not use it up by
/// reading it (the window may be closed again).
#[test]
fn a_pending_set_up_step_is_resumed_only_while_it_is_still_true() {
    let alice = account_binding::Identity::new(Some("user-alice"), Some("alice@example.eu"));
    let slot = Mutex::new(None);
    assert_eq!(
        resume_pending_setup(&slot, true, false, &alice),
        None,
        "nothing was set"
    );
    remember_pending_setup(&slot, true, alice.clone());
    assert_eq!(
        resume_pending_setup(&slot, true, false, &alice),
        Some(PendingSetup { key_replaced: true })
    );
    assert_eq!(
        resume_pending_setup(&slot, true, false, &alice),
        Some(PendingSetup { key_replaced: true }),
        "reading it does not use it up"
    );
    assert_eq!(
        resume_pending_setup(&slot, true, true, &alice),
        None,
        "the keys are here: nothing is owed"
    );
    assert_eq!(
        resume_pending_setup(&slot, false, false, &alice),
        None,
        "nobody is signed in: nothing to resume"
    );
    remember_pending_setup(&slot, false, alice.clone());
    assert_eq!(
        resume_pending_setup(&slot, true, false, &alice),
        Some(PendingSetup { key_replaced: false }),
        "the newest sign-in's step replaces an older one"
    );
    forget_pending_setup(&slot);
    assert_eq!(resume_pending_setup(&slot, true, false, &alice), None);
}

/// Review P3-4 of the delta review: the step belongs to the account it was set for. A signs in without a key, A signs out
/// (the sign-out code is untouched and leaves the slot), B signs in and is signed in and locked: B must not be sent to
/// A's recovery phrase. Only the same account resumes; an identity that cannot be compared resumes nothing.
#[test]
fn a_pending_set_up_step_belongs_to_the_account_it_was_set_for() {
    let alice = account_binding::Identity::new(Some("user-alice"), Some("alice@example.eu"));
    let bob = account_binding::Identity::new(Some("user-bob"), Some("bob@example.eu"));
    let slot = Mutex::new(None);
    remember_pending_setup(&slot, true, alice.clone());
    assert_eq!(
        resume_pending_setup(&slot, true, false, &alice),
        Some(PendingSetup { key_replaced: true })
    );
    assert_eq!(
        resume_pending_setup(&slot, true, false, &bob),
        None,
        "another account is signed in and locked"
    );
    assert_eq!(
        resume_pending_setup(&slot, true, false, &account_binding::Identity::default()),
        None,
        "an account that cannot be named is not compared, so nothing resumes"
    );
    // The same user under another spelling of its email, with no user id on one side: the canonical email decides.
    let by_email = account_binding::Identity::new(None, Some(" Alice@Example.eu "));
    let named_by_email = Mutex::new(None);
    remember_pending_setup(
        &named_by_email,
        false,
        account_binding::Identity::new(None, Some("alice@example.eu")),
    );
    assert_eq!(
        resume_pending_setup(&named_by_email, true, false, &by_email),
        Some(PendingSetup { key_replaced: false })
    );
    // B enters the step in its turn: the slot now answers B and no longer A.
    remember_pending_setup(&slot, false, bob.clone());
    assert_eq!(
        resume_pending_setup(&slot, true, false, &bob),
        Some(PendingSetup { key_replaced: false })
    );
    assert_eq!(resume_pending_setup(&slot, true, false, &alice), None);
}

#[test]
fn arriving_keys_forget_the_pending_set_up_step() {
    let alice = account_binding::Identity::new(Some("user-alice"), Some("alice@example.eu"));
    for from in [KeysFrom::Unlock, KeysFrom::SignIn] {
        let state = AppState::default();
        remember_pending_setup(&state.pending_setup, false, alice.clone());
        keys_arrived(&state, from);
        assert_eq!(
            resume_pending_setup(&state.pending_setup, true, false, &alice),
            None,
            "{from:?}"
        );
    }
}

/// The window reads `{ "key_replaced": bool }` strictly (`flowForPendingSetup`), and an absent step is JSON null.
#[test]
fn the_pending_set_up_step_is_the_json_the_window_reads() {
    assert_eq!(
        serde_json::to_value(PendingSetup { key_replaced: true }).unwrap(),
        serde_json::json!({ "key_replaced": true })
    );
    assert_eq!(
        serde_json::to_value(Option::<PendingSetup>::None).unwrap(),
        serde_json::Value::Null
    );
}

#[test]
fn the_pending_set_up_commands_are_registered_and_touch_no_secret() {
    let production = super::finder_setup_command_tests::production_source();
    let handler = super::finder_setup_command_tests::body_between(&production, "tauri::generate_handler![", "])");
    // Whole lines: `set_pending_setup,` contains `pending_setup,`, so a substring test cannot tell the two apart.
    for name in ["set_pending_setup,", "pending_setup,"] {
        assert!(handler.lines().any(|line| line.trim() == name), "{name} is registered");
    }
    for signature in ["fn set_pending_setup(", "fn pending_setup("] {
        let body = super::finder_setup_command_tests::body_between(&production, signature, "\n}\n");
        for forbidden in ["keychain", "Keychain", "master_key", "token"] {
            assert!(
                !body.contains(forbidden),
                "{signature} must not touch {forbidden}:\n{body}"
            );
        }
    }
    // It asks the same thing `sync_status` does (a session in memory, or a token present) and nothing more.
    let ask = super::finder_setup_command_tests::body_between(&production, "fn pending_setup(", "\n}\n");
    assert!(
            ask.contains("resume_pending_setup(\n        &state.pending_setup,\n        vault_unlocked || auth_present,\n        vault_unlocked,\n        &pending_setup_identity(&acct),\n    )"),
            "{ask}"
        );
    // Both commands name the account the same way: the one whose credentials this Mac holds.
    let set = super::finder_setup_command_tests::body_between(&production, "fn set_pending_setup(", "\n}\n");
    assert!(set.contains("pending_setup_identity(&acct)"), "{set}");
}

/// Review P2-3 b: the Keychain holds a token and no key. `unlock_vault` says the approved Set up this Mac sentence on a
/// Mac (the window recognises it; `tests/signInModel.test.ts` pins the TypeScript copy equal), not the Keychain's words.
#[test]
fn unlock_vault_says_the_set_up_sentence_when_the_key_is_not_there() {
    let keychain = "unlock vault key from Keychain: secret not found".to_string();
    #[cfg(target_os = "macos")]
    {
        assert_eq!(unlock_failure(keychain), NO_VAULT_KEY_UNLOCK_ERROR);
        assert_eq!(
            NO_VAULT_KEY_UNLOCK_ERROR,
            "This Mac does not have your encryption keys yet. Restore them to continue."
        );
    }
    #[cfg(not(target_os = "macos"))]
    assert_eq!(
        unlock_failure(keychain.clone()),
        keychain,
        "Windows and Linux keep what they showed"
    );
    // Any other failure is shown as it is.
    for other in [
        "load session from Keychain: the Keychain is locked",
        "Sign in before unlocking the vault.",
        "",
    ] {
        assert_eq!(unlock_failure(other.to_string()), other);
    }
}

#[test]
fn unlock_vault_maps_only_the_keychain_load_and_never_the_session_it_installs() {
    let production = super::finder_setup_command_tests::production_source();
    let unlock = super::finder_setup_command_tests::body_between(&production, "async fn unlock_vault(", "\n}\n");
    assert!(
        unlock.contains("load_session_from_keychain(acct.id.as_str(), email)\n            .map_err(unlock_failure)?"),
        "{unlock}"
    );
    // The startup restore's own test of the same failure is untouched: it reads the Keychain's words.
    let restore =
        super::finder_setup_command_tests::body_between(&production, "async fn restore_session_inner(", "\n}\n");
    assert!(!restore.contains("unlock_failure"), "{restore}");
}

// ---- task 1747 Part 2: the Keychain-presence signal (`vault_unlock_route`) ----

/// A store that answers ONE question, “is a vault key stored?”, and panics on anything that reads or writes a secret: a
/// route that read the key (or anything else) would fail the test that uses it.
struct HoldsAKey(fn() -> keychain::AuthResult<bool>);

impl keychain::AuthSecretStore for HoldsAKey {
    fn save_session_token(&self, _: &SessionToken) -> keychain::AuthResult<()> {
        unreachable!("the route writes nothing")
    }
    fn load_session_token(&self) -> keychain::AuthResult<Option<SessionToken>> {
        unreachable!("the route reads no token")
    }
    fn delete_session_token(&self) -> keychain::AuthResult<()> {
        unreachable!("the route deletes nothing")
    }
    fn save_wrapped_master_key(&self, _: SecretBytes) -> keychain::AuthResult<()> {
        unreachable!("the route writes nothing")
    }
    fn load_wrapped_master_key(&self) -> keychain::AuthResult<Option<SecretBytes>> {
        unreachable!("the route never reads the key")
    }
    fn delete_wrapped_master_key(&self) -> keychain::AuthResult<()> {
        unreachable!("the route deletes nothing")
    }
    fn holds_wrapped_master_key(&self) -> keychain::AuthResult<bool> {
        (self.0)()
    }
}

const ABSENT: HoldsAKey = HoldsAKey(|| Ok(false));
const PRESENT: HoldsAKey = HoldsAKey(|| Ok(true));
const NOT_FOUND: HoldsAKey = HoldsAKey(|| Err(keychain::AuthStoreError::NotFound));
const UNREADABLE: HoldsAKey = HoldsAKey(|| Err(keychain::AuthStoreError::Backend("the Keychain is locked".into())));

fn route_over(id_keyed: &HoldsAKey, legacy: &HoldsAKey) -> VaultUnlockRoute {
    unlock_route(vault_key_in_either_layout(id_keyed, legacy))
}

/// Item 1, through the real account-slot function and the in-memory Keychain a test build resolves.
#[test]
fn the_route_is_keychain_when_the_key_is_there() {
    use keychain::AuthSecretStore as _;
    let id = "route-the-key-is-there";
    AuthVault::new(platform_keychain_store_for(id))
        .store_wrapped_master_key(SecretBytes::new_master_key(&[7u8; 32]))
        .unwrap();
    assert_eq!(vault_unlock_route_for(id), VaultUnlockRoute::Keychain);
    assert_eq!(route_over(&PRESENT, &ABSENT), VaultUnlockRoute::Keychain);
    platform_keychain_store_for(id).delete_wrapped_master_key().unwrap();
}

/// Item 2: a session and no key (the interrupted first run) is the recovery phrase. `NotFound` and a store that cannot hold
/// secrets say the same as an empty one.
#[test]
fn the_route_is_recovery_phrase_when_neither_layout_holds_a_key() {
    use keychain::AuthSecretStore as _;
    let id = "route-no-key-anywhere";
    platform_keychain_store_for(id)
        .save_session_token(&SessionToken::new("tok-no-key".to_string()).unwrap())
        .unwrap();
    assert_eq!(
        vault_unlock_route_for(id),
        VaultUnlockRoute::RecoveryPhrase,
        "a token is not a key"
    );
    assert_eq!(route_over(&ABSENT, &ABSENT), VaultUnlockRoute::RecoveryPhrase);
    assert_eq!(route_over(&NOT_FOUND, &NOT_FOUND), VaultUnlockRoute::RecoveryPhrase);
    platform_keychain_store_for(id).delete_session_token().unwrap();
}

/// Item 3: a key that only the legacy flat layout holds is a key (the unlock reads it as a fallback).
#[test]
fn the_route_reads_the_legacy_layout_too() {
    assert_eq!(route_over(&ABSENT, &PRESENT), VaultUnlockRoute::Keychain);
    assert_eq!(route_over(&PRESENT, &PRESENT), VaultUnlockRoute::Keychain);
}

/// Item 4, pinned so nobody “fixes” it: a Keychain that cannot be read routes to the Keychain unlock, whose own error then
/// reaches the person. Set up this Mac would end by writing a key over one that may well be there.
#[test]
fn an_unreadable_keychain_routes_to_the_keychain_unlock() {
    assert_eq!(route_over(&UNREADABLE, &ABSENT), VaultUnlockRoute::Keychain);
    assert_eq!(route_over(&ABSENT, &UNREADABLE), VaultUnlockRoute::Keychain);
    assert_eq!(route_over(&UNREADABLE, &UNREADABLE), VaultUnlockRoute::Keychain);
}

/// What the window parses: snake_case strings, and no other fields.
#[test]
fn the_route_is_a_snake_case_string_on_the_wire() {
    assert_eq!(
        serde_json::to_value(VaultUnlockRoute::Keychain).unwrap(),
        serde_json::json!("keychain")
    );
    assert_eq!(
        serde_json::to_value(VaultUnlockRoute::RecoveryPhrase).unwrap(),
        serde_json::json!("recovery_phrase")
    );
}

/// Item 5: the command reaches the Keychain only through `keychain_vault_key_present`, which asks both layouts and never
/// unlocks (`the_vault_key_probe_asks_both_layouts_and_never_unlocks` holds that helper); a worker that failed to answer is
/// the Keychain unlock; and the command is registered on a Mac only.
#[test]
fn the_route_never_reads_the_key() {
    let production = super::finder_setup_command_tests::production_source();
    let body =
        |signature: &str| super::finder_setup_command_tests::body_between(&production, signature, "\n}\n").to_string();
    let route_for = body("fn vault_unlock_route_for(");
    assert!(
        route_for.contains("unlock_route(keychain_vault_key_present(account_id))"),
        "{route_for}"
    );
    let command = body("async fn vault_unlock_route(");
    assert!(command.contains("vault_unlock_route_for(&account_id)"), "{command}");
    assert!(
        command.contains(".unwrap_or(VaultUnlockRoute::Keychain)"),
        "fails closed:\n{command}"
    );
    for (name, text) in [
        ("the command", &command),
        ("the account function", &route_for),
        ("the decision", &body("fn unlock_route(")),
    ] {
        for forbidden in [
            "load_",
            "AuthVault",
            "unlock_vault",
            "master_key",
            "unlock(",
            "session",
            "token",
            ".ok()",
            "unwrap_or(false)",
        ] {
            assert!(!text.contains(forbidden), "{name} must not touch {forbidden}:\n{text}");
        }
    }
    let handler = super::finder_setup_command_tests::body_between(&production, "tauri::generate_handler![", "])");
    let at = handler.find("vault_unlock_route,").expect("registered");
    assert!(
        handler[..at].trim_end().ends_with("#[cfg(target_os = \"macos\")]"),
        "registered on a Mac only"
    );
}

/// Item 6 (plan D5): the route is its own command; the 1 Hz `sync_status` poll never reaches the Keychain for it.
#[test]
fn sync_status_does_not_read_the_keychain() {
    let production = super::finder_setup_command_tests::production_source();
    let body = super::finder_setup_command_tests::body_between(&production, "async fn sync_status(", "\n}\n")
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    for forbidden in [
        "keychain",
        "vault_unlock_route",
        "unlock_route",
        "holds_vault_key",
        "vault_key_in_either_layout",
    ] {
        assert!(!body.contains(forbidden), "sync_status must not reach {forbidden}");
    }
}

/// Review P3-1: `open_reauth_window` on a Mac asks the window for “Sign in again”, whatever the session says; the
/// sign-in itself still lets the window decide.
#[test]
fn open_reauth_window_asks_the_beebeeb_window_for_sign_in_again() {
    assert_eq!(
        serde_json::to_value(MainNav::Reauth).unwrap(),
        serde_json::json!({ "to": "reauth" })
    );
    let production = super::finder_setup_command_tests::production_source();
    let window = super::finder_setup_command_tests::body_between(&production, "async fn open_reauth_window(", "\n}\n");
    let mac =
        super::finder_setup_command_tests::body_between(window, "#[cfg(target_os = \"macos\")]\n    {", "\n    }\n");
    assert!(mac.contains("show_reauth_in_beebeeb_window(&app)"), "{mac}");
    let ask =
        super::finder_setup_command_tests::body_between(&production, "fn show_reauth_in_beebeeb_window(", "\n}\n");
    assert!(ask.contains("show_main_window(app, MainNav::Reauth);"), "{ask}");
    // The plain sign-in is unchanged: Home, and the window decides.
    let plain =
        super::finder_setup_command_tests::body_between(&production, "fn show_sign_in_in_beebeeb_window(app", "\n}\n");
    assert!(plain.contains("show_main_window(app, MainNav::Home);"), "{plain}");
}

/// §6.4 (review I8): the switch's count logs the line `sign_out_count` logs, with counts only. A device step reads
/// it to prove a change was waiting (D1).
#[tokio::test]
async fn the_switch_count_logs_counts_only() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        let staged = queue_write(&db, "w1", "f1", b"bytes");
        queue_rename(&db, "r1");
        let state = AppState::default();
        let mut counted = None;
        let logs = capture_logs(|| counted = Some(switch_snapshot(&state)));
        assert!(counted.expect("the count ran").unwrap().is_some(), "a snapshot");
        let line = logs
            .lines()
            .find(|line| line.contains("sign-out flow counted"))
            .unwrap_or_else(|| panic!("the switch's count is logged:\n{logs}"));
        let fields: Vec<&str> = line.split_whitespace().collect();
        for field in [
            "files=1",
            "copied_files=1",
            "others=1",
            "missing_files=0",
            "can_send=false",
            "earlier_folder=false",
        ] {
            assert!(fields.contains(&field), "{field}: {line}");
        }
        assert!(
            !logs.contains("f1.bin") && !logs.contains(&*staged.to_string_lossy()),
            "no name and no path:\n{logs}"
        );
    })
    .await;
}

/// §5.1, the entry points' side of the request. They need the app's windows, so their text is read: the request is
/// stored before Settings opens and is told, so a window that has not loaded yet finds it when it mounts; the
/// compact page's command opens the flow; Settings' command takes the one request; both are registered once.
#[test]
fn a_flow_request_is_stored_before_settings_opens_and_is_told() {
    let squeeze = crate::source_pin::squeeze;
    let production = super::finder_setup_command_tests::production_source();
    let body = |start: &str| {
        squeeze(super::finder_setup_command_tests::body_between(
            &production,
            start,
            "\n}\n",
        ))
    };
    let open = body("fn open_sign_out_flow(");
    let at = |step: &str| {
        open.find(&squeeze(step))
            .unwrap_or_else(|| panic!("open_sign_out_flow: {step}\n{open}"))
    };
    let stored = at("app.state::<AppState>().sign_out.flow_request.store(true, Ordering::SeqCst);");
    // Task 1683 slice 1 (two-surface §8.1, §8.2): the flow's owner is the Beebeeb window, on Settings › Account.
    let shown = at("show_main_window(app, MainNav::settings(Some(\"account\")));");
    let told = at("let _ = app.emit_to(\"main\", \"menu:sign-out-flow\", ());");
    assert!(stored < shown && shown < told, "store, open, then tell: {open}");
    assert!(
        body("fn request_sign_out_flow(").contains(&squeeze("open_sign_out_flow(&app);")),
        "the compact page's command opens the flow"
    );
    assert!(
        body("fn consume_sign_out_flow_request(").contains(&squeeze("take_sign_out_flow_request(&state.sign_out)")),
        "Settings takes the one request"
    );
    let handler = super::finder_setup_command_tests::body_between(&production, "tauri::generate_handler![", "])");
    for command in ["request_sign_out_flow,", "consume_sign_out_flow_request,"] {
        assert_eq!(
            handler.lines().filter(|line| line.trim() == command).count(),
            1,
            "{command} registered once"
        );
    }
}

/// U20 (W12): spec A's R10 reset keeps every waiting write before its purge, asking nobody: the other changes are
/// dropped with their count in its log line, and the purge runs with the guard. A failed keep fails the reset and
/// deletes nothing. A reset whose purge fails after a completed keep still hands its folder on to be surfaced, as
/// the recorded folder is on disk either way.
#[tokio::test]
async fn the_r10_reset_keeps_before_it_purges() {
    in_own_dirs(async {
        let state_dir = state_paths::beebeeb_state_dir().unwrap();
        let sync_root = tempfile::tempdir().unwrap();
        let db = state_db_in_scratch();
        queue_write(&db, "w1", "f1", b"another account's save");
        queue_rename(&db, "r1");
        let data = StateDbLocalData::for_engine_start(&LocalDataPaths::for_test(&state_dir, sync_root.path())).unwrap();
        let logs = capture_logs_async(async {
            account_binding::LocalData::reset(&data).unwrap();
        })
        .await;
        let kept = data.kept.lock().unwrap().clone().expect("the reset kept the write");
        let folder = kept.folder.clone().unwrap();
        assert_eq!(std::fs::read(folder.join("f1.bin")).unwrap(), b"another account's save");
        assert_eq!(
            DesktopConfig::load().unwrap().kept_unsent_folder,
            Some(folder.to_string_lossy().into_owned()),
            "recorded"
        );
        assert!(kept.others_acknowledged.contains("r1"));
        assert!(
            logs.contains("others_dropped=1"),
            "the dropped rename is counted in the reset's line: {logs}"
        );
        assert!(
            db.waiting_rows().unwrap().is_empty(),
            "then the purge ran, with the guard"
        );
        assert!(
            DesktopConfig::load().unwrap().kept_unsent_unfinished.is_empty(),
            "a completed reset clears the unfinished list, as every completed purge does (§7.3)"
        );
        let production = super::finder_setup_command_tests::production_source();
        let reset = super::finder_setup_command_tests::body_between(
            &production,
            "    fn reset(&self) -> Result<(), String> {",
            "\n    }\n",
        );
        let squeezed = crate::source_pin::squeeze(reset);
        let call = crate::source_pin::squeeze(
            "purge_local_state_files_in(&self.db, self.sync_root.as_deref(), &self.staging_dirs, kept.as_ref())",
        );
        assert!(squeezed.contains(&call), "the guard is in the reset's path: {reset}");
        // W13 and W10: the reset clears the unfinished list with the sign-out purge's own statement, so a clear
        // that fails is logged by the step that failed, never by an error's text.
        let marker_clear = crate::source_pin::squeeze(concat!(
            "#[cfg(target_os = \"macos\")] if let Err(step) = DesktopConfig::path()",
            ".map_err(|_| \"path\".to_string())",
            ".and_then(|path| sign_out::kept_record::clear_unfinished_at(&path))",
            " { tracing::warn!(step = %step, \"could not clear the unfinished sign-out marker\"); }",
        ));
        let sign_out_purge = crate::source_pin::squeeze(super::finder_setup_command_tests::body_between(
            &production,
            "fn purge_local_data_for_sign_out(",
            "\n}\n",
        ));
        assert!(
            sign_out_purge.contains(&marker_clear) && squeezed.contains(&marker_clear),
            "the reset clears the marker as the sign-out does, its warning naming the step: {reset}"
        );
        // A failing copy: the reset fails, nothing is deleted, no record.
        use std::os::unix::fs::PermissionsExt;
        clean_scratch(&db);
        let staged = queue_write(&db, "w2", "f2", b"bytes");
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o000)).unwrap();
        let data = StateDbLocalData::for_engine_start(&LocalDataPaths::for_test(&state_dir, sync_root.path())).unwrap();
        let mut failed = None;
        let logs = capture_logs_async(async {
            failed = Some(account_binding::LocalData::reset(&data));
        })
        .await;
        // A copy the reset deleted cannot have its permissions put back: the next line names that failure.
        let _ = std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600));
        assert!(staged.exists(), "nothing deleted: the staged copy is still there");
        assert!(failed.unwrap().unwrap_err().starts_with("unsent_copy_failed: "));
        let stopped = "R10: local data reset stopped: the waiting changes could not be kept; nothing was purged";
        assert!(
            logs.contains(stopped),
            "the reset's failed keep is logged as the reset's: {logs}"
        );
        assert!(!logs.contains("sign-out stopped"), "a reset is not a sign-out: {logs}");
        assert!(
            !db.finder_accepts_closed().unwrap(),
            "a reset that stops reopens Finder accepts (spec §15 question 3)"
        );
        assert_eq!(db.waiting_rows().unwrap().len(), 1, "nothing deleted");
        assert_eq!(DesktopConfig::load().unwrap().kept_unsent_folder, None, "no record");
        // W12: a reset whose purge fails after a completed keep still hands its folder on to be surfaced (the alert
        // and the row), as the recorded folder is on disk either way.
        clean_scratch(&db);
        let blocker = std::env::temp_dir().join(format!("bb-1887-{}-blocker", std::process::id()));
        std::fs::create_dir_all(&blocker).unwrap();
        db.track_staged_payload(blocker.to_str().unwrap(), None, true).unwrap(); // a "file" no removal can take
        queue_write(&db, "w3", "f3", b"kept, then the purge failed");
        let data = StateDbLocalData::for_engine_start(&LocalDataPaths::for_test(&state_dir, sync_root.path())).unwrap();
        let failed = account_binding::LocalData::reset(&data);
        std::fs::remove_dir(&blocker).unwrap();
        assert!(failed.is_err(), "the purge failed on the folder at a journal path");
        assert!(
            !db.finder_accepts_closed().unwrap(),
            "a reset whose purge failed reopens Finder accepts (spec §15 question 3)"
        );
        let surfaced = AppState::default();
        hand_on_reset_kept(&surfaced, &data);
        let (folder, files) = surfaced
            .sign_out
            .reset_kept
            .lock()
            .unwrap()
            .clone()
            .expect("the recorded folder is handed on");
        assert_eq!(files, 1);
        assert_eq!(
            std::fs::read(Path::new(&folder).join("f3.bin")).unwrap(),
            b"kept, then the purge failed"
        );
        assert_eq!(
            DesktopConfig::load().unwrap().kept_unsent_unfinished,
            [folder.as_str()],
            "a reset that stops after its keep leaves its folder named (W13)"
        );
        let bind = super::finder_setup_command_tests::body_between(&production, "fn bind_data_to_session(", "\n}\n");
        let ran = bind.find("bind_before_engine_start(").expect("the binding runs");
        let handed = bind
            .find("hand_on_reset_kept(state, data);")
            .expect("the binding hands the folder on");
        assert!(
            ran < handed && handed < bind.find("match bound?").expect("before it looks at the result"),
            "the folder is handed on after the binding ran, before its answer is looked at: {bind}"
        );
    })
    .await;
}

/// W12 through the binding (U20): an R10 reset that completes, for another account's data and its waiting save,
/// hands its kept folder to the state, where the engine start surfaces it.
#[tokio::test]
async fn a_completed_r10_reset_hands_its_folder_on_through_the_binding() {
    in_own_dirs(async {
        let state_dir = state_paths::beebeeb_state_dir().unwrap();
        let sync_root = tempfile::tempdir().unwrap();
        let db = state_db_in_scratch();
        let alice = account_binding::Identity::new(Some("u-alice"), Some("alice@beebeeb.io"));
        let bob = account_binding::Identity::new(Some("u-bob"), Some("bob@beebeeb.io"));
        db.set_owner(&alice).unwrap();
        queue_write(&db, "w1", "f1", b"alice's save");
        let data = StateDbLocalData::for_engine_start(&LocalDataPaths::for_test(&state_dir, sync_root.path())).unwrap();
        let state = AppState::default();
        assert_eq!(bind_data_to_session(&state, &data, &bob), Ok(()), "the reset completed");
        assert_eq!(db.owner().unwrap(), Some(bob), "the data is the new account's");
        let (folder, files) = state
            .sign_out
            .reset_kept
            .lock()
            .unwrap()
            .clone()
            .expect("a completed reset hands its folder on through the binding");
        assert_eq!(files, 1);
        assert_eq!(
            std::fs::read(Path::new(&folder).join("f1.bin")).unwrap(),
            b"alice's save"
        );
    })
    .await;
}

/// W12 through the binding (U20): an R10 reset whose purge fails after a completed keep answers an error, so
/// nothing starts, and the binding still hands the kept folder to the state: it is recorded and on disk either way.
#[tokio::test]
async fn an_r10_reset_whose_purge_fails_hands_its_folder_on_through_the_binding() {
    in_own_dirs(async {
        let state_dir = state_paths::beebeeb_state_dir().unwrap();
        let sync_root = tempfile::tempdir().unwrap();
        let db = state_db_in_scratch();
        let alice = account_binding::Identity::new(Some("u-alice"), Some("alice@beebeeb.io"));
        let bob = account_binding::Identity::new(Some("u-bob"), Some("bob@beebeeb.io"));
        db.set_owner(&alice).unwrap();
        // A journal path that holds a directory: a removal the purge cannot make. In the test's own state dir.
        let blocker = state_dir.join("bb-1887-blocker");
        std::fs::create_dir_all(&blocker).unwrap();
        db.track_staged_payload(blocker.to_str().unwrap(), None, true).unwrap();
        queue_write(&db, "w1", "f1", b"kept, then the purge failed");
        let data = StateDbLocalData::for_engine_start(&LocalDataPaths::for_test(&state_dir, sync_root.path())).unwrap();
        let state = AppState::default();
        let bound = bind_data_to_session(&state, &data, &bob);
        assert!(
            bound
                .as_ref()
                .is_err_and(|error| error.contains("could not be removed")),
            "the purge failed, so nothing starts: {bound:?}"
        );
        assert_eq!(db.owner().unwrap(), Some(alice), "no new owner over data still there");
        let (folder, files) = state
            .sign_out
            .reset_kept
            .lock()
            .unwrap()
            .clone()
            .expect("a failed purge's folder is handed on through the binding");
        assert_eq!(files, 1);
        assert_eq!(
            std::fs::read(Path::new(&folder).join("f1.bin")).unwrap(),
            b"kept, then the purge failed"
        );
    })
    .await;
}

/// Spec §15 question 3 for the R10 reset's last step: a reset whose row clear fails, after its keep and its purge,
/// answers that error and reopens Finder accepts at once, as a reset whose purge fails does.
#[tokio::test]
async fn an_r10_reset_whose_row_clear_fails_reopens_finder_accepts() {
    in_own_dirs(async {
        let state_dir = state_paths::beebeeb_state_dir().unwrap();
        let sync_root = tempfile::tempdir().unwrap();
        let db = state_db_in_scratch();
        queue_write(&db, "w1", "f1", b"a save");
        let data = StateDbLocalData::for_engine_start(&LocalDataPaths::for_test(&state_dir, sync_root.path())).unwrap();
        // A row only the row clear deletes, and a trigger on the reset's own connection that refuses that delete.
        data.db
            .hold_lock_for_test()
            .execute_batch(
                "INSERT INTO sync_state (key, value) VALUES ('bb-1887-clear-blocker', '1');
                     CREATE TEMP TRIGGER bb_1887_clear_fails BEFORE DELETE ON sync_state
                     WHEN OLD.key = 'bb-1887-clear-blocker'
                     BEGIN SELECT RAISE(ABORT, 'the row clear fails'); END;",
            )
            .unwrap();
        let error = account_binding::LocalData::reset(&data).expect_err("the row clear failed");
        assert!(error.starts_with("clear the local data: "), "{error}");
        assert!(
            db.waiting_rows().unwrap().is_empty(),
            "the keep and the purge ran first"
        );
        assert!(
            !db.finder_accepts_closed().unwrap(),
            "a reset whose row clear failed reopens Finder accepts (spec §15 question 3)"
        );
    })
    .await;
}

/// U20, the surfacing (W12, §9): an R10 reset's kept folder reaches the person the way the app-start sweep's does.
/// The engine start that ran the reset needs the app, so its text is read: the start takes the handed-on folder
/// after the bound start returned, whatever it returned, and surfaces it once; the surface tells the windows (the
/// row), then shows one alert that names our folder alone.
#[test]
fn the_r10_reset_folder_is_surfaced_after_the_start_returns() {
    let squeeze = crate::source_pin::squeeze;
    let production = super::finder_setup_command_tests::production_source();
    // A function's code without its comment lines, squeezed: a comment can neither meet nor break a step.
    let body = |start: &str| {
        let code: Vec<&str> = super::finder_setup_command_tests::body_between(&production, start, "\n}\n")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect();
        squeeze(&code.join("\n"))
    };
    let gate = body("fn spawn_bound_engine(");
    let at = |step: &str| {
        gate.find(&squeeze(step))
            .unwrap_or_else(|| panic!("spawn_bound_engine: {step}\n{gate}"))
    };
    let cloned = at("let surface_app = app.clone();");
    let started = at("let started = start_engine_bound(");
    let taken = at("let reset_kept = state.sign_out.reset_kept.lock().unwrap_or_else(|p| p.into_inner()).take();");
    let surfaced =
        at("if let Some((folder, files)) = reset_kept { surface_reset_folder(&surface_app, &folder, files); }");
    assert!(
        cloned < started && started < taken && taken < surfaced,
        "clone the app, start, then take and surface: {gate}"
    );
    let between = &gate[started..surfaced];
    assert!(
        !between.contains('?') && !between.contains("return") && gate.ends_with("started"),
        "whatever the start returned, nothing returns before the surfacing; the start's result comes last: {gate}"
    );
    let surface = body("fn surface_reset_folder(");
    let told = surface
        .find(&squeeze("emit_kept_folder_changed(app);"))
        .unwrap_or_else(|| panic!("the windows are told: {surface}"));
    let alert = surface
        .find(&squeeze("sign_out::copy::sign_out_alert(Some((folder, files)), None)"))
        .unwrap_or_else(|| panic!("one alert, our folder alone: {surface}"));
    let shown = surface
        .find(".dialog()")
        .unwrap_or_else(|| panic!("the alert is shown: {surface}"));
    assert!(told < alert && alert < shown, "told, then the alert: {surface}");
}
