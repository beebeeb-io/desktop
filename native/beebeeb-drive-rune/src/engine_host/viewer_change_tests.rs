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
