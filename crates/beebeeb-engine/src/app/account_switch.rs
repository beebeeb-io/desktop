//! w26-account-switch (v5 bug 6): account A signed out and left its local data bound to it (a sign-out keeps it), and
//! account B starts sync on the same PC. B must start clean and never see A's names, so before the binding of step 7
//! A's data is removed, when it provably is another account's and nothing of it needs a person's answer.
//!
//! Only two USER IDS that differ prove another account: an owner recorded by an email alone, and unowned data, are
//! task 2267's cases (the ownership proof, nothing deleted). Anything that cannot be removed leaves A's owner record in
//! place, so the binding refuses B as it always did. The order: the folder's items (through the port), the files the
//! rows point at, then the one transaction that clears every row and the owner record.
//!
//! Only after that transaction (dc-014): the account's entries in desktop.toml ([`forget_in_settings`]), its share
//! pins and guide mark (that account's alone, never another's) and its vault key, if B's sign-in left it. A clear
//! that fails leaves every one of them as it was, and B is refused with words.
//!
//! The folder is not locked between the check and the removal, so the removal is told which paths were found empty
//! (`remove_synced_items`' `unasked`) and deletes only what it can prove synced and unchanged when it reaches each
//! item; an item made since refuses the switch, nothing deleted (w28).
//!
//! Must not: remove anything while a queued change, a staged copy, a held edit or a file of unknown content that may
//! hold bytes on this PC is left;
//! clear the owner record before every item is gone; log a name, a path, an email or an id.
//!
//! dc-111, "Discard and switch": when such a file keeps B out ([`super::SyncStartRefused::UnsentFileOfAnotherAccount`]),
//! the person may let it go. [`Engine::discard_and_switch_ask`] only reads: what would be lost (a count and up to
//! five names, `account_binding::NAMES_SHOWN`), so a Cancel changes nothing. [`Engine::discard_and_switch`] takes
//! that ask back and, under
//! the start's locks: refuses when what is unsent is no longer what the person saw; keeps A's working key in A's own
//! key slot (R4: a switch forgets no account's key slot); copies every unsent file into the kept folder ("Changes not
//! sent", the 1887 keep step: never over a name) and records that folder durably; takes each original out of the sync
//! folder only once its copy holds the same bytes; then starts as any start does, so the clear above runs in 014's
//! order.
//!
//! dc-167, the account switcher (Guus 2026-11-21: a switch keeps the key, a sign-out forgets): every account signed
//! in on this PC keeps its session (token, owner, email, lock entries) in a parked slot of its own
//! ([`parked_slot_name`]) while another account is the current one, and its vault key in its R4 slot. A switch parks
//! the current session, moves the target's parked session into the install slot ([`AuthVault::bind_session_owner`]
//! then makes its kept key the working key) and restores it as a start would: no twelve words. The host then starts
//! sync, so step 7 clears the previous account's local data in 014's order, and an unsent file of it offers 111's
//! discard. Signing out of a parked account ends its server session and forgets its parked session and its key slot
//! only; the current account and every other one are left alone. The ids of parked accounts are listed in
//! [`PARKED_INDEX`] beside state.db (ids only: the emails stay in the credential store).

use std::path::{Path, PathBuf};

use super::{CredentialStores, Engine, LocalState};
use crate::account::SessionTransition;
use crate::account_binding::{DiscardAsk, DiscardDone, DiscardRefused, Identity, canonical_email};
use crate::config::DesktopConfig;
use crate::keychain::{AuthSecretStore, AuthVault, LockFlag};
use crate::signin::gate::GateState;
use crate::state_db::StateDb;
use crate::sync_root_port::{OsSyncRoots, SyncRootPort};

/// The reason a file that may hold unsent bytes keeps the switch out.
const GUARDED_FILE: &str = "guarded file";

