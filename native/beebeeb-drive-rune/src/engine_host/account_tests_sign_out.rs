//! Devices and sessions on the real engine: a sign-out of another session, every way it can end (taken, already
//! gone, refused, not known, never sent), and what the pane says and reads after each. The fake and its helpers
//! are the parent module's.

use super::*;

#[test]
fn a_sign_out_is_sent_once_by_id_and_the_row_goes_only_when_beebeeb_says_so() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);

    // Never this session, never an id the list does not have: refused before anything is sent, and said.
    for id in ["s-this", "s-unknown", ""] {
        assert!(!source.sign_out_device(id), "{id:?}");
        assert_eq!(source.notice(), Some(NOT_SENT), "{id:?}");
        assert!(source.notice_failed(), "{id:?}: said as a failure");
        assert_eq!(source.notice_action(), None, "{id:?}: nothing to send again");
        source.clear_notice();
    }
    // Never the plural through an id.
    assert!(!source.sign_out_device("*"));
    settle(&mut source);
    assert!(server.deleted.lock().unwrap().is_empty());
    assert_eq!(server.revoked_all.load(Ordering::SeqCst), 0);
    assert_eq!(source.notice(), None);

    server.change_delay_ms.store(300, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    assert!(
        !source.sign_out_device("s-phone"),
        "a second press while it is on its way sends nothing"
    );
    assert_eq!(source.notice(), Some(STILL_SIGNING_OUT));
    assert_eq!(
        ids(&source),
        ["s-this", "s-phone", "s-web"],
        "the row stays until Beebeeb answered"
    );
    settle(&mut source);
    assert_eq!(
        server.deleted.lock().unwrap().clone(),
        ["s-phone"],
        "sent once, by its id"
    );
    assert_eq!(source.notice(), Some(SIGNED_OUT));
    assert_eq!(ids(&source), ["s-this", "s-web"], "read again after the answer");

    // The mark is cleared: another session can be signed out, and the same press works again later.
    server.change_delay_ms.store(0, Ordering::SeqCst);
    assert!(source.sign_out_device("s-web"));
    assert_eq!(source.notice(), None, "the old toast goes with the next press");
    settle(&mut source);
    assert_eq!(ids(&source), ["s-this"]);
    assert_eq!(server.deleted.lock().unwrap().len(), 2);
    source.clear_notice();
    assert_eq!(source.notice(), None);
}

/// Beebeeb answered a sign-out with an error (a 5xx): it is said as not done, never as "nothing changed", the
/// list is read again, and the toast can send it again.
#[test]
fn a_sign_out_beebeeb_did_not_take_is_said_read_again_and_can_be_sent_again() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    let lists = |server: &Server| server.asked("GET /api/v1/account/sessions");

    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let reads = lists(&server);
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT));
    assert!(source.notice_failed());
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    assert_eq!(lists(&server), reads + 1, "the list is read again after the answer");
    assert_eq!(ids(&source).len(), 3, "Beebeeb still lists it: the row stands");

    // "Try again" sends the same sign-out once more, through the same checks.
    server.change_mode.store(ANSWER, Ordering::SeqCst);
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone", "s-phone"]);
    assert_eq!(source.notice(), Some(SIGNED_OUT));
    assert!(!source.notice_failed());
    assert_eq!(source.notice_action(), None);
    assert_eq!(ids(&source), ["s-this", "s-web"]);
    // Nothing left to send again.
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(server.deleted.lock().unwrap().len(), 2);

    // The plural has its own words, and the same way back.
    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.sign_out_everywhere_else();
    settle(&mut source);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT_ELSEWHERE));
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    assert_eq!(server.revoked_all.load(Ordering::SeqCst), 0);
    server.change_mode.store(ANSWER, Ordering::SeqCst);
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(server.revoked_all.load(Ordering::SeqCst), 1);
    assert_eq!(source.notice(), Some(SIGNED_OUT_ELSEWHERE));
    assert_eq!(ids(&source), ["s-this"]);
}

/// A gateway answered 502 for a sign-out Beebeeb took: the words follow the list that is read after it.
#[test]
fn a_sign_out_taken_behind_an_error_is_said_as_signed_out_once_the_list_says_so() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.change_mode.store(TAKEN_BEHIND_A_502, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(ids(&source), ["s-this", "s-web"], "the row goes with Beebeeb's list");
    assert_eq!(
        source.notice(),
        Some(IS_SIGNED_OUT),
        "not \"couldn't\": the list no longer has it"
    );
    assert_eq!(source.notice_action(), None, "and nothing to send again");
}

