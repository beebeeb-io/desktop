//! The bridge between the engine crate and the native shell's trait system.
//!
//! [`open::open`] is the start path: it makes the one engine handle of this process, for the identity this
//! binary was built as (`shell::identity::engine_identity`, the only place the native app asks for it).
//! `LiveEngine` wraps that handle and implements `EngineActions`. `EngineSignInSource` implements
//! `SignInSource` by spawning worker threads for each blocking engine call and delivering results through a
//! channel. `EngineDriveSource` implements `DriveSource` on the handle's local state, with the engine's own
//! view models. Every type is generic over the credential stores, so a test runs the same code on stores in
//! memory; the app uses the platform's.
//!
//! No per-app name is spelled here: the sync folder, the settings file and the state folder come from the
//! identity the handle was opened with.
//!
//! Secrets: the password, codes and recovery phrase arrive as `&str` from a secure `TextEdit`
//! (wiped on drop) and go into the engine call as `Zeroizing<String>`, then nowhere else: not
//! into the element tree, not into a label, not into a log line, not into the feedback mode's
//! capture. The engine events that cross to the UI use the redacting `Debug`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use zeroize::Zeroizing;

use beebeeb_engine::app::{CredentialStores, Engine, SyncStart};
use beebeeb_engine::drive_v2::DriveError;
use beebeeb_engine::drive_v2::model::{self, Sort as EngineSort};
use beebeeb_engine::file_ops;
use beebeeb_engine::host::HostMessage;
use beebeeb_engine::identity::EngineIdentity;
use beebeeb_engine::signin::gate::{
    GateState as EngineGate, NextAfterSignIn as EngineNext, SignInProblem as EngineProblem,
};

use crate::frames::files::source::{
    self as files_source, Account, ActionError, Change, Counts, Details, FilesSource, StorageSplit,
};
use crate::frames::signin::source::{
    CodeKind, GateState, HelloOutcome, NextAfterSignIn, PhraseProblem, SharedSession, SignInEvent, SignInProblem,
    SignInSource, TwoFactorProblem,
};
use crate::shell::actions::{self, EngineActions};
use crate::shell::app::{GateState as ShellGate, Session};
use crate::shell::instance::ProfileHolder;
use crate::shell::tray::{EngineState, TrayInputs};
use crate::source::{DriveItem, DriveSource, Listing, SourceError, StorageSummary};

pub mod backups;
pub mod open;
#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// Gate / type mapping (one place, never a name or a path)
// ---------------------------------------------------------------------------

fn map_gate_shell(g: EngineGate) -> ShellGate {
    match g {
        EngineGate::Ready => ShellGate::Ready,
        EngineGate::SignedOut => ShellGate::SignedOut,
        EngineGate::Locked => ShellGate::Locked,
        EngineGate::NeedsRecoveryPhrase => ShellGate::NeedsRecoveryPhrase,
        EngineGate::NoPlan => ShellGate::NoPlan,
        EngineGate::KeyStoreUnavailable => ShellGate::KeyStoreUnavailable,
    }
}

fn map_gate_signin(g: EngineGate) -> GateState {
    match g {
        EngineGate::Ready => GateState::Ready,
        EngineGate::SignedOut => GateState::SignedOut,
        EngineGate::Locked => GateState::Locked,
        EngineGate::NeedsRecoveryPhrase => GateState::NeedsRecoveryPhrase,
        EngineGate::NoPlan => GateState::NoPlan,
        EngineGate::KeyStoreUnavailable => GateState::KeyStoreUnavailable,
    }
}

fn map_next(n: EngineNext) -> NextAfterSignIn {
    match n {
        EngineNext::Drive => NextAfterSignIn::Drive,
        EngineNext::RecoveryPhrase => NextAfterSignIn::RecoveryPhrase,
        EngineNext::TwoFactor => NextAfterSignIn::TwoFactor,
        EngineNext::Reauthenticated => NextAfterSignIn::Reauthenticated,
        EngineNext::AccountMismatch => NextAfterSignIn::AccountMismatch,
    }
}

