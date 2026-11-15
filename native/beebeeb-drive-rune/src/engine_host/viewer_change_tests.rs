//! Available offline and saving a text, against the engine of the test's own: the press is answered later, by
//! `settled`, and only while it is still the session and the account of the press. Every name and byte is invented.

use super::*;

/// Poll until the change is settled.
fn settled(source: &mut Source, id: &str) -> Settled {
    let began = Instant::now();
    loop {
        if let Some(settled) = source.settled(id) {
            return settled;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "the change did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn what_the_local_state_says_is_what_the_switch_and_the_version_show() {
    let fake = Fake::new();
    fake.s().edit = Some(EditFacts {
        version: 7,
        kept: true,
        can_write: true,
    });
    let (source, file, _) = opened(&fake, NOTE);
    assert!(source.knows_offline());
    assert!(file.details.offline, "kept on this PC, as read");
    assert_eq!(file.version, 7, "the version the local state holds");
    assert!(source.can_save());

    // Not read: no switch, no editor (a state that was not read is not shown).
    fake.s().edit = None;
    let (source, file, _) = opened(&fake, NOTE);
    assert!(!source.knows_offline());
    assert!(!source.can_save());
    assert!(!file.details.offline);

    // Read only: the switch is there, the editor is not.
    fake.s().edit = Some(EditFacts {
        version: 3,
        kept: false,
        can_write: false,
    });
    let (source, _, _) = opened(&fake, NOTE);
    assert!(source.knows_offline() && !source.can_save());
}

#[test]
fn the_switch_moves_when_the_engine_did_and_not_before() {
    let fake = Fake::new();
    let (mut source, _, _) = opened(&fake, NOTE);
    let door = fake.door.lock().unwrap();
    fake.s().change_waits = true;
    assert!(source.set_offline(NOTE, true), "handed to a worker");
    assert!(source.answers_later());
    // On its way: nothing is settled, the engine has not answered.
    assert!(source.settled(NOTE).is_none());
    assert!(fake.s().kept_calls.is_empty());
    drop(door);
    assert!(matches!(
        settled(&mut source, NOTE),
        Settled::Offline { on: true, done: true }
    ));
    assert_eq!(fake.s().kept_calls, [(NOTE.to_string(), true)]);
}

#[test]
fn a_refusal_of_the_engine_is_said_not_shown_as_done() {
    let fake = Fake::new();
    fake.s().kept_refused = true;
    let (mut source, _, _) = opened(&fake, NOTE);
    assert!(source.set_offline(NOTE, true));
    assert!(matches!(
        settled(&mut source, NOTE),
        Settled::Offline { on: true, done: false }
    ));
}

#[test]
fn an_answer_after_the_session_moved_on_is_dropped() {
    let fake = Fake::new();
    let (mut source, _, _) = opened(&fake, NOTE);
    let door = fake.door.lock().unwrap();
    fake.s().change_waits = true;
    assert!(source.set_offline(NOTE, true));
    fake.new_session("b@example.eu");
    drop(door);
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        source.settled(NOTE).is_none(),
        "another session's answer is never shown"
    );
}

#[test]
fn a_save_is_bound_to_the_version_it_began_from_and_comes_back_as_the_next_one() {
    let fake = Fake::new();
    let (mut source, file, _) = opened(&fake, NOTE);
    assert_eq!(file.version, 3);
    assert_eq!(
        source.save(NOTE, "my invented words\n", file.version),
        SaveOutcome::Working
    );
    let Settled::Saved { id, version, body, .. } = settled(&mut source, NOTE) else {
        panic!("a saved text");
    };
    assert_eq!((id.as_str(), version), (NOTE, 4));
    assert_eq!(words(&body), "my invented words\n");
    assert_eq!(
        fake.s().saves,
        [(NOTE.to_string(), "my invented words\n".to_string(), 3)],
        "the base is the version the edit began from"
    );
}

#[test]
fn a_newer_version_is_a_conflict_whose_other_side_is_another_device() {
    let fake = Fake::new();
    fake.s().save_answer = Ok(SaveEnd::Conflict { theirs: 5 });
    let (mut source, file, _) = opened(&fake, NOTE);
    assert_eq!(source.save(NOTE, "mine", file.version), SaveOutcome::Working);
    let Settled::Conflict(other) = settled(&mut source, NOTE) else {
        panic!("a conflict");
    };
    assert_eq!((other.from.as_str(), other.version), ("Another device", 5));
}

#[test]
fn a_save_that_is_only_stored_here_says_so_and_a_refused_one_says_not_saved() {
    let fake = Fake::new();
    fake.s().save_answer = Ok(SaveEnd::Queued);
    let (mut source, file, _) = opened(&fake, NOTE);
    source.save(NOTE, "mine", file.version);
    assert!(matches!(settled(&mut source, NOTE), Settled::Queued { .. }));
    fake.s().save_answer = Err(EditRefused::Failed);
    source.save(NOTE, "mine", file.version);
    assert!(matches!(settled(&mut source, NOTE), Settled::Failed));
}

#[test]
fn the_three_keeps() {
    let fake = Fake::new();
    let (mut source, file, _) = opened(&fake, NOTE);
    // Keep mine: saved from the version the file is at now.
    fake.s().edit = Some(EditFacts {
        version: 5,
        kept: false,
        can_write: true,
    });
    assert_eq!(source.resolve(NOTE, "mine", Keep::Mine), SaveOutcome::Working);
    assert!(matches!(settled(&mut source, NOTE), Settled::Saved { .. }));
    assert_eq!(fake.s().saves.last().map(|save| save.2), Some(5));
    // Keep both: a new file next to the first.
    assert_eq!(source.resolve(NOTE, "mine", Keep::Both), SaveOutcome::Working);
    let Settled::Saved { id, name, .. } = settled(&mut source, NOTE) else {
        panic!("the copy");
    };
    assert_eq!(id, "f-copy");
    assert!(name.contains("This PC"));
    // Keep theirs: the file as Beebeeb has it, my edit dropped.
    assert_eq!(source.resolve(NOTE, "mine", Keep::Theirs), SaveOutcome::Working);
    let Settled::Opened { body, version } = settled(&mut source, NOTE) else {
        panic!("theirs");
    };
    assert_eq!((words(&body).as_str(), version), (NOTE_TEXT, 5));
    assert_eq!(file.version, 3);
}

/// Poll `settled` until the version read on its worker is over and its answer (if any) was taken.
fn newer_answer(source: &mut Source, id: &str) -> Option<Settled> {
    let began = Instant::now();
    loop {
        if let Some(settled) = source.settled(id) {
            return Some(settled);
        }
        if !source.changes.checking.load(Ordering::SeqCst) && source.changes.checked.is_some() {
            // One more look: the answer may have been sent just before the worker let go of its flag.
            return source.settled(id);
        }
        assert!(began.elapsed() < Duration::from_secs(10), "the read did not end");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn bump(fake: &Fake, version: i64) {
    fake.s().edit = Some(EditFacts {
        version,
        kept: false,
        can_write: true,
    });
}

#[test]
fn a_version_past_the_one_on_screen_raises_the_banner_without_a_device_name() {
    let fake = Fake::new();
    let (mut source, file, _) = opened(&fake, NOTE);
    assert_eq!(file.version, 3);
    assert!(file.newer.is_none(), "nothing newer at the open");
    source.changes.every = Duration::ZERO;
    // Nothing changed: nothing is said, however often it is read.
    for _ in 0..3 {
        assert!(newer_answer(&mut source, NOTE).is_none());
    }
    // The sync engine raised the file's version in the local state.
    bump(&fake, 5);
    let Some(Settled::Newer(newer)) = newer_answer(&mut source, NOTE) else {
        panic!("a newer version is announced");
    };
    assert!(newer.from.is_empty(), "no device name in 0.9.0");
    let at = newer.at.as_bytes();
    assert!(at.len() == 5 && at[2] == b':', "a time as HH:MM: {}", newer.at);
    // Once per version: the same version is not announced again, the next one is.
    assert!(newer_answer(&mut source, NOTE).is_none());
    bump(&fake, 6);
    assert!(matches!(newer_answer(&mut source, NOTE), Some(Settled::Newer(_))));
}

#[test]
fn the_views_own_save_is_not_a_newer_version() {
    let fake = Fake::new();
    let (mut source, _, _) = opened(&fake, NOTE);
    assert_eq!(source.save(NOTE, "mine", 3), SaveOutcome::Working);
    assert!(matches!(settled(&mut source, NOTE), Settled::Saved { version: 4, .. }));
    // The local state now holds the version the save became.
    bump(&fake, 4);
    source.changes.every = Duration::ZERO;
    for _ in 0..3 {
        assert!(
            newer_answer(&mut source, NOTE).is_none(),
            "the save's own version is not another device's"
        );
    }
    bump(&fake, 5);
    assert!(matches!(newer_answer(&mut source, NOTE), Some(Settled::Newer(_))));
}

#[test]
fn a_banner_for_a_session_that_moved_on_is_dropped() {
    let fake = Fake::new();
    let (mut source, _, _) = opened(&fake, NOTE);
    source.changes.every = Duration::ZERO;
    bump(&fake, 5);
    // The read starts; the account is another one before its answer is taken.
    assert!(source.settled(NOTE).is_none());
    let began = Instant::now();
    while source.changes.checking.load(Ordering::SeqCst) {
        assert!(began.elapsed() < Duration::from_secs(10), "the read did not end");
        std::thread::sleep(Duration::from_millis(2));
    }
    fake.new_session("b@example.eu");
    assert!(source.settled(NOTE).is_none(), "another session's answer is dropped");
}

#[test]
fn show_it_reads_the_file_again_as_beebeeb_has_it() {
    let fake = Fake::new();
    let (mut source, file, _) = opened(&fake, NOTE);
    assert_eq!(file.version, 3);
    bump(&fake, 5);
    let again = source.newer(NOTE).expect("the file opens again");
    assert_eq!(again.version, 5, "the version Beebeeb holds now");
    assert!(again.newer.is_none());
    assert!(
        matches!(again.body, Body::Decrypting { .. }),
        "the content comes on the poll"
    );
    assert_eq!(words(&answered(&mut source, NOTE)), NOTE_TEXT);
}
