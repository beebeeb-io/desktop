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
    // Its Try again sends by id, for the sessions that were confirmed (here the one that is left): never the
    // one request for "all others" a second time.
    server.change_mode.store(ANSWER, Ordering::SeqCst);
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(server.revoked_all.load(Ordering::SeqCst), 0);
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone", "s-phone", "s-web"]);
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
fn an_unconfirmed_sign_out_reads_again_and_a_failed_re_read_keeps_the_list_and_the_way_back() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    let reads = server.asked("GET /api/v1/account/sessions");

    // The connection broke: not known, so the list is read again.
    server.change_mode.store(DROP_CONNECTION, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    // The list answers and still has the session: it was not signed out, and that is what is said now (never
    // "checking" after the check is done), with the way to send it again.
    assert_eq!(server.asked("GET /api/v1/account/sessions"), reads + 1);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT));
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    assert_eq!(ids(&source).len(), 3);
    // Not known, and the check fails too: no longer "checking", never "signed out", and the list that was
    // drawn stays (nothing was taken for certain), so the row can be pressed again.
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(source.notice(), Some(UNCONFIRMED_SIGN_OUT));
    assert!(source.notice_failed());
    assert_eq!(source.notice_action(), None, "not known is not \"send it again\"");
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::Ready);
    assert_eq!(ids(&source).len(), 3);
    // Its time is up: the pane still says what it draws is old.
    later(&mut source, STAYS_FAILED);
    assert_eq!(source.notice(), Some(NOT_REFRESHED));

    // Refused (a 5xx), and the re-read fails in the same outage: the refusal's words stay, the list stays, and
    // TRY AGAIN STILL WORKS: it sends that sign-out once more.
    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT));
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::Ready);
    let sent = server.deleted.lock().unwrap().len();
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(server.deleted.lock().unwrap().len(), sent + 1, "sent again");
    assert_eq!(
        source.notice(),
        Some(NOT_SIGNED_OUT),
        "refused again: the button is still there"
    );
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    // The outage ends for the sign-out, not yet for the list: sent, taken, said as made; and the list from
    // before the change is NOT drawn as the list of now.
    server.change_mode.store(ANSWER, Ordering::SeqCst);
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(server.deleted.lock().unwrap().len(), sent + 2);
    assert_eq!(source.notice(), Some(SIGNED_OUT));
    assert!(!source.notice_failed());
    assert!(source.sessions().is_empty(), "never the list from before the change");
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::Failed);

    // A 404 (Beebeeb has no such session), and the re-read fails: not said as signed out, for the list has not
    // confirmed it.
    server.read_mode.store(ANSWER, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    settle(&mut source);
    assert_eq!(ids(&source), ["s-this", "s-web"]);
    server.change_mode.store(A_404_THAT_ENDED_NOTHING, Ordering::SeqCst);
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert!(source.sign_out_device("s-web"));
    settle(&mut source);
    assert_eq!(source.notice(), Some(UNCONFIRMED_SIGN_OUT));
    assert!(source.notice_failed(), "never the mark of a thing that worked");
    assert_eq!(ids(&source), ["s-this", "s-web"]);
    // The list answers and still has it: not signed out, and it can be sent again.
    server.read_mode.store(ANSWER, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    settle(&mut source);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT));
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));

    // A toast is never drawn in another session, even when nothing else is held for the poll to drop.
    let engine = source.engine.clone();
    engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(server.people[0].phrase.clone()))
        .expect("the phrase unlocks");
    assert_eq!(source.notice(), None, "before any poll");
    assert_eq!(source.notice_action(), None);
}