/// The session had ended before the press (signed out on the device itself, or from the web): Beebeeb answers
/// 404. It is not a failure: the list is read again and the row goes.
#[test]
fn a_session_that_was_already_signed_out_goes_when_the_list_is_read_again() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    // It ends elsewhere while the pane still lists it.
    server.people[0]
        .sessions
        .lock()
        .unwrap()
        .retain(|s| s["id"] != json!("s-phone"));
    assert_eq!(ids(&source).len(), 3, "the stale row");
    let reads = server.asked("GET /api/v1/account/sessions");
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(
        server.deleted.lock().unwrap().clone(),
        ["s-phone"],
        "asked once; Beebeeb said 404"
    );
    assert_eq!(server.asked("GET /api/v1/account/sessions"), reads + 1);
    assert_eq!(ids(&source), ["s-this", "s-web"], "the row is gone after the re-read");
    assert_eq!(source.notice(), Some(IS_SIGNED_OUT));
    assert_eq!(source.notice_action(), None);
    assert!(!source.notice_failed());
    // The row is gone: a late press for it is refused before anything is sent.
    assert!(!source.sign_out_device("s-phone"));
    assert_eq!(source.notice(), Some(NOT_SENT));
    assert_eq!(server.deleted.lock().unwrap().len(), 1);
}

/// A 404 that did not come from Beebeeb's list (something in front answered it): the list that is read after it
/// still has the session, so the words are taken back and the sign-out can be sent again.
#[test]
fn a_404_for_a_session_the_list_still_has_is_not_said_as_signed_out() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.change_mode.store(A_404_THAT_ENDED_NOTHING, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(ids(&source).len(), 3, "Beebeeb still lists it");
    assert_eq!(
        source.notice(),
        Some(NOT_SIGNED_OUT),
        "never \"signed out\" beside a row that stands"
    );
    assert!(source.notice_failed());
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
}

/// A list that was asked before a sign-out, and answers after the person left the pane, is stale: it is not
/// drawn, and nothing is read for it until the pane shows again.
#[test]
fn a_stale_list_is_not_read_again_while_the_pane_does_not_show() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.read_delay_ms.store(300, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    assert!(source.sign_out_device("s-phone"));
    source.opened(Pane::Account);
    settle(&mut source);
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone"]);
    assert_eq!(
        server.asked("GET /api/v1/account/sessions"),
        2,
        "the first read and the one that was out: none is started for a pane that does not show"
    );
    server.read_delay_ms.store(0, Ordering::SeqCst);
    opened(&mut source, Pane::Sessions);
    assert_eq!(
        server.asked("GET /api/v1/account/sessions"),
        3,
        "read when it shows again"
    );
    assert_eq!(ids(&source), ["s-this", "s-web"]);
}

#[test]
fn an_unconfirmed_sign_out_reads_again_and_a_failed_re_read_keeps_the_words() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    let reads = server.asked("GET /api/v1/account/sessions");

    // The connection broke: not known, so the list is read again; and when that read fails too, the words stay.
    server.change_mode.store(DROP_CONNECTION, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    // The list answers and still has the session: it was not signed out, and that is what is said now (never
    // "checking" after the check is done), with the way to send it again.
    assert_eq!(server.asked("GET /api/v1/account/sessions"), reads + 1);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT));
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    assert_eq!(ids(&source).len(), 3);
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(
        source.notice(),
        Some(UNKNOWN_SIGN_OUT),
        "the re-read failed: the words stay"
    );
    assert!(source.notice_failed());
    assert_eq!(source.notice_action(), None, "not known is not \"send it again\"");
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::Failed);

    // Refused, then the re-read fails: the refusal's words stay too.
    server.read_mode.store(ANSWER, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    settle(&mut source);
    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT));
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::Failed);

    // Made, then the re-read fails: said as made, and the list is not drawn from before the change.
    server.read_mode.store(ANSWER, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    settle(&mut source);
    server.change_mode.store(ANSWER, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    settle(&mut source);
    assert_eq!(source.notice(), Some(SIGNED_OUT));
    assert!(source.sessions().is_empty(), "never the list from before the change");

    // A toast is never drawn in another session, even when nothing else is held for the poll to drop.
    let engine = source.engine.clone();
    engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(server.people[0].phrase.clone()))
        .expect("the phrase unlocks");
    assert_eq!(source.notice(), None, "before any poll");
    assert_eq!(source.notice_action(), None);
}

