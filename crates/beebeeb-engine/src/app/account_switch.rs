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
//! [`NAMES_SHOWN`] names), so a Cancel changes nothing. [`Engine::discard_and_switch`] takes that ask back and, under
//! the start's locks: refuses when what is unsent is no longer what the person saw; keeps A's working key in A's own
//! key slot (R4: a switch forgets no account's key slot); copies every unsent file into the kept folder ("Changes not
//! sent", the 1887 keep step: never over a name) and records that folder durably; takes each original out of the sync
//! folder only once its copy holds the same bytes; then starts as any start does, so the clear above runs in 014's
//! order.

use std::path::{Path, PathBuf};

use super::{CredentialStores, Engine, LocalState, SyncStart, SyncStartRefused};
use crate::account_binding::{Identity, canonical_email};
use crate::config::DesktopConfig;
use crate::keychain::AuthSecretStore;
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
            // The purge forgot the owner record (a sign-out's R10 step); A's rows are still here, so it is put back:
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

/// How many names the confirmation shows; the rest are a count.
pub const NAMES_SHOWN: usize = 5;

/// What "Discard and switch" would lose, as the confirmation shows it: every unsent file of the previous account, by
/// count, and the names (never a path) of the first [`NAMES_SHOWN`]. The full set is held here, so the discard does
/// exactly what was confirmed or nothing. Made by [`Engine::discard_and_switch_ask`]; dropping it is the Cancel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscardAsk {
    /// How many files would be taken out of the sync folder (each kept as a copy first).
    pub files: usize,
    /// The names of the first of them, at most [`NAMES_SHOWN`].
    pub names: Vec<String>,
    paths: Vec<String>,
}

impl DiscardAsk {
    fn of(paths: Vec<String>) -> Self {
        let names = paths
            .iter()
            .take(NAMES_SHOWN)
            .map(|path| path.rsplit('/').find(|part| !part.is_empty()).unwrap_or(path).to_string())
            .collect();
        Self {
            files: paths.len(),
            names,
            paths,
        }
    }

    /// The files beyond the named ones ("and 3 more").
    pub fn more(&self) -> usize {
        self.files - self.names.len()
    }
}

/// Why "Discard and switch" did nothing. No variant carries a name, a path or an id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardRefused {
    /// No session, or nothing of another account's is unsent here (the plain start is the way on).
    NothingToDiscard,
    /// What is unsent now is not what the person confirmed: ask again.
    Changed,
    /// Something else of the previous account's needs a person's answer (a queued change, a held edit…).
    OtherWork,
    /// The previous account's key could not be kept in its own slot: nothing was copied or removed (R4).
    KeyNotKept,
    /// A copy could not be made or recorded: nothing was removed from the sync folder.
    KeepFailed,
    /// Sync runs: there is nothing to switch.
    SyncRunning,
    /// This OS cannot remove another account's synced items, so a switch cannot finish here: nothing is offered.
    Unsupported,
    /// The local state could not be read.
    Unreadable,
}

/// A discard that went through: where the copies are, and the start that followed (014's clear, then B's start).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscardDone {
    pub kept_in: PathBuf,
    pub kept: usize,
    pub start: Result<SyncStart, SyncStartRefused>,
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