/// Two sign-outs answered one after the other, the first refused, the second taken: the failure is not replaced
/// by the success. Each toast has its time from the moment it shows, then the next one shows.
#[test]
fn a_failure_is_not_hidden_by_a_later_success_and_each_toast_has_its_time() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    *server.refused_ids.lock().unwrap() = vec!["s-phone"];
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT));
    // The second press keeps what was not done for the first.
    assert!(source.sign_out_device("s-web"));
    assert_eq!(
        source.notice(),
        Some(NOT_SIGNED_OUT),
        "a press for another row keeps the failure"
    );
    settle(&mut source);
    assert_eq!(ids(&source), ["s-this", "s-phone"]);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT), "the success did not replace it");
    assert!(source.notice_failed());
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    // The failure's time, from when it showed; the window is told when to look again.
    let wake = source.pump().wake_in.expect("a toast shows: the window looks again");
    assert!(
        wake <= STAYS_FAILED && wake > STAYS_FAILED - Duration::from_secs(4),
        "{wake:?}"
    );
    let early = later(&mut source, STAYS_FAILED - Duration::from_secs(4));
    assert!(
        early.wake_in.is_some_and(|left| left <= Duration::from_secs(4)),
        "{early:?}"
    );
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT), "not before its time");
    let pumped = later(&mut source, Duration::from_secs(4));
    assert!(pumped.changed, "the window draws the next one");
    assert_eq!(source.notice(), Some(SIGNED_OUT), "then what worked is said");
    assert!(!source.notice_failed());
    assert_eq!(
        pumped.wake_in,
        Some(STAYS_DONE),
        "its time starts when it shows, not when it was said"
    );
    assert_eq!(later(&mut source, STAYS_DONE).wake_in, None);
    assert_eq!(source.notice(), None, "and it goes by itself");
    assert_eq!(source.notice_action(), None);

    // The other order: what worked is up, then a refusal lands: the refusal shows at once.
    let server = self::server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    *server.refused_ids.lock().unwrap() = vec!["s-phone"];
    assert!(source.sign_out_device("s-web"));
    settle(&mut source);
    assert_eq!(source.notice(), Some(SIGNED_OUT));
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT));

    // Two refusals: both are said, one after the other, each with its own Try again; and Try again sends the
    // one its toast is about.
    let server = self::server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    *server.refused_ids.lock().unwrap() = vec!["s-phone", "s-web"];
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert!(source.sign_out_device("s-web"));
    settle(&mut source);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT));
    later(&mut source, STAYS_FAILED);
    assert_eq!(
        source.notice(),
        Some(NOT_SIGNED_OUT),
        "the second refusal, after the first had its time"
    );
    server.refused_ids.lock().unwrap().clear();
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(
        server.deleted.lock().unwrap().clone(),
        ["s-phone", "s-web", "s-web"],
        "the one that toast was about"
    );
    assert_eq!(ids(&source), ["s-this", "s-phone"]);

    // A refusal that waited behind another and never showed is not lost when the pane is left: it is said
    // when Devices and sessions shows again. The one that did show is not said twice.
    let server = self::server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    *server.refused_ids.lock().unwrap() = vec!["s-phone", "s-web"];
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert!(source.sign_out_device("s-web"));
    settle(&mut source);
    source.clear_notice();
    opened(&mut source, Pane::Account);
    assert_eq!(source.notice(), None, "no toast about a sign-out over Account");
    server.refused_ids.lock().unwrap().clear();
    source.clear_notice();
    opened(&mut source, Pane::Sessions);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT));
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(
        server.deleted.lock().unwrap().clone(),
        ["s-phone", "s-web", "s-web"],
        "the one that had not been said"
    );
    later(&mut source, STAYS_DONE);
    assert_eq!(source.notice(), None, "nothing else waits");
}

/// The line the toasts wait in: what was not done goes before what worked (which is up or waits), refusals keep
/// their order, one toast per sign-out, and what worked replaces only what worked.
#[test]
fn the_line_puts_what_was_not_done_first_and_never_drops_it_for_what_worked() {
    let server = server();
    let source = Source::new(signed_in(&server));
    let turn = source.engine.session_turn();
    let about = |id: &str| {
        Some(About {
            key: id.to_string(),
            ids: vec![id.to_string()],
        })
    };
    let words = |line: &Line| line.iter().map(|(_, said)| said.words).collect::<Vec<_>>();
    let mut line = Line::new();
    line_up(&mut line, turn, Said::done(SIGNED_OUT));
    line_up(&mut line, turn, Said::failed(NOT_SIGNED_OUT, about("a"), true));
    assert_eq!(words(&line), [NOT_SIGNED_OUT, SIGNED_OUT], "the refusal shows first");
    line_up(&mut line, turn, Said::failed(UNKNOWN_SIGN_OUT, about("b"), false));
    assert_eq!(
        words(&line),
        [NOT_SIGNED_OUT, UNKNOWN_SIGN_OUT, SIGNED_OUT],
        "refusals in their order"
    );
    line_up(&mut line, turn, Said::done(SIGNED_OUT_ELSEWHERE));
    assert_eq!(
        words(&line),
        [NOT_SIGNED_OUT, UNKNOWN_SIGN_OUT, SIGNED_OUT_ELSEWHERE],
        "what worked replaces what worked, and nothing else"
    );
    // A newer word about the same sign-out replaces the older one.
    line_up(&mut line, turn, Said::failed(NOT_SENT, about("a"), true));
    assert_eq!(words(&line), [UNKNOWN_SIGN_OUT, NOT_SENT, SIGNED_OUT_ELSEWHERE]);
    // The same sentence about no sign-out is said once.
    line_up(&mut line, turn, Said::failed(STILL_SIGNING_OUT, None, false));
    line_up(&mut line, turn, Said::failed(STILL_SIGNING_OUT, None, false));
    assert_eq!(line.len(), 4);
}

