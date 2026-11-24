//! The viewer's save and the write queue: a save never queues behind a write of the
//! file that is not its own opening's. On the engine of `viewer_api_tests.rs`, against the local state alone (sync
//! does not run: every save is stored here and answers at once). Every name and byte is invented.

use super::*;
use crate::app::viewer_edit_api::EditRefused;

/// The note at version 4 on this PC, writable by the account.
fn writable_note_at_4(engine: &TestEngine, fake: &Fake) -> (tempfile::TempDir, tempfile::TempDir, StateDb) {
    signed_in(engine, &fake.first);
    let (dir, root, db) = with_folder(engine, &fake.first, FileStatus::Local, Some(&note_text()));
    set_current_version(&db, NOTE, 4);
    let mut contract = db.get_file_contract_state(NOTE).unwrap().unwrap();
    contract.permission_bits |= crate::state_db::PERMISSION_READ | crate::state_db::PERMISSION_WRITE;
    db.set_file_contract_state(&contract).unwrap();
    (dir, root, db)
}

/// The first opening's save is stored here (Queued from 4) and has not landed. The file is
/// opened again, shows version 4, and the person saves another edit of it from 4. That save is never queued behind the
/// first one (both would land, 5 then 6, and nobody was asked): nothing is queued for it, the file's write is still
/// the first save's, and the row is as the first save left it.
#[test]
fn a_new_openings_save_never_queues_behind_an_earlier_openings_unsent_save() {
    let fake = fake("chain");
    let engine = engine_at("http://127.0.0.1:9");
    let (_dir, _root, db) = writable_note_at_4(&engine, &fake);
    let session = engine.acct.session_generation();
    let first_opening = engine.viewer_opening(session, NOTE);
    let first = engine.save_text_for(session, NOTE, "first invented edit\n", 4, first_opening);
    assert!(
        matches!(first, Ok(SaveEnd::Queued { base: 4 })),
        "{:?}",
        first.map(|_| ())
    );
    let held = db.held_write(NOTE).unwrap().expect("the first save's write");
    let row = db.get_file(NOTE).unwrap().expect("the note");
    let again = engine.viewer_opening(session, NOTE);
    let second = engine.save_text_for(session, NOTE, "another invented edit, of version 4\n", 4, again);
    assert!(second.is_err(), "nothing queued behind it: {:?}", second.map(|_| ()));
    assert_eq!(second.err(), Some(EditRefused::EarlierChangeNotSent), "said why");
    assert_eq!(
        db.list_operations_for_file(NOTE).unwrap().len(),
        1,
        "the first save's op alone"
    );
    assert_eq!(
        db.held_write(NOTE).unwrap(),
        Some(held),
        "the file's write is still the first save's"
    );
    let after = db.get_file(NOTE).unwrap().expect("the note");
    assert_eq!(after.size_bytes, row.size_bytes, "the row as the first save left it");
}

/// The same, while the first opening's save is parked (held back: its base was not known, or it met a newer
/// version). Deliberate: queued behind it, the new save would park behind it too, unasked. Said as held back (task 2530,
/// item 1): nothing sends it until the person settles it.
#[test]
fn a_new_openings_save_never_queues_behind_an_earlier_openings_parked_save() {
    let fake = fake("chain-parked");
    let engine = engine_at("http://127.0.0.1:9");
    let (_dir, _root, db) = writable_note_at_4(&engine, &fake);
    let session = engine.acct.session_generation();
    let first_opening = engine.viewer_opening(session, NOTE);
    let first = engine.save_text_for(session, NOTE, "first invented edit\n", 4, first_opening);
    assert!(
        matches!(first, Ok(SaveEnd::Queued { base: 4 })),
        "{:?}",
        first.map(|_| ())
    );
    let op = db
        .list_operations_for_file(NOTE)
        .unwrap()
        .pop()
        .expect("the first save's op");
    db.park_for_test(&op.op_id);
    let again = engine.viewer_opening(session, NOTE);
    let second = engine.save_text_for(session, NOTE, "another invented edit, of version 4\n", 4, again);
    assert!(second.is_err(), "nothing queued behind it: {:?}", second.map(|_| ()));
    assert_eq!(
        second.err(),
        Some(EditRefused::EarlierChangeHeld),
        "said why: held back"
    );
    assert_eq!(
        db.list_operations_for_file(NOTE).unwrap().len(),
        1,
        "the parked op alone"
    );
}

