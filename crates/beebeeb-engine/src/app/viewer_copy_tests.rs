//! A viewer save refused behind an earlier change to the file on this PC (task 2530, item 1): the refusal says whether
//! that change is held back or only not sent yet, and Save a copy (Keep both) keeps the edit as a new file while the
//! original stays at its version with its queued change. On the engine of `viewer_api_tests.rs`, against the local
//! state alone (sync does not run: every save is stored here and answers at once). Every name and byte is invented.

use super::*;

const EDIT: &str = "an invented edit, of version 4, made in a second opening\n";

/// The first opening's save of the note, stored here from version 4 and not sent: its op.
fn first_save_queued(engine: &TestEngine, db: &StateDb) -> PendingOperation {
    let session = engine.acct.session_generation();
    let first_opening = engine.viewer_opening(session, NOTE);
    let first = engine.save_text_for(session, NOTE, "first invented edit\n", 4, first_opening);
    assert!(
        matches!(first, Ok(SaveEnd::Queued { base: 4 })),
        "{:?}",
        first.map(|_| ())
    );
    db.list_operations_for_file(NOTE)
        .unwrap()
        .pop()
        .expect("the first save's op")
}

/// A second opening of the note saves another edit from version 4.
fn second_opening_saves(engine: &TestEngine) -> Result<SaveEnd, EditRefused> {
    let session = engine.acct.session_generation();
    let again = engine.viewer_opening(session, NOTE);
    engine.save_text_for(session, NOTE, EDIT, 4, again)
}

/// The earlier save is parked (its write used up its tries: nothing sends it until the person settles it): the refusal
/// says it is held back, never that it is on its way.
#[test]
fn a_save_behind_a_held_change_says_held() {
    let fake = fake("copy-parked");
    let engine = engine_at("http://127.0.0.1:9");
    let (_dir, _root, db) = writable_note_at_4(&engine, &fake);
    let op = first_save_queued(&engine, &db);
    db.park_for_test(&op.op_id);
    let save = second_opening_saves(&engine);
    assert_eq!(save.err(), Some(EditRefused::EarlierChangeHeld), "held back, said so");
    assert_eq!(
        db.list_operations_for_file(NOTE).unwrap().len(),
        1,
        "nothing queued for it"
    );
}

/// The same when the file has a held edit of task 2524 (a change the scan holds back) next to the queued save.
#[test]
fn a_save_behind_a_held_sync_folder_edit_says_held() {
    let fake = fake("copy-held-edit");
    let engine = engine_at("http://127.0.0.1:9");
    let (_dir, _root, db) = writable_note_at_4(&engine, &fake);
    first_save_queued(&engine, &db);
    assert!(
        db.hold_unsent_edit(NOTE, "base_unknown", "invented-fingerprint", 20)
            .unwrap()
    );
    let save = second_opening_saves(&engine);
    assert_eq!(save.err(), Some(EditRefused::EarlierChangeHeld), "held back, said so");
    assert_eq!(
        db.list_operations_for_file(NOTE).unwrap().len(),
        1,
        "nothing queued for it"
    );
}

/// A `content_unknown` row (a file hydrated before 0.8.13: this PC cannot tell whether it changed) is no earlier change:
/// behind a queued save, the refusal still says not sent yet, never held back.
#[test]
fn a_content_unknown_row_is_not_a_held_change() {
    let fake = fake("copy-content-unknown");
    let engine = engine_at("http://127.0.0.1:9");
    let (_dir, _root, db) = writable_note_at_4(&engine, &fake);
    first_save_queued(&engine, &db);
    assert!(
        db.hold_unsent_edit(NOTE, "content_unknown", "invented-fingerprint", 20)
            .unwrap()
    );
    let save = second_opening_saves(&engine);
    assert_eq!(save.err(), Some(EditRefused::EarlierChangeNotSent), "waiting, said so");
}

/// Behind a change that is only waiting to be sent (queued, not parked, nothing held): not sent yet.
#[test]
fn a_save_behind_a_queued_change_says_not_sent() {
    let fake = fake("copy-queued");
    let engine = engine_at("http://127.0.0.1:9");
    let (_dir, _root, db) = writable_note_at_4(&engine, &fake);
    first_save_queued(&engine, &db);
    let save = second_opening_saves(&engine);
    assert_eq!(save.err(), Some(EditRefused::EarlierChangeNotSent), "waiting, said so");
}

/// R1 and R3: after the refusal, Save a copy (Keep both) makes ONE new file with the edit's text, next to the note.
/// The note stays at version 4, its file's write and its queued change are the first save's, and its row is as the
/// first save left it: the copy is a create, never a modify of the note.
#[test]
fn save_a_copy_after_the_refusal_keeps_the_original() {
    let fake = fake("copy-keeps");
    let engine = engine_at("http://127.0.0.1:9");
    let (_dir, _root, db) = writable_note_at_4(&engine, &fake);
    let op = first_save_queued(&engine, &db);
    let held = db.held_write(NOTE).unwrap().expect("the first save's write");
    let row = db.get_file(NOTE).unwrap().expect("the note");
    assert_eq!(
        second_opening_saves(&engine).err(),
        Some(EditRefused::EarlierChangeNotSent)
    );
    let session = engine.acct.session_generation();
    let copy = engine
        .save_text_copy_for(session, NOTE, EDIT, "Invented-PC", "09:41")
        .unwrap_or_else(|refused| panic!("the copy is made: {refused:?}"));
    assert_ne!(copy.id, NOTE, "a new file");
    assert_eq!(
        copy.name, "Invented note (Invented-PC, 09:41).txt",
        "named as every conflict copy is"
    );
    // The note: its version, its write, its queued change and its row are as the first save left them.
    assert_eq!(db.get_file_contract_state(NOTE).unwrap().unwrap().current_version, 4);
    assert_eq!(
        db.held_write(NOTE).unwrap(),
        Some(held),
        "the note's write is still the first save's"
    );
    let ops: Vec<String> = db
        .list_operations_for_file(NOTE)
        .unwrap()
        .into_iter()
        .map(|op| op.op_id)
        .collect();
    assert_eq!(ops, [op.op_id], "the note's queued change alone, unchanged");
    assert_eq!(db.get_file(NOTE).unwrap().expect("the note").size_bytes, row.size_bytes);
    // The copy: ONE create, whose staged bytes are the edit.
    let made = db.list_operations_for_file(&copy.id).unwrap();
    assert_eq!(made.len(), 1, "one change for the copy");
    let staged = made[0].payload_path.as_deref().expect("the copy's staged bytes");
    assert_eq!(std::fs::read(staged).expect("the staged copy reads"), EDIT.as_bytes());
    // Next to the note: under the note's own parent (a create's row path is built under its parent; this harness's
    // rows carry none, so the path is the copy's name alone).
    let parent = |id: &str| db.get_file_contract_state(id).unwrap().expect("a record").parent_id;
    assert_eq!(parent(&copy.id), parent(NOTE), "next to the note");
    let path = db.get_file(&copy.id).unwrap().expect("the copy's row").path;
    assert_eq!(path.rsplit('/').next(), Some("Invented note (Invented-PC, 09:41).txt"));
}