/// The answer to a sign-out lands after the person left Devices and sessions: no toast over another pane, no
/// read for a pane that does not show, and the list is read when the pane opens again.
#[test]
fn a_sign_out_answered_after_the_pane_was_left_says_nothing_and_is_read_on_return() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    opened(&mut source, Pane::Account);
    source.opened(Pane::Sessions);
    for mode in [SERVER_ERROR, ANSWER] {
        server.change_mode.store(mode, Ordering::SeqCst);
        server.change_delay_ms.store(200, Ordering::SeqCst);
        assert!(source.sign_out_device("s-phone"), "mode {mode}");
        source.clear_notice();
        source.opened(Pane::Account);
        let reads = server.asked("GET /api/v1/account/sessions");
        settle(&mut source);
        assert_eq!(source.notice(), None, "mode {mode}: no toast over Account");
        assert_eq!(source.notice_action(), None, "mode {mode}");
        assert_eq!(
            server.asked("GET /api/v1/account/sessions"),
            reads,
            "mode {mode}: nothing is read"
        );
        // Back within the freshness window: what is held is older than the answer, so it is read. A sign-out
        // that was not done is said now, over its own pane, with the way to send it again.
        opened(&mut source, Pane::Sessions);
        assert_eq!(server.asked("GET /api/v1/account/sessions"), reads + 1, "mode {mode}");
        let failed = mode == SERVER_ERROR;
        assert_eq!(source.notice(), failed.then_some(NOT_SIGNED_OUT), "mode {mode}");
        assert_eq!(source.notice_action(), failed.then_some(TRY_AGAIN), "mode {mode}");
    }
    assert_eq!(ids(&source), ["s-this", "s-web"]);
}

/// A worker that panics answers as a call that failed: "Couldn't load" with a way to try again, never
/// "Loading" for ever; and a sign-out whose worker panicked is said as not known.
#[test]
fn a_worker_that_panics_ends_as_a_failed_read_and_an_unknown_sign_out() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    for pane in [Pane::Account, Pane::Sessions] {
        source.panic_next.store(true, Ordering::SeqCst);
        opened(&mut source, pane);
        assert_eq!(source.pane_state(pane), PaneState::Failed, "{pane:?}");
        source.reload(pane);
        settle(&mut source);
        assert_eq!(source.pane_state(pane), PaneState::Ready, "{pane:?}: Try again reads");
    }
    let reads = server.asked("GET /api/v1/account/sessions");
    source.panic_next.store(true, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert!(server.deleted.lock().unwrap().is_empty(), "the worker never asked");
    assert_eq!(
        server.asked("GET /api/v1/account/sessions"),
        reads + 1,
        "the list is read again"
    );
    assert_eq!(
        source.notice(),
        Some(NOT_SIGNED_OUT),
        "not known, then the list still has it"
    );
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    // The mark is cleared: the press works again.
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(source.notice(), Some(SIGNED_OUT));
}

/// A list that was asked before a sign-out and answers after it is older than the change: never drawn.
#[test]
fn a_list_read_before_a_sign_out_is_not_drawn_after_it() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    // The fake takes the list when it is asked and answers 1.5 s later: this read holds three rows.
    server.read_delay_ms.store(1_500, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    let began = Instant::now();
    while server.asked("GET /api/v1/account/sessions") < 2 {
        assert!(began.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone"]);
    assert_eq!(
        ids(&source),
        ["s-this", "s-web"],
        "the list from before the sign-out is dropped and read again"
    );
    assert_eq!(server.asked("GET /api/v1/account/sessions"), 3);
}

#[test]
fn sign_out_everywhere_else_is_sent_once_and_leaves_this_session() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.change_delay_ms.store(200, Ordering::SeqCst);
    source.sign_out_everywhere_else();
    source.sign_out_everywhere_else();
    assert_eq!(source.notice(), Some(STILL_SIGNING_OUT));
    settle(&mut source);
    assert_eq!(server.revoked_all.load(Ordering::SeqCst), 1, "sent once");
    assert!(server.deleted.lock().unwrap().is_empty());
    assert_eq!(source.notice(), Some(SIGNED_OUT_ELSEWHERE));
    assert_eq!(ids(&source), ["s-this"]);
    // Nobody else is signed in: nothing to send.
    source.sign_out_everywhere_else();
    settle(&mut source);
    assert_eq!(server.revoked_all.load(Ordering::SeqCst), 1);
}

#[test]
fn a_session_that_changes_while_a_sign_out_is_on_its_way_says_nothing_to_the_next_one() {
    let server = server();
    let engine = signed_in(&server);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Sessions);
    server.change_delay_ms.store(300, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    // Lock, then unlock: another session of the same account.
    engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(server.people[0].phrase.clone()))
        .expect("the phrase unlocks");
    assert!(
        source.sessions().is_empty(),
        "the list of the session before is not handed out"
    );
    settle(&mut source);
    assert_eq!(source.notice(), None, "the answer of the session before is not said");
    // Whether the sign-out left before the lock or was refused by the engine after it: the new session draws
    // the list Beebeeb has now, read by itself.
    let has_now: Vec<String> = server.people[0]
        .sessions
        .lock()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect();
    source.reload(Pane::Sessions);
    settle(&mut source);
    assert_eq!(ids(&source), has_now, "the new session read its own list");
    assert_eq!(source.notice(), None);
    assert_eq!(source.pump(), Pumped::default(), "no mark of the old change is left");
}
