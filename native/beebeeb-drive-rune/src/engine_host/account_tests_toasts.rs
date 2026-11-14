//! The toasts over Devices and sessions, where the line's rules decide what the person is told: a check that
//! fails late, a toast that takes another's place, a pane that is left while a check is out, and "everywhere
//! else" sent again one by one. The fake and its helpers are the parent module's; nothing here sleeps to order
//! two answers (a gate in the fake's handler holds a call until the test opens it).

use std::sync::mpsc;

use super::*;

/// A gate for one of the fake's hooks: the call waits in the handler until the test sends.
fn gate() -> (mpsc::Sender<()>, Box<dyn FnOnce() + Send>) {
    let (open, wait) = mpsc::channel();
    (
        open,
        Box::new(move || {
            let _ = wait.recv_timeout(Duration::from_secs(10));
        }),
    )
}

/// Poll until `until` holds (at most 10 s).
fn pump_until(source: &mut Source, what: &str, until: impl Fn(&Source) -> bool) {
    let began = Instant::now();
    loop {
        source.pump();
        if until(source) {
            return;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn about(id: &str) -> About {
    About {
        key: id.to_string(),
        ids: vec![id.to_string()],
    }
}

/// "Checking what Beebeeb has" was held far past its time because the check was out, and then the check
/// fails: "Couldn't confirm the sign-out" is a toast of its own, drawn for its whole time from then. It is never
/// gone in the poll that took the failed check in.
#[test]
fn a_check_that_fails_late_is_said_for_its_whole_time() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.change_mode.store(DROP_CONNECTION, Ordering::SeqCst);
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let (open, hook) = gate();
    *server.during_failed_read.lock().unwrap() = Some(hook);
    assert!(source.sign_out_device("s-phone"));
    pump_until(&mut source, "the sign-out never answered", |s| {
        s.notice() == Some(UNKNOWN_SIGN_OUT)
    });
    // Far past the toast's time, with the check still out.
    assert!(
        later(&mut source, STAYS_FAILED * 3).busy,
        "precondition: the check is out"
    );
    assert_eq!(source.notice(), Some(UNKNOWN_SIGN_OUT));
    open.send(()).unwrap();
    settle(&mut source);
    assert_eq!(source.notice(), Some(UNCONFIRMED_SIGN_OUT), "the failed check is said");
    assert!(source.notice_failed());
    let wake = source.pump().wake_in.expect("it shows: the window looks again");
    assert!(
        wake > STAYS_FAILED - Duration::from_secs(1),
        "its own time, from now: {wake:?}"
    );
    later(&mut source, STAYS_FAILED - Duration::from_secs(1));
    assert_eq!(source.notice(), Some(UNCONFIRMED_SIGN_OUT), "still within its time");
    later(&mut source, Duration::from_secs(1));
    assert_ne!(source.notice(), Some(UNCONFIRMED_SIGN_OUT), "and then it goes");
}

/// A list read that was out BEFORE the sign-out was answered fails: that was not the check. The check is the
/// read made after it, and until that one answers the toast still says "checking".
#[test]
fn a_read_from_before_the_answer_that_fails_does_not_end_the_check() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    let reads = server.asked("GET /api/v1/account/sessions");
    // A read that will fail is out, held in the fake.
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let (open_old, hook) = gate();
    *server.during_failed_read.lock().unwrap() = Some(hook);
    source.reload(Pane::Sessions);
    pump_until(&mut source, "the read never reached the fake", |_| {
        server.asked("GET /api/v1/account/sessions") == reads + 1
    });
    // The sign-out's connection breaks: not known. The read after it answers, and is held too.
    server.change_mode.store(DROP_CONNECTION, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    pump_until(&mut source, "the sign-out never answered", |s| {
        s.notice() == Some(UNKNOWN_SIGN_OUT)
    });
    server.read_mode.store(ANSWER, Ordering::SeqCst);
    let (open_check, hook) = gate();
    *server.during_read.lock().unwrap() = Some(hook);
    open_old.send(()).unwrap();
    pump_until(&mut source, "the check was never made", |_| {
        server.asked("GET /api/v1/account/sessions") == reads + 2
    });
    assert_eq!(
        source.notice(),
        Some(UNKNOWN_SIGN_OUT),
        "the check is out: still checking"
    );
    open_check.send(()).unwrap();
    settle(&mut source);
    assert_eq!(
        source.notice(),
        Some(NOT_SIGNED_OUT),
        "the list still has it: the check's word"
    );
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
}

/// Three sign-outs of three sessions are refused. Each toast has the same button, so each one that takes the
/// place of another reads differently from the one before it, and the button sends the session of the toast
/// that is drawn.
#[test]
fn a_toast_that_takes_the_place_of_one_about_another_session_reads_differently() {
    let server = server();
    server.people[0].sessions.lock().unwrap().push(
        json!({ "id": "s-new", "device_name": "An invented tablet", "device_kind": "android",
                "country_code": "FI", "last_active_at": null, "is_current": false }),
    );
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    *server.refused_ids.lock().unwrap() = vec!["s-phone", "s-web", "s-new"];
    for id in ["s-phone", "s-web", "s-new"] {
        assert!(source.sign_out_device(id));
        settle(&mut source);
    }
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT));
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    later(&mut source, STAYS_FAILED);
    assert_eq!(
        source.notice(),
        Some(NOT_SIGNED_OUT_ANOTHER),
        "not the sentence that just went"
    );
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    // Its button is about ITS session, and a refusal again keeps its words (the same session, the same toast).
    let sent = server.deleted.lock().unwrap().len();
    source.run_notice_action();
    // The third takes its place while it is sent: again not the sentence that was just there.
    source.pump();
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT_ONE_MORE));
    settle(&mut source);
    assert_eq!(
        server.deleted.lock().unwrap()[sent..],
        ["s-web"],
        "the session of the toast that was drawn"
    );
    // The toast that is drawn stays; what was not done for s-web again waits behind it, and then shows in
    // words that are not the ones that just went.
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT_ONE_MORE));
    later(&mut source, STAYS_FAILED);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT_ANOTHER));
    let sent = server.deleted.lock().unwrap().len();
    server.refused_ids.lock().unwrap().clear();
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(server.deleted.lock().unwrap()[sent..], ["s-web"]);
    // The line emptied: the next failure is not "another" one.
    later(&mut source, STAYS_FAILED);
    assert_eq!(source.notice(), None);
    *server.refused_ids.lock().unwrap() = vec!["s-phone"];
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT));
}