/// A toast is never said over "your session ended", also when nothing else of the account is held.
#[test]
fn no_toast_is_said_once_the_engine_holds_the_session_as_ended() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert!(source.sign_out_device("s-web"));
    settle(&mut source);
    assert!(
        source.sessions().is_empty(),
        "taken, and not read after it: no list is held"
    );
    assert_eq!(source.notice(), Some(SIGNED_OUT));
    for _ in 0..3 {
        source.engine.note_call_for_test(true);
    }
    assert_eq!(source.notice(), None, "before any poll");
    assert_eq!(source.notice_action(), None);
    assert!(source.pump().changed, "and the poll drops it");
    source.engine.note_call_for_test(false);
    source.pump();
    settle(&mut source);
    assert_eq!(source.notice(), None, "it does not come back with the session");
}

/// A sign-out whose result is not known says "checking" for as long as the check is out, however long: the
/// list's answer must find the toast to put its word in. And every 404 of a Try again for "everywhere else" is
/// not known either, until the list says so.
#[test]
fn checking_stays_until_the_list_answered_and_a_404_one_by_one_is_not_a_yes() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.change_mode.store(DROP_CONNECTION, Ordering::SeqCst);
    server.read_delay_ms.store(600, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    let began = Instant::now();
    while source.notice() != Some(UNKNOWN_SIGN_OUT) {
        assert!(began.elapsed() < Duration::from_secs(10), "the answer never came");
        std::thread::sleep(Duration::from_millis(5));
        source.pump();
    }
    // Far past the toast's time, with the list still out.
    assert!(
        later(&mut source, STAYS_FAILED * 3).busy,
        "precondition: the list is still out"
    );
    assert_eq!(source.notice(), Some(UNKNOWN_SIGN_OUT), "still checking");
    settle(&mut source);
    assert_eq!(
        source.notice(),
        Some(NOT_SIGNED_OUT),
        "the list's word reached the toast"
    );
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    server.read_delay_ms.store(0, Ordering::SeqCst);
    later(&mut source, STAYS_FAILED);
    assert_eq!(
        source.notice(),
        None,
        "that toast had its time: the line is empty for what follows"
    );

    // "Everywhere else" refused, then its Try again is answered 404 for every session while they all stand.
    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.sign_out_everywhere_else();
    settle(&mut source);
    server.change_mode.store(A_404_THAT_ENDED_NOTHING, Ordering::SeqCst);
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(ids(&source).len(), 3, "Beebeeb still lists them");
    assert_eq!(
        source.notice(),
        Some(NOT_SIGNED_OUT_ELSEWHERE),
        "never \"signed out\" beside rows that stand"
    );
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    // And with the list failing after it: the rows that were drawn stay (nothing was taken for certain).
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(source.notice(), Some(UNCONFIRMED_SIGN_OUT));
    assert_eq!(ids(&source).len(), 3);
}