impl<C: CredentialStores> Engine<C> {
    /// Step 7's part: see the module. A no-op unless `data` is recorded as another account's by user id. Best effort in
    /// the sense that a failure changes nothing the binding reads: the start then refuses as before. `true` when it
    /// stopped at a file that may hold bytes a person never sent (w29: the refusal then says what to do).
    pub(super) fn clear_the_previous_account(
        &self,
        data: &LocalState,
        session: &Identity,
        sync_root: &Path,
        port: &dyn SyncRootPort,
    ) -> bool {
        let owner = data.db.owner().ok().flatten();
        let other_account = match (owner.as_ref().and_then(|o| o.user_id.as_deref()), session.user_id.as_deref()) {
            (Some(recorded), Some(now)) => recorded != now,
            _ => false,
        };
        if !other_account {
            return false;
        }
        let Some(previous) = owner.as_ref() else { return false };
        match self.remove_the_previous_account(data, previous, sync_root, port) {
            Ok(()) => {
                tracing::info!("account switch: the previous account's local data was removed");
                self.wipe_thumb_cache_at_session_end(); // A's thumbnails (079); the cache is keyed by this install's slot.
                false
            }
            Err(why) => {
                tracing::warn!(why, "account switch: the previous account's local data stays");
                why == GUARDED_FILE
            }
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
        // w29: an item that is there and holds no bytes (a cloud placeholder nothing was fetched into) is a synced
        // item like any other: the removal deletes it after proving that, holding it. Only a path with no item at all
        // is `unasked`: whatever is found there later was made since and refuses the switch.
        let mut unasked = Vec::new();
        for path in &guarded {
            let nothing_there = matches!(
                sync_root.join(path.trim_start_matches('/')).symlink_metadata(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound
            );
            // An OS that cannot tell (the port's default says `true`) is believed only for an item that is there: a
            // path with no item cannot hold an unsent edit, so the refusal stays the plain `OtherAccountsData`.
            if port.may_hold_bytes(sync_root, path) && !nothing_there {
                return Err(GUARDED_FILE);
            }
            if nothing_there {
                unasked.push(path.clone());
            }
        }
        if db.has_account_data().map_err(|_| "unreadable")? {
            port.remove_synced_items(db, sync_root, &unasked).map_err(|_| "the folder's items stay")?;
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
        // The owner record goes in this transaction: until it commits, B is still refused, and A's settings, share
        // pins and key are all still here (dc-014), so A signs back in to exactly what it left.
        if db.clear_account_data(false).is_err() {
            // The purge forgot the owner record; A's rows are still here, so it is put back:
            // A signs in again to its own data, and B stays refused as before.
            if db.set_owner(previous).is_err() {
                tracing::warn!("account switch: the previous account's owner record could not be put back");
            }
            return Err("the rows stay");
        }
        // Forgotten last, only once A's data is gone. A failure here no longer keeps B out (the owner record is B's
        // to take now): what is left is A's alone, keyed by A's id, and B never reads it.
        if let Err(why) = self.forget_its_settings(previous) {
            tracing::warn!(why, "account switch: the previous account's settings stay");
        }
        if let Some(user_id) = previous.user_id.as_deref() {
            if db.forget_share_trust_rows(user_id).is_err() {
                tracing::warn!("account switch: the previous account's share marks stay");
            }
            self.forget_its_key(user_id);
        }
        Ok(())
    }

    /// The previous account's vault key, if the credential store still holds it: a token-only sign-in of B leaves a
    /// key tagged with A's id in place (`key_owner::remove_foreign_key`; the loader refuses it for B), so that a clear
    /// that fails leaves A able to sign back in without the twelve words. Removed only here, after A's data is gone,
    /// and only while its tag is still A's: B's own key (or one of unknown owner) is never touched.
    fn forget_its_key(&self, user_id: &str) {
        let mut vault = self.vault();
        let its = vault
            .vault_key_owner()
            .ok()
            .flatten()
            .is_some_and(|tag| crate::signin::key_owner::tag_is_account(&tag, user_id));
        if its && vault.delete_vault_key_and_owner().is_err() {
            tracing::warn!("account switch: the previous account's key stays (another sign-in removes it)");
        }
    }

    /// The account's entries in desktop.toml go with its data: its chosen backup folders and their switches, its
    /// selective-sync ids and the address the sign-in form prefills (see [`forget_in_settings`]). After the owner
    /// record went (dc-014), so a clear that fails leaves A's settings untouched.
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

impl<C: CredentialStores> Engine<C> {
    /// What "Discard and switch" would lose. Only reads: a Cancel (dropping the answer) leaves every row, item, key
    /// and setting as it was.
    pub fn discard_and_switch_ask(&self, sync_root: &Path) -> Result<DiscardAsk, DiscardRefused> {
        if !discard_offered_on_this_os() {
            return Err(DiscardRefused::Unsupported);
        }
        self.discard_ask_on(sync_root, &OsSyncRoots::new(self.identity))
    }

    /// See the module: the confirmed `ask`, kept, cleared and switched, or nothing changed.
    pub fn discard_and_switch(&self, ask: &DiscardAsk, sync_root: PathBuf) -> Result<DiscardDone, DiscardRefused> {
        if !discard_offered_on_this_os() {
            return Err(DiscardRefused::Unsupported);
        }
        self.discard_on(ask, sync_root, &OsSyncRoots::new(self.identity))
    }

    /// [`Engine::discard_and_switch_ask`] with the OS's answers from `port`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn discard_and_switch_ask_with(
        &self,
        sync_root: &Path,
        port: &dyn SyncRootPort,
    ) -> Result<DiscardAsk, DiscardRefused> {
        self.discard_ask_on(sync_root, port)
    }

    /// [`Engine::discard_and_switch`] with the OS's answers from `port`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn discard_and_switch_with(
        &self,
        ask: &DiscardAsk,
        sync_root: PathBuf,
        port: &dyn SyncRootPort,
    ) -> Result<DiscardDone, DiscardRefused> {
        self.discard_on(ask, sync_root, port)
    }

    fn discard_ask_on(&self, sync_root: &Path, port: &dyn SyncRootPort) -> Result<DiscardAsk, DiscardRefused> {
        let (db, _previous) = self.previous_accounts_data()?;
        let paths = unsent_paths(&db, sync_root, port)?;
        if paths.is_empty() {
            return Err(DiscardRefused::NothingToDiscard);
        }
        Ok(DiscardAsk::of(paths))
    }

    fn discard_on(
        &self,
        ask: &DiscardAsk,
        sync_root: PathBuf,
        port: &dyn SyncRootPort,
    ) -> Result<DiscardDone, DiscardRefused> {
        let (kept_in, kept) = self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            let slot = self.acct.engine.lock().await;
            if slot.as_ref().is_some_and(|runner| runner.is_running()) {
                return Err(DiscardRefused::SyncRunning);
            }
            let (db, previous) = self.previous_accounts_data()?;
            if unsent_paths(&db, &sync_root, port)? != ask.paths {
                return Err(DiscardRefused::Changed);
            }
            self.keep_its_key_in_its_slot(&previous)?;
            let root = self.kept_root().ok_or(DiscardRefused::KeepFailed)?;
            let sources: Vec<PathBuf> = ask.paths.iter().map(|path| item_at(&sync_root, path)).collect();
            let (kept_in, copies) = keep_copies(&root, &ask.paths, &sources).map_err(|why| {
                tracing::warn!(why, "discard and switch: nothing was removed");
                DiscardRefused::KeepFailed
            })?;
            self.record_kept_folder(&kept_in)?;
            // Every copy is proven before the first original goes: a file that changed since keeps everything here.
            for (source, copy) in sources.iter().zip(&copies) {
                if !same_bytes(source, copy).unwrap_or(false) {
                    return Err(DiscardRefused::Changed);
                }
            }
            for source in &sources {
                match std::fs::remove_file(source) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(_) => return Err(DiscardRefused::KeepFailed),
                }
            }
            Ok((kept_in, copies.len()))
        })?;
        tracing::info!(kept, "discard and switch: the unsent files were kept; the switch goes on");
        // The plain start, after the locks: step 7 now finds nothing unsent and clears in 014's order.
        let start = self.start_on(sync_root, port);
        Ok(DiscardDone { kept_in, kept, start })
    }

    /// The local data, when it is recorded as another account's by user id (the only proof the switch accepts).
    fn previous_accounts_data(&self) -> Result<(StateDb, Identity), DiscardRefused> {
        let identity = match crate::signin::start::keys_for_engine_start(&self.acct) {
            Ok((_keys, identity)) => self.with_recorded_user_id(identity),
            Err(_) => return Err(DiscardRefused::NothingToDiscard),
        };
        let path = crate::state_paths::state_db_path_from_state_dir(&self.config.state_dir);
        if !path.exists() {
            return Err(DiscardRefused::NothingToDiscard);
        }
        let db = StateDb::open(&path).map_err(|_| DiscardRefused::Unreadable)?;
        let owner = db.owner().map_err(|_| DiscardRefused::Unreadable)?;
        match owner {
            Some(previous)
                if previous.user_id.is_some()
                    && identity.user_id.is_some()
                    && previous.user_id != identity.user_id =>
            {
                Ok((db, previous))
            }
            _ => Err(DiscardRefused::NothingToDiscard),
        }
    }

    /// R4: a switch forgets no account's key slot. When the working key is the previous account's, it is first kept
    /// in that account's own slot (and read back), so 014's last step, which removes only the working copy, leaves A
    /// able to sign back in without the twelve words. Any other working key, and every slot, is left alone.
    fn keep_its_key_in_its_slot(&self, previous: &Identity) -> Result<(), DiscardRefused> {
        let Some(user_id) = previous.user_id.as_deref() else { return Ok(()) };
        let vault = self.vault();
        let its = vault
            .vault_key_owner()
            .map_err(|_| DiscardRefused::KeyNotKept)?
            .is_some_and(|tag| crate::signin::key_owner::tag_is_account(&tag, user_id));
        if !its || !vault.has_vault_key().unwrap_or(false) {
            return Ok(());
        }
        vault.bind_vault_key_owner(user_id).map_err(|_| DiscardRefused::KeyNotKept)?;
        let store = self.stores.for_account(self.identity, &self.config.account_id);
        match store.load_account_key(user_id) {
            Ok(Some(_)) => Ok(()),
            _ => Err(DiscardRefused::KeyNotKept),
        }
    }

    /// The 1887 kept folder ("Changes not sent"). A test keeps under its own state folder.
    fn kept_root(&self) -> Option<PathBuf> {
        #[cfg(test)]
        return Some(self.config.state_dir.join(crate::sign_out::kept_names::KEPT_FOLDER_NAME));
        #[cfg(not(test))]
        crate::sign_out::kept_names::kept_root_for_this_app(self.identity)
    }

    /// 1887 W9: the folder is recorded with a write that survives a power loss, before anything is removed.
    fn record_kept_folder(&self, kept_in: &Path) -> Result<(), DiscardRefused> {
        let path = DesktopConfig::path_for(self.identity).map_err(|_| DiscardRefused::KeepFailed)?;
        let folder = kept_in.to_string_lossy().into_owned();
        DesktopConfig::update_at_durable_with(
            &path,
            |config| {
                config.kept_unsent_folder = Some(folder);
                (true, ())
            },
            &crate::config::RealConfigFs,
        )
        .map_err(|_| DiscardRefused::KeepFailed)
    }
}

