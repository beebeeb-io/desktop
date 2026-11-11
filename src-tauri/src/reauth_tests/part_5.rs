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
        persist_session_token_to_keychain(&SessionWrite::for_test(), fx.id(), "tok-a", Some("a@beebeeb.io")).unwrap();
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
    persist_session_token_to_keychain(&SessionWrite::for_test(), fx.id(), "tok-a2", Some("a@beebeeb.io")).unwrap();
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
                *seen.borrow_mut() = Some((released.load(Ordering::SeqCst), token.to_string(), due_now(&fx.db())));
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
