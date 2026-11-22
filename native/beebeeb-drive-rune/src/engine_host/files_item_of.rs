//! `FilesSource::item_of` on the real engine: the row of one id, as the signed-in account's local state holds it
//! (the way `file_ops::resolve` finds a file for an action). The handle behind [`super::EngineFilesSource::local`] is
//! the account-bound one, so an id of another account's state is not found. Must not log an id or a name.

use beebeeb_engine::drive_v2::model;
use beebeeb_engine::state_db::StateDb;

use crate::source::DriveItem;
use crate::source_engine;

pub(super) fn item_of(db: &StateDb, id: &str) -> Option<DriveItem> {
    model::details(db, id).ok().map(|details| source_engine::item(details.item))
}
