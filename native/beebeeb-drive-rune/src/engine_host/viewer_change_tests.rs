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
        own: false,
    });
    let (source, file, _) = opened(&fake, NOTE);
    assert!(source.knows_offline());
    assert!(
        file.details.offline,
        concat!("kept on ", beebeeb_engine::device_word!(), ", as read")
    );
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
        own: false,
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

/// Delta check of #432, round 3: the engine cannot say who saved the newer version (another device, or an earlier
/// opening of this file on this PC): the conflict names nobody.
#[test]
fn a_newer_version_is_a_conflict_whose_other_side_is_not_named() {
    let fake = Fake::new();
    fake.s().save_answer = Ok(SaveEnd::Conflict { theirs: 5 });
    let (mut source, file, _) = opened(&fake, NOTE);
    assert_eq!(source.save(NOTE, "mine", file.version), SaveOutcome::Working);
    let Settled::Conflict(other) = settled(&mut source, NOTE) else {
        panic!("a conflict");
    };
    assert_eq!((other.from.as_str(), other.version), ("", 5));
}

#[test]
fn a_save_that_is_only_stored_here_says_so_and_a_refused_one_says_not_saved() {
    let fake = Fake::new();
    fake.s().save_answer = Ok(SaveEnd::Queued { base: 0 });
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
        own: false,
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
    assert!(name.contains(concat!(beebeeb_engine::device_word_cap!())));
    // Keep theirs: the file as Beebeeb has it, my edit dropped.
    assert_eq!(source.resolve(NOTE, "mine", Keep::Theirs), SaveOutcome::Working);
    let Settled::Opened { body, version } = settled(&mut source, NOTE) else {
        panic!("theirs");
    };
    assert_eq!((words(&body).as_str(), version), (NOTE_TEXT, 5));
    assert_eq!(file.version, 3);
}

/// One read of the version, from its start to its end, and what it said. `settled` starts a read (the interval is
/// zero) and takes an answer; the answer is taken only after the worker let go of its flag, so no new read starts
/// between the wait and the take.
///
/// Every take that finds nothing newer starts the next read, so one is on its way when this is called, begun before
/// whatever the test changed since. It ends first: its answer (the version as it was, or already the new one) is
/// taken by the first `settled`, and the read this one waits for began after the change.
fn newer_answer(source: &mut Source, id: &str) -> Option<Settled> {
    read_done(source);
    if let Some(settled) = source.settled(id) {
        return Some(settled);
    }
    read_done(source);
    source.settled(id)
}

impl Fake {
    /// The upload of a save answered Queued lands as `version`: the local state holds it, and it is the viewer's own
    /// while the engine still holds that save's record (the same opening).
    fn land(&self, version: i64) {
        let mut s = self.s();
        if let Some(edit) = s.edit.as_mut() {
            edit.version = version;
        }
        let queued = s.queued_save.take();
        if queued == Some(s.epoch) {
            s.own_versions.push(version);
        }
    }
}

fn bump(fake: &Fake, version: i64) {
    fake.s().edit = Some(EditFacts {
        version,
        kept: false,
        can_write: true,
        own: false,
    });
}

/// Wait until a read of the version started by `settled` has sent its answer (not taken yet).
fn read_done(source: &Source) {
    let began = Instant::now();
    while source.changes.checking.load(Ordering::SeqCst) {
        assert!(began.elapsed() < Duration::from_secs(10), "the read did not end");
        std::thread::sleep(Duration::from_millis(2));
    }
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
    // The control (review of #432, P3): the same steps with the session unchanged raise the banner, so the drop
    // below is the session's doing, not a read that never answered.
    let fake = Fake::new();
    let (mut source, _, _) = opened(&fake, NOTE);
    source.changes.every = Duration::ZERO;
    bump(&fake, 5);
    assert!(source.settled(NOTE).is_none());
    read_done(&source);
    assert!(matches!(source.settled(NOTE), Some(Settled::Newer(_))), "the control");

    let fake = Fake::new();
    let (mut source, _, _) = opened(&fake, NOTE);
    source.changes.every = Duration::ZERO;
    bump(&fake, 5);
    // The read starts; the account is another one before its answer is taken.
    assert!(source.settled(NOTE).is_none());
    read_done(&source);
    fake.new_session("b@example.eu");
    assert!(source.settled(NOTE).is_none(), "another session's answer is dropped");
}