fn map_problem(p: EngineProblem) -> SignInProblem {
    match p {
        EngineProblem::Invalid => SignInProblem::Invalid,
        EngineProblem::LockedHint => SignInProblem::LockedHint,
        EngineProblem::Offline => SignInProblem::Offline,
        EngineProblem::RateLimited { seconds } => SignInProblem::RateLimited { seconds },
        EngineProblem::StaleKey => SignInProblem::StaleKey,
        EngineProblem::Other => SignInProblem::Other,
    }
}

/// The engine answers a failed second factor with one of its own sentences (`gate::two_factor_error`); the
/// screen wants the kind. By the exact sentence: an answer this does not know is `Other`, never "wrong code".
fn two_factor_problem(words: &str) -> TwoFactorProblem {
    use beebeeb_engine::signin::gate;
    if words == gate::WRONG_2FA_CODE {
        TwoFactorProblem::WrongCode
    } else if words == gate::TWO_FACTOR_EXPIRED {
        TwoFactorProblem::Expired
    } else if words == gate::two_factor_error("offline") {
        TwoFactorProblem::Offline
    } else if words == gate::two_factor_error("rate_limited") {
        TwoFactorProblem::RateLimited
    } else {
        TwoFactorProblem::Other
    }
}

/// The same for the twelve words (`gate::phrase_error`). "The phrase is right, sync did not start" is not a
/// wrong phrase.
fn phrase_problem(words: &str) -> PhraseProblem {
    use beebeeb_engine::signin::gate;
    if words == gate::WRONG_PHRASE {
        PhraseProblem::WrongPhrase { bad_word: None }
    } else if words == gate::TOO_MANY_PHRASE_TRIES {
        PhraseProblem::TooManyTries
    } else if words == gate::phrase_error("offline") {
        PhraseProblem::Offline
    } else {
        PhraseProblem::Other
    }
}

fn log(message: impl AsRef<str>) {
    eprintln!("beebeeb engine_host: {}", message.as_ref());
}

// ---------------------------------------------------------------------------
// OS integration: open a file or folder, reveal in Explorer/Finder
// ---------------------------------------------------------------------------

/// Open a file or folder with the OS's default handler.
fn open_path(path: &std::path::Path) -> Result<(), files_source::ActionError> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .spawn()
            .map_err(|_| files_source::ActionError::Failed)?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|_| files_source::ActionError::Failed)?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|_| files_source::ActionError::Failed)?;
    }
    Ok(())
}

/// Reveal a file or folder in File Explorer / Finder / Files.
fn reveal_in_file_explorer(path: &std::path::Path) -> Result<(), files_source::ActionError> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.to_string_lossy()))
            .spawn()
            .map_err(|_| files_source::ActionError::Failed)?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path.to_string_lossy()])
            .spawn()
            .map_err(|_| files_source::ActionError::Failed)?;
    }
    #[cfg(target_os = "linux")]
    {
        // xdg-open on the parent folder.
        if let Some(parent) = path.parent() {
            std::process::Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(|_| files_source::ActionError::Failed)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// LiveEngine: EngineActions on the real engine
// ---------------------------------------------------------------------------

/// The real engine behind the shell's `EngineActions`.
pub struct LiveEngine<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    /// The identity the handle was opened with: the settings file and the suggested sync folder are its.
    identity: &'static EngineIdentity,
    /// What the engine reported since the last look. Drained on every refresh, so the queue never grows; the
    /// shell reads the state it needs from the handle itself.
    messages: Mutex<mpsc::Receiver<HostMessage>>,
}

impl<C: CredentialStores> LiveEngine<C> {
    pub fn new(opened: &open::Opened<C>, messages: mpsc::Receiver<HostMessage>) -> Self {
        Self {
            engine: opened.engine.clone(),
            identity: opened.identity,
            messages: Mutex::new(messages),
        }
    }

    /// Takes everything the engine queued. Returns how many messages there were (a test reads it).
    pub fn drain(&self) -> usize {
        let messages = self.messages.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        messages.try_iter().count()
    }

    /// Where this app syncs: the folder the settings name, else the identity's own suggestion (never another
    /// app's folder: the engine refuses a folder registered to another provider, `sync_root_owner`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn sync_root(&self) -> Result<PathBuf, String> {
        let config = beebeeb_engine::config::DesktopConfig::load_for(self.identity)?;
        Ok(config
            .sync_root
            .unwrap_or_else(|| beebeeb_engine::config::default_sync_root_suggestion(self.identity)))
    }
}