/// The same rule on the line itself, for "nothing was sent" and for a toast with no such button: only a toast
/// whose button sends ONE session's sign-out changes its words, and only after one about another session.
#[test]
fn only_a_toast_with_the_same_button_about_another_session_changes_its_words() {
    let turn = SessionGeneration::for_test(1);
    let now = Instant::now();
    let mut line = Line::new();
    let mut drawn = Drawn::default();
    for id in ["a", "b", "c"] {
        line_up(&mut line, turn, Said::failed(NOT_SENT, Some(about(id)), true));
    }
    // Everywhere else has its own sentence, and a toast without a button none to mistake.
    let elsewhere = About {
        key: ELSEWHERE.to_string(),
        ids: vec!["a".into()],
    };
    line_up(
        &mut line,
        turn,
        Said::failed(NOT_SIGNED_OUT_ELSEWHERE, Some(elsewhere), true),
    );
    line_up(&mut line, turn, Said::unconfirmed(Some(about("d"))));
    let mut words = Vec::new();
    for step in 0..5 {
        fade(&mut line, now + STAYS_FAILED * step, false, &mut drawn);
        words.push(line.front().map(|(_, said)| said.shown()));
    }
    assert_eq!(
        words,
        [
            Some(NOT_SENT),
            Some(NOT_SENT_ANOTHER),
            Some(NOT_SENT_ONE_MORE),
            Some(NOT_SIGNED_OUT_ELSEWHERE),
            Some(UNCONFIRMED_SIGN_OUT),
        ]
    );
    // About the SAME session again (the list put the toast in the line anew): the words it had.
    let mut line = Line::new();
    let mut drawn = Drawn::default();
    line_up(&mut line, turn, Said::failed(NOT_SIGNED_OUT, Some(about("a")), true));
    line_up(&mut line, turn, Said::failed(NOT_SIGNED_OUT, Some(about("b")), true));
    fade(&mut line, now, false, &mut drawn);
    fade(&mut line, now + STAYS_FAILED, false, &mut drawn);
    assert_eq!(line.front().map(|(_, said)| said.shown()), Some(NOT_SIGNED_OUT_ANOTHER));
    line_up(&mut line, turn, Said::failed(NOT_SIGNED_OUT, Some(about("b")), true));
    fade(&mut line, now + STAYS_FAILED, false, &mut drawn);
    assert_eq!(line.front().map(|(_, said)| said.shown()), Some(NOT_SIGNED_OUT_ANOTHER));
}