/// Only where the OS can remove another account's synced items (`SyncRootPort::remove_synced_items`) can a switch
/// finish: elsewhere taking the files out would change nothing about the refusal.
fn discard_offered_on_this_os() -> bool {
    cfg!(any(target_os = "windows", test, feature = "test-support"))
}

/// The previous account's files that may hold bytes it never sent, in the database's order: what keeps B out
/// (`remove_the_previous_account`'s guarded check). Other work of its refuses: that needs its account.
fn unsent_paths(db: &StateDb, sync_root: &Path, port: &dyn SyncRootPort) -> Result<Vec<String>, DiscardRefused> {
    if db.work_waiting_for_a_person().map_err(|_| DiscardRefused::Unreadable)?.is_some() {
        return Err(DiscardRefused::OtherWork);
    }
    let guarded = db.guarded_paths().map_err(|_| DiscardRefused::Unreadable)?;
    Ok(guarded
        .into_iter()
        .filter(|path| {
            let there = item_at(sync_root, path).symlink_metadata();
            there.is_ok_and(|item| item.is_file()) && port.may_hold_bytes(sync_root, path)
        })
        .collect())
}

fn item_at(sync_root: &Path, path: &str) -> PathBuf {
    sync_root.join(path.trim_start_matches('/'))
}

