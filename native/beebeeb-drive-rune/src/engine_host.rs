//! The bridge between the engine crate and the native shell's trait system.
//!
//! [`open::open`] is the start path: it makes the one engine handle of this process, for the identity this
//! binary was built as (`shell::identity::engine_identity`, the only place the native app asks for it).
//! `LiveEngine` wraps that handle and implements `EngineActions`. `EngineSignInSource` implements
//! `SignInSource` by spawning worker threads for each blocking engine call and delivering results through a
//! channel. `EngineDriveSource` implements `DriveSource` on the handle's local state, with the engine's own
//! view models. `EngineTraySource` ([`tray`]) is the flyout's. Every type is generic over the credential
//! stores, so a test runs the same code on stores in memory; the app uses the platform's.
//!
//! No per-app name is spelled here: the sync folder, the settings file and the state folder come from the
//! identity the handle was opened with.
//!
//! Secrets: the password, codes and recovery phrase arrive as `&str` from a secure `TextEdit`
//! (wiped on drop) and go into the engine call as `Zeroizing<String>`, then nowhere else: not
//! into the element tree, not into a label, not into a log line, not into the feedback mode's
//! capture. The engine events that cross to the UI use the redacting `Debug`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;

use zeroize::Zeroizing;

use beebeeb_engine::app::{CredentialStores, Engine, LocalStateRefused};
use beebeeb_engine::drive_v2::DriveError;
use beebeeb_engine::drive_v2::model::{self, Sort as EngineSort};
use beebeeb_engine::engine_status::EngineStatusView;
use beebeeb_engine::file_ops;
use beebeeb_engine::host::{EngineEvent, HostMessage, PauseState};
use beebeeb_engine::identity::EngineIdentity;
use beebeeb_engine::signin::gate::{
    GateState as EngineGate, NextAfterSignIn as EngineNext, SignInProblem as EngineProblem,
};
use beebeeb_engine::state_db::StateDb;
use beebeeb_engine::sync_view::Start;

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
use crate::shell::tray::TrayInputs;
use crate::source::{DriveItem, DriveSource, Listing, SourceError, StorageSummary};

pub mod backups;
#[cfg(test)]
mod binding_tests;
pub mod drive_settings;
pub mod open;
pub mod settings;
pub mod sync;
#[cfg(test)]
pub(crate) mod tests;
pub mod this_pc;
pub mod tray;

pub use sync::EngineGlobalSource;
pub use tray::EngineTraySource;

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
//
// SECURITY: file names are ATTACKER-controlled (a shared folder's owner names
// the files). Nothing here routes a path through cmd.exe, PowerShell, bash or
// any shell. On Windows, `ShellExecuteW` takes the path as one wide string,
// never a command line. On macOS/Linux, `Command::new("open"/"xdg-open").arg`
// passes it as a single argv entry, never through a shell.
// ---------------------------------------------------------------------------

/// Open a file or folder with the OS's default handler. Never passes the path
/// through a shell interpreter.
fn open_path(path: &std::path::Path) -> Result<(), files_source::ActionError> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let verb: Vec<u16> = "open\0".encode_utf16().collect();
        // SAFETY: ShellExecuteW is called with valid null-terminated wide strings. The function
        // returns a value > 32 on success. No shell interpretation of the path occurs.
        let result = unsafe {
            windows_sys::Win32::UI::Shell::ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
            )
        };
        if (result as usize) <= 32 {
            return Err(files_source::ActionError::Failed);
        }
    }
    #[cfg(target_os = "macos")]
    {
        // `open` on macOS: the path is passed as a single argv element, not through a shell.
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|_| files_source::ActionError::Failed)?;
    }
    #[cfg(target_os = "linux")]
    {
        // `xdg-open` on Linux: the path is passed as a single argv element, not through a shell.
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|_| files_source::ActionError::Failed)?;
    }
    Ok(())
}