/// Review of #432, P1-1: a save that ends "Saved on this PC" lands later as the version it becomes. That version
/// is the view's own (the engine says so: `EditFacts::own`), never "A newer version arrived"; a version past
/// it is another device's.
#[test]
fn a_queued_save_that_lands_later_is_not_announced_as_a_newer_version() {
    let fake = Fake::new();
    fake.s().save_answer = Ok(SaveEnd::Queued { base: 0 });
    let (mut source, file, _) = opened(&fake, NOTE);
    assert_eq!(source.save(NOTE, "mine", file.version), SaveOutcome::Working);
    assert!(matches!(settled(&mut source, NOTE), Settled::Queued { .. }));
    source.changes.every = Duration::ZERO;
    // Not landed yet: the local state still holds 3.
    assert!(newer_answer(&mut source, NOTE).is_none());
    // Landed: 4, the version the view's own save became.
    fake.land(4);
    for _ in 0..3 {
        assert!(
            newer_answer(&mut source, NOTE).is_none(),
            "the view's own landed save is not another device's"
        );
    }
    // Another device saved after it.
    bump(&fake, 5);
    assert!(matches!(newer_answer(&mut source, NOTE), Some(Settled::Newer(_))));
}

/// The own save landed as 4 and another device saved 5 inside one interval: the first read sees 5, which is not the
/// view's own, and says so.
#[test]
fn another_devices_version_past_the_own_landed_one_is_announced_at_the_first_read() {
    let fake = Fake::new();
    fake.s().save_answer = Ok(SaveEnd::Queued { base: 0 });
    let (mut source, file, _) = opened(&fake, NOTE);
    source.save(NOTE, "mine", file.version);
    assert!(matches!(settled(&mut source, NOTE), Settled::Queued { .. }));
    source.changes.every = Duration::ZERO;
    fake.land(4);
    bump(&fake, 5);
    assert!(matches!(newer_answer(&mut source, NOTE), Some(Settled::Newer(_))));
}

/// Review of #432, P2: a save answered with the two-versions question leaves the view on its own version; the
/// person cancels and discards, and the newer version is still announced by the next read.
#[test]
fn a_conflict_does_not_hide_the_newer_version_from_the_next_read() {
    let fake = Fake::new();
    fake.s().save_answer = Ok(SaveEnd::Conflict { theirs: 5 });
    let (mut source, file, _) = opened(&fake, NOTE);
    source.save(NOTE, "mine", file.version);
    assert!(matches!(settled(&mut source, NOTE), Settled::Conflict(_)));
    bump(&fake, 5);
    source.changes.every = Duration::ZERO;
    assert!(matches!(newer_answer(&mut source, NOTE), Some(Settled::Newer(_))));
}

/// Review of #432, P2: a read that noticed another device's version comes back in the same drain as a change's
/// answer. The change's answer comes first; the banner on the next pump, not never.
#[test]
fn a_newer_version_read_in_the_same_drain_as_a_change_is_kept_for_the_next_pump() {
    let fake = Fake::new();
    let (mut source, _, _) = opened(&fake, NOTE);
    source.changes.every = Duration::ZERO;
    let door = fake.door.lock().unwrap();
    fake.s().change_waits = true;
    assert!(source.set_offline(NOTE, true));
    bump(&fake, 5);
    let wakes = fake.s().wakes;
    // The read starts (the change waits at the door) and answers; then the change answers.
    assert!(source.settled(NOTE).is_none());
    read_done(&source);
    drop(door);
    until("the change answered", || fake.s().wakes >= wakes + 2);
    assert!(matches!(
        source.settled(NOTE),
        Some(Settled::Offline { on: true, done: true })
    ));
    // Not because a read is due (delta check of #432): the next one is an hour away.
    source.changes.every = Duration::from_secs(3600);
    source.changes.checked = Some(Instant::now());
    assert!(
        source.next_check().is_some_and(|after| after.is_zero()),
        "woken at once"
    );
    assert!(matches!(source.settled(NOTE), Some(Settled::Newer(_))));
}

/// Review of #432, P2: the window is woken for the next read while an open text's version is read, so the banner
/// comes without a press. Not for a kind that draws no banner, and not without the version.
#[test]
fn the_window_is_woken_for_the_next_read_of_an_open_text() {
    let fake = Fake::new();
    let (source, _, _) = opened(&fake, NOTE);
    let after = source.next_check().expect("an open note's version is read");
    assert!(after <= super::super::change::NEWER_EVERY, "{after:?}");
    fake.s().edit = None;
    let (source, _, _) = opened(&fake, NOTE);
    assert!(source.next_check().is_none(), "no version read: nothing to wake for");
}