/// Whether two files hold the same bytes, read in pieces.
fn same_bytes(a: &Path, b: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let (mut a, mut b) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
    if a.metadata()?.len() != b.metadata()?.len() {
        return Ok(false);
    }
    let (mut x, mut y) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
        let n = a.read(&mut x)?;
        if n == 0 {
            return Ok(true);
        }
        b.read_exact(&mut y[..n])?;
        if x[..n] != y[..n] {
            return Ok(false);
        }
    }
}

/// The 1887 keep step (macOS: `KeptWriter`, an APFS clone, never over a name): one new event folder under `root`,
/// each file at its folder path inside it, every directory flushed. A failure removes the half-made folder.
#[cfg(target_os = "macos")]
fn keep_copies(root: &Path, paths: &[String], sources: &[PathBuf]) -> Result<(PathBuf, Vec<PathBuf>), &'static str> {
    use crate::sign_out::kept_names::{KeptRole, event_folder_name, kept_relative_path};
    use crate::sign_out::kept_writer::{KeptWriter, SystemClone};
    let clone = SystemClone;
    let mut writer = KeptWriter::create(root, &event_folder_name(chrono::Local::now()), &clone)
        .map_err(|_| "the kept folder could not be made")?;
    let mut copies = Vec::with_capacity(paths.len());
    for (path, source) in paths.iter().zip(sources) {
        match writer.keep(&kept_relative_path(Some(path), "unsent", KeptRole::Newest), source) {
            Ok(copy) => copies.push(copy),
            Err(_) => {
                writer.abandon();
                return Err("a copy could not be made");
            }
        }
    }
    let folder = writer.finish().map_err(|_| "the kept folder could not be flushed")?;
    Ok((folder.path, copies))
}

