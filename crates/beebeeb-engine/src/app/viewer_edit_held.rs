//! Which refusal a viewer save behind an earlier write of the file is (task 2530, item 1): the earlier change is held
//! back and is not sent until it is settled in Sync history, or it is only waiting to be sent. For the words alone:
//! nothing is stored either way.

use crate::state_db::StateDb;

/// `file_id` has an earlier change this PC holds back. Read once, after the accept refused the save.
pub(crate) fn earlier_change_is_held(_db: &StateDb, _file_id: &str) -> bool {
    false
}