/// Start sync into the folder the settings name, else the identity's suggestion. Blocks.
fn start_sync_now<C: CredentialStores>(
    engine: &Engine<C>,
    identity: &'static EngineIdentity,
) -> Result<SyncStart, String> {
    let config = beebeeb_engine::config::DesktopConfig::load_for(identity)?;
    let sync_root = config
        .sync_root
        .unwrap_or_else(|| beebeeb_engine::config::default_sync_root_suggestion(identity));
    engine.start_sync(sync_root)
}

impl<C: CredentialStores> EngineActions for LiveEngine<C> {
    fn session(&self) -> Session {
        self.drain();
        let gate = map_gate_shell(self.engine.gate());
        // Whose session: the email is known from the stored session at start, the user id only once the
        // server named it in this run. One of them, always the same one for one account in one run.
        let account = self
            .engine
            .account_email()
            .or_else(|| self.engine.user_id())
            .unwrap_or_default();
        Session::of(gate, account)
    }

    fn tray_inputs(&self) -> TrayInputs {
        let gate = self.engine.gate();
        let signed_in = gate != EngineGate::SignedOut;
        let paused = self.engine.is_paused();
        let syncing = self.engine.is_syncing();
        TrayInputs {
            signed_in,
            locked: gate == EngineGate::Locked,
            paused,
            explorer_ready: false, // Cloud Files registration not wired
            engine: if syncing && !paused {
                EngineState::Syncing
            } else {
                EngineState::Idle
            },
            ..TrayInputs::default()
        }
    }

    fn pause_sync(&self) -> Result<(), actions::ActionError> {
        self.engine.pause().map_err(actions::ActionError::Failed)
    }

    fn resume_sync(&self) -> Result<(), actions::ActionError> {
        self.engine.resume().map_err(actions::ActionError::Failed)
    }

    fn open_in_file_explorer(&self) -> Result<(), actions::ActionError> {
        Err(actions::ActionError::NotBuilt(
            "open in File Explorer from the native app",
        ))
    }

    fn profile_holder(&self) -> Option<ProfileHolder> {
        None
    }

    /// Asks the engine to start, on a worker thread: the engine's start blocks (it binds the local data and,
    /// on Windows, registers the folder) and this is called from the window's thread. `Ok` means "asked"; the
    /// outcome is logged, and the session and the b show whether sync runs. Without a session nothing is asked.
    fn start_sync(&self) -> Result<(), actions::ActionError> {
        if self.engine.gate() != EngineGate::Ready {
            return Err(actions::ActionError::NotAvailable("not signed in and unlocked"));
        }
        let (engine, identity) = (self.engine.clone(), self.identity);
        thread::Builder::new()
            .name("start-sync".into())
            .spawn(move || match start_sync_now(&engine, identity) {
                Ok(SyncStart::Started) => log("sync started"),
                Ok(SyncStart::AlreadyRunning) => {}
                Ok(SyncStart::NoSession) => log("sync was not started: no session"),
                // The engine's sentence names a folder by its name at most, never a path, a key or a token.
                Err(error) => log(format!("sync was not started: {error}")),
            })
            .map(|_| ())
            .map_err(|_| actions::ActionError::Failed("could not start a thread".into()))
    }

    fn stop_for_quit(&self) {
        let _ = self.engine.stop();
    }

    fn resume_after_cancelled_quit(&self) {
        log("NotWired(resume_after_cancelled_quit)");
    }
}

// ---------------------------------------------------------------------------
// EngineSignInSource: SignInSource on the real engine
// ---------------------------------------------------------------------------

/// `SignInSource` backed by the real engine. Each `begin_*` spawns a named thread that calls the
/// blocking engine method and sends the result through a channel; `poll` drains it. Secrets go
/// from the `&str` argument into `Zeroizing<String>` for the thread and nowhere else.
pub struct EngineSignInSource<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    tx: mpsc::Sender<SignInEvent>,
    rx: mpsc::Receiver<SignInEvent>,
}