/// The same step where `KeptWriter` (macOS only) is not built: 1887's folder and names, every file made new
/// (`create_new`, `name (2)` when taken), flushed. Untested on Windows.
#[cfg(not(target_os = "macos"))]
fn keep_copies(root: &Path, paths: &[String], sources: &[PathBuf]) -> Result<(PathBuf, Vec<PathBuf>), &'static str> {
    use crate::sign_out::kept_names::{KeptRole, event_folder_name, kept_relative_path, with_suffix};
    use std::io::ErrorKind;
    std::fs::create_dir_all(root).map_err(|_| "the kept folder could not be made")?;
    let name = event_folder_name(chrono::Local::now());
    let mut n = 1;
    let event = loop {
        let candidate = root.join(if n == 1 { name.clone() } else { format!("{name} ({n})") });
        match std::fs::create_dir(&candidate) {
            Ok(()) => break candidate,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => n += 1,
            Err(_) => return Err("the kept folder could not be made"),
        }
    };
    let copy_one = |path: &str, source: &Path| -> std::io::Result<PathBuf> {
        let components = kept_relative_path(Some(path), "unsent", KeptRole::Newest);
        let (file, dirs) = components.split_last().expect("never empty");
        let directory = dirs.iter().fold(event.clone(), |at, part| at.join(part));
        std::fs::create_dir_all(&directory)?;
        let mut n = 1;
        loop {
            let name = if n == 1 { file.clone() } else { with_suffix(file, &format!(" ({n})")) };
            let destination = directory.join(name);
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&destination) {
                Ok(mut to) => {
                    let copied = std::fs::File::open(source)
                        .and_then(|mut from| std::io::copy(&mut from, &mut to))
                        .and_then(|_| to.sync_all());
                    if let Err(error) = copied {
                        drop(to);
                        let _ = std::fs::remove_file(&destination);
                        return Err(error);
                    }
                    return Ok(destination);
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => n += 1,
                Err(e) => return Err(e),
            }
        }
    };
    let mut copies = Vec::with_capacity(paths.len());
    for (path, source) in paths.iter().zip(sources) {
        match copy_one(path, source) {
            Ok(copy) => copies.push(copy),
            Err(_) => {
                let _ = std::fs::remove_dir_all(&event);
                return Err("a copy could not be made");
            }
        }
    }
    Ok((event, copies))
}

/// The file beside state.db that lists the parked accounts' server user ids, one per line.
pub const PARKED_INDEX: &str = "accounts-on-this-pc";

/// One account signed in on this PC, as the switcher lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountOnThisPc {
    pub user_id: String,
    /// As its sign-in stored it; `None` when the store has none.
    pub email: Option<String>,
    /// The account whose session is the install slot's now.
    pub current: bool,
}

/// Why a switch, an "Add another account" or a sign-out of a parked account did nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchRefused {
    /// No such account is signed in on this PC (or its parked session is gone).
    NotOnThisPc,
    /// An open document has edits no save has landed: save or discard first (as a sign-out refuses).
    UnsavedEdits,
    /// The engine's end was not confirmed; nothing was moved.
    EngineStillRunning,
    /// The current session has no server user id yet, so it has no slot to wait in.
    CurrentUnknown,
    /// The credential store refused; what was there stays.
    Store,
}

