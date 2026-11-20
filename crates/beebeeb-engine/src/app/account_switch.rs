//! w26-account-switch (v5 bug 6): account A signed out and left its local data bound to it (a sign-out keeps it), and
//! account B starts sync on the same PC. B must start clean and never see A's names, so before the binding of step 7
//! A's data is removed, when it provably is another account's and nothing of it needs a person's answer.
//!
//! Only two USER IDS that differ prove another account: an owner recorded by an email alone, and unowned data, are
//! task 2267's cases (the ownership proof, nothing deleted). Anything that cannot be removed leaves A's owner record in
//! place, so the binding refuses B as it always did. The order: the folder's items (through the port), the files the
//! rows point at, then the one transaction that clears every row and the owner record.
//!
//! The account's entries in desktop.toml go with it ([`forget_in_settings`]), just before the owner record.
//!
//! Must not: remove anything while a queued change, a staged copy, a held edit or a file of unknown content that may
//! hold bytes on this PC is left;
//! clear the owner record before every item is gone; log a name, a path, an email or an id.

use std::path::Path;

use super::{CredentialStores, Engine, LocalState};
use crate::account_binding::{Identity, canonical_email};
use crate::config::DesktopConfig;
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
        let Some(previous) = owner.as_ref() else { return };
        match self.remove_the_previous_account(data, previous, sync_root, port) {
            Ok(()) => tracing::info!("account switch: the previous account's local data was removed"),
            Err(why) => tracing::warn!(why, "account switch: the previous account's local data stays"),
        }
    }

    fn remove_the_previous_account(
        &self,
        data: &LocalState,
        previous: &Identity,
        sync_root: &Path,
        port: &dyn SyncRootPort,
    ) -> Result<(), &'static str> {
        let db = &data.db;
        if let Some(kind) = db.work_waiting_for_a_person().map_err(|_| "unreadable")? {
            return Err(kind);
        }
        // A file of unknown content keeps B out only while its item in the folder may hold bytes (an edit of A's
        // nobody sent). With none on this PC there is nothing a person could lose: the row goes with the others.
        let guarded = db.guarded_paths().map_err(|_| "unreadable")?;
        if guarded.iter().any(|path| port.may_hold_bytes(sync_root, path)) {
            return Err("guarded file");
        }
        if db.has_account_data().map_err(|_| "unreadable")? {
            port.remove_synced_items(db, sync_root, &[]).map_err(|_| "the folder's items stay")?;
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
        self.forget_its_settings(previous)?;
        // The owner record goes in this transaction, last: until it commits, B is still refused.
        db.clear_account_data(false).map_err(|_| "the rows stay")
    }

    /// The account's entries in desktop.toml go with its data: its chosen backup folders and their switches, its
    /// selective-sync ids and the address the sign-in form prefills (see [`forget_in_settings`]). Before the owner
    /// record goes, so a settings file that cannot be written keeps the refusal like any other leftover.
    fn forget_its_settings(&self, previous: &Identity) -> Result<(), &'static str> {
        let path = DesktopConfig::path_for(self.identity).map_err(|_| "the settings stay")?;
        DesktopConfig::update_at(&path, |config| {
            let changed = forget_in_settings(config, previous);
            (changed, ())
        })
        .map_err(|_| "the settings stay")
    }
}

/// `previous`'s entries out of `config`: the chosen backup folders tagged with its id (and the `custom:<name>` switch
/// of each), the selective-sync ids (folder ids of its vault, meaningless for another account), and the prefilled
/// address when it is its own. Another account's entries and untagged ones stay. Whether anything changed.
fn forget_in_settings(config: &mut DesktopConfig, previous: &Identity) -> bool {
    let mut changed = false;
    let mut gone = Vec::new();
    config.backup_chosen_folders.retain(|folder| {
        let its = crate::app::bindings::backup_folders::is_owners(folder, Some(previous));
        if its {
            gone.push(format!("custom:{}", folder.name));
        }
        changed |= its;
        !its
    });
    let switches = config.known_folder_backup.len();
    config.known_folder_backup.retain(|key| !gone.contains(key));
    changed |= config.known_folder_backup.len() != switches;
    if config.excluded_folder_ids.take().is_some() {
        changed = true;
    }
    let prefilled_is_its = match (config.last_signed_in_email.as_deref(), previous.email.as_deref()) {
        (Some(prefilled), Some(email)) => canonical_email(prefilled) == canonical_email(email),
        _ => false,
    };
    if prefilled_is_its {
        config.last_signed_in_email = None;
        changed = true;
    }
    changed
}