impl<C: CredentialStores> EngineSignInSource<C> {
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { engine, tx, rx }
    }
}

impl<C: CredentialStores> SignInSource for EngineSignInSource<C> {
    fn gate(&self) -> GateState {
        map_gate_signin(self.engine.gate())
    }

    fn shared_session(&self) -> Option<SharedSession> {
        None // Windows Hello / shared session not wired
    }

    fn begin_sign_in(&mut self, email: &str, password: &str) {
        let engine = self.engine.clone();
        let email = email.to_string();
        let password = Zeroizing::new(password.to_string());
        let tx = self.tx.clone();
        thread::Builder::new()
            .name("signin".into())
            .spawn(move || {
                let result = engine.sign_in_password(&email, password);
                let event = match result {
                    Ok(next) => SignInEvent::SignedIn(Ok(map_next(next))),
                    Err(problem) => SignInEvent::SignedIn(Err(map_problem(problem))),
                };
                let _ = tx.send(event);
            })
            .expect("spawn sign-in thread");
    }

    fn begin_hello(&mut self) {
        log("NotWired(begin_hello)");
        let _ = self.tx.send(SignInEvent::Hello(HelloOutcome::Cancelled));
    }

    fn cancel_hello(&mut self) {
        log("NotWired(cancel_hello)");
    }

    fn begin_device_request(&mut self) {
        log("NotWired(begin_device_request)");
    }

    fn cancel_device_request(&mut self) {
        log("NotWired(cancel_device_request)");
    }

    fn begin_two_factor(&mut self, code: &str, _kind: CodeKind) {
        let engine = self.engine.clone();
        let code = Zeroizing::new(code.to_string());
        let tx = self.tx.clone();
        thread::Builder::new()
            .name("2fa".into())
            .spawn(move || {
                let result = engine.two_factor(code);
                let event = match result {
                    Ok(next) => SignInEvent::TwoFactor(Ok(map_next(next))),
                    Err(words) => SignInEvent::TwoFactor(Err(two_factor_problem(words))),
                };
                let _ = tx.send(event);
            })
            .expect("spawn 2fa thread");
    }

    fn begin_restore(&mut self, words: &[&str]) {
        let engine = self.engine.clone();
        let phrase = Zeroizing::new(words.join(" "));
        let tx = self.tx.clone();
        thread::Builder::new()
            .name("restore".into())
            .spawn(move || {
                let result = engine.unlock_with_phrase(phrase);
                let event = match result {
                    Ok(()) => SignInEvent::PhraseRestored(Ok(NextAfterSignIn::Drive)),
                    Err(words) => SignInEvent::PhraseRestored(Err(phrase_problem(words))),
                };
                let _ = tx.send(event);
            })
            .expect("spawn restore thread");
    }

    fn begin_passkey(&mut self) {
        log("NotWired(begin_passkey)");
    }

    fn begin_unlock(&mut self, _password: &str) {
        log("NotWired(begin_unlock)");
    }

    fn begin_recovery_verify(&mut self, _email: &str, _words: &[&str]) {
        log("NotWired(begin_recovery_verify)");
    }

    fn begin_set_password(&mut self, _password: &str) {
        log("NotWired(begin_set_password)");
    }

    fn poll(&mut self) -> Option<SignInEvent> {
        self.rx.try_recv().ok()
    }
}

// ---------------------------------------------------------------------------
// EngineDriveSource: DriveSource on the handle's local state
// ---------------------------------------------------------------------------

/// The handle's local state as a `DriveSource`: the same view models (`drive_v2::model`) and the same row
/// mapping as the read-only `source_engine::EngineSource`. Names in a synced `state.db` are the decrypted ones
/// the engine stored; nothing here logs them.
pub struct EngineDriveSource<C: CredentialStores> {
    engine: Arc<Engine<C>>,
}

impl<C: CredentialStores> EngineDriveSource<C> {
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        Self { engine }
    }
}