impl SwitchRefused {
    /// One line a person reads.
    pub fn words(self) -> &'static str {
        match self {
            Self::NotOnThisPc => "That account is no longer signed in on this PC. Sign in to it again.",
            Self::UnsavedEdits => "A document has changes that are not saved. Save or close it first.",
            Self::EngineStillRunning => "Beebeeb could not stop syncing in time. Try again in a moment.",
            Self::CurrentUnknown => "Beebeeb is still checking this account. Try again in a moment.",
            Self::Store => "Beebeeb could not reach this PC's credential store. Nothing was changed.",
        }
    }
}

/// The credential-store slot a parked session waits in. Server user ids only (the R4 slot name's rule): an id that is
/// not one has no slot.
pub fn parked_slot_name(user_id: &str) -> Option<String> {
    let kept = crate::app::credential_stores::account_key_slot_name(user_id).ok()?;
    Some(kept.replacen("account-key-", "parked-session-", 1))
}

impl<C: CredentialStores> Engine<C> {
    /// Every account signed in on this PC: the current one first (when there is one), then the parked ones in the
    /// order they were parked. Reads the credential store; no network.
    pub fn accounts_on_this_pc(&self) -> Vec<AccountOnThisPc> {
        let vault = self.vault();
        let current = vault
            .session_token()
            .ok()
            .flatten()
            .and_then(|_| vault.session_owner().ok().flatten().or_else(|| self.user_id()));
        let mut out = Vec::new();
        if let Some(user_id) = current.clone() {
            let email = vault.account_email().ok().flatten().or_else(|| self.account_email());
            out.push(AccountOnThisPc { user_id, email, current: true });
        }
        for user_id in self.parked_ids() {
            if current.as_deref() == Some(user_id.as_str()) {
                continue;
            }
            let Some(store) = self.parked_store(&user_id) else { continue };
            if !matches!(store.load_session_token(), Ok(Some(_))) {
                continue;
            }
            let email = store.load_account_email().ok().flatten();
            out.push(AccountOnThisPc { user_id, email, current: false });
        }
        out
    }

    /// "Add another account": the current session waits in its slot (its key in its R4 slot), and the window shows
    /// the sign-in. Nothing of the current account is forgotten.
    pub fn add_another_account(&self) -> Result<(), SwitchRefused> {
        let result = self.runtime.block_on(async {
            let _step = self.session_ending_step().await;
            self.leave_current_session().await
        });
        self.publish();
        result
    }

    /// Switch to `user_id`, an account signed in on this PC: the current session is parked, the target's comes back
    /// with its own key (no twelve words). The gate the window shows next. The caller starts sync afterwards (the
    /// start clears the previous account's local data; an unsent file of it is offered "Discard and switch").
    pub fn switch_account(&self, user_id: &str) -> Result<GateState, SwitchRefused> {
        let target = self.parked_store(user_id).ok_or(SwitchRefused::NotOnThisPc)?;
        if !matches!(target.load_session_token(), Ok(Some(_))) {
            return Err(SwitchRefused::NotOnThisPc);
        }
        let current = self.vault().session_owner().ok().flatten();
        if current.as_deref() == Some(user_id.trim()) {
            return Ok(self.gate());
        }
        let moved = self.runtime.block_on(async {
            let _step = self.session_ending_step().await;
            self.leave_current_session().await?;
            self.take_parked_session(user_id)
        });
        if let Err(why) = moved {
            self.publish();
            return Err(why);
        }
        tracing::info!("account switch: another account signed in on this PC is the current one");
        Ok(self.restore())
    }

    /// Sign out of `user_id` while it is parked: its server session ends (best effort, owed until it does), and its
    /// parked session and its key slot are forgotten. The current account and every other one stay as they are.
    /// The current account signs out through the plain sign-out.
    pub fn sign_out_parked_account(&self, user_id: &str) -> Result<(), SwitchRefused> {
        let store = self.parked_store(user_id).ok_or(SwitchRefused::NotOnThisPc)?;
        let token = store.load_session_token().map_err(|_| SwitchRefused::Store)?;
        let cleared = clear_parked(&store);
        let mut vault = self.vault();
        let forgotten = vault.forget_account_key(user_id.trim());
        if cleared.is_err() || forgotten.is_err() {
            return Err(SwitchRefused::Store);
        }
        self.unlist_parked(user_id.trim());
        if let Some(token) = token {
            // Recorded as owed before this returns; the server is asked by a task of its own (task 1886).
            self.runtime.block_on(async {
                super::session_revoke::revoke_detached(
                    self.client.clone(),
                    self.config.api_base.clone(),
                    token.expose_for_request().to_string(),
                    self.owed.clone(),
                );
            });
        }
        tracing::info!("account switch: an account signed in on this PC was signed out");
        self.publish();
        Ok(())
    }

