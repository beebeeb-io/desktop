//! w26-account-switch (v5 bug 6): account A signed out and left its local data bound to it (a sign-out keeps it), and
//! account B starts sync on the same PC. B must start clean and never see A's names, so before the binding of step 7
//! A's data is removed, when it provably is another account's and nothing of it needs a person's answer.
//!
//! Only two USER IDS that differ prove another account: an owner recorded by an email alone, and unowned data, are
//! task 2267's cases (the ownership proof, nothing deleted). Anything that cannot be removed leaves A's owner record in
//! place, so the binding refuses B as it always did. The order: the folder's items (through the port), the files the
//! rows point at, then the one transaction that clears every row and the owner record.
//!
//! Must not: remove anything while a queued change, a staged copy, a held edit or a file of unknown content is left;
//! clear the owner record before every item is gone; log a name, a path, an email or an id.

use std::path::Path;

use super::{CredentialStores, Engine, LocalState};
use crate::account_binding::Identity;
use crate::sync_root_port::SyncRootPort;

impl<C: CredentialStores> Engine<C> {
    /// Step 7's part: see the module. A no-op unless `data` is recorded as another account's by user id. Best effort in
    /// the sense that a failure changes nothing the binding reads: the start then refuses as before.
    pub(super) fn clear_the_previous_account(
        &self,
        data: &LocalState,
        session: &Identity,
        sync_root: &Path,
        port: &dyn SyncRootPort,
    ) {
        let owner = data.db.owner().ok().flatten();
        let other_account = match (owner.as_ref().and_then(|o| o.user_id.as_deref()), session.user_id.as_deref()) {
            (Some(recorded), Some(now)) => recorded != now,
            _ => false,
        };
        if !other_account {
            return;
        }
        match self.remove_the_previous_account(data, sync_root, port) {
            Ok(()) => tracing::info!("account switch: the previous account's local data was removed"),
            Err(why) => tracing::warn!(why, "account switch: the previous account's local data stays"),
        }
    }

    fn remove_the_previous_account(
        &self,
        data: &LocalState,
        sync_root: &Path,
        port: &dyn SyncRootPort,
    ) -> Result<(), &'static str> {
        let db = &data.db;
        if let Some(kind) = db.work_waiting_for_a_person().map_err(|_| "unreadable")? {
            return Err(kind);
        }
        if db.has_account_data().map_err(|_| "unreadable")? {
            port.remove_synced_items(db, sync_root).map_err(|_| "the folder's items stay")?;
        }
        let purge = db.purge_all_local_state().map_err(|_| "the rows stay")?;
        for path in purge.payload_paths.iter().chain(purge.cache_paths.iter()) {
            let path = Path::new(path);
            if !crate::is_disposable_cache_path(self.identity, path) {
                return Err("a cached file is not ours to delete");
            }
            match std::fs::remove_file(path) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(_) => return Err("a cached file stays"),
            }
        }
        db.forget_share_trust_rows().map_err(|_| "the share marks stay")?;
        // The owner record goes in this transaction, last: until it commits, B is still refused.
        db.clear_account_data(false).map_err(|_| "the rows stay")
    }
}