impl<C: CredentialStores> DriveSource for EngineDriveSource<C> {
    /// The whole folder, name ascending, with its crumbs from the root. Before the first sync there is no local
    /// state: that, a folder that is gone and a database that cannot be read are errors, never an empty folder.
    fn list(&self, folder_id: Option<&str>) -> Result<Listing, SourceError> {
        let db = self.engine.state_db().map_err(|_| SourceError::Unreadable)?;
        let read = || -> Result<Listing, DriveError> {
            let (rows, _total) = model::children_of(&db, folder_id, EngineSort::default(), 0, usize::MAX)?;
            let folder = model::resolve_folder(&db, folder_id)?;
            let path: Vec<_> = model::crumbs(&db, folder.as_ref())?
                .into_iter()
                .map(crate::source_engine::crumb)
                .collect();
            let folder = path.last().cloned().ok_or(DriveError::NotFound)?;
            Ok(Listing {
                folder,
                path,
                items: rows.into_iter().map(crate::source_engine::item).collect(),
            })
        };
        read().map_err(|error| match error {
            DriveError::NotFound => SourceError::NotFound,
            _ => SourceError::Unreadable,
        })
    }

    fn storage(&self) -> Option<StorageSummary> {
        self.engine.storage_info().ok().map(|(used, quota)| StorageSummary {
            used_bytes: used,
            quota_bytes: quota,
        })
    }
}

// ---------------------------------------------------------------------------
// EngineFilesSource: FilesSource (most actions not wired)
// ---------------------------------------------------------------------------

/// `FilesSource` on the real engine. Actions call through the `Engine` handle; Cloud Files actions
/// (`keep_offline`, `free_up_space`) stay `not_wired` (they need a registered sync root).
pub struct EngineFilesSource<C: CredentialStores> {
    engine: Arc<Engine<C>>,
}

impl<C: CredentialStores> EngineFilesSource<C> {
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        Self { engine }
    }
}

fn not_wired(what: &str) -> ActionError {
    log(format!("NotWired({what})"));
    ActionError::Unavailable
}

/// Map a `DriveError` to the UI's `ActionError`.
fn map_drive_err(e: DriveError) -> ActionError {
    match e {
        DriveError::ReadOnly => ActionError::Protected,
        DriveError::NameTaken | DriveError::InvalidName => ActionError::NameTaken,
        DriveError::NotFound => ActionError::Failed,
        DriveError::Conflict => ActionError::Failed,
        _ => ActionError::Failed,
    }
}

impl<C: CredentialStores> EngineFilesSource<C> {
    /// The sync root and state database, both required for any local file operation.
    fn local(&self) -> Result<(beebeeb_engine::state_db::StateDb, PathBuf), ActionError> {
        let sync_root = self.engine.sync_root().map_err(|_| ActionError::Unavailable)?;
        let db = self.engine.state_db().map_err(|_| ActionError::Unavailable)?;
        Ok((db, sync_root))
    }
}

impl<C: CredentialStores> FilesSource for EngineFilesSource<C> {
    fn counts(&self) -> Option<Counts> {
        None
    }

    fn account(&self) -> Option<Account> {
        let email = self.engine.account_email()?;
        let name = email.split('@').next().unwrap_or(&email).to_string();
        let first = name.chars().next().unwrap_or('?').to_uppercase().to_string();
        Some(Account {
            name,
            email,
            initials: first,
        })
    }

    fn storage_split(&self) -> Option<StorageSplit> {
        None
    }

    fn sync_state(&self) -> Option<String> {
        if self.engine.is_paused() {
            Some("Paused".into())
        } else if self.engine.is_syncing() {
            Some("Syncing".into())
        } else {
            Some("Up to date".into())
        }
    }

    fn details(&self, item: &DriveItem, folder: &str) -> Details {
        files_source::details_of_row(item, folder)
    }

    fn changed_label(&self, _: &DriveItem) -> Option<String> {
        None
    }

    fn can_add(&self, _folder: Option<&str>) -> bool {
        // Writable when there is a sync root and a state database.
        self.local().is_ok()
    }

    fn sort_of(&self, _: Option<&str>) -> Option<crate::frames::files::Sort> {
        None
    }

    fn set_sort(&mut self, _: Option<&str>, _: crate::frames::files::Sort) {
        log("NotWired(set_sort)");
    }