/// The person leaves Devices and sessions while "Checking what Beebeeb has" shows. Nothing is said over the
/// other pane; back on the pane, how the sign-out ended is said: not signed out (with the way to send it again),
/// or signed out.
#[test]
fn leaving_while_a_check_shows_still_says_how_the_sign_out_ended() {
    for taken in [false, true] {
        let server = server();
        let mut source = Source::new(signed_in(&server));
        opened(&mut source, Pane::Sessions);
        server.change_mode.store(DROP_CONNECTION, Ordering::SeqCst);
        let (open, hook) = gate();
        *server.during_read.lock().unwrap() = Some(hook);
        assert!(source.sign_out_device("s-phone"));
        pump_until(&mut source, "the sign-out never answered", |s| {
            s.notice() == Some(UNKNOWN_SIGN_OUT)
        });
        // It shows (the poll stamped it), and the person goes to Account.
        source.clear_notice();
        source.opened(Pane::Account);
        if taken {
            server.people[0]
                .sessions
                .lock()
                .unwrap()
                .retain(|s| s["id"] != json!("s-phone"));
        }
        open.send(()).unwrap();
        settle(&mut source);
        assert_eq!(source.notice(), None, "taken {taken}: no toast over Account");
        opened(&mut source, Pane::Sessions);
        if taken {
            assert_eq!(source.notice(), Some(IS_SIGNED_OUT), "the check's answer is said");
            assert!(!source.notice_failed());
            assert_eq!(ids(&source), ["s-this", "s-web"]);
        } else {
            assert_eq!(source.notice(), Some(NOT_SIGNED_OUT), "the check's answer is said");
            assert_eq!(source.notice_action(), Some(TRY_AGAIN));
            assert_eq!(ids(&source).len(), 3);
        }
    }
    // What worked and was SEEN is not said a second time on return.
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.change_mode.store(TAKEN_BEHIND_A_502, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(source.notice(), Some(IS_SIGNED_OUT));
    source.pump();
    source.clear_notice();
    opened(&mut source, Pane::Account);
    opened(&mut source, Pane::Sessions);
    assert_eq!(source.notice(), None);
}

/// "Everywhere else" sent again one by one. A call that gets no answer ends it: the next session is not sent
/// (each would wait its full time without a word), the rest count as not done, and the button sends them. An
/// error Beebeeb ANSWERED with does not end it.
#[test]
fn everywhere_else_one_by_one_stops_at_a_call_that_gets_no_answer() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.sign_out_everywhere_else();
    settle(&mut source);
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    assert!(server.deleted.lock().unwrap().is_empty());

    server.change_mode.store(DROP_CONNECTION, Ordering::SeqCst);
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(
        server.deleted.lock().unwrap().clone(),
        ["s-phone"],
        "nothing is sent after a call that got no answer"
    );
    // Not known for the one, not sent for the other: the list says both stand, and both can be sent again.
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT_ELSEWHERE));
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    assert_eq!(ids(&source).len(), 3);

    // Beebeeb answers the first with an error: the second is still sent, and it ends.
    server.change_mode.store(ANSWER, Ordering::SeqCst);
    *server.refused_ids.lock().unwrap() = vec!["s-phone"];
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(
        server.deleted.lock().unwrap().clone(),
        ["s-phone", "s-phone", "s-web"],
        "an answered error does not end it, and the unsent one was kept"
    );
    assert_eq!(source.notice(), Some(NOT_ALL_SIGNED_OUT));
    assert_eq!(ids(&source), ["s-this", "s-phone"]);

    // An answered error for one, then no answer for the next: the outcome is NOT known (that one may have
    // landed), whatever the first was answered. With the list unreadable too it is said so, never as a refusal.
    let server = self::server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.sign_out_everywhere_else();
    settle(&mut source);
    *server.refused_ids.lock().unwrap() = vec!["s-phone"];
    server.change_mode.store(DROP_CONNECTION, Ordering::SeqCst);
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.run_notice_action();
    settle(&mut source);
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone", "s-web"]);
    assert_eq!(source.notice(), Some(UNCONFIRMED_SIGN_OUT));
    assert_eq!(source.notice_action(), None, "not known is not \"send it again\"");
}

/// One by one: Beebeeb answered "no such session" for the first, and the engine refused the next before
/// sending it. A request DID go out: never "Nothing was sent", and the list is read again for what stands.
#[test]
fn gone_and_then_a_refusal_is_not_nothing_was_sent() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    let reads = server.asked("GET /api/v1/account/sessions");
    let answer = |source: &Source, gone| Answer::SignedOut {
        about: About {
            key: ELSEWHERE.to_string(),
            ids: vec!["s-web".into()],
        },
        sent_for: source.engine.session_turn(),
        made_for: Some(stamp(&source.engine)),
        ended: 0,
        gone,
        one_by_one: true,
        result: Err(AccountCallError::NotSignedIn),
    };
    let it = answer(&source, true);
    source.take_in(it);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT_ELSEWHERE));
    assert_eq!(source.notice_action(), Some(TRY_AGAIN));
    settle(&mut source);
    assert_eq!(
        server.asked("GET /api/v1/account/sessions"),
        reads + 1,
        "the list is read after a request that went out"
    );
    // Nothing went out at all: that, and only that, is "Nothing was sent", and nothing is read for it.
    let it = answer(&source, false);
    source.take_in(it);
    assert_eq!(source.notice(), Some(NOT_SENT));
    settle(&mut source);
    assert_eq!(server.asked("GET /api/v1/account/sessions"), reads + 1);
}