/// Review of #432, P3: only a text and a note draw the banner, so only their version is read.
#[test]
fn the_version_of_a_photo_is_not_read() {
    let fake = Fake::new();
    fake.put("f-photo", "Invented photo.png", &png_bytes(4, 4));
    let (mut source, _, _) = opened(&fake, "f-photo");
    source.changes.every = Duration::ZERO;
    let reads = fake.s().edit_reads;
    for _ in 0..3 {
        assert!(source.settled("f-photo").is_none());
        read_done(&source);
    }
    assert_eq!(fake.s().edit_reads, reads, "no read of the version");
    assert!(source.next_check().is_none());
}

/// Review of #432, P3: a read begun under one opening is not taken by the next (Show it opened the file again).
#[test]
fn a_read_begun_before_show_it_is_not_the_new_openings() {
    let fake = Fake::new();
    let (mut source, _, _) = opened(&fake, NOTE);
    source.changes.every = Duration::from_secs(3600);
    source.changes.checked = None;
    bump(&fake, 6);
    // The read answers 6, not taken yet.
    assert!(source.settled(NOTE).is_none());
    read_done(&source);
    // Show it opens the file again; the local state now holds 5 (the new opening holds 5).
    bump(&fake, 5);
    source.newer(NOTE).expect("the file opens again");
    assert!(
        source.settled(NOTE).is_none(),
        "the earlier opening's read says nothing about this one"
    );
}

/// Review of #432, P3: Show it that cannot read the file leaves the open one as it was: its editor and its reads.
#[test]
fn show_it_that_is_refused_keeps_the_file_open_as_it_was() {
    let fake = Fake::new();
    let (mut source, _, _) = opened(&fake, NOTE);
    fake.s().facts_error = Some((OpenFileError::NotFound, 1));
    assert_eq!(source.newer(NOTE).err(), Some(OpenError::NotFound));
    assert!(source.can_save(), "still open: the editor is offered");
    source.changes.every = Duration::ZERO;
    bump(&fake, 5);
    assert!(
        matches!(newer_answer(&mut source, NOTE), Some(Settled::Newer(_))),
        "and read"
    );
}

/// Review of #432, P1-2: Keep theirs reads the version before the content. A version that lands during the
/// download leaves the label older than the bytes (a save from there is a conflict), never newer.
#[test]
fn keep_theirs_labels_the_content_with_the_version_read_before_it() {
    let fake = Fake::new();
    let (mut source, _, _) = opened(&fake, NOTE);
    bump(&fake, 5);
    fake.s().lands_during_fetch = Some(6);
    assert_eq!(source.resolve(NOTE, "mine", Keep::Theirs), SaveOutcome::Working);
    let Settled::Opened { version, .. } = settled(&mut source, NOTE) else {
        panic!("theirs");
    };
    assert_eq!(version, 5, "the version read before the download began");
}

/// Review of #432, P1-2: a copy on this PC older than Beebeeb's version now is not shown as it (the engine says
/// `Behind`): Beebeeb's is downloaded, and the line says the copy here could not be confirmed as this version.
#[test]
fn a_copy_behind_beebeebs_version_is_downloaded_and_said_unconfirmed() {
    let fake = Fake::new();
    fake.keep(NOTE, OnThisPc::InSync, Some(b"the older invented words\n"));
    fake.s().local_behind = true;
    let (source, _, body) = opened(&fake, NOTE);
    assert_eq!(words(&body), NOTE_TEXT, "Beebeeb's version, downloaded");
    assert_eq!(fake.s().fetches, [NOTE.to_string()]);
    assert_eq!(source.origin(NOTE), Origin::BeebeebUnconfirmedHere);
}

