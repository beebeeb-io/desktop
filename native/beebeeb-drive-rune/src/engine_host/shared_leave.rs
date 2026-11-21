//! Leave a share someone made with the person, and where a row of Shared points (w17-shared-items). Part of
//! [`EngineSharedSource`] (`shared.rs`), which hands `leave` and `target_of` here.
//!
//! - Leave: only for a row of the current session's "Shared with me" list that came with an invite; it sends
//!   `ShareChange::Leave` for that invite through the adapter's change channel, so its rules hold unchanged (one per id
//!   at a time, an answer of another session is dropped, nothing leaves the list before Beebeeb answered and the lists
//!   were read again). The view asks first (`ConfirmLeave`).
//! - Target: what a row stands for in Files: a shared item (a folder or a file), a link's item, or the folder a file
//!   request's uploads land in. Read from the lists held; never from a draw call's engine read.
//!
//! Must not: log a name or an id.

use super::*;
use crate::frames::shared::parts::table::Kind as RowKind;

impl<C: CredentialStores> EngineSharedSource<C> {
    pub(super) fn shared_leave(&mut self, id: &str) -> bool {
        let Some((session, invite)) = self
            .current()
            .and_then(|held| held.invites.get(id).map(|invite| (held.session, invite.clone())))
        else {
            return false;
        };
        self.change(session, id, ShareChange::Leave { invite_id: invite })
    }

    pub(super) fn shared_target_of(&self, id: &str) -> Option<RowTarget> {
        let held = self.current()?;
        if let Some(row) = held.with_me.iter().find(|row| row.id == id) {
            return Some(RowTarget::Item {
                id: row.id.clone(),
                is_folder: matches!(row.kind, RowKind::Folder),
            });
        }
        if let Some(item) = held.link_items.get(id) {
            // A link row does not say whether its item is a folder; a link to a folder is a share with people too.
            let is_folder = held
                .people
                .iter()
                .find(|p| p.id == *item)
                .is_some_and(|p| matches!(p.kind, RowKind::Folder));
            return Some(RowTarget::Item {
                id: item.clone(),
                is_folder,
            });
        }
        if let Some(share) = held.people.iter().find(|p| p.id == id) {
            return Some(RowTarget::Item {
                id: share.id.clone(),
                is_folder: matches!(share.kind, RowKind::Folder),
            });
        }
        held.request_folders
            .get(id)
            .map(|folder| RowTarget::Folder(folder.clone()))
    }
}
