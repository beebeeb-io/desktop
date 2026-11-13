//! A Trash restore belongs to the session and the account of its press (`EngineTrashSource::restore`): held on its
//! worker across a sign-out and another account's sign-in, or across a lock, nothing is sent under any token; the
//! account that pressed is told once its session is back, and no other account is; an answer that lands under a
//! later session touches no row of it. Requests are counted at the fake as they arrive, before it looks at a token.

use super::*;

const WAIT: Duration = Duration::from_secs(30);
const ALL_THREE: [&str; 3] = ["IMG_0001.HEIC", "Invented contract.pdf", "Invented old invoices"];

/// Every restore request that reached the fake, whatever token it carried.
fn restore_requests(fake: &FakeApi) -> Vec<Request> {
    fake.requests()
        .into_iter()
        .filter(|r| r.route().starts_with("POST /api/v1/files/") && r.route().ends_with("/restore"))
        .collect()
}

/// The next restore waits on its worker before it calls the engine, until the first is sent on; the second says
/// the engine's call ended and its answer is in the source's inbox.
fn hold_next_restore(source: &mut EngineTrashSource<MemoryCredentials>) -> (mpsc::Sender<()>, mpsc::Receiver<()>) {
    let ((release, hold), (ended, has_ended)) = (mpsc::channel(), mpsc::channel());
    source.next_restore = Some((hold, ended));
    (release, has_ended)
}

/// The next restore request waits at the fake: the first says it arrived, the second lets it go on.
fn hold_next_restore_request(fake: &Fake) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    let ((reached, was_reached), (release, released)) = (mpsc::channel(), mpsc::channel());
    *fake.restore_hold.lock().unwrap() = Some((reached, released));
    (was_reached, release)
}

fn unlock(engine: &Engine<MemoryCredentials>, phrase: &str) {
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the phrase unlocks");
}

#[test]
fn a_restore_pressed_before_another_account_signed_in_is_sent_under_no_token() {
    let fake = fake("switch");
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine.clone());
    opened(&mut source);
    let (release, ended) = hold_next_restore(&mut source);
    assert!(source.restore(CONTRACT), "pressed under the first account");

    // The switch, while the restore has not yet reached the engine.
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    release.send(()).expect("the worker waits");
    ended.recv_timeout(WAIT).expect("the engine's call ended");

    assert_eq!(
        restore_requests(&fake.fake).len(),
        0,
        "restore requests that reached the fake, under any token"
    );
    assert_eq!(settle(&mut source), [], "the second account is told nothing");
    assert_eq!(opened(&mut source), []);
    assert_eq!(names(&source), ["Second account budget.xlsx"]);
    assert!(
        fake.first.restore_item(CONTRACT),
        "the item was still in the first account's Trash"
    );
}

#[test]
fn a_restore_pressed_before_a_lock_is_not_sent_and_the_one_who_pressed_is_told_after_the_unlock() {
    let fake = fake("locked");
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine.clone());
    opened(&mut source);
    let (release, ended) = hold_next_restore(&mut source);
    assert!(source.restore(PHOTO));

    let _ = engine.lock();
    release.send(()).expect("the worker waits");
    ended.recv_timeout(WAIT).expect("the engine's call ended");
    assert_eq!(
        restore_requests(&fake.fake).len(),
        0,
        "nothing is sent for a locked session"
    );
    let locked = source.pump();
    assert_eq!(
        (locked.notice, locked.busy),
        (None, false),
        "nothing is said while locked"
    );

    // The same account, unlocked: told once its Trash is listed again (the window is open on it), without a name,
    // and the item is still in its Trash.
    unlock(&engine, &fake.first_phrase);
    assert_eq!(settle(&mut source), [], "not before the Trash is listed again");
    assert_eq!(
        restore_requests(&fake.fake).len(),
        0,
        "and not sent after the unlock either"
    );
    assert_eq!(opened(&mut source), [Notice::RestoreNotDone]);
    assert_eq!(settle(&mut source), [], "said once");
    assert_eq!(names(&source), ALL_THREE);
}

#[test]
fn a_restore_taken_after_its_session_moved_on_is_not_drawn_and_the_list_is_read_again() {
    let fake = fake("taken-late");
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine.clone());
    opened(&mut source);
    // The engine's own send is not held, the request is: it waits at the fake with nothing done yet.
    let (release, ended) = hold_next_restore(&mut source);
    release.send(()).expect("the hook is set");
    let (reached, let_go) = hold_next_restore_request(&fake);
    assert!(source.restore(CONTRACT));
    reached.recv_timeout(WAIT).expect("the request reached the fake");

    let _ = engine.lock();
    unlock(&engine, &fake.first_phrase);
    assert_eq!(opened(&mut source), []);
    assert_eq!(
        names(&source),
        ALL_THREE,
        "read under the new session, before Beebeeb restored"
    );
    let read_before = listings(&fake.fake);

    let_go.send(()).expect("the fake holds the request");
    ended.recv_timeout(WAIT).expect("the engine's call ended");
    assert_eq!(settle(&mut source), [], "taken: nothing to say");
    assert_eq!(names(&source), ["IMG_0001.HEIC", "Invented old invoices"]);
    assert_eq!(
        listings(&fake.fake),
        read_before + 1,
        "the list on screen was read again"
    );
    let sent = restore_requests(&fake.fake);
    assert_eq!(sent.len(), 1, "sent once");
    assert_eq!(
        sent[0].bearer(),
        Some(fake.first.session_token.as_str()),
        "under the session of the press"
    );
}