/// Delta check of #432, round 3: a copy here whose change is not settled with Beebeeb, and also behind: the line says
/// the known fact (not in sync), never the weaker "could not confirm".
#[test]
fn a_copy_not_settled_and_behind_is_said_not_in_sync() {
    let fake = Fake::new();
    fake.keep(NOTE, OnThisPc::InSync, Some(b"the older invented words\n"));
    fake.s().files.get_mut(NOTE).expect("the note").unsettled = true;
    fake.s().local_behind = true;
    let (source, _, body) = opened(&fake, NOTE);
    assert_eq!(words(&body), NOTE_TEXT, "Beebeeb's version, downloaded");
    assert_eq!(source.origin(NOTE), Origin::BeebeebNotThisPcs);
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

/// Delta check of #432, P1-1: a save answered Queued, then the file opened again (Show it). The new opening holds
/// none of the earlier saves: when that save lands, its version is announced as a newer version, and a save from
/// the version the new opening shows goes out from that version (the engine then answers Conflict: never continued
/// from the earlier save's landing).
#[test]
fn a_new_opening_does_not_take_an_earlier_saves_landing_as_its_own() {
    let fake = Fake::new();
    fake.s().save_answer = Ok(SaveEnd::Queued { base: 0 });
    let (mut source, file, _) = opened(&fake, NOTE);
    assert_eq!(source.save(NOTE, "mine", file.version), SaveOutcome::Working);
    assert!(matches!(settled(&mut source, NOTE), Settled::Queued { version: 3, .. }));
    let forgets = fake.s().forgets;
    let again = source.newer(NOTE).expect("the file opens again");
    assert_eq!(again.version, 3);
    assert_eq!(
        fake.s().forgets,
        forgets + 1,
        "the new opening forgot the earlier saves"
    );
    assert_eq!(words(&answered(&mut source, NOTE)), NOTE_TEXT);
    fake.land(4);
    source.changes.every = Duration::ZERO;
    assert!(
        matches!(newer_answer(&mut source, NOTE), Some(Settled::Newer(_))),
        "the earlier save's landing is a newer version for this opening"
    );
    fake.s().save_answer = Ok(SaveEnd::Conflict { theirs: 4 });
    assert_eq!(source.save(NOTE, "edit of 3", again.version), SaveOutcome::Working);
    assert!(matches!(settled(&mut source, NOTE), Settled::Conflict(_)));
    assert_eq!(
        fake.s().saves.last().map(|save| save.2),
        Some(3),
        "sent from the version on screen"
    );
}

/// Delta check of #432, P1-2: Show it waits while a change is on its way (its answer belongs to what is on screen).
/// When it answers, a save stored here is labelled with the version it was saved from.
#[test]
fn show_it_waits_while_a_save_is_on_its_way() {
    let fake = Fake::new();
    fake.s().save_answer = Ok(SaveEnd::Queued { base: 0 });
    let (mut source, file, _) = opened(&fake, NOTE);
    let door = fake.door.lock().unwrap();
    fake.s().change_waits = true;
    assert_eq!(
        source.save(NOTE, "my invented words\n", file.version),
        SaveOutcome::Working
    );
    bump(&fake, 4);
    assert!(source.saving());
    assert_eq!(
        source.newer(NOTE).err(),
        Some(OpenError::NotNow),
        "not while the save is on its way"
    );
    drop(door);
    let Settled::Queued { version, body } = settled(&mut source, NOTE) else {
        panic!("stored here");
    };
    assert_eq!(version, 3, "labelled with its base, not the newer 4");
    assert_eq!(words(&body), "my invented words\n");
    assert!(!source.saving());
}

/// Delta check of #432, P1-2: a change answered for an earlier opening never replaces the view: its outcome is
/// said, its text overwritten, and what is on screen stays (here the newer version Show it opened).
#[test]
fn a_change_answered_for_an_earlier_opening_never_replaces_the_view() {
    let fake = Fake::new();
    fake.s().save_answer = Ok(SaveEnd::Queued { base: 0 });
    let (mut source, file, _) = opened(&fake, NOTE);
    let door = fake.door.lock().unwrap();
    fake.s().change_waits = true;
    assert_eq!(source.save(NOTE, "mine", file.version), SaveOutcome::Working);
    // Whatever opened the file again while the save was on its way (Show it waits for it; this goes past that).
    source.changes.busy = false;
    bump(&fake, 4);
    let again = source.newer(NOTE).expect("the newer version opens");
    assert_eq!(again.version, 4);
    drop(door);
    let said = settled(&mut source, NOTE);
    let Settled::Said(words) = said else {
        panic!("said, not applied");
    };
    assert_eq!(
        words,
        concat!(
            "Your earlier edit was saved on ",
            beebeeb_engine::device_word!(),
            " \u{00B7} sends when Beebeeb can be reached"
        )
    );
    // The view's opening still holds the newer version: a read of 4 is nothing new.
    source.changes.every = Duration::ZERO;
    assert!(newer_answer(&mut source, NOTE).is_none());
}

/// Delta check of #432, P3: Keep mine stored here only is labelled with the version it was saved from (the one the
/// person chose to replace), and that version is the view's: no banner about it.
#[test]
fn keep_mine_stored_here_holds_the_version_it_replaced() {
    let fake = Fake::new();
    fake.s().save_answer = Ok(SaveEnd::Queued { base: 0 });
    let (mut source, _, _) = opened(&fake, NOTE);
    bump(&fake, 5);
    assert_eq!(source.resolve(NOTE, "mine", Keep::Mine), SaveOutcome::Working);
    let Settled::Queued { version, .. } = settled(&mut source, NOTE) else {
        panic!("stored here");
    };
    assert_eq!(version, 5, "labelled with the version it was saved from");
    assert_eq!(fake.s().saves.last().map(|save| save.2), Some(5));
    source.changes.every = Duration::ZERO;
    for _ in 0..3 {
        assert!(
            newer_answer(&mut source, NOTE).is_none(),
            "the version the person chose to replace is not news"
        );
    }
}

/// Delta check of #432, P3: while a read is on its way the window is not asked for frames: the read wakes it when
/// it ends.
#[test]
fn no_wake_is_asked_for_while_a_read_is_on_its_way() {
    let fake = Fake::new();
    let (mut source, _, _) = opened(&fake, NOTE);
    source.changes.every = Duration::ZERO;
    let door = fake.door.lock().unwrap();
    fake.s().edit_waits = true;
    assert!(source.settled(NOTE).is_none(), "the read starts and waits");
    until("the read is on its way", || {
        source.changes.checking.load(Ordering::SeqCst)
    });
    assert_eq!(source.next_check(), None);
    let wakes = fake.s().wakes;
    // The read ends without an answer (the local state no longer gives the version): it still wakes the window.
    fake.s().edit = None;
    drop(door);
    read_done(&source);
    until("the read woke the window", || fake.s().wakes > wakes);
    fake.s().edit_waits = false;
}

/// Delta check of #432, round 3, P1: a save is on its way when the person leaves the file and opens it again from
/// Files (another source: its own `busy`). The save ends after that, under the earlier opening: the engine never
/// takes it as the new opening's own. Its landing is a newer version for the new one, and a save from the version the
/// new one shows goes out from it, under the new opening (the engine answers Conflict).
#[test]
fn a_save_of_an_earlier_opening_that_ends_after_the_file_opened_again_is_not_the_new_ones() {
    let fake = Fake::new();
    fake.keep(NOTE, OnThisPc::InSync, Some(NOTE_TEXT.as_bytes()));
    let (mut first, file, _) = opened(&fake, NOTE);
    let door = fake.door.lock().unwrap();
    fake.s().change_waits = true;
    assert_eq!(first.save(NOTE, "mine\n", file.version), SaveOutcome::Working);
    // Opened again from Files while the save waits (the copy here: no download through the door).
    let (mut second, again, _) = opened(&fake, NOTE);
    assert_eq!(again.version, 3);
    drop(door);
    assert!(matches!(settled(&mut first, NOTE), Settled::Saved { version: 4, .. }));
    fake.s().change_waits = false;
    bump(&fake, 4);
    second.changes.every = Duration::ZERO;
    assert!(
        matches!(newer_answer(&mut second, NOTE), Some(Settled::Newer(_))),
        "the earlier opening's save is a newer version for this one"
    );
    fake.s().save_answer = Ok(SaveEnd::Conflict { theirs: 4 });
    assert_eq!(second.save(NOTE, "an edit of 3\n", again.version), SaveOutcome::Working);
    assert!(matches!(settled(&mut second, NOTE), Settled::Conflict(_)));
    let s = fake.s();
    assert_eq!(
        s.saves.last().map(|save| save.2),
        Some(3),
        "sent from the version on screen"
    );
    assert_ne!(s.save_openings[0], s.save_openings[1], "each under its own opening");
}

/// Delta check of #432, round 3, P2: a save is on its way and the Available offline switch is pressed: refused at
/// once. That refusal never ends the save's turn: Show it still waits, and no second save starts.
#[test]
fn a_refused_switch_press_never_ends_a_save_on_its_way() {
    let fake = Fake::new();
    fake.s().save_answer = Ok(SaveEnd::Queued { base: 0 });
    let (mut source, file, _) = opened(&fake, NOTE);
    let door = fake.door.lock().unwrap();
    fake.s().change_waits = true;
    assert_eq!(
        source.save(NOTE, "first invented words\n", file.version),
        SaveOutcome::Working
    );
    assert!(source.set_offline(NOTE, true), "said at once");
    assert!(matches!(
        source.settled(NOTE),
        Some(Settled::Offline { on: true, done: false })
    ));
    assert!(source.saving(), "the save is still on its way");
    bump(&fake, 4);
    assert_eq!(source.newer(NOTE).err(), Some(OpenError::NotNow), "Show it waits");
    assert_eq!(
        source.save(NOTE, "second invented words\n", file.version),
        SaveOutcome::Failed,
        "one change at a time"
    );
    drop(door);
    let Settled::Queued { body, version } = settled(&mut source, NOTE) else {
        panic!("stored here");
    };
    assert_eq!((words(&body).as_str(), version), ("first invented words\n", 3));
    assert!(!source.saving());
    assert_eq!(fake.s().saves.len(), 1);
}

/// Delta check of #432, round 3, P2: the refusal of a switch press and the save's own answer come back in one drain.
/// The refusal is taken first; the save's answer is kept for the next pump (the window is woken at once), never wiped.
#[test]
fn a_save_answer_drained_with_another_answer_is_taken_on_the_next_pump() {
    let fake = Fake::new();
    fake.s().save_answer = Ok(SaveEnd::Queued { base: 0 });
    let (mut source, file, _) = opened(&fake, NOTE);
    let door = fake.door.lock().unwrap();
    fake.s().change_waits = true;
    assert_eq!(
        source.save(NOTE, "my invented words\n", file.version),
        SaveOutcome::Working
    );
    assert!(source.set_offline(NOTE, true));
    let wakes = fake.s().wakes;
    drop(door);
    until("the save answered", || fake.s().wakes > wakes);
    assert!(matches!(
        source.settled(NOTE),
        Some(Settled::Offline { on: true, done: false })
    ));
    assert!(source.saving(), "its answer is not taken yet");
    assert!(
        source.next_check().is_some_and(|after| after.is_zero()),
        "woken at once"
    );
    let Some(Settled::Queued { body, version }) = source.settled(NOTE) else {
        panic!("the save's answer, kept for this pump");
    };
    assert_eq!((words(&body).as_str(), version), ("my invented words\n", 3));
    assert!(!source.saving());
}

/// Delta check of #432, round 3: a save of an earlier opening that met a newer version says so without naming who
/// saved it (the engine cannot tell another device from this PC's earlier opening).
#[test]
fn an_earlier_openings_conflict_is_said_without_naming_a_device() {
    let fake = Fake::new();
    fake.s().save_answer = Ok(SaveEnd::Conflict { theirs: 4 });
    let (mut source, file, _) = opened(&fake, NOTE);
    let door = fake.door.lock().unwrap();
    fake.s().change_waits = true;
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Working);
    // Whatever opened the file again while the save was on its way (Show it waits for it; this goes past that).
    source.changes.busy = false;
    bump(&fake, 4);
    source.newer(NOTE).expect("the newer version opens");
    drop(door);
    let Settled::Said(words) = settled(&mut source, NOTE) else {
        panic!("said, not applied");
    };
    assert_eq!(
        words,
        "Your earlier edit was not saved: the file changed since you opened it"
    );
}

