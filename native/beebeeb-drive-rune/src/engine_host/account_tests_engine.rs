//! The engine's own account calls (`app::account_api`) against the parent module's fake: each answers for its
//! session and sends only for it, they count in the engine's run of 401s, and an answer that lands after its
//! session moved on is handed to nobody. Every name is invented.

use super::*;

/// The engine's own wrappers against its fake: the answer names its session; a change for a session that is not
/// the one in memory sends nothing.
#[test]
fn the_engines_account_calls_answer_for_their_session_and_send_only_for_it() {
    let server = server();
    let engine = signed_in(&server);
    let turn = engine.session_turn();
    let profile = engine.account_profile().expect("answers");
    assert_eq!((profile.session, profile.value.email.as_str()), (turn, FIRST));
    assert!(
        !format!("{profile:?}").contains(FIRST),
        "an answer's Debug names no email"
    );
    assert_eq!(
        engine.account_display_name().expect("answers").value.as_deref(),
        Some(FIRST_NAME)
    );
    let plan = engine.account_subscription().expect("answers");
    assert_eq!((plan.session, plan.value.quota_bytes), (turn, 1_000_000_000_000));
    let list = engine.account_sessions().expect("answers");
    assert_eq!((list.session, list.value.sessions.len()), (turn, 3));

    // The one read Account makes: the three answers, for this session, each part with its own failure.
    let asked = server.fake.routes().len();
    let basics = engine.account_basics().expect("answers");
    assert_eq!(server.fake.routes().len(), asked + 3, "three requests");
    assert_eq!((basics.session, basics.value.profile.email.as_str()), (turn, FIRST));
    assert_eq!(basics.value.display_name, Ok(Some(FIRST_NAME.to_string())));
    assert_eq!(
        basics.value.subscription.as_ref().map(|plan| plan.quota_bytes),
        Ok(1_000_000_000_000)
    );
    assert!(
        !format!("{basics:?}").contains(FIRST),
        "an answer's Debug names no email"
    );
    server.name_mode.store(SERVER_ERROR, Ordering::SeqCst);
    server.plan_mode.store(UNREADABLE, Ordering::SeqCst);
    let basics = engine.account_basics().expect("the profile answered").value;
    assert_eq!(basics.profile.email, FIRST);
    assert_eq!(basics.display_name, Err(AccountCallError::ServerError));
    assert_eq!(basics.subscription.err(), Some(AccountCallError::Unreadable));
    server.name_mode.store(SESSION_ENDED, Ordering::SeqCst);
    server.plan_mode.store(ANSWER, Ordering::SeqCst);
    // A 401 on one of the three is the whole call's refusal (and one refusal in the engine's run, below).
    assert_eq!(engine.account_basics().err(), Some(AccountCallError::SessionEnded));
    server.name_mode.store(ANSWER, Ordering::SeqCst);
    server.read_mode.store(EMPTY, Ordering::SeqCst);
    assert!(engine.account_sessions().expect("answers").value.sessions.is_empty());
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert_eq!(engine.account_sessions().err(), Some(AccountCallError::ServerError));
    assert_eq!(engine.account_display_name().err(), Some(AccountCallError::ServerError));
    server.read_mode.store(SESSION_ENDED, Ordering::SeqCst);
    assert_eq!(engine.account_sessions().err(), Some(AccountCallError::SessionEnded));
    server.read_mode.store(ANSWER, Ordering::SeqCst);

    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert_eq!(
        engine.revoke_account_session(turn, "s-web").err(),
        Some(AccountCallError::ServerError)
    );
    assert_eq!(
        engine.revoke_other_account_sessions(turn).err(),
        Some(AccountCallError::ServerError)
    );
    assert_eq!(
        engine.end_account_session(turn, "s-web").err(),
        Some(AccountCallError::ServerError),
        "a 500 is not a 404"
    );
    server.change_mode.store(ANSWER, Ordering::SeqCst);
    assert_eq!(engine.revoke_account_session(turn, "s-web"), Ok(()));
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-web", "s-web", "s-web"]);
    // Ended before: Beebeeb answers 404, told apart from every other refusal. For the plain call there is
    // nothing left to end either.
    assert_eq!(engine.end_account_session(turn, "s-web"), Ok(SessionEnd::AlreadyGone));
    assert_eq!(engine.revoke_account_session(turn, "s-web"), Ok(()));
    assert_eq!(engine.end_account_session(turn, "s-phone"), Ok(SessionEnd::Ended));
    assert_eq!(server.deleted.lock().unwrap().len(), 6);

    // The engine holds the session as ended: nothing is asked and nothing is sent.
    for _ in 0..3 {
        engine.note_call_for_test(true);
    }
    assert!(engine.account_session_refused());
    let asked = server.fake.routes().len();
    assert_eq!(engine.account_basics().err(), Some(AccountCallError::SessionEnded));
    assert_eq!(
        engine.end_account_session(turn, "s-this").err(),
        Some(AccountCallError::SessionEnded)
    );
    assert_eq!(server.fake.routes().len(), asked);
    engine.note_call_for_test(false);
    assert!(!engine.account_session_refused());

    // Another session (a lock and an unlock), then another account: a change stamped with the old session sends
    // nothing, and a read answers for the account that is signed in.
    engine.lock();
    assert_eq!(engine.account_sessions().err(), Some(AccountCallError::NotSignedIn));
    assert_eq!(
        engine.revoke_account_session(turn, "s-phone").err(),
        Some(AccountCallError::NotSignedIn)
    );
    engine
        .unlock_with_phrase(Zeroizing::new(server.people[0].phrase.clone()))
        .expect("the phrase unlocks");
    assert_eq!(
        engine.revoke_account_session(turn, "s-phone").err(),
        Some(AccountCallError::NotSignedIn)
    );
    assert_eq!(
        engine.revoke_other_account_sessions(turn).err(),
        Some(AccountCallError::NotSignedIn)
    );
    engine.sign_out_keeping_local_data().expect("signs out");
    sign_in(&engine, &server.people[1]);
    assert_eq!(
        engine.revoke_account_session(turn, "t-this").err(),
        Some(AccountCallError::NotSignedIn)
    );
    assert_eq!(
        engine.end_account_session(turn, "t-this").err(),
        Some(AccountCallError::NotSignedIn)
    );
    assert_eq!(server.deleted.lock().unwrap().len(), 6, "nothing more was sent");
    assert_eq!(server.revoked_all.load(Ordering::SeqCst), 0);
    assert_eq!(engine.account_profile().expect("answers").value.email, SECOND);
    assert_eq!(engine.account_display_name().expect("answers").value, None);
    let basics = engine.account_basics().expect("answers");
    assert_eq!(basics.session, engine.session_turn());
    assert_eq!(
        (basics.value.profile.email.as_str(), &basics.value.display_name),
        (SECOND, &Ok(None))
    );
    assert_eq!(engine.revoke_other_account_sessions(engine.session_turn()), Ok(0));
}

