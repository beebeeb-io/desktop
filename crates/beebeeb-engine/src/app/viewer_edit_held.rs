//! Which refusal a viewer save behind an earlier write of the file is (task 2530, item 1): the earlier change is held
//! back and is not sent until it is settled in Sync history, or it is only waiting to be sent. For the words alone:
//! nothing is stored either way.

use crate::state_db::StateDb;

/// `file_id` has an earlier change this PC holds back. Read once, after the accept refused the save:
/// - a queued write of the file whose tries are used up (parked, or given up: `ItemPresentation::parked_finder_upload`).
///   Nothing sends it until the person settles it; This PC lists it under Sync history's problems;
/// - a held edit of task 2524 (`held_edits`) whose base is not known: a change the scan found and does not send, which
///   This PC lists as its own row. Not a `content_unknown` one: that only says this PC cannot tell whether the file
///   changed (a file hydrated before 0.8.13, mostly untouched; This PC: "Nothing to do now"), and it stays until the
///   next scan finds the queued write, so it is no earlier change.
///
/// Per file, from the readers This PC's own rows come from. A read that fails answers `false` (the words then say
/// "not sent yet"); nothing but the words depends on it.
pub(crate) fn earlier_change_is_held(db: &StateDb, file_id: &str) -> bool {
    let parked = db
        .item_presentation(file_id)
        .ok()
        .flatten()
        .is_some_and(|item| item.parked_finder_upload);
    // Every held edit, by its index (`list_held_edits`); at most one row is this file's.
    parked
        || db.list_held_edits().is_ok_and(|held| {
            held.iter()
                .any(|(id, _, reason, _)| id == file_id && reason != "content_unknown")
        })
}