/// A list read that was out when a sign-out was answered, and then fails: the list is still read once for
/// that answer (as it is when that earlier read answers).
#[test]
fn a_read_that_fails_after_an_answered_sign_out_is_followed_by_the_read_for_that_answer() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    // The read is asked first and dropped by the fake 400 ms later; the sign-out is answered in between.
    server.read_mode.store(DROP_CONNECTION, Ordering::SeqCst);
    *server.during_failed_read.lock().unwrap() = Some(Box::new(|| std::thread::sleep(Duration::from_millis(400))));
    source.reload(Pane::Sessions);
    let began = Instant::now();
    while server.asked("GET /api/v1/account/sessions") < 2 {
        assert!(began.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    server.read_mode.store(ANSWER, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(
        server.asked("GET /api/v1/account/sessions"),
        3,
        "the first, the one that failed, and the one after the answer"
    );
    assert_eq!(ids(&source), ["s-this", "s-web"]);
    assert_eq!(source.notice(), Some(SIGNED_OUT));
}

/// Two sign-outs that were taken behind an error, and one list read that shows both gone: the same sentence
/// is said once, not once per toast.
#[test]
fn the_list_rewriting_two_toasts_to_the_same_words_says_them_once() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.change_mode.store(TAKEN_BEHIND_A_502, Ordering::SeqCst);
    server.read_delay_ms.store(300, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    assert!(source.sign_out_device("s-web"));
    settle(&mut source);
    assert_eq!(ids(&source), ["s-this"]);
    assert_eq!(source.notice(), Some(IS_SIGNED_OUT));
    assert!(!source.notice_failed());
    later(&mut source, STAYS_DONE);
    assert_eq!(source.notice(), None, "said once");
}

/// A refusal's Try again when there is no list to send against (another sign-out was taken, and the list could
/// not be read after it): nothing is sent, the toast keeps its button, and it works once the list is read.
#[test]
fn try_again_with_no_list_keeps_its_button_until_the_list_is_read() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    *server.refused_ids.lock().unwrap() = vec!["s-phone"];
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert!(source.sign_out_device("s-web"));
    settle(&mut source);
    assert_eq!(
        source.pane_state(Pane::Sessions),
        PaneState::Failed,
        "taken, and not read after it"
    );
    assert_eq!(
        source.notice(),
        Some(NOT_SIGNED_OUT),
        "the refusal is still what is said"
    );
    server.refused_ids.lock().unwrap().clear();
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(
        server.deleted.lock().unwrap().clone(),
        ["s-phone", "s-web"],
        "nothing was sent"
    );
    assert_eq!(source.notice(), Some(NOT_SENT));
    assert_eq!(source.notice_action(), Some(TRY_AGAIN), "the way back is not lost");
    server.read_mode.store(ANSWER, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    settle(&mut source);
    assert_eq!(ids(&source), ["s-this", "s-phone"]);
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone", "s-web", "s-phone"]);
    assert_eq!(source.notice(), Some(SIGNED_OUT));
    assert_eq!(ids(&source), ["s-this"]);
}

/// "Sign out everywhere else" that Beebeeb did not take: its Try again fades as every toast does, and it ends
/// only the sessions that were listed at the confirm. A session made since is never ended by it.
#[test]
fn try_again_for_everywhere_else_ends_only_the_sessions_that_were_confirmed() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.sign_out_everywhere_else();
    settle(&mut source);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT_ELSEWHERE));
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));

    // Someone signs in on another device while the toast is up, and the list is read again.
    let made_since = json!({ "id": "s-new", "device_name": "An invented tablet", "device_kind": "android",
        "country_code": "FI", "last_active_at": null, "is_current": false });
    server.people[0].sessions.lock().unwrap().push(made_since.clone());
    source.reload(Pane::Sessions);
    settle(&mut source);
    assert_eq!(ids(&source), ["s-this", "s-phone", "s-web", "s-new"]);
    assert_eq!(source.notice_action(), Some(TRY_AGAIN), "the toast is still up");

    // One of the confirmed sessions is refused again: per session, what ended and what did not.
    server.change_mode.store(ANSWER, Ordering::SeqCst);
    *server.refused_ids.lock().unwrap() = vec!["s-web"];
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(
        server.revoked_all.load(Ordering::SeqCst),
        0,
        "never \"all others\" again"
    );
    assert_eq!(
        server.deleted.lock().unwrap().clone(),
        ["s-phone", "s-web"],
        "never s-new"
    );
    assert_eq!(ids(&source), ["s-this", "s-web", "s-new"]);
    assert_eq!(source.notice(), Some(NOT_ALL_SIGNED_OUT));
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    // Again: only the one that is left of the confirmed set.
    server.refused_ids.lock().unwrap().clear();
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone", "s-web", "s-web"]);
    assert_eq!(server.revoked_all.load(Ordering::SeqCst), 0);
    assert_eq!(ids(&source), ["s-this", "s-new"], "the session made since stands");
    assert_eq!(
        source.notice(),
        Some(SIGNED_OUT_THOSE),
        "not \"everywhere else\": another session is listed"
    );
    assert_eq!(source.notice_action(), None);

    // The toast fades: after its time there is no button left to press at some later moment.
    let server = self::server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.sign_out_everywhere_else();
    settle(&mut source);
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    assert!(source.pump().wake_in.is_some());
    later(&mut source, STAYS_FAILED);
    assert_eq!(source.notice(), None);
    assert_eq!(source.notice_action(), None);
    server.change_mode.store(ANSWER, Ordering::SeqCst);
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(
        server.revoked_all.load(Ordering::SeqCst),
        0,
        "a press on nothing sends nothing"
    );
    assert!(server.deleted.lock().unwrap().is_empty());
    assert_eq!(ids(&source).len(), 3);

    // The result is not known (the connection broke) and Beebeeb took it: the list has none of the confirmed
    // sessions, and no other: "everywhere else".
    server.change_mode.store(DROP_CONNECTION, Ordering::SeqCst);
    source.sign_out_everywhere_else();
    server.people[0]
        .sessions
        .lock()
        .unwrap()
        .retain(|s| s["is_current"] == json!(true));
    settle(&mut source);
    assert_eq!(source.notice(), Some(SIGNED_OUT_ELSEWHERE));
    assert_eq!(ids(&source), ["s-this"]);
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