/// What stays: the SAME opening's next save, while its own first save is still unsent, is queued behind it (the text
/// on screen is the queued text, so following it is right).
#[test]
fn the_same_openings_next_save_still_queues_behind_its_own_unsent_save() {
    let fake = fake("chain-own");
    let engine = engine_at("http://127.0.0.1:9");
    let (_dir, _root, db) = writable_note_at_4(&engine, &fake);
    let session = engine.acct.session_generation();
    let opening = engine.viewer_opening(session, NOTE);
    let first = engine.save_text_for(session, NOTE, "first invented edit\n", 4, opening);
    assert!(
        matches!(first, Ok(SaveEnd::Queued { base: 4 })),
        "{:?}",
        first.map(|_| ())
    );
    let held = db.held_write(NOTE).unwrap().expect("the first save's write");
    let second = engine.save_text_for(session, NOTE, "first invented edit, and more\n", 4, opening);
    assert!(
        matches!(second, Ok(SaveEnd::Queued { base: 4 })),
        "{:?}",
        second.map(|_| ())
    );
    assert_eq!(
        db.list_operations_for_file(NOTE).unwrap().len(),
        2,
        "queued behind the first"
    );
    assert_ne!(
        db.held_write(NOTE).unwrap().map(|write| write.write_id),
        Some(held.write_id),
        "the file's write is now the second save's"
    );
}

/// Once the earlier opening's save has landed, the refusal is over: the new opening's save from the version it shows
/// answers the two-versions question (the file is at the version that save became), and nothing is queued.
#[test]
fn once_the_earlier_openings_save_landed_the_new_openings_save_answers_conflict() {
    let fake = fake("chain-landed");
    let engine = engine_at("http://127.0.0.1:9");
    let (_dir, _root, db) = writable_note_at_4(&engine, &fake);
    let session = engine.acct.session_generation();
    let first_opening = engine.viewer_opening(session, NOTE);
    let first = engine.save_text_for(session, NOTE, "first invented edit\n", 4, first_opening);
    assert!(
        matches!(first, Ok(SaveEnd::Queued { base: 4 })),
        "{:?}",
        first.map(|_| ())
    );
    let op = db
        .list_operations_for_file(NOTE)
        .unwrap()
        .pop()
        .expect("the first save's op");
    land_viewer_save(&db, &op.op_id, 4, 5);
    let again = engine.viewer_opening(session, NOTE);
    let second = engine.save_text_for(session, NOTE, "another invented edit, of version 4\n", 4, again);
    assert_eq!(second, Ok(SaveEnd::Conflict { theirs: 5 }), "the person decides");
    assert!(db.list_operations_for_file(NOTE).unwrap().is_empty(), "nothing queued");
}

/// The unsent change is a File Provider save of the file (on macOS, a save from another app), not the viewer's: the
/// viewer's save is not queued after it either.
#[test]
fn a_viewer_save_never_queues_behind_an_unsent_file_provider_save() {
    let fake = fake("chain-fp");
    let engine = engine_at("http://127.0.0.1:9");
    let (_dir, _root, db) = writable_note_at_4(&engine, &fake);
    db.track_staged_payload("/staged/finder", None, false).unwrap();
    let accepted = db
        .accept_finder_write(
            &FinderAccept {
                retire_parked: None,
                op_id: "finder-op",
                file_id: NOTE,
                kind: FinderAcceptKind::Modify {
                    incoming_base: Some("4"),
                },
                parent_id: None,
                target_path: Some("Invented folder/Deeper/Invented note.txt"),
                metadata_json: "{}",
                payload_path: "/staged/finder",
                size_bytes: 3,
                modified_at: 20,
                backup_source_key: None,
                now: 20,
            },
            &|_, _| Vec::new(),
        )
        .unwrap();
    assert!(
        matches!(accepted, AcceptOutcome::Queued { .. }),
        "the File Provider save is queued"
    );
    let session = engine.acct.session_generation();
    let opening = engine.viewer_opening(session, NOTE);
    let save = engine.save_text_for(session, NOTE, "an invented edit\n", 4, opening);
    assert_eq!(save.err(), Some(EditRefused::EarlierChangeNotSent));
    assert_eq!(
        db.list_operations_for_file(NOTE).unwrap().len(),
        1,
        "the File Provider's op alone"
    );
}

#[path = "viewer_copy_tests.rs"]
mod viewer_copy;
