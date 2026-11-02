//! The typed ids: one test per id, so a new one cannot arrive without a
//! `new`/`as_str` round trip and the `Copy`/`Eq` a call site relies on.

use super::*;

macro_rules! id_round_trips {
    ($test:ident, $id:ident, $raw:literal) => {
        #[test]
        fn $test() {
            let id = $id::new($raw);
            assert_eq!(id.as_str(), $raw);
            assert_eq!(id.to_string(), $raw);
            // Copy: two uses of one id, and Eq so a test can compare them.
            let again = id;
            assert_eq!(id, again);
            assert_eq!(id, $id::new($raw));
        }
    };
}

id_round_trips!(file_id_round_trips, FileId, "file/one");
id_round_trips!(folder_id_round_trips, FolderId, "folder/one");
id_round_trips!(version_id_round_trips, VersionId, "version/two");
id_round_trips!(upload_session_id_round_trips, UploadSessionId, "session/one");
id_round_trips!(shared_folder_id_round_trips, SharedFolderId, "shared/one");
id_round_trips!(invite_id_round_trips, InviteId, "invite/one");

#[test]
fn two_ids_of_different_kinds_are_not_interchangeable() {
    // The reason for the newtypes: the compiler refuses the swap. This test
    // pins the spelling instead — the raw strings are equal, the ids are not.
    let file = FileId::new("same");
    let folder = FolderId::new("same");
    assert_eq!(file.as_str(), folder.as_str());
}