/// One rule for "Beebeeb no longer accepts this session": the account's calls count in the engine's run of
/// 401s. Three in a row and the engine holds the session as ended (nothing more is asked); fewer do not; a call
/// that works clears the run; a failure that is not a 401 neither counts nor clears; the three requests of the
/// Account read are one call; an answer that lands after its session moved on touches nothing.
#[test]
fn the_accounts_calls_count_in_the_engines_run_of_401s() {
    let server = server();
    let engine = signed_in(&server);
    let turn = engine.session_turn();
    let refuse = |times: usize| {
        server.read_mode.store(SESSION_ENDED, Ordering::SeqCst);
        for _ in 0..times {
            assert_eq!(engine.account_sessions().err(), Some(AccountCallError::SessionEnded));
        }
        server.read_mode.store(ANSWER, Ordering::SeqCst);
    };
    // Two 401s, then a call that works, then two more: never three in a row.
    refuse(2);
    assert!(!engine.account_session_refused(), "two are not a run");
    engine.account_sessions().expect("answers");
    refuse(2);
    assert!(
        !engine.account_session_refused(),
        "the call that worked cleared the two before it"
    );
    // A failure that is not a 401 neither counts nor clears: 2, a 500, then 1 more is three.
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert_eq!(engine.account_sessions().err(), Some(AccountCallError::ServerError));
    assert!(!engine.account_session_refused(), "a 500 is not a refusal");
    refuse(1);
    assert!(engine.account_session_refused(), "three 401s in a row");
    let asked = server.fake.routes().len();
    assert_eq!(engine.account_sessions().err(), Some(AccountCallError::SessionEnded));
    assert_eq!(server.fake.routes().len(), asked, "held as ended: nothing is asked");
    engine.note_call_for_test(false);

    // The Account read is three requests and ONE call: a 401 on two of them at once is one refusal.
    server.name_mode.store(SESSION_ENDED, Ordering::SeqCst);
    server.plan_mode.store(SESSION_ENDED, Ordering::SeqCst);
    for _ in 0..2 {
        assert_eq!(engine.account_basics().err(), Some(AccountCallError::SessionEnded));
    }
    assert!(!engine.account_session_refused(), "two reads, two refusals: not six");
    assert_eq!(engine.account_basics().err(), Some(AccountCallError::SessionEnded));
    assert!(engine.account_session_refused(), "the third read");
    server.name_mode.store(ANSWER, Ordering::SeqCst);
    server.plan_mode.store(ANSWER, Ordering::SeqCst);
    engine.note_call_for_test(false);
    // A part that fails without a 401 is not a refusal, and the read that answered clears the run.
    refuse(2);
    server.plan_mode.store(SERVER_ERROR, Ordering::SeqCst);
    engine.account_basics().expect("the profile answered");
    server.plan_mode.store(ANSWER, Ordering::SeqCst);
    refuse(2);
    assert!(!engine.account_session_refused());
    engine.note_call_for_test(false);

    // A change counts the same way.
    server.change_mode.store(SESSION_ENDED, Ordering::SeqCst);
    for id in ["s-web", "s-phone"] {
        assert_eq!(
            engine.end_account_session(turn, id).err(),
            Some(AccountCallError::SessionEnded)
        );
    }
    assert!(!engine.account_session_refused());
    assert_eq!(
        engine.revoke_other_account_sessions(turn).err(),
        Some(AccountCallError::SessionEnded)
    );
    assert!(engine.account_session_refused(), "three changes answered 401");
    server.change_mode.store(ANSWER, Ordering::SeqCst);
    engine.note_call_for_test(false);

    // An answer that lands after its session moved on (a lock during the call) clears nothing: two 401s, a
    // read that answers too late, one more 401 after the unlock is three in a row.
    refuse(2);
    let locks = engine.clone();
    *server.during_read.lock().unwrap() = Some(Box::new(move || {
        locks.lock();
    }) as Box<dyn FnOnce() + Send>);
    assert_eq!(engine.account_sessions().err(), Some(AccountCallError::NotSignedIn));
    engine
        .unlock_with_phrase(Zeroizing::new(server.people[0].phrase.clone()))
        .expect("the phrase unlocks");
    assert!(!engine.account_session_refused());
    refuse(1);
    assert!(
        engine.account_session_refused(),
        "the late answer of the session before did not clear the run"
    );
}

/// The session is locked while Beebeeb is answering: the engine hands the answer to nobody, and the source
/// (asked the same way) holds nothing of it.
#[test]
fn an_answer_that_lands_after_a_lock_is_handed_to_nobody() {
    let server = server();
    let engine = signed_in(&server);
    let lock = |engine: &TestEngine| {
        let engine = engine.clone();
        Box::new(move || {
            engine.lock();
        }) as Box<dyn FnOnce() + Send>
    };
    *server.during_read.lock().unwrap() = Some(lock(&engine));
    assert_eq!(engine.account_sessions().err(), Some(AccountCallError::NotSignedIn));
    assert_eq!(
        server.asked("GET /api/v1/account/sessions"),
        1,
        "it was asked, and answered"
    );

    engine
        .unlock_with_phrase(Zeroizing::new(server.people[0].phrase.clone()))
        .expect("the phrase unlocks");
    let mut source = Source::new(engine.clone());
    *server.during_read.lock().unwrap() = Some(lock(&engine));
    opened(&mut source, Pane::Sessions);
    assert!(source.sessions().is_empty());
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::SignedOut);
}