/// Delta 3 of desktop#432: the engine refused the save because an earlier change to the file on this PC has not been
/// sent yet. Nothing was stored; the answer says why (not a bare "Not saved"), and the change is over.
#[test]
fn a_save_refused_behind_an_unsent_change_says_why() {
    let fake = Fake::new();
    fake.s().save_answer = Err(EditRefused::EarlierChangeNotSent);
    let (mut source, file, _) = opened(&fake, NOTE);
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Working);
    let Settled::Refused(words) = settled(&mut source, NOTE) else {
        panic!("refused, and why");
    };
    assert_eq!(words, super::super::change::EARLIER_NOT_SENT);
    assert!(!source.saving());
}

/// Review of #452 (P1-2): an edit kept open across a lock is saved for the session after it, by the same account, under
/// a NEW opening of the file (the engine's record of the earlier opening's saves is not this one's), from the version
/// the edit began from: a file that changed meanwhile is the two-versions question, never overwritten.
#[test]
fn an_edit_kept_across_a_lock_is_saved_for_the_session_after_it_under_a_new_opening() {
    let fake = Fake::new();
    let (mut source, file, _) = opened(&fake, NOTE);
    let first = fake.s().epoch;
    fake.new_session("a@example.eu");
    assert_eq!(source.poll(NOTE), Some(Body::Stopped(Stopped::SessionChanged)));
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Working);
    assert!(matches!(settled(&mut source, NOTE), Settled::Saved { version: 4, .. }));
    let s = fake.s();
    assert_eq!(
        s.saves.last().map(|save| save.2),
        Some(3),
        "from the version the edit began from"
    );
    assert!(
        s.save_openings.last().is_some_and(|epoch| *epoch > first),
        "under a new opening"
    );
    assert_eq!(
        s.save_turns.last(),
        Some(&SessionGeneration::for_test(2)),
        "for the session after the lock"
    );
}

