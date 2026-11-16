//! How a local file operation's `DriveError` reads in the UI (`ActionError`): the one mapping every Files action uses,
//! and New file's, which says a full disk and a too-large file in their own words.

use beebeeb_engine::drive_v2::DriveError;
use crate::frames::files::source::ActionError;

/// Map a `DriveError` to the UI's `ActionError`.
pub(super) fn map_drive_err(e: DriveError) -> ActionError {
    match e {
        DriveError::ReadOnly => ActionError::Protected,
        DriveError::NameTaken | DriveError::InvalidName => ActionError::NameTaken,
        DriveError::NotFound => ActionError::Failed,
        DriveError::Conflict => ActionError::Failed,
        _ => ActionError::Failed,
    }
}

/// [`map_drive_err`] for New file, which says two refusals in their own words: a full disk and a file too large
/// (every other refusal is [`map_drive_err`]'s).
pub(super) fn map_new_file_err(e: DriveError) -> ActionError {
    match e {
        DriveError::Quota => ActionError::NoRoom,
        DriveError::TooLarge => ActionError::TooLarge,
        other => map_drive_err(other),
    }
}

/// New file's mapping: the two refusals that have their own words, and every other refusal as before.
#[cfg(test)]
#[test]
fn new_file_says_a_full_disk_and_a_too_large_file_in_their_own_words() {
    // Mutant: `DriveError::Quota => ActionError::Failed` -> red (the generic "could not make the file").
    assert_eq!(map_new_file_err(DriveError::Quota), ActionError::NoRoom);
    assert_eq!(map_new_file_err(DriveError::TooLarge), ActionError::TooLarge);
    assert_eq!(map_new_file_err(DriveError::NameTaken), ActionError::NameTaken);
    assert_eq!(map_new_file_err(DriveError::ReadOnly), ActionError::Protected);
    assert_eq!(map_new_file_err(DriveError::Failed("disk")), ActionError::Failed);
    // The generic mapping is unchanged: the other actions still say "Couldn't ...".
    assert_eq!(map_drive_err(DriveError::Quota), ActionError::Failed);
}
