//! Slice B4 (plan40 batch 1): Make available offline on a file someone else shares (DW-0505 to DW-0412).
//!
//! Session-bound like its neighbours (`*_for(session)`): every call is asked for the session the host read at its
//! press, and its answer is handed over only while that session is still the one in memory.
//!
//! One call: [`Engine::keep_shared_offline_for`]. A file shared with this account has its row in the account's own
//! local state (the `SharedWithMe` namespace, read-only or not), so keeping it is the same pin change Files' own
//! "Make available offline" makes ([`Engine::set_kept_in`]): the state database, the queued pin for other devices,
//! and on Windows the placeholder's pin state. No new crypto and no new key use: the bytes are fetched and opened
//! by the engine's existing hydrate path, with the key the share already carries, and land where Files' offline
//! copies land (under the sync folder, in "Shared with me"). Nothing is written anywhere else.
//!
//! BLOCKS (the state database, and on Windows a Cloud Files call): a worker's, never the window's thread.
//! Must not: log a name, a path or an id.

use std::collections::HashSet;

use crate::account::SessionGeneration;
use crate::app::kept_api::NOT_SIGNED_IN;
use crate::app::{CredentialStores, Engine};
use crate::finder_show::item_namespace;
use crate::state_db::{Namespace, StateDb};

/// Why [`Engine::keep_shared_offline_for`] kept nothing. Nothing was changed in any of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharedKeepRefused {
    /// No session in memory, or not the session of the press.
    NotSignedIn,
    /// The id is not a row someone else shares with this account (an own file, a Backups file, an unknown id).
    /// Files' own Make available offline is the way for those.
    NotShared,
    /// The local state could not be read, is another account's, no sync folder is set, or the pin change failed.
    NotKept,
}

impl<C: CredentialStores> Engine<C> {
    /// Keep the file or folder `item_id`, shared with this account, on this PC (a folder with everything in it).
    /// Nothing is changed unless the session in memory is still `session` and the row is a shared-with-me row of the
    /// signed-in account's local state. Asked again after the change: a session that moved meanwhile answers
    /// `NotSignedIn`, and the host drops that answer.
    pub fn keep_shared_offline_for(&self, session: SessionGeneration, item_id: &str) -> Result<(), SharedKeepRefused> {
        if !self.acct.session_unchanged_since(session) {
            return Err(SharedKeepRefused::NotSignedIn);
        }
        let state = self.local_state_for_changes().map_err(|_| SharedKeepRefused::NotKept)?;
        if state.session() != session {
            return Err(SharedKeepRefused::NotSignedIn);
        }
        // Only a row of the shared-with-me namespace: this call never pins an own file or a backup through the
        // shared view's id.
        match state.with(|db| item_namespace(db, item_id)) {
            Ok(Ok(Some(Namespace::SharedWithMe))) => {}
            Ok(Ok(_)) => return Err(SharedKeepRefused::NotShared),
            Ok(Err(_)) => return Err(SharedKeepRefused::NotKept),
            Err(_) => return Err(SharedKeepRefused::NotSignedIn),
        }
        self.set_kept_in(Some(session), item_id, true).map_err(|why| {
            if why == NOT_SIGNED_IN {
                SharedKeepRefused::NotSignedIn
            } else {
                SharedKeepRefused::NotKept
            }
        })
    }

    /// Which of `item_ids` (ids of Shared with me rows, as the lists name them) are kept on this PC now. Read only:
    /// it changes nothing. The same session checks as [`Self::keep_shared_offline_for`]; an id with no row, or a row
    /// that is not a shared-with-me row (an own file), is simply not in the answer. The ids come back as asked, not
    /// as the alias they resolved to.
    pub fn shared_kept_of_for(
        &self,
        session: SessionGeneration,
        item_ids: &[String],
    ) -> Result<HashSet<String>, SharedKeepRefused> {
        if !self.acct.session_unchanged_since(session) {
            return Err(SharedKeepRefused::NotSignedIn);
        }
        let state = self.local_state_for_changes().map_err(|_| SharedKeepRefused::NotKept)?;
        if state.session() != session {
            return Err(SharedKeepRefused::NotSignedIn);
        }
        match state.with(|db| shared_kept_in(db, item_ids)) {
            Ok(Ok(kept)) => Ok(kept),
            Ok(Err(_)) => Err(SharedKeepRefused::NotKept),
            Err(_) => Err(SharedKeepRefused::NotSignedIn),
        }
    }
}

/// The ids among `item_ids` whose row is a shared-with-me row AND kept (the rule of `kept_rows`), in one
/// `sizes_and_kept` query. An alias is followed to the row it became before either check (as [`item_namespace`]
/// does), and the id is returned as asked.
fn shared_kept_in(db: &StateDb, item_ids: &[String]) -> Result<HashSet<String>, String> {
    let mut resolved: Vec<(&String, String)> = Vec::new();
    for id in item_ids {
        let landed_as = db
            .resolve_alias(id)
            .map_err(|error| format!("read item: {error}"))?
            .unwrap_or_else(|| id.clone());
        if item_namespace(db, id)? == Some(Namespace::SharedWithMe) {
            resolved.push((id, landed_as));
        }
    }
    let ask: Vec<String> = resolved.iter().map(|(_, row)| row.clone()).collect();
    let marks = db.sizes_and_kept(&ask).map_err(|error| format!("read kept: {error}"))?;
    Ok(resolved
        .into_iter()
        .filter(|(_, row)| marks.get(row).is_some_and(|(_, kept)| *kept))
        .map(|(id, _)| id.clone())
        .collect())
}

#[cfg(test)]
#[path = "shared_offline_tests.rs"]
mod tests;