#[test]
fn an_edit_is_never_saved_for_another_account() {
    let fake = Fake::new();
    let (mut source, file, _) = opened(&fake, NOTE);
    fake.new_session("b@example.eu");
    assert_eq!(source.poll(NOTE), Some(Body::Stopped(Stopped::SessionChanged)));
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Failed);
    // And not once the first account is back: the opening ended with the other account.
    fake.new_session("a@example.eu");
    assert_eq!(source.poll(NOTE), None);
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Failed);
    assert!(fake.s().saves.is_empty());
}

/// Review of #452 (P1-2): a save on its way when the screen locked is answered for the session before the lock. The
/// same account back: what it came to is said (never its text), and nothing is on its way any more.
#[test]
fn a_save_on_its_way_at_a_lock_says_what_it_came_to_after_the_unlock() {
    let fake = Fake::new();
    let (mut source, file, _) = opened(&fake, NOTE);
    let door = fake.door.lock().unwrap();
    fake.s().change_waits = true;
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Working);
    fake.new_session("a@example.eu");
    drop(door);
    let Settled::Said(words) = settled(&mut source, NOTE) else {
        panic!("what the earlier save came to is said");
    };
    assert_eq!(
        words,
        "Your earlier edit was saved \u{00B7} encrypted \u{00B7} version 4"
    );
    assert!(!source.saving());
}