/// Reveal a file or folder in File Explorer / Finder / Files. Never passes the
/// path through a shell interpreter.
fn reveal_in_file_explorer(path: &std::path::Path) -> Result<(), files_source::ActionError> {
    #[cfg(target_os = "windows")]
    {
        // `explorer.exe /select,<path>` as ONE argv element: the process is started directly
        // via `CreateProcessW`, no shell in between. The path is never parsed by cmd.exe.
        std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", path.display()))
            .spawn()
            .map_err(|_| files_source::ActionError::Failed)?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map_err(|_| files_source::ActionError::Failed)?;
    }
    #[cfg(target_os = "linux")]
    {
        // Open the parent folder.
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
    /// The engine's last report of itself, folded from the messages by [`LiveEngine::drain`] ([`fold`]): one
    /// value, never a list. The b and the flyout read it; whoever drains first moves it for both.
    status: Mutex<EngineStatusView>,
    /// One start at a time, and the name of the folder it asked for ([`sync::StartWorker`]).
    starts: sync::StartWorker,
}

impl<C: CredentialStores> LiveEngine<C> {
    pub fn new(opened: &open::Opened<C>, messages: mpsc::Receiver<HostMessage>) -> Self {
        Self {
            engine: opened.engine.clone(),
            identity: opened.identity,
            messages: Mutex::new(messages),
            status: Mutex::new(EngineStatusView::default()),
            starts: sync::StartWorker::default(),
        }
    }

    /// Takes everything the engine queued, folds the engine's reports of itself into the status ([`fold`]) and
    /// drops the rest. Returns how many messages there were (a test reads it).
    pub fn drain(&self) -> usize {
        let messages = self.messages.lock().unwrap_or_else(PoisonError::into_inner);
        let mut status = self.status.lock().unwrap_or_else(PoisonError::into_inner);
        let mut count = 0;
        for message in messages.try_iter() {
            fold(&mut status, &message);
            count += 1;
        }
        count
    }

    /// The engine's last report of itself, as the last drain left it. Empty (`state` "") before the first.
    pub fn status_view(&self) -> EngineStatusView {
        self.status.lock().unwrap_or_else(PoisonError::into_inner).clone()
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

/// What the tray reads of one message. The engine's report of itself (`engine-status`) goes into `view` as the
/// engine's own listener folds it (`EngineStatusView::apply`), and so does the pause switch's `paused`. The resume
/// switch's `idle` says nothing about a check (it is not a report of one), so it empties the state: not known
/// until the next report, never "up to date". Every other message is dropped. Nothing of a message is logged.
fn fold(view: &mut EngineStatusView, message: &HostMessage) {
    match message {
        HostMessage::Event(EngineEvent::Status(status)) => {
            if let Ok(payload) = serde_json::to_value(status) {
                view.apply(&payload);
            }
        }
        HostMessage::Event(EngineEvent::PauseChanged(change)) => {
            let state = match change.state {
                PauseState::Paused => "paused",
                PauseState::Idle => "",
            };
            view.apply(&serde_json::json!({ "state": state }));
        }
        _ => {}
    }
}

impl<C: CredentialStores> EngineActions for LiveEngine<C> {
    /// The gate from the handle's view (read without waiting: the UI thread asks it on every frame).
    fn session(&self) -> Session {
        self.drain();
        let gate = map_gate_shell(self.engine.view().gate);
        // Whose session: the email is known from the stored session at start, the user id only once the
        // server named it in this run. One of them, always the same one for one account in one run.
        let account = self
            .engine
            .account_email()
            .or_else(|| self.engine.user_id())
            .unwrap_or_default();
        Session::of(gate, account)
    }

    /// The b's inputs: the session from the handle, the engine's state from its last report while it runs
    /// ([`fold`]; the flyout reads the same report). Not running is the b's "not syncing"; running, before a
    /// finished check, its "checking": "up to date" only after the engine reported a finished check.
    fn tray_inputs(&self) -> TrayInputs {
        self.drain();
        sync::tray_inputs_of(&self.engine.view(), &self.status_view())
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

    /// Asks the engine to start, on a `start-sync` worker, one at a time (contract §4.3): the engine's start blocks
    /// (it binds the local data and, on Windows, registers the folder) and this is called from the window's thread.
    /// `Ok` means "asked"; the view says where the start got to, and a refusal opens the modal. Without a session
    /// nothing is asked.
    fn start_sync(&self) -> Result<(), actions::ActionError> {
        if self.engine.view().gate != EngineGate::Ready {
            return Err(actions::ActionError::NotAvailable("not signed in and unlocked"));
        }
        let (engine, folder) = (self.engine.clone(), self.starts.folder());
        self.starts
            .ask(move || sync::start_sync_now(&engine, &folder))
            .map(|_| ())
            .map_err(|_| actions::ActionError::Failed("could not start a thread".into()))
    }

    fn sync_refusal(&self) -> Option<crate::frames::global::sync_off::SyncOff> {
        sync::sync_refusal(&self.engine.view(), self.starts.folder_name())
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
    /// The gate from the handle's view: read without waiting, on every frame.
    fn gate(&self) -> GateState {
        map_gate_signin(self.engine.view().gate)
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

/// What the window says for a local state that is not handed out: local data that is not provably this account's
/// has its own words; everything else (signed out, locked, nothing synced yet, unreadable) is "cannot be read".
/// Never an empty folder.
fn source_error(refused: LocalStateRefused) -> SourceError {
    if refused.not_this_accounts() {
        SourceError::NotThisAccounts
    } else {
        SourceError::Unreadable
    }
}

impl<C: CredentialStores> DriveSource for EngineDriveSource<C> {
    /// The whole folder, name ascending, with its crumbs from the root. Before the first sync there is no local
    /// state: that, a folder that is gone and a database that cannot be read are errors, never an empty folder.
    fn list(&self, folder_id: Option<&str>) -> Result<Listing, SourceError> {
        let read = |db: &StateDb| -> Result<Listing, DriveError> {
            let (rows, _total) = model::children_of(db, folder_id, EngineSort::default(), 0, usize::MAX)?;
            let folder = model::resolve_folder(db, folder_id)?;
            let path: Vec<_> = model::crumbs(db, folder.as_ref())?
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
        // The account-bound handle (task 2488): a state that is not the signed-in account's answers nothing, and
        // an answer read across a lock or a sign-out is dropped.
        self.engine
            .local_state()
            .and_then(|state| state.with(read))
            .map_err(source_error)?
            .map_err(|error| match error {
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
    /// A local file operation, on the signed-in account's local state (the account-bound handle, task 2488) and
    /// the sync root: both are required. Refused when sync is not running for this handle (`Engine::is_syncing`):
    /// changing a local folder that nothing syncs is not an honest operation. Refused too when the session
    /// changed while the operation ran.
    fn local<T>(&self, operation: impl FnOnce(&StateDb, &Path) -> Result<T, ActionError>) -> Result<T, ActionError> {
        if !self.engine.is_syncing() {
            return Err(ActionError::Unavailable);
        }
        let sync_root = self.engine.sync_root().map_err(|_| ActionError::Unavailable)?;
        self.engine
            .local_state_for_changes()
            .and_then(|state| state.with(|db| operation(db, &sync_root)))
            .map_err(|_| ActionError::Unavailable)?
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

    /// The status line's word for the handle's view (`sync::sync_word`): read without waiting, on every frame, and
    /// never "Up to date" for an engine that does not run.
    fn sync_state(&self) -> Option<String> {
        sync::sync_word(&self.engine.view()).map(str::to_string)
    }

    fn details(&self, item: &DriveItem, folder: &str) -> Details {
        files_source::details_of_row(item, folder)
    }

    fn changed_label(&self, _: &DriveItem) -> Option<String> {
        None
    }

    fn can_add(&self, _folder: Option<&str>) -> bool {
        // Writable while an engine runs on the folder (asked on every frame: the view, which never waits). The
        // operation itself goes through `local()`, which asks the engine again.
        self.engine.view().start == Start::Running
    }

    fn sort_of(&self, _: Option<&str>) -> Option<crate::frames::files::Sort> {
        None
    }

    fn set_sort(&mut self, _: Option<&str>, _: crate::frames::files::Sort) {
        log("NotWired(set_sort)");
    }

    fn trash(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        // Trash is an API call, not a sync-folder operation: it is NOT gated by is_syncing().
        // Validate against the state database when it is available (a best-effort guard against
        // trashing an uploading or protected item); when the database is not available, the
        // server's own checks are the backstop.
        if let Ok(state) = self.engine.local_state() {
            let checked = state.with(|db| {
                for item in items {
                    if let Ok(entry) = file_ops::resolve(db, &item.id) {
                        file_ops::check_trashable(db, &entry).map_err(map_drive_err)?;
                    }
                }
                Ok(())
            });
            if let Ok(refused @ Err(_)) = checked {
                return refused;
            }
        }
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        self.engine.trash_items(&ids).map_err(|_| ActionError::Failed)
    }

    fn restore(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        self.engine.restore_items(&ids).map_err(|_| ActionError::Failed)
    }

    fn duplicate(&mut self, items: &[DriveItem], _taken: &[&str]) -> Result<(), ActionError> {
        self.local(|db, sync_root| {
            for item in items {
                let entry = file_ops::resolve(db, &item.id).map_err(map_drive_err)?;
                file_ops::duplicate(db, sync_root, &entry).map_err(map_drive_err)?;
            }
            Ok(())
        })
    }

    fn keep_offline(&mut self, _: &[DriveItem], _: bool) -> Result<(), ActionError> {
        Err(not_wired("keep_offline"))
    }

    fn copy_link(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(not_wired("copy_link"))
    }

    fn open(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        // The paths are resolved under the handle; the OS is asked after it answered.
        let paths = self.local(|db, sync_root| {
            let mut paths = Vec::with_capacity(items.len());
            for item in items {
                let entry = file_ops::resolve(db, &item.id).map_err(map_drive_err)?;
                let path = file_ops::item_path(sync_root, &entry).map_err(map_drive_err)?;
                if !entry.is_dir() && file_ops::is_dangerous_executable(&path) {
                    return Err(ActionError::Failed);
                }
                paths.push(path);
            }
            Ok(paths)
        })?;
        paths.iter().try_for_each(|path| open_path(path))
    }

    fn show_in_file_explorer(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        let paths = self.local(|db, sync_root| {
            let mut paths = Vec::with_capacity(items.len());
            for item in items {
                let entry = file_ops::resolve(db, &item.id).map_err(map_drive_err)?;
                paths.push(file_ops::item_path(sync_root, &entry).map_err(map_drive_err)?);
            }
            Ok(paths)
        })?;
        paths.iter().try_for_each(|path| reveal_in_file_explorer(path))
    }

    fn upload(&mut self, folder: Option<&str>, files: &[std::path::PathBuf]) -> Result<(), ActionError> {
        self.local(|db, sync_root| {
            let parent = file_ops::resolve_folder(db, folder).map_err(map_drive_err)?;
            file_ops::copy_in(db, sync_root, parent.as_ref(), files).map_err(map_drive_err)?;
            Ok(())
        })
    }

    fn new_folder(&mut self, parent: Option<&str>, name: &str) -> Result<(), ActionError> {
        self.local(|db, sync_root| {
            let parent_entry = file_ops::resolve_folder(db, parent).map_err(map_drive_err)?;
            file_ops::create_folder(db, sync_root, parent_entry.as_ref(), name).map_err(map_drive_err)?;
            Ok(())
        })
    }

    fn rename(&mut self, item: &DriveItem, name: &str) -> Result<(), ActionError> {
        self.local(|db, sync_root| {
            let entry = file_ops::resolve(db, &item.id).map_err(map_drive_err)?;
            file_ops::rename(db, sync_root, &entry, name).map_err(map_drive_err)?;
            Ok(())
        })
    }

    fn move_to(&mut self, items: &[DriveItem], folder: Option<&str>) -> Result<(), ActionError> {
        self.local(|db, sync_root| {
            let entries: Vec<_> = items
                .iter()
                .map(|i| file_ops::resolve(db, &i.id))
                .collect::<Result<_, _>>()
                .map_err(map_drive_err)?;
            let target = file_ops::resolve_folder(db, folder).map_err(map_drive_err)?;
            file_ops::move_items(db, sync_root, &entries, target.as_ref()).map_err(map_drive_err)?;
            Ok(())
        })
    }

    fn free_up_space(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(not_wired("free_up_space"))
    }

    fn open_in_file_explorer(&mut self, folder: Option<&str>) -> Result<(), ActionError> {
        let path = self.local(|db, sync_root| match folder {
            Some(id) => {
                let entry = file_ops::resolve(db, id).map_err(map_drive_err)?;
                file_ops::item_path(sync_root, &entry).map_err(map_drive_err)
            }
            None => Ok(sync_root.to_path_buf()),
        })?;
        open_path(&path)
    }

    fn changes(&mut self) -> Vec<Change> {
        Vec::new()
    }
}
