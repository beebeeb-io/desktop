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
    persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-old", Some("sam@beebeeb.io")).unwrap();
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
    persist_session_token_to_keychain(&SessionWrite::for_test(), fx.id(), "tok-1", Some("sam@beebeeb.io")).unwrap();
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
    // LD-0113: the triggers that name the column go with it (SQLite refuses the drop otherwise); the open
    // below re-creates them.
    state_db::StateDb::drop_column_for_test(&raw_state_db(&fx), "operation_queue", "payload_path").unwrap();
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
                let (handle, rx) =
                    finder_setup::driver::FinderSetupHandle::for_test(finder_setup::driver::FinderSetupView::initial(
                        finder_setup::launch_location::LaunchLocation::Applications,
                    ));
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
                persist_session_token_to_keychain(&SessionWrite::for_test(), &id, "tok-b", Some("b@beebeeb.io"))
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