/// The engine could not confirm the save (the lock landed while it ran: it may have been stored before that): the
/// window says it is not confirmed, never "not saved"; another account hears nothing of it.
#[test]
fn a_save_on_its_way_at_a_lock_that_was_not_confirmed_says_so_and_another_account_hears_nothing() {
    let fake = Fake::new();
    fake.s().save_answer = Err(EditRefused::NotSignedIn);
    let (mut source, file, _) = opened(&fake, NOTE);
    let door = fake.door.lock().unwrap();
    fake.s().change_waits = true;
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Working);
    fake.new_session("a@example.eu");
    drop(door);
    let Settled::Said(words) = settled(&mut source, NOTE) else {
        panic!("the earlier save's end is said");
    };
    assert_eq!(words, super::super::change::NOT_CONFIRMED_AT_LOCK);

    let (mut source, file, _) = opened(&fake, NOTE);
    let door = fake.door.lock().unwrap();
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Working);
    fake.new_session("b@example.eu");
    drop(door);
    until("the save ended", || fake.s().saves.len() >= 2);
    std::thread::sleep(Duration::from_millis(100));
    assert!(source.settled(NOTE).is_none(), "another account hears nothing");
}

/// A second lock after the resumed edit was saved (and edited again): the file is opened again once more, a new
/// opening for the session after that lock, never adopted silently under the earlier opening.
#[test]
fn a_second_lock_after_a_resumed_save_opens_the_file_again_once_more() {
    let fake = Fake::new();
    let (mut source, file, _) = opened(&fake, NOTE);
    fake.new_session("a@example.eu");
    assert_eq!(source.poll(NOTE), Some(Body::Stopped(Stopped::SessionChanged)));
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Working);
    assert!(matches!(settled(&mut source, NOTE), Settled::Saved { version: 4, .. }));
    let resumed = *fake.s().save_openings.last().unwrap();
    fake.new_session("a@example.eu");
    assert_eq!(
        source.poll(NOTE),
        Some(Body::Stopped(Stopped::SessionChanged)),
        "what is under the editor goes at the second lock too"
    );
    fake.s().save_answer = Ok(SaveEnd::Saved { version: 5 });
    assert_eq!(source.save(NOTE, "mine again\n", 4), SaveOutcome::Working);
    assert!(matches!(settled(&mut source, NOTE), Settled::Saved { version: 5, .. }));
    let s = fake.s();
    assert!(
        s.save_openings.last().is_some_and(|epoch| *epoch > resumed),
        "a new opening"
    );
    assert_eq!(
        s.save_turns.last(),
        Some(&SessionGeneration::for_test(3)),
        "for the session after the second lock"
    );
}