#[test]
fn the_answer_of_an_earlier_session_does_not_take_out_a_row_a_later_press_put_in_flight() {
    let fake = fake("same-row");
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine.clone());
    opened(&mut source);
    let (release, ended) = hold_next_restore(&mut source);
    assert!(source.restore(PHOTO), "the first press, held on its worker");

    let _ = engine.lock();
    unlock(&engine, &fake.first_phrase);
    assert_eq!(opened(&mut source), []);
    assert_eq!(names(&source), ALL_THREE);

    // The same row again, under the new session: its request waits at the fake, which will refuse it.
    fake.restore_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let (reached, let_go) = hold_next_restore_request(&fake);
    assert!(source.restore(PHOTO), "the second press");
    reached
        .recv_timeout(WAIT)
        .expect("the second press's request reached the fake");

    // The first press's answer lands now: nothing was sent for it, and the second press's row stays in flight.
    release.send(()).expect("the worker waits");
    ended.recv_timeout(WAIT).expect("the first press's call ended");
    let pumped = source.pump();
    assert_eq!(
        pumped.notice,
        Some(Notice::RestoreNotDone),
        "the first press is answered"
    );
    assert!(pumped.busy, "the second press is still on its way");
    assert_eq!(names(&source), ["Invented contract.pdf", "Invented old invoices"]);

    let_go.send(()).expect("the fake holds the request");
    assert_eq!(
        settle(&mut source),
        [Notice::RestoreRefused("IMG_0001.HEIC".into())],
        "the second press's refusal brings its row back and says so"
    );
    assert_eq!(names(&source), ALL_THREE);
    assert_eq!(restore_requests(&fake.fake).len(), 1, "only the second press was sent");
}

#[test]
fn a_restore_taken_late_while_the_new_sessions_first_read_is_on_its_way_is_read_again() {
    let fake = fake("taken-mid-read");
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine.clone());
    opened(&mut source);
    let (release, ended) = hold_next_restore(&mut source);
    release.send(()).expect("the hook is set");
    let (reached, let_go) = hold_next_restore_request(&fake);
    assert!(source.restore(CONTRACT));
    reached.recv_timeout(WAIT).expect("the request reached the fake");

    let _ = engine.lock();
    assert!(source.pump().changed, "the lock drops the list");
    unlock(&engine, &fake.first_phrase);
    // The new session's first read is slow, and saw the item still in Trash; no list is held yet.
    let read_before = listings(&fake.fake);
    fake.list_delay_ms.store(600, Ordering::SeqCst);
    source.opened();
    let began = Instant::now();
    while listings(&fake.fake) == read_before {
        assert!(began.elapsed() < WAIT, "the read did not reach the fake");
        std::thread::sleep(Duration::from_millis(5));
    }
    fake.list_delay_ms.store(0, Ordering::SeqCst);
    let_go.send(()).expect("the fake holds the request");
    ended.recv_timeout(WAIT).expect("the engine's call ended");
    assert_eq!(settle(&mut source), []);
    assert_eq!(names(&source), ["IMG_0001.HEIC", "Invented old invoices"]);
    assert_eq!(
        listings(&fake.fake),
        read_before + 2,
        "read once more after the slow read"
    );
}

#[test]
fn a_refusal_that_lands_after_the_session_moved_on_is_not_called_not_sent_and_the_list_is_read_again() {
    let fake = fake("refused-late");
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine.clone());
    opened(&mut source);
    let (release, ended) = hold_next_restore(&mut source);
    release.send(()).expect("the hook is set");
    fake.restore_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let (reached, let_go) = hold_next_restore_request(&fake);
    assert!(source.restore(CONTRACT));
    reached.recv_timeout(WAIT).expect("the request reached the fake");

    let _ = engine.lock();
    unlock(&engine, &fake.first_phrase);
    assert_eq!(opened(&mut source), []);
    let read_before = listings(&fake.fake);
    let_go.send(()).expect("the fake holds the request");
    ended.recv_timeout(WAIT).expect("the engine's call ended");
    assert_eq!(
        settle(&mut source),
        [],
        "it was sent: what happened to it is what the list says"
    );
    assert_eq!(
        listings(&fake.fake),
        read_before + 1,
        "the list on screen was read again"
    );
    assert_eq!(names(&source), ALL_THREE);
}
