//! The sidebar's Trash badge (`EngineTrashSource::count`) is the server's own number: read on its own (no listing),
//! from every section (the poll asks, not only the Trash opening), moved by a restore and by a delete for good, kept
//! when a refresh fails, never a zero Beebeeb did not say, and never shown under another session or account than the
//! one it was said to. Requests are counted at the fake as they arrive.

use super::*;

fn count_requests(fake: &FakeApi) -> usize {
    fake.requests()
        .iter()
        .filter(|r| r.route() == "GET /api/v1/files/count")
        .count()
}

fn unlock(engine: &Engine<MemoryCredentials>, phrase: &str) {
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the phrase unlocks");
}

#[test]
fn the_badge_is_the_servers_number_read_without_listing_and_without_the_section_open() {
    let fake = fake("badge");
    let mut source = EngineTrashSource::new(signed_in(&fake));
    assert_eq!(source.count(), None, "nothing is drawn as real before Beebeeb answered");
    assert_eq!(count_requests(&fake.fake), 0, "drawing asks nothing");

    // The poll alone (no `opened`: the user is in Files) asks for it.
    assert!(settle(&mut source).is_empty());
    assert_eq!(source.count(), Some(3));
    assert_eq!(count_requests(&fake.fake), 1);
    assert_eq!(listings(&fake.fake), 0, "no full-list read for the badge");
    assert_eq!(source.loaded(), Loaded::Loading, "the list itself was not read");

    // Polling again asks nothing more while the badge is there.
    for _ in 0..5 {
        let _ = source.pump();
    }
    assert_eq!(count_requests(&fake.fake), 1);
}

#[test]
fn the_badge_moves_after_a_restore_and_after_a_delete_for_good() {
    let fake = fake("moves");
    let mut source = EngineTrashSource::new(signed_in(&fake));
    opened(&mut source);
    assert_eq!(source.count(), Some(3));

    assert!(source.restore(CONTRACT));
    settle(&mut source);
    assert_eq!(source.count(), Some(2), "a restored item left the count");
    assert_eq!(names(&source).len(), 2);

    assert!(source.purge(Some(DOCS), PASSWORD));
    let _ = super::purge::settle_purge(&mut source);
    assert_eq!(source.count(), Some(1), "a deleted item left the count");
}

#[test]
fn a_failed_count_is_no_badge_not_a_zero_and_is_not_asked_again_at_once() {
    let fake = fake("failed");
    fake.count_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let mut source = EngineTrashSource::new(signed_in(&fake));
    settle(&mut source);
    assert_eq!(source.count(), None);
    for _ in 0..5 {
        let _ = source.pump();
    }
    settle(&mut source);
    assert_eq!(
        count_requests(&fake.fake),
        1,
        "the poll does not hammer a failing route"
    );

    // Opening Trash tries again; a number already shown stays when a later refresh fails.
    fake.count_mode.store(ANSWER, Ordering::SeqCst);
    opened(&mut source);
    assert_eq!(source.count(), Some(3));
    fake.count_mode.store(SESSION_ENDED, Ordering::SeqCst);
    source.expire();
    opened(&mut source);
    assert_eq!(source.count(), Some(3), "the last thing Beebeeb said stays");
}

#[test]
fn a_badge_is_never_shown_under_another_session_or_account() {
    let fake = fake("session");
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine.clone());
    settle(&mut source);
    assert_eq!(source.count(), Some(3));

    // A lock and an unlock: the number goes at once, before any poll, and the new session is asked.
    let _ = engine.lock();
    assert_eq!(source.count(), None, "locked");
    unlock(&engine, &fake.first_phrase);
    assert_eq!(source.count(), None, "a new session has not been told yet");
    settle(&mut source);
    assert_eq!(source.count(), Some(3));

    // Another account on the same handle: its own number, never the first's.
    engine.sign_out_keeping_local_data().expect("signed out");
    assert_eq!(source.count(), None, "signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    assert_eq!(source.count(), None, "the first account's number is not the second's");
    settle(&mut source);
    assert_eq!(source.count(), Some(1), "the second account has one item in Trash");
}

#[test]
fn an_answer_that_lands_under_a_later_session_is_dropped() {
    let fake = fake("late");
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine.clone());
    let _ = source.pump(); // starts the read
    let began = Instant::now();
    while count_requests(&fake.fake) == 0 {
        assert!(
            began.elapsed() < Duration::from_secs(10),
            "the read never reached the fake"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(300)); // its answer is in the inbox now

    let _ = engine.lock();
    unlock(&engine, &fake.first_phrase);
    let _ = source.pump(); // takes the answer in
    assert_eq!(source.count(), None, "the answer was made for a session that is gone");
    settle(&mut source);
    assert_eq!(source.count(), Some(3), "the new session was asked for its own");
    assert_eq!(count_requests(&fake.fake), 2);
}

#[test]
fn without_a_session_nothing_is_asked() {
    let fake = fake("signed-out");
    let mut source = EngineTrashSource::new(engine_at(&fake.fake.base));
    source.opened();
    settle(&mut source);
    assert_eq!(source.count(), None);
    assert_eq!(count_requests(&fake.fake), 0);
}