    fn trash(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        let (db, _) = self.local()?;
        // Validate every item first.
        for item in items {
            let entry = file_ops::resolve(&db, &item.id).map_err(map_drive_err)?;
            file_ops::check_trashable(&db, &entry).map_err(map_drive_err)?;
        }
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        self.engine.trash_items(&ids).map_err(|_| ActionError::Failed)
    }

    fn restore(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        self.engine.restore_items(&ids).map_err(|_| ActionError::Failed)
    }

    fn duplicate(&mut self, items: &[DriveItem], _taken: &[&str]) -> Result<(), ActionError> {
        let (db, sync_root) = self.local()?;
        for item in items {
            let entry = file_ops::resolve(&db, &item.id).map_err(map_drive_err)?;
            file_ops::duplicate(&db, &sync_root, &entry).map_err(map_drive_err)?;
        }
        Ok(())
    }

    fn keep_offline(&mut self, _: &[DriveItem], _: bool) -> Result<(), ActionError> {
        Err(not_wired("keep_offline"))
    }

    fn copy_link(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(not_wired("copy_link"))
    }

    fn open(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        let (db, sync_root) = self.local()?;
        for item in items {
            let entry = file_ops::resolve(&db, &item.id).map_err(map_drive_err)?;
            let path = file_ops::item_path(&sync_root, &entry).map_err(map_drive_err)?;
            if !entry.is_dir() && file_ops::is_dangerous_executable(&path) {
                return Err(ActionError::Failed);
            }
            open_path(&path)?;
        }
        Ok(())
    }

    fn show_in_file_explorer(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        let (db, sync_root) = self.local()?;
        for item in items {
            let entry = file_ops::resolve(&db, &item.id).map_err(map_drive_err)?;
            let path = file_ops::item_path(&sync_root, &entry).map_err(map_drive_err)?;
            reveal_in_file_explorer(&path)?;
        }
        Ok(())
    }

    fn upload(&mut self, folder: Option<&str>, files: &[std::path::PathBuf]) -> Result<(), ActionError> {
        let (db, sync_root) = self.local()?;
        let parent = file_ops::resolve_folder(&db, folder).map_err(map_drive_err)?;
        file_ops::copy_in(&db, &sync_root, parent.as_ref(), files).map_err(map_drive_err)?;
        Ok(())
    }

    fn new_folder(&mut self, parent: Option<&str>, name: &str) -> Result<(), ActionError> {
        let (db, sync_root) = self.local()?;
        let parent_entry = file_ops::resolve_folder(&db, parent).map_err(map_drive_err)?;
        file_ops::create_folder(&db, &sync_root, parent_entry.as_ref(), name).map_err(map_drive_err)?;
        Ok(())
    }

    fn rename(&mut self, item: &DriveItem, name: &str) -> Result<(), ActionError> {
        let (db, sync_root) = self.local()?;
        let entry = file_ops::resolve(&db, &item.id).map_err(map_drive_err)?;
        file_ops::rename(&db, &sync_root, &entry, name).map_err(map_drive_err)?;
        Ok(())
    }

    fn move_to(&mut self, items: &[DriveItem], folder: Option<&str>) -> Result<(), ActionError> {
        let (db, sync_root) = self.local()?;
        let entries: Vec<_> = items
            .iter()
            .map(|i| file_ops::resolve(&db, &i.id))
            .collect::<Result<_, _>>()
            .map_err(map_drive_err)?;
        let target = file_ops::resolve_folder(&db, folder).map_err(map_drive_err)?;
        file_ops::move_items(&db, &sync_root, &entries, target.as_ref()).map_err(map_drive_err)?;
        Ok(())
    }

    fn free_up_space(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(not_wired("free_up_space"))
    }

    fn open_in_file_explorer(&mut self, folder: Option<&str>) -> Result<(), ActionError> {
        let (db, sync_root) = self.local()?;
        let path = match folder {
            Some(id) => {
                let entry = file_ops::resolve(&db, id).map_err(map_drive_err)?;
                file_ops::item_path(&sync_root, &entry).map_err(map_drive_err)?
            }
            None => sync_root,
        };
        open_path(&path)
    }

    fn changes(&mut self) -> Vec<Change> {
        Vec::new()
    }
}