    /// The current session out of the install slot and memory, kept whole in its parked slot; its key kept in its R4
    /// slot. Under the session step. A refusal leaves the install slot as it was.
    async fn leave_current_session(&self) -> Result<(), SwitchRefused> {
        if self.office_unsaved_count() > 0 {
            return Err(SwitchRefused::UnsavedEdits);
        }
        let mut vault = self.vault();
        let token = vault.session_token().map_err(|_| SwitchRefused::Store)?;
        let Some(token) = token else { return Ok(()) };
        let user_id = vault
            .session_owner()
            .map_err(|_| SwitchRefused::Store)?
            .or_else(|| self.user_id())
            .ok_or(SwitchRefused::CurrentUnknown)?;
        let parked = self.parked_store(&user_id).ok_or(SwitchRefused::CurrentUnknown)?;
        let _ = self.stop_proofs().await;
        if super::unfinished_sign_out::a_task_may_hold_the_keys(&self.acct, self.stop_holding_step().await) {
            return Err(SwitchRefused::EngineStillRunning);
        }
        keep_key_in_its_slot(&vault, &user_id)?;
        park(&vault, &parked, &token, &user_id).inspect_err(|_| {
            let _ = clear_parked(&parked);
        })?;
        self.list_parked(&user_id).map_err(|_| SwitchRefused::Store)?;
        // What is the session's in memory and on disk goes as a sign-out's does; its key, slots and server session stay.
        self.office_before_session_end(super::office_session_end::SessionEnding::SignOut);
        self.wipe_office_recovery_at_session_end();
        self.end_office_scratch_at_session_end();
        self.wipe_thumb_cache_at_session_end();
        self.forget_session(SessionTransition::SignOut);
        let cleared = vault
            .clear_session_token()
            .and_then(|()| self.stores.for_account(self.identity, &self.config.account_id).delete_account_email())
            .and_then(|()| vault.clear_lock_entries());
        if let Ok(mut email) = self.acct.auth_email.lock() {
            *email = None;
        }
        if let Ok(mut profile) = self.acct.cached_profile.lock() {
            *profile = None;
        }
        self.remember_session_user_id(None);
        self.acct.auth_health.note_result(None);
        self.set_auth_present(false);
        cleared.map_err(|_| SwitchRefused::Store)
    }

    /// `user_id`'s parked session into the install slot: token, email, lock entries, then its owner, which makes its
    /// kept key the working key (R4). The parked copy goes only once the install slot holds it.
    fn take_parked_session(&self, user_id: &str) -> Result<(), SwitchRefused> {
        let user_id = user_id.trim();
        let parked = self.parked_store(user_id).ok_or(SwitchRefused::NotOnThisPc)?;
        let token = parked
            .load_session_token()
            .map_err(|_| SwitchRefused::Store)?
            .ok_or(SwitchRefused::NotOnThisPc)?;
        let vault = self.vault();
        vault.install_session(token.clone()).map_err(|_| SwitchRefused::Store)?;
        if let Some(email) = parked.load_account_email().ok().flatten() {
            vault.store_account_email(&email).map_err(|_| SwitchRefused::Store)?;
        }
        vault.clear_lock_entries().map_err(|_| SwitchRefused::Store)?;
        for flag in LockFlag::ALL {
            if let Some(bytes) = parked.load_lock_flag(flag).map_err(|_| SwitchRefused::Store)? {
                vault.set_lock_entry(flag, &bytes).map_err(|_| SwitchRefused::Store)?;
            }
        }
        vault.bind_session_owner(user_id).map_err(|_| SwitchRefused::Store)?;
        if !matches!(vault.session_token(), Ok(Some(stored)) if stored == token) {
            return Err(SwitchRefused::Store);
        }
        if clear_parked(&parked).is_err() {
            tracing::warn!("account switch: a parked session copy stays (the next switch writes over it)");
        }
        self.unlist_parked(user_id);
        Ok(())
    }

    fn parked_store(&self, user_id: &str) -> Option<C::Store> {
        Some(self.stores.for_account(self.identity, &parked_slot_name(user_id)?))
    }

    fn parked_index(&self) -> PathBuf {
        self.config.state_dir.join(PARKED_INDEX)
    }

