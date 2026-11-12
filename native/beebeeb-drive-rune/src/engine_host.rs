//! The bridge between the engine crate and the native shell's trait system.
//!
//! `LiveEngine` wraps `Arc<Engine<PlatformCredentials>>` and implements `EngineActions`.
//! `EngineSignInSource` implements `SignInSource` by spawning worker threads for each blocking
//! engine call and delivering results through a channel. `EngineDriveSource` implements
//! `DriveSource` on the live engine's listing.
//!
//! Secrets: the password, codes and recovery phrase arrive as `&str` from a secure `TextEdit`
//! (wiped on drop) and go into the engine call as `Zeroizing<String>`, then nowhere else: not
//! into the element tree, not into a label, not into a log line, not into the feedback mode's
//! capture. The engine events that cross to the UI use the redacting `Debug`.

use std::sync::{Arc, mpsc};
use std::thread;

use zeroize::Zeroizing;

use beebeeb_engine::app::{Engine, PlatformCredentials, SyncStart};
use beebeeb_engine::drive_v2::dto;
use beebeeb_engine::drive_v2::model::Sort as EngineSort;
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
use crate::source::{Crumb, DriveItem, DriveSource, Listing, SourceError, StorageSummary};

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

fn log(message: impl AsRef<str>) {
    eprintln!("beebeeb engine_host: {}", message.as_ref());
}

// ---------------------------------------------------------------------------
// LiveEngine: EngineActions on the real engine
// ---------------------------------------------------------------------------

/// The real engine behind the shell's `EngineActions`.
pub struct LiveEngine {
    engine: Arc<Engine<PlatformCredentials>>,
}

impl LiveEngine {
    pub fn new(engine: Arc<Engine<PlatformCredentials>>) -> Self {
        Self { engine }
    }
}

impl EngineActions for LiveEngine {
    fn session(&self) -> Session {
        let gate = map_gate_shell(self.engine.gate());
        let account = self.engine.user_id().unwrap_or_default();
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

    fn start_sync(&self) -> Result<(), actions::ActionError> {
        let sync_root = beebeeb_engine::config::default_sync_root_suggestion();
        match self.engine.start_sync(sync_root) {
            Ok(SyncStart::Started) => Ok(()),
            Ok(SyncStart::AlreadyRunning) => Ok(()),
            Ok(SyncStart::NoSession) => Err(actions::ActionError::NotAvailable("not signed in")),
            Err(e) => Err(actions::ActionError::Failed(e)),
        }
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
pub struct EngineSignInSource {
    engine: Arc<Engine<PlatformCredentials>>,
    tx: mpsc::Sender<SignInEvent>,
    rx: mpsc::Receiver<SignInEvent>,
}

impl EngineSignInSource {
    pub fn new(engine: Arc<Engine<PlatformCredentials>>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { engine, tx, rx }
    }
}

impl SignInSource for EngineSignInSource {
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
                    Err(words) => {
                        let problem = if words.contains("expired") || words.contains("again") {
                            TwoFactorProblem::Expired
                        } else if words.contains("offline") || words.contains("connect") {
                            TwoFactorProblem::Offline
                        } else {
                            TwoFactorProblem::WrongCode
                        };
                        SignInEvent::TwoFactor(Err(problem))
                    }
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
                    Err(words) => {
                        let problem = if words.contains("phrase") || words.contains("not a valid") {
                            PhraseProblem::WrongPhrase { bad_word: None }
                        } else {
                            PhraseProblem::Other
                        };
                        SignInEvent::PhraseRestored(Err(problem))
                    }
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
// EngineDriveSource: DriveSource on the live engine listing
// ---------------------------------------------------------------------------

/// The engine's listing as a `DriveSource`. Names in a synced `state.db` are the decrypted ones
/// the engine stored; nothing here logs them.
pub struct EngineDriveSource {
    engine: Arc<Engine<PlatformCredentials>>,
}

impl EngineDriveSource {
    pub fn new(engine: Arc<Engine<PlatformCredentials>>) -> Self {
        Self { engine }
    }
}

fn map_item(item: dto::DriveItem) -> DriveItem {
    DriveItem {
        id: item.id,
        parent_id: item.parent_id,
        name: item.name,
        is_folder: item.is_folder,
        size: item.size,
        child_count: item.child_count,
        modified_at: item.modified_at,
        mime: item.mime,
        has_thumbnail: item.has_thumbnail,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: item.local.as_ref().and_then(|local| match local {
            dto::LocalState::Downloaded => Some("on_this_pc".to_string()),
            dto::LocalState::OnlineOnly => Some("online_only".to_string()),
            dto::LocalState::AvailableOffline => Some("available_offline".to_string()),
            dto::LocalState::Syncing | dto::LocalState::Failed => None,
        }),
        is_protected: item.backup.is_protected,
    }
}

impl DriveSource for EngineDriveSource {
    fn list(&self, folder_id: Option<&str>) -> Result<Listing, SourceError> {
        let (items, _total) = self
            .engine
            .listing(folder_id, EngineSort::default(), 0, usize::MAX)
            .map_err(|_| SourceError::Unreadable)?;

        let folder_name = if folder_id.is_some() { "Folder" } else { "Files" };
        let folder = Crumb {
            id: folder_id.map(str::to_string),
            name: folder_name.to_string(),
        };
        let root = Crumb {
            id: None,
            name: "Files".into(),
        };
        let path = if folder_id.is_some() {
            vec![root, folder.clone()]
        } else {
            vec![folder.clone()]
        };

        Ok(Listing {
            folder,
            path,
            items: items.into_iter().map(map_item).collect(),
        })
    }

    fn storage(&self) -> Option<StorageSummary> {
        None // storage numbers not in the engine handle yet
    }
}

// ---------------------------------------------------------------------------
// EngineFilesSource: FilesSource (most actions not wired)
// ---------------------------------------------------------------------------

/// `FilesSource` on the real engine. Actions the engine cannot perform yet answer
/// `Err(ActionError::Unavailable)` and log `NotWired(<action>)`.
pub struct EngineFilesSource {
    engine: Arc<Engine<PlatformCredentials>>,
}

impl EngineFilesSource {
    pub fn new(engine: Arc<Engine<PlatformCredentials>>) -> Self {
        Self { engine }
    }
}

fn not_wired(what: &str) -> ActionError {
    log(format!("NotWired({what})"));
    ActionError::Unavailable
}

impl FilesSource for EngineFilesSource {
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

    fn can_add(&self, _: Option<&str>) -> bool {
        false
    }

    fn sort_of(&self, _: Option<&str>) -> Option<crate::frames::files::Sort> {
        None
    }

    fn set_sort(&mut self, _: Option<&str>, _: crate::frames::files::Sort) {
        log("NotWired(set_sort)");
    }

    fn trash(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(not_wired("trash"))
    }
    fn restore(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(not_wired("restore"))
    }
    fn duplicate(&mut self, _: &[DriveItem], _: &[&str]) -> Result<(), ActionError> {
        Err(not_wired("duplicate"))
    }
    fn keep_offline(&mut self, _: &[DriveItem], _: bool) -> Result<(), ActionError> {
        Err(not_wired("keep_offline"))
    }
    fn copy_link(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(not_wired("copy_link"))
    }
    fn open(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(not_wired("open"))
    }
    fn show_in_file_explorer(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(not_wired("show_in_file_explorer"))
    }
    fn upload(&mut self, _: Option<&str>, _: &[std::path::PathBuf]) -> Result<(), ActionError> {
        Err(not_wired("upload"))
    }

    fn changes(&mut self) -> Vec<Change> {
        Vec::new()
    }
}