/// Task 2530, item 1: the earlier change is held back (parked, or a held edit): the refusal says so, and never that it
/// is on its way.
#[test]
fn a_held_refusal_shows_the_held_words() {
    let fake = Fake::new();
    fake.s().save_answer = Err(EditRefused::EarlierChangeHeld);
    let (mut source, file, _) = opened(&fake, NOTE);
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Working);
    let Settled::Refused(words) = settled(&mut source, NOTE) else {
        panic!("refused, and why");
    };
    assert_eq!(words, super::super::change::EARLIER_HELD);
    assert!(!source.saving());
}

/// Save a copy after a refusal, answered after the session moved to another account: dropped, nothing applied (the
/// stamp rule every change follows). The edit stays with the window.
#[test]
fn a_late_copy_answer_after_a_session_change_is_dropped() {
    let fake = Fake::new();
    fake.s().save_answer = Err(EditRefused::EarlierChangeNotSent);
    let (mut source, file, _) = opened(&fake, NOTE);
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Working);
    assert!(matches!(settled(&mut source, NOTE), Settled::Refused(_)));
    let door = fake.door.lock().unwrap();
    fake.s().change_waits = true;
    assert_eq!(source.resolve(NOTE, "mine\n", Keep::Both), SaveOutcome::Working);
    fake.new_session("b@example.eu");
    drop(door);
    until("the copy was asked for", || fake.s().copies.len() == 1);
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        source.settled(NOTE).is_none(),
        "another session's copy answer is never applied, nor said"
    );
    assert!(
        fake.s().saves.len() == 1,
        "nothing was written to the note after the refusal"
    );
}

/// R5: a refusal of the copy itself is said as the two-versions Keep both says it (not saved); nothing moves.
#[test]
fn a_refused_copy_is_said_and_moves_nothing() {
    let fake = Fake::new();
    fake.s().save_answer = Err(EditRefused::EarlierChangeNotSent);
    fake.s().copy_refused = Some(EditRefused::ReadOnly);
    let (mut source, file, _) = opened(&fake, NOTE);
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Working);
    assert!(matches!(settled(&mut source, NOTE), Settled::Refused(_)));
    assert_eq!(source.resolve(NOTE, "mine\n", Keep::Both), SaveOutcome::Working);
    assert!(matches!(settled(&mut source, NOTE), Settled::Failed));
    assert_eq!(
        fake.s().copies,
        [(NOTE.to_string(), "mine\n".to_string())],
        "the edit's text, unchanged"
    );
}

/// Review of #452 (P2-B): the open file ended at a lock; after the unlock the first press could not read the file's
/// facts. That opening is not taken up: the next press opens the file again and the save goes out.
#[test]
fn a_resume_whose_facts_were_not_read_is_tried_again_at_the_next_press() {
    let fake = Fake::new();
    let (mut source, file, _) = opened(&fake, NOTE);
    fake.new_session("a@example.eu");
    assert_eq!(source.poll(NOTE), Some(Body::Stopped(Stopped::SessionChanged)));
    let facts = fake.s().edit.take();
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Failed);
    fake.s().edit = facts;
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Working);
    assert!(matches!(settled(&mut source, NOTE), Settled::Saved { version: 4, .. }));
    assert_eq!(
        fake.s().save_turns.last(),
        Some(&SessionGeneration::for_test(2)),
        "for the session after the lock"
    );
}

/// Review of #452 (P2-B): a press that reaches the view while Beebeeb is locked (the lock landed, the screen not
/// redrawn yet) opens nothing again and saves nothing; after the unlock, the save goes out.
#[test]
fn a_press_while_beebeeb_is_locked_opens_nothing_again() {
    let fake = Fake::new();
    let (mut source, file, _) = opened(&fake, NOTE);
    fake.new_session("a@example.eu");
    assert_eq!(source.poll(NOTE), Some(Body::Stopped(Stopped::SessionChanged)));
    fake.s().locked = true;
    let forgets = fake.s().forgets;
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Failed);
    assert_eq!(fake.s().forgets, forgets, "no new opening while locked");
    assert!(fake.s().saves.is_empty(), "nothing saved while locked");
    fake.s().locked = false;
    assert_eq!(source.save(NOTE, "mine\n", file.version), SaveOutcome::Working);
    assert!(matches!(settled(&mut source, NOTE), Settled::Saved { version: 4, .. }));
}