    fn parked_ids(&self) -> Vec<String> {
        std::fs::read_to_string(self.parked_index())
            .map(|text| text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
            .unwrap_or_default()
    }

    fn write_parked_ids(&self, ids: &[String]) -> std::io::Result<()> {
        let path = self.parked_index();
        let temporary = path.with_extension("new");
        let mut text = ids.join("\n");
        text.push('\n');
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&temporary)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
        }
        std::fs::rename(&temporary, &path)
    }

    fn list_parked(&self, user_id: &str) -> std::io::Result<()> {
        let mut ids = self.parked_ids();
        if ids.iter().any(|id| id == user_id) {
            return Ok(());
        }
        ids.push(user_id.to_string());
        self.write_parked_ids(&ids)
    }

    fn unlist_parked(&self, user_id: &str) {
        let mut ids = self.parked_ids();
        let before = ids.len();
        ids.retain(|id| id != user_id);
        if ids.len() != before && self.write_parked_ids(&ids).is_err() {
            tracing::warn!("account switch: the list of accounts on this PC was not updated");
        }
    }
}

/// R4: the working key, when it is `user_id`'s, is in that account's slot (written and read back) before its session
/// leaves the install slot. A working key of another or unknown owner, or none (a closed vault never read it), is left
/// alone: that account then unlocks as it did before.
fn keep_key_in_its_slot<S: AuthSecretStore>(vault: &AuthVault<S>, user_id: &str) -> Result<(), SwitchRefused> {
    let its = vault
        .vault_key_owner()
        .map_err(|_| SwitchRefused::Store)?
        .is_some_and(|tag| crate::signin::key_owner::tag_is_account(&tag, user_id));
    if !its || !vault.has_vault_key().unwrap_or(false) {
        return Ok(());
    }
    vault.bind_vault_key_owner(user_id).map_err(|_| SwitchRefused::Store)?;
    Ok(())
}

/// The install slot's session into `parked`, read back.
fn park<S: AuthSecretStore, P: AuthSecretStore>(
    vault: &AuthVault<S>,
    parked: &P,
    token: &crate::keychain::SessionToken,
    user_id: &str,
) -> Result<(), SwitchRefused> {
    let store = |r: crate::keychain::AuthResult<()>| r.map_err(|_| SwitchRefused::Store);
    store(parked.save_session_token(token))?;
    store(parked.save_session_owner(user_id))?;
    if let Some(email) = vault.account_email().map_err(|_| SwitchRefused::Store)? {
        store(parked.save_account_email(&email))?;
    }
    for flag in LockFlag::ALL {
        match vault.lock_entry(flag).map_err(|_| SwitchRefused::Store)? {
            Some(bytes) => store(parked.save_lock_flag(flag, &bytes))?,
            None => store(parked.delete_lock_flag(flag))?,
        }
    }
    match parked.load_session_token() {
        Ok(Some(kept)) if &kept == token => Ok(()),
        _ => Err(SwitchRefused::Store),
    }
}

/// Every item of a parked slot; every removal is tried.
fn clear_parked<P: AuthSecretStore>(parked: &P) -> crate::keychain::AuthResult<()> {
    #[allow(clippy::manual_try_fold)] // every removal must be tried, so the first error must not stop the loop
    let flags = LockFlag::ALL
        .into_iter()
        .fold(Ok(()), |done, flag| done.and(parked.delete_lock_flag(flag)));
    parked
        .delete_session_token()
        .and(parked.delete_session_owner())
        .and(parked.delete_account_email())
        .and(flags)
}

#[cfg(test)]
#[path = "account_switch_switcher_tests.rs"]
mod switcher_tests;

#[cfg(test)]
mod discard_ask_tests {
    use super::*;

    #[test]
    fn the_confirmation_names_at_most_five_and_counts_every_file() {
        let paths: Vec<String> = (1..=7).map(|n| format!("/dir/sub/file {n}.txt")).collect();
        let ask = DiscardAsk::of(paths);
        assert_eq!(ask.files, 7);
        assert_eq!(ask.names, (1..=5).map(|n| format!("file {n}.txt")).collect::<Vec<_>>());
        assert_eq!(ask.more(), 2);
        let few = DiscardAsk::of(vec!["a.txt".into(), "b/c.txt".into()]);
        assert_eq!((few.files, few.names.clone(), few.more()), (2, vec!["a.txt".into(), "c.txt".into()], 0));
    }
}
