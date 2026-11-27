//! The bridge between the engine crate and the native shell's trait system.
//!
//! [`open::open`] is the start path: it makes the one engine handle of this process, for the identity this
//! binary was built as (`shell::identity::engine_identity`, the only place the native app asks for it).
//! `LiveEngine` wraps that handle and implements `EngineActions`. `EngineSignInSource` ([`signin`]) implements
//! `SignInSource`: one worker runs the blocking engine calls and delivers results through a channel. `EngineDriveSource` implements `DriveSource` on the handle's local state, with the engine's own
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

use beebeeb_engine::app::{CredentialStores, Engine, LocalStateRefused};
use beebeeb_engine::drive_v2::DriveError;
use beebeeb_engine::drive_v2::case_place;
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
use crate::frames::signin::source::{GateState, NextAfterSignIn, PhraseProblem, SignInProblem, TwoFactorProblem};
use crate::shell::actions::{self, EngineActions};
use crate::shell::app::{GateState as ShellGate, Session};
use crate::shell::instance::ProfileHolder;
use crate::shell::tray::TrayInputs;
use crate::source::{DriveItem, DriveSource, Listing, SourceError, StorageSummary};

pub mod account;
mod app_lock;
mod away;
mod drive_errors;
use drive_errors::{map_drive_err, map_new_file_err};
pub mod backups;
// Backups' "Other devices" and "Delete this backup" (`backups::EngineBackupsSource` delegates to it).
pub mod backups_devices;
#[cfg(test)]
mod binding_tests;
pub mod drive_previews;
pub mod drive_settings;
pub mod files;
#[cfg(any(target_os = "windows", test))]
#[cfg_attr(not(test), allow(dead_code))] // the card is not wired into the Windows app yet
pub mod found_while_paused;
#[cfg(test)]
mod files_case_twins_tests;
mod files_conflicts;
mod files_downloads;
mod files_facts;
mod files_item_of;
mod files_pins;
#[cfg(test)]
mod files_item_of_tests;
mod files_launch;
mod files_link;
#[cfg(test)]
mod files_open_with_tests;
mod files_places;
mod files_preview;
mod files_trash;
mod files_uploads;
mod files_versions;
#[cfg(test)]
mod files_window_support;
pub mod guide;
mod launcher;
mod open_folder;
// The Windows Hello prompt of the lock screen, behind one seam.
pub mod hello;
// An item's read-only Preview over Trash (from Beebeeb only) and Shared (`item_preview`).
pub mod item_preview;
// The player's bytes: the engine's range reader for the session of the press (`media`).
pub mod media;
// An Office document's confined renderer: one helper per document, every answer checked (`office_host`).
pub mod office_host;
#[cfg(test)]
mod office_host_tests;
pub mod open;
// DW-1562's Open With: one lock around every process the app starts (#460 F11), and the apps Windows reports.
pub(crate) mod launch;
#[cfg(test)]
mod launch_tests;
mod open_with_apps;
pub mod passkeys;
pub mod photos;
pub mod search;
pub mod settings;
pub mod shared;
pub mod signin;
mod stamp;
pub mod sync;
#[cfg(test)]
pub(crate) mod tests;
pub mod this_pc;
pub mod thumb_repair;
pub mod thumbs;
pub mod trash;
// The first run's steps that have an engine function behind them (`setup`).
pub mod setup;
pub mod tray;
// An open file: its facts from the local state, its content decrypted in memory (`viewer`).
pub mod viewer;
// "Save a copy…" and "Download to…": a copy of one file at a place the person chose (`shell::viewer_save`; Photos reuses it).
pub mod save_copy;

pub use signin::EngineSignInSource;
pub use sync::EngineGlobalSource;
pub use tray::EngineTraySource;

// ---------------------------------------------------------------------------
// Gate / type mapping (one place, never a name or a path)
// ---------------------------------------------------------------------------

fn map_gate_shell(g: EngineGate) -> ShellGate {
    match g {
        EngineGate::Ready => ShellGate::Ready,
        EngineGate::SignedOut => ShellGate::SignedOut,
        // Both lock levels close the window's gate; which one holds is the sign-in source's word
        // (`map_gate_signin`), and the lock screens are drawn from it (`frames::signin::lock`).
        EngineGate::ScreenLocked | EngineGate::Locked => ShellGate::Locked,
        EngineGate::NeedsRecoveryPhrase => ShellGate::NeedsRecoveryPhrase,
        EngineGate::NoPlan => ShellGate::NoPlan,
        EngineGate::KeyStoreUnavailable => ShellGate::KeyStoreUnavailable,
    }
}

fn map_gate_signin(g: EngineGate) -> GateState {
    match g {
        EngineGate::Ready => GateState::Ready,
        EngineGate::SignedOut => GateState::SignedOut,
        EngineGate::ScreenLocked => GateState::Locked,
        EngineGate::Locked => GateState::VaultClosed,
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
// passes it as a single argv entry, never through a shell. The app chooser (`choose_app`) takes the path as one
// wide string in a struct field, the same way.
//
// Every start below holds the launch lock (`launch`, #460 F11): no other start can inherit a helper's pipe ends.
// ---------------------------------------------------------------------------

/// Open a file or folder with the OS's default handler. Never passes the path
/// through a shell interpreter. `still` is asked inside the launch lock, right before the start: `false` (the
/// press's session moved on) starts nothing.
fn open_path(path: &std::path::Path, still: &dyn Fn() -> bool) -> Result<(), files_source::ActionError> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let verb: Vec<u16> = "open\0".encode_utf16().collect();
        // SAFETY: ShellExecuteW is called with valid null-terminated wide strings. The function
        // returns a value > 32 on success. No shell interpretation of the path occurs.
        let result = launch::locked(|| {
            still().then(|| unsafe {
                windows_sys::Win32::UI::Shell::ShellExecuteW(
                    std::ptr::null_mut(),
                    verb.as_ptr(),
                    wide.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
                )
            })
        })
        .ok_or(files_source::ActionError::Unavailable)?;
        if (result as usize) <= 32 {
            return Err(files_source::ActionError::Failed);
        }
    }
    #[cfg(target_os = "macos")]
    {
        // `open` on macOS: the path is passed as a single argv element, not through a shell.
        launch::locked(|| still().then(|| std::process::Command::new("open").arg(path).spawn()))
            .ok_or(files_source::ActionError::Unavailable)?
            .map_err(|_| files_source::ActionError::Failed)?;
    }
    #[cfg(target_os = "linux")]
    {
        // `xdg-open` on Linux: the path is passed as a single argv element, not through a shell.
        launch::locked(|| still().then(|| std::process::Command::new("xdg-open").arg(path).spawn()))
            .ok_or(files_source::ActionError::Unavailable)?
            .map_err(|_| files_source::ActionError::Failed)?;
    }
    Ok(())
}

/// Whether the system has an app for the kind of this file (`open_file` step 2). Windows asks the file association
/// of the extension; the other systems cannot say cheaply, and their `open` / `xdg-open` handles the rest.
fn has_default_app(path: &std::path::Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        let Some(ext) = path.extension() else { return false };
        let dotted: Vec<u16> = std::ffi::OsStr::new(".")
            .encode_wide()
            .chain(ext.encode_wide())
            .chain(std::iter::once(0))
            .collect();
        let verb: Vec<u16> = "open\0".encode_utf16().collect();
        let mut out = [0u16; 520];
        let mut len = out.len() as u32;
        // SAFETY: valid null-terminated wide strings and a buffer of `len` u16s; the call writes at most `len`.
        // ASSOCF_INIT_IGNOREUNKNOWN (0x400): no answer for a kind Windows does not know. ASSOCSTR_EXECUTABLE (2).
        let hr = unsafe {
            windows_sys::Win32::UI::Shell::AssocQueryStringW(
                0x400,
                2,
                dotted.as_ptr(),
                verb.as_ptr(),
                out.as_mut_ptr(),
                &mut len,
            )
        };
        hr == 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        path.extension().is_some()
    }
}

/// Windows' "Open with" chooser for this file, on a thread of its own (the dialog is modal for the thread that
/// shows it: never the window's). `OAIF_EXEC`: the app picked opens the file; no `OAIF_ALLOW_REGISTRATION`, so
/// the chooser cannot change the PC's default app. `Ok`: the thread started. Every other system: not there.
///
/// The launch lock is held for the whole call: the chooser starts the app picked INSIDE `SHOpenWithDialog`, and the
/// call has no point between "picked" and "started" where the lock could be taken. So while the chooser is open,
/// every other start of the app (a helper, Open, Show in File Explorer) waits until it closes. A chooser that waited
/// for another one opens only while `still()` holds once the lock is taken (the press's session).
fn choose_app(
    path: &std::path::Path,
    still: impl Fn() -> bool + Send + 'static,
) -> Result<(), files_source::ActionError> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
        use windows_sys::Win32::UI::Shell::{OAIF_EXEC, OPENASINFO, SHOpenWithDialog};
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        std::thread::Builder::new()
            .name("files-open-with".into())
            .spawn(move || {
                // SAFETY: COM is initialised on this thread and uninitialised before it ends (only when the
                // initialisation succeeded); `wide` is NUL-ended and outlives the call, `pcszClass` is null.
                unsafe {
                    let com = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
                    let info = OPENASINFO {
                        pcszFile: wide.as_ptr(),
                        pcszClass: std::ptr::null(),
                        oaifInFlags: OAIF_EXEC,
                    };
                    let _ = launch::locked(|| still().then(|| SHOpenWithDialog(std::ptr::null_mut(), &info)));
                    if com >= 0 {
                        CoUninitialize();
                    }
                }
            })
            .map(|_| ())
            .map_err(|_| files_source::ActionError::Failed)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (path, still);
        Err(files_source::ActionError::NotOnThisSystem)
    }
}

/// The one raw argument explorer.exe is given to select `path`: `/select,"<path>"`, the path always quoted.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn explorer_select_arg(path: &std::path::Path) -> String {
    format!("/select,\"{}\"", path.display())
}

#[cfg(test)]
mod explorer_select_tests {
    use super::explorer_select_arg;
    use std::path::Path;

    #[test]
    fn the_path_is_quoted_inside_the_select_argument() {
        assert_eq!(
            explorer_select_arg(Path::new("C:\\Users\\Jo Doe\\Beebeeb\\my folder\\r\u{e9}sum\u{e9} \u{4e2d}.pdf")),
            "/select,\"C:\\Users\\Jo Doe\\Beebeeb\\my folder\\r\u{e9}sum\u{e9} \u{4e2d}.pdf\""
        );
        assert_eq!(explorer_select_arg(Path::new(r"C:\a\b.txt")), "/select,\"C:\\a\\b.txt\"");
    }
}

/// Reveal a file or folder in File Explorer / Finder / Files. Never passes the
/// path through a shell interpreter. `still`: as [`open_path`]'s.
fn reveal_in_file_explorer(path: &std::path::Path, still: &dyn Fn() -> bool) -> Result<(), files_source::ActionError> {
    #[cfg(target_os = "windows")]
    {
        // `explorer.exe /select,"<path>"`: the process is started directly via `CreateProcessW`, no shell in
        // between. explorer.exe parses its own command line, so the argument is passed raw (std would quote the
        // whole `/select,<path>` element when the path has a space, and explorer opens with nothing selected).
        use std::os::windows::process::CommandExt;
        let select = explorer_select_arg(path);
        launch::locked(|| still().then(|| std::process::Command::new("explorer.exe").raw_arg(select).spawn()))
            .ok_or(files_source::ActionError::Unavailable)?
            .map_err(|_| files_source::ActionError::Failed)?;
    }
    #[cfg(target_os = "macos")]
    {
        launch::locked(|| still().then(|| std::process::Command::new("open").arg("-R").arg(path).spawn()))
            .ok_or(files_source::ActionError::Unavailable)?
            .map_err(|_| files_source::ActionError::Failed)?;
    }
    #[cfg(target_os = "linux")]
    {
        // Open the parent folder.
        if let Some(parent) = path.parent() {
            launch::locked(|| still().then(|| std::process::Command::new("xdg-open").arg(parent).spawn()))
                .ok_or(files_source::ActionError::Unavailable)?
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
    /// Whether this PC still has its first run to do. Not due until the app's start reads it
    /// ([`LiveEngine::wire_first_run`]): a handle nobody wired never shows one.
    first_run: Arc<setup::FirstRun>,
    /// The menu's and the tray's Open in File Explorer start here, never on the window's thread (R-OW1).
    launcher: launcher::Launcher,
}

impl<C: CredentialStores> LiveEngine<C> {
    pub fn new(opened: &open::Opened<C>, messages: mpsc::Receiver<HostMessage>) -> Self {
        Self {
            engine: opened.engine.clone(),
            identity: opened.identity,
            messages: Mutex::new(messages),
            status: Mutex::new(EngineStatusView::default()),
            starts: sync::StartWorker::default(),
            first_run: Arc::default(),
            launcher: launcher::Launcher::default(),
        }
    }

    /// At the app's start, before the window's first frame: read whether this PC still has its first run to do
    /// (the settings file, once). From then on a session that is signed in and unlocked shows the first-run steps
    /// until they end ([`EngineActions::session`]).
    pub fn wire_first_run(&self) {
        self.first_run.read(&*self.engine);
    }

    /// What the first-run source asks of the shell: the one start path, the folder opener, the start-at-login
    /// entry and the gate's flag.
    pub fn setup_hooks(self: &Arc<Self>, platform: Arc<dyn crate::shell::platform::Platform>) -> setup::Hooks
    where
        C: 'static,
    {
        // dc-167: the account switcher reaches this handle (installed once; a second call changes nothing).
        crate::frames::account::switcher::install(self.clone());
        let live = self.clone();
        setup::Hooks {
            start_sync: Arc::new(move || live.start_sync().is_ok()),
            open_folder: |folder| open_path(folder, &|| true).is_ok(),
            platform,
            first_run: self.first_run.clone(),
            explorer_step: cfg!(target_os = "windows"),
            look_later: {
                let engine = self.engine.clone();
                Arc::new(move |after| {
                    let engine = engine.clone();
                    let _ = std::thread::Builder::new()
                        .name("first-run-look".into())
                        .spawn(move || {
                            std::thread::sleep(after);
                            engine.wake_host();
                        });
                })
            },
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
            // A notice of the engine as a Windows toast (`windows_toast::show`, nothing yet).
            #[cfg(windows)]
            if let HostMessage::Notice(notice) = &message {
                crate::shell::platform::windows_toast::show(notice);
            }
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

/// dc-167: the account switcher's calls, forwarded to the handle (`app::account_switch`). Run off the window's thread.
impl<C: CredentialStores + 'static> crate::frames::account::switcher::Switcher for LiveEngine<C> {
    fn accounts(&self) -> Vec<crate::frames::account::switcher::Listed> {
        self.engine
            .accounts_on_this_pc()
            .into_iter()
            .map(|account| crate::frames::account::switcher::Listed {
                email: account.email.unwrap_or_else(|| "Account".to_string()),
                id: account.user_id,
                current: account.current,
            })
            .collect()
    }

    /// The switch, then sync starts for the account it came to (its start clears the previous account's local data,
    /// and an unsent file of it is offered "Discard and switch" by the sync-off banner). A gate that is not open
    /// (the account's own app lock) starts nothing here: its unlock does.
    fn switch_to(&self, id: &str) -> Result<(), String> {
        self.engine.switch_account(id).map_err(|why| why.words().to_string())?;
        let _ = self.start_sync();
        Ok(())
    }

    fn add_another(&self) -> Result<(), String> {
        self.engine.add_another_account().map_err(|why| why.words().to_string())
    }

    fn sign_out(&self, id: &str) -> Result<(), String> {
        self.engine.sign_out_parked_account(id).map_err(|why| why.words().to_string())
    }
}

/// Sync anyway on `engine`: the lift is asked, then a tick wakes to apply it. Shared by the tray and Settings.
pub(crate) fn sync_anyway_now<C: CredentialStores>(engine: &beebeeb_engine::app::Engine<C>) {
    beebeeb_engine::power::sync_anyway();
    engine.retry_first_list();
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
        let gate = match map_gate_shell(self.engine.view().gate) {
            // Signed in and unlocked, and this PC has not had its first run: its steps, not the main window.
            ShellGate::Ready if self.first_run.due() => ShellGate::FirstRun,
            gate => gate,
        };
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

    /// The engine lifts its metered or battery-saver hold until the connection changes, and its next tick runs now
    /// (the whole-tick wake that Try again on the first list uses). Never waits.
    fn sync_anyway(&self) -> Result<(), actions::ActionError> {
        sync_anyway_now(&self.engine);
        Ok(())
    }

    fn open_in_file_explorer(&self) -> Result<(), actions::ActionError> {
        let path = open_folder::beebeeb_folder(&self.engine)
            .map_err(|_| actions::ActionError::NotAvailable("sync is not running"))?;
        let asked = self
            .launcher
            .open_for(&self.engine, open_path, path, "the folder did not open");
        asked.map_err(|()| actions::ActionError::Failed("the Beebeeb folder did not open".to_string()))
    }

    fn profile_holder(&self) -> Option<ProfileHolder> {
        None
    }

    /// Asks the engine to start, on a `start-sync` worker, one at a time (contract §4.3): the engine's start blocks
    /// (it binds the local data and, on Windows, registers the folder) and this is called from the window's thread.
    /// `Ok` means "asked"; the view says where the start got to, and a refusal opens the modal. Without a session
    /// nothing is asked.
    fn start_sync(&self) -> Result<(), actions::ActionError> {
        if !sync::may_start(self.engine.view().gate) {
            return Err(actions::ActionError::NotAvailable("not signed in and unlocked"));
        }
        let (engine, folder, first_run) = (self.engine.clone(), self.starts.folder(), self.first_run.clone());
        self.starts
            .ask(move || sync::start_sync_now(&engine, &folder, &first_run))
            .map(|_| ())
            .map_err(|_| actions::ActionError::Failed("could not start a thread".into()))
    }

    fn sync_refusal(&self) -> Option<crate::frames::global::sync_off::SyncOff> {
        sync::sync_refusal(&self.engine.view(), self.starts.folder_name())
    }

    /// dc-111: what "Discard and switch" would lose, in the folder the refused start asked for (the settings' folder,
    /// else the suggestion, as `setup_folder::start_now` picks it). The engine's own ask rides along as the token.
    fn discard_ask(&self) -> Result<crate::frames::signin::discard_switch::Ask, actions::ActionError> {
        use setup::SetupEngine as _;
        let folder = self.engine.settings_folder().ok().flatten().unwrap_or_else(|| self.engine.suggested_folder());
        let ask = self
            .engine
            .discard_and_switch_ask(&folder)
            .map_err(|why| actions::ActionError::Failed(format!("{why:?}")))?;
        Ok(crate::frames::signin::discard_switch::Ask {
            files: ask.files,
            names: ask.names.clone(),
            token: Arc::new(ask),
        })
    }

    /// dc-111: discard exactly the shown ask and switch, on the start worker (the engine copies files, then starts).
    /// A refused discard changed nothing; a plain start then follows, so the refusal modal opens again with the truth.
    fn discard_and_switch(&self, ask: crate::frames::signin::discard_switch::Ask) -> Result<(), actions::ActionError> {
        use setup::SetupEngine as _;
        let Ok(ask) = ask.token.downcast::<beebeeb_engine::account_binding::DiscardAsk>() else {
            return Err(actions::ActionError::NotAvailable("not this engine's ask"));
        };
        let folder = self.engine.settings_folder().ok().flatten().unwrap_or_else(|| self.engine.suggested_folder());
        let (engine, name, first_run) = (self.engine.clone(), self.starts.folder(), self.first_run.clone());
        self.starts
            .ask(move || match engine.discard_and_switch(&ask, folder) {
                Ok(done) => tracing::info!(kept = done.kept, "discard and switch: done"),
                Err(why) => {
                    tracing::warn!(?why, "discard and switch: nothing was removed");
                    sync::start_sync_now(&engine, &name, &first_run);
                }
            })
            .map(|_| ())
            .map_err(|_| actions::ActionError::Failed("could not start a thread".into()))
    }

    /// dc-RC-b: the previous account's leftover files move beside the folder the refused start asked for, and the
    /// start follows, on the start worker (the engine copies files). A refused move changed nothing in the sync
    /// folder; a plain start then follows, so the refusal modal opens again with the truth.
    fn keep_aside_and_switch(&self) -> Result<(), actions::ActionError> {
        use setup::SetupEngine as _;
        let folder = self.engine.settings_folder().ok().flatten().unwrap_or_else(|| self.engine.suggested_folder());
        let (engine, name, first_run) = (self.engine.clone(), self.starts.folder(), self.first_run.clone());
        self.starts
            .ask(move || match engine.keep_aside_and_switch(folder) {
                Ok(done) => tracing::info!(kept = done.kept, "keep aside: done"),
                Err(why) => {
                    tracing::warn!(?why, "keep aside: nothing was moved");
                    sync::start_sync_now(&engine, &name, &first_run);
                }
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

    /// The away lock: the engine compares and locks (`engine_host/away.rs`).
    fn away_job(&self, seen: actions::Away) -> Option<actions::AwayJob> {
        Some(away::job(&self.engine, seen))
    }

    fn screen_open(&self) -> bool {
        away::screen_open(&self.engine)
    }

    /// The wake cover's sign-in path: the vault's lock (key out of memory), whose way back is the password.
    /// Off the UI thread: the lock waits for the engine's writes and its stop (like `signin.rs`'s worker).
    fn lock_vault_now(&self) {
        let engine = self.engine.clone();
        on_worker("lock-vault", move || {
            let _ = engine.lock();
            engine.wake_host();
        });
    }
}

/// Runs `job` on its own named thread; the caller never waits for it. A thread that cannot start is logged.
fn on_worker(name: &str, job: impl FnOnce() + Send + 'static) {
    if std::thread::Builder::new().name(name.into()).spawn(job).is_err() {
        log("a worker did not start");
    }
}

#[cfg(test)]
mod worker_tests {
    use std::sync::mpsc;
    use std::time::Duration;

    /// The wake cover's Sign in must not hold the UI thread while the engine locks.
    /// Mutant: call the job inline in `on_worker` -> red (the caller blocks on the job's own gate).
    #[test]
    fn on_worker_returns_before_the_job_finishes_and_runs_it_elsewhere() {
        let (release, gate) = mpsc::channel::<()>();
        let (done, finished) = mpsc::channel();
        let caller = std::thread::current().id();
        super::on_worker("test-worker", move || {
            gate.recv().unwrap();
            done.send(std::thread::current().id()).unwrap();
        });
        assert!(finished.try_recv().is_err(), "the caller got back while the job still waits");
        release.send(()).unwrap();
        let ran_on = finished.recv_timeout(Duration::from_secs(5)).expect("the job ran");
        assert_ne!(ran_on, caller);
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
        let sync_root = self.engine.sync_root().ok();
        let read = |db: &StateDb| -> Result<Listing, DriveError> {
            let (mut rows, _total) = model::children_of(db, folder_id, EngineSort::default(), 0, usize::MAX)?;
            // A row whose place on this PC is another item's is marked: its presses on disk are refused.
            if let Some(sync_root) = &sync_root {
                case_place::mark_shared_places(db, sync_root, &mut rows);
            }
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

/// `FilesSource` on the real engine. Actions call through the `Engine` handle; what this PC holds of an item
/// (`keep_offline`, `free_up_space`) changes on a worker and is answered later ([`files::LocalCopies`]), and so
/// do Move to Trash and its Undo, each bound to the session of its press ([`files_trash::TrashCalls`]).
pub struct EngineFilesSource<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    /// Keep on this PC, Free up space, what they did, and the Backups folder's id.
    copies: files::LocalCopies<Engine<C>>,
    /// Move to Trash and its Undo.
    trash: files_trash::TrashCalls<C>,
    /// Copy link and the Share dialog's link.
    links: files_link::LinkCalls<C>,
    /// After a refused upload: the files of it that were not copied in.
    upload_left: Option<Vec<PathBuf>>,
    /// The sidebar's figures and each folder's sort.
    facts: files_facts::Facts<C>,
    /// The sidebar's Pinned folders.
    pins: files_pins::Pins<C>,
    /// A file's versions and the restore of one.
    versions: files_versions::VersionCalls<C>,
    /// The files in two versions and the answer to one.
    conflicts: files_conflicts::ConflictCalls<Engine<C>>,
    /// Where each row stands in the Backups tree.
    places: files_places::Places<C>,
    /// The Uploads panel: what uploads, Try again, Pause all, Cancel.
    uploads: files_uploads::UploadCalls<Engine<C>>,
    /// The copies coming down: the rows' "Downloading" figures.
    downloads: files_downloads::DownloadCalls<Engine<C>>,
    /// What the Preview panel shows of a file.
    previews: files_preview::Previews<viewer::LiveViewerEngine<C>>,
    /// Open With's apps per type (DW-1562), asked of Windows once per type.
    apps: open_with_apps::Apps,
    /// A start that did not happen after its press was accepted (Open, Show in File Explorer, an Open With app): said
    /// through `changes`.
    start_failed: Arc<Mutex<Vec<Change>>>,
    /// Files' Open and Show in File Explorer start here, never on the window's thread.
    later: launch::Later,
    /// The starts handed to `later` and not begun yet: a session end empties it (`forget_starts`).
    pending: files_launch::Pending,
    /// What starts a file or a folder (the system's own; a test's stand-in).
    starts: files_launch::Starts,
}

impl<C: CredentialStores> EngineFilesSource<C> {
    /// Starts no thread and reads nothing.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        Self {
            copies: files::LocalCopies::new(engine.clone()),
            trash: files_trash::TrashCalls::new(engine.clone()),
            uploads: files_uploads::UploadCalls::new(engine.clone()),
            downloads: files_downloads::DownloadCalls::new(engine.clone()),
            links: files_link::LinkCalls::new(engine.clone()),
            facts: files_facts::Facts::new(engine.clone()),
            pins: files_pins::Pins::new(engine.clone()),
            versions: files_versions::VersionCalls::new(engine.clone()),
            conflicts: files_conflicts::ConflictCalls::new(engine.clone()),
            places: files_places::Places::new(engine.clone()),
            previews: files_preview::Previews::new(Arc::new(viewer::LiveViewerEngine::new(engine.clone()))),
            apps: open_with_apps::Apps::default(),
            start_failed: Arc::new(Mutex::new(Vec::new())),
            later: launch::Later::default(),
            pending: files_launch::Pending::default(),
            starts: files_launch::Starts::default(),
            engine,
            upload_left: None,
        }
    }
}

/// The rows as the worker is asked for them: the id, and whether the row says it is kept on this PC.
fn local_items(items: &[DriveItem]) -> Vec<(String, bool)> {
    items
        .iter()
        .map(|item| (item.id.clone(), item.pc_status.as_deref() == Some("available_offline")))
        .collect()
}

/// What `upload_left` answers after a drop of which `copied` files went in, in the order dropped: the files that
/// are NOT in the folder yet. `None`: it was not refused, or nothing went in (Try again sends all of it).
fn upload_rest(files: &[PathBuf], copied: usize, refused: bool) -> Option<Vec<PathBuf>> {
    (refused && copied > 0).then(|| files.iter().skip(copied).cloned().collect())
}

/// The path the system is asked to open for the item `id`, read under the account-bound handle: below the
/// account's folder on this PC, and never a file of a kind that is not opened for safety. The ONE check behind
/// every hand-over of a file to another app (the Files window's Open, a viewer's "Open in another app…").
fn path_the_system_opens(db: &StateDb, sync_root: &Path, id: &str) -> Result<std::path::PathBuf, ActionError> {
    let entry = file_ops::resolve(db, id).map_err(map_drive_err)?;
    let path = file_ops::item_path_for(db, sync_root, &entry).map_err(map_drive_err)?;
    if !entry.is_dir() && file_ops::is_dangerous_executable(&path) {
        return Err(ActionError::Failed);
    }
    Ok(path)
}

/// The file `id` downloaded and decrypted to a copy of its own outside the Beebeeb folder, for an app to open: the
/// double-click on a file that is not on this PC yet. The copy is the answer's path. Blocks (it runs on the launch
/// worker, before the launch lock is taken); the press's toast already says it is downloading.
fn fetch_for_open<C: CredentialStores>(
    engine: &Engine<C>,
    turn: beebeeb_engine::account::SessionGeneration,
    id: &str,
    wanted: &Path,
) -> Result<PathBuf, ActionError> {
    use std::sync::atomic::AtomicBool;
    let name = wanted.file_name().ok_or(ActionError::Failed)?;
    let dest = std::env::temp_dir().join("beebeeb-open").join(id).join(name);
    std::fs::create_dir_all(dest.parent().ok_or(ActionError::Failed)?).map_err(|_| ActionError::Failed)?;
    save_copy::save_copy(engine, turn, id, &dest, &|_, _| {}, &AtomicBool::new(false)).map_err(|why| match why {
        save_copy::SaveRefused::NotSignedIn => ActionError::Unavailable,
        _ => ActionError::Failed,
    })?;
    Ok(dest)
}

impl<C: CredentialStores> EngineFilesSource<C> {
    /// A local file operation, on the signed-in account's local state (the account-bound handle, task 2488) and
    /// the sync root: both are required. Refused when sync is not running for this handle, as the published view
    /// says (`start == Running`; not `Engine::is_syncing`, which waits for the engine slot a start holds while it
    /// registers the folder, and this runs on the window's thread): changing a local folder that nothing syncs is not
    /// an honest operation. Refused too when the session changed while the operation ran.
    fn local<T>(&self, operation: impl FnOnce(&StateDb, &Path) -> Result<T, ActionError>) -> Result<T, ActionError> {
        if self.engine.view().start != Start::Running {
            return Err(ActionError::Unavailable);
        }
        let sync_root = self.engine.sync_root().map_err(|_| ActionError::Unavailable)?;
        self.engine
            .local_state_for_changes()
            .and_then(|state| state.with(|db| operation(db, &sync_root)))
            .map_err(|_| ActionError::Unavailable)?
    }
}

impl<C: CredentialStores> EngineFilesSource<C> {
    /// "Open in another app…": the path is resolved under the account-bound handle FIRST (the one check behind
    /// every hand-over of a file, [`path_the_system_opens`]); `choose` is asked only after that answered `Ok`.
    fn open_with_by(
        &self,
        item: &DriveItem,
        choose: impl FnOnce(&Path) -> Result<(), ActionError>,
    ) -> Result<(), ActionError> {
        if item.is_folder {
            return Err(ActionError::Failed);
        }
        let path = self.local(|db, sync_root| path_the_system_opens(db, sync_root, &item.id))?;
        choose(&path)
    }

    /// Start each of `paths` with `start` on the launch worker, in order, never on the window's thread (R-OW1): the
    /// paths were resolved at the press, under the account-bound handle. `Ok`: handed over. Each start asks for the
    /// press's session inside the launch lock and starts nothing once it moved; a session end drops what has not
    /// begun (`files_launch`). A start that fails is said through `changes` (the first one; the rest are not
    /// started), unless the session moved on since the press.
    fn start_later(
        &mut self,
        action: files_source::Action,
        paths: Vec<PathBuf>,
        start: files_launch::Start,
    ) -> Result<(), ActionError> {
        self.start_later_fetching(action, paths.into_iter().map(|path| (path, None)).collect(), start)
    }

    /// [`Self::start_later`], where a path paired with an id is downloaded first when no file is at it (a cloud-only
    /// file on a system with no placeholders: nothing else would fetch it), then started from the copy.
    fn start_later_fetching(
        &mut self,
        action: files_source::Action,
        paths: Vec<(PathBuf, Option<String>)>,
        start: files_launch::Start,
    ) -> Result<(), ActionError> {
        let turn = self.engine.session_turn();
        let (engine, failed) = (self.engine.clone(), self.start_failed.clone());
        let job = move || {
            let still = || engine.session_turn() == turn;
            let started = paths.iter().try_for_each(|(path, id)| {
                let path = match id {
                    Some(id) if !path.exists() => fetch_for_open(&engine, turn, id, path)?,
                    _ => path.clone(),
                };
                start(&path, &still)
            });
            if let Err(why) = started
                && still()
            {
                failed
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(Change::Failed(action, why));
                engine.wake_host();
            }
        };
        self.pending
            .run_later(&mut self.later, job)
            .map_err(|()| ActionError::Failed)
    }

    /// Starts `path` with the system's app when it has one for the kind (and the kind may be run), else in the file
    /// manager with the file selected, on the launch worker.
    fn start_outside(
        &mut self,
        path: PathBuf,
        id: Option<String>,
        openable: bool,
    ) -> Result<files_source::Outside, ActionError> {
        let default = openable && (self.starts.has_default_app)(&path);
        let (outside, start, fetch) = if default {
            (files_source::Outside::DefaultApp, self.starts.open, id)
        } else {
            (files_source::Outside::Explorer, self.starts.reveal, None)
        };
        self.start_later_fetching(files_source::Action::Open, vec![(path, fetch)], start)?;
        Ok(outside)
    }

    /// Is the file with this id kept on this PC, by the engine, for the session `turn`: kept (by its own pin or a kept
    /// folder's) AND its bytes here. A copy that is only listed, or still coming down, is not: handing it to an app
    /// would make Windows fetch it and decrypt it for that app.
    fn kept_here_for(&self, turn: beebeeb_engine::account::SessionGeneration, id: &str) -> bool {
        self.engine.edit_facts_for(turn, id, 0).is_ok_and(|facts| facts.kept)
            && self
                .engine
                .open_file_facts(id)
                .is_ok_and(|facts| facts.session == turn && facts.on_this_pc.is_some())
    }
}

impl<C: CredentialStores> FilesSource for EngineFilesSource<C> {
    fn take_rebuilt_notice(&mut self) -> Option<String> {
        self.engine.take_local_list_rebuilt_notice().map(str::to_string)
    }

    /// Files kept on this PC, once read (`files_facts`); the other two counts are not known here (0: no badge).
    fn counts(&self) -> Option<Counts> {
        self.facts.counts()
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
        self.facts.storage_split()
    }

    /// The status line's word for the handle's view (`sync::sync_word`): read without waiting, on every frame, and
    /// never "Up to date" for an engine that does not run.
    fn sync_state(&self) -> Option<String> {
        let word = files_downloads::with_downloads(
            sync::sync_word(&self.engine.view()),
            !self.downloads.downloads().is_empty(),
        );
        files_uploads::with_uploads(word, self.uploads.sending()).map(str::to_string)
    }

    fn details(&self, item: &DriveItem, folder: &str) -> Details {
        files_source::details_of_row(item, folder)
    }

    fn changed_label(&self, item: &DriveItem) -> Option<String> {
        files_facts::changed_label(item, chrono::Utc::now())
    }

    fn can_add(&self, _folder: Option<&str>) -> bool {
        // Writable while an engine runs on the folder (asked on every frame: the view, which never waits). The
        // operation itself goes through `local()`, which asks the engine again.
        self.engine.view().start == Start::Running
    }

    fn sort_of(&self, folder: Option<&str>) -> Option<crate::frames::files::Sort> {
        self.facts.sort_of(folder)
    }

    fn set_sort(&mut self, folder: Option<&str>, sort: crate::frames::files::Sort) {
        self.facts.set_sort(folder, sort);
    }

    /// An API call, not a sync-folder operation: not gated by `is_syncing()`. Accepted here, sent on a worker for
    /// the session of this press, answered through `changes`.
    fn trash(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.trash.trash(items)
    }

    /// Undo of a move to Trash, under the session of that move.
    fn restore(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.trash.restore(items)
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

    fn keep_offline(&mut self, items: &[DriveItem], keep: bool) -> Result<(), ActionError> {
        let action = if keep {
            files_source::Action::KeepOffline
        } else {
            files_source::Action::RemoveOffline
        };
        self.copies.ask(action, local_items(items))
    }

    /// Accepted here, asked of the engine on a worker for the session of this press, answered through `changes`:
    /// the window copies the link then.
    fn copy_link(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.links.copy(items)
    }

    fn link_pending(&self) -> bool {
        self.links.pending()
    }

    /// The Share dialog's link: as `copy_link`, a new link with the choices.
    fn share_link(
        &mut self,
        items: &[DriveItem],
        options: &crate::frames::files::actions_data::ShareOptions,
    ) -> Result<(), ActionError> {
        self.links.share(items, options)
    }

    fn open(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        // The paths are resolved under the handle; the OS is asked after it answered.
        let paths = self.local(|db, sync_root| {
            items
                .iter()
                .map(|item| path_the_system_opens(db, sync_root, &item.id))
                .collect::<Result<Vec<_>, _>>()
        })?;
        let open = self.starts.open;
        let paths = paths.into_iter().zip(items.iter().map(|item| Some(item.id.clone()))).collect();
        self.start_later_fetching(files_source::Action::Open, paths, open)
    }

    /// Other… (Windows' app chooser). It opens only while the session of this press is still the one once the
    /// launch lock is taken: another chooser that holds it may stay open across a lock or a sign-out.
    /// `open_file` steps 2 and 3: the app the system has for the kind, else the file manager with the file selected.
    /// The path is resolved under the account-bound handle first, as for every hand-over of a file; a kind that is not
    /// opened for safety (`path_the_system_opens` refuses it) is only ever shown in the file manager, never run.
    fn open_outside(&mut self, item: &DriveItem) -> Result<files_source::Outside, ActionError> {
        if item.is_folder {
            return Err(ActionError::Failed);
        }
        let (path, openable) = self.local(|db, sync_root| {
            let entry = file_ops::resolve(db, &item.id).map_err(map_drive_err)?;
            let path = file_ops::item_path_for(db, sync_root, &entry).map_err(map_drive_err)?;
            Ok((path, path_the_system_opens(db, sync_root, &item.id).is_ok()))
        })?;
        self.start_outside(path, Some(item.id.clone()), openable)
    }

    fn open_path_outside(&mut self, path: &Path) -> Result<files_source::Outside, ActionError> {
        self.start_outside(path.to_path_buf(), None, true)
    }

    fn open_with(&mut self, item: &DriveItem) -> Result<(), ActionError> {
        let (engine, turn) = (self.engine.clone(), self.engine.session_turn());
        self.open_with_by(item, move |path| {
            choose_app(path, move || engine.session_turn() == turn)
        })
    }

    /// Kept AND its bytes here, by the engine's facts now (Open With's lines other than Beebeeb ask it at the press).
    fn kept_here(&self, item: &DriveItem) -> bool {
        !item.is_folder && self.kept_here_for(self.engine.session_turn(), &item.id)
    }

    fn forget_starts(&mut self) {
        self.pending.forget();
    }

    /// The apps Windows reports for the file's type (DW-1562): what is held; a type not asked yet is asked once on a
    /// COM thread, and the window is woken when they came. Never waits.
    fn apps_for(&self, item: &DriveItem) -> Vec<files_source::AppChoice> {
        if item.is_folder {
            return Vec::new();
        }
        let engine = self.engine.clone();
        self.apps.for_name(&item.name, move || engine.wake_host())
    }

    /// Open With ▸ an app (DW-1562): any copy, as for the default app (Cloud Files fetches a placeholder), its
    /// path resolved under the account-bound handle at the press (never a path from the menu's time, never one made
    /// from a name). Nothing is copied here. The app starts on a COM thread, only while the
    /// press's session is still the one inside the launch lock; a start that fails there is said through `changes`,
    /// unless the session moved on since the press.
    fn open_in_app(&mut self, item: &DriveItem, app: &files_source::AppChoice) -> Result<(), ActionError> {
        if item.is_folder {
            return Err(ActionError::Failed);
        }
        // One rule with `open_outside`: the person asked, so a cloud-only placeholder is handed over too and Cloud Files
        // hydrates it. Refused only when sync is off or the vault is locked (`local`: `Unavailable`).
        let turn = self.engine.session_turn();
        let path = self.local(|db, sync_root| path_the_system_opens(db, sync_root, &item.id))?;
        let (engine, failed) = (self.engine.clone(), self.start_failed.clone());
        let now = self.engine.clone();
        let still = move || now.session_turn() == turn;
        open_with_apps::open_in(self.starts.app, path, app.id.clone(), still, move |answer| {
            if answer.is_err() && engine.session_turn() == turn {
                failed
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(Change::Failed(files_source::Action::OpenWith, ActionError::Failed));
                engine.wake_host();
            }
        })
    }

    /// Invite someone… (DW-0514): the item's own id while it is one of the account's own (`Engine::share_target_of`,
    /// the account-bound local state only: nothing is asked of the server on the press).
    fn share_of(&self, item: &DriveItem) -> Option<String> {
        self.engine.share_target_of(self.engine.session_turn(), &item.id)
    }

    fn pinned(&self) -> Vec<DriveItem> {
        self.pins.pinned()
    }

    fn pinned_ids(&self) -> Vec<String> {
        self.pins.pinned_ids()
    }

    fn set_pinned(&mut self, folder: &DriveItem, pin: bool) -> Result<(), ActionError> {
        self.pins.set(folder, pin)
    }

    fn refresh_pins(&mut self) {
        self.pins.refresh();
    }

    fn item_of(&self, id: &str) -> Option<DriveItem> {
        // Not `local`: no sync root is needed to read one row, and a listing reads without one too.
        let state = self.engine.local_state_for_changes().ok()?;
        state.with(|db| files_item_of::item_of(db, id)).ok().flatten()
    }

    fn versions(&self, item: &DriveItem) -> Vec<crate::frames::files::actions_data::Version> {
        self.versions.versions(item)
    }

    fn ask_versions(&mut self, item: &DriveItem) {
        self.versions.ask(item);
    }

    fn versions_state(&self, item: &DriveItem) -> files_source::Read {
        self.versions.state(item)
    }

    fn restore_version(&mut self, item: &DriveItem, version: &str) -> Result<(), ActionError> {
        self.versions.restore(item, version)
    }

    fn conflicts(&self) -> Vec<crate::frames::files::actions_data::Conflict> {
        self.conflicts.conflicts()
    }

    fn keep_version(&mut self, file: &str, keep: crate::frames::files::actions_data::Keep) -> Result<(), ActionError> {
        self.conflicts.keep(file, keep)
    }

    fn preview(&mut self, item: &DriveItem) -> Option<crate::frames::files::actions_data::PreviewOf> {
        // Answered later: the read starts here, and the panel asks `preview_later` until it lands.
        self.previews.start(item);
        None
    }

    fn preview_later(&mut self, item: &DriveItem) -> Option<crate::frames::files::actions_data::Previewed> {
        Some(self.previews.later(item))
    }

    fn cancel_preview(&mut self) {
        self.previews.cancel();
    }

    fn folder_shown(&mut self, folder: Option<&str>) {
        self.places.shown(folder);
    }

    fn protection(
        &self,
        folder: Option<&str>,
        item: Option<&DriveItem>,
    ) -> Option<crate::frames::files::protected::Protection> {
        self.places.protection(folder, item)
    }

    fn backup_entry(&self, item: &DriveItem) -> Option<crate::frames::files::protected::BackupEntry> {
        self.places.backup_entry(item)
    }

    fn show_in_file_explorer(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        let paths = self.local(|db, sync_root| {
            let mut paths = Vec::with_capacity(items.len());
            for item in items {
                let entry = file_ops::resolve(db, &item.id).map_err(map_drive_err)?;
                paths.push(file_ops::item_path_for(db, sync_root, &entry).map_err(map_drive_err)?);
            }
            Ok(paths)
        })?;
        let reveal = self.starts.reveal;
        self.start_later(files_source::Action::ShowInFileExplorer, paths, reveal)
    }

    fn upload(&mut self, folder: Option<&str>, files: &[std::path::PathBuf]) -> Result<(), ActionError> {
        // One file at a time, so a refusal part-way knows what is already in the folder: Try again sends the rest
        // alone (`upload_left`), never a second copy of a file that went in.
        self.upload_left = None;
        // How many files are in: the copies stop at the first refusal, so what is left is the files from there on.
        let mut copied = 0;
        let result = self.local(|db, sync_root| {
            let parent = file_ops::resolve_folder(db, folder).map_err(map_drive_err)?;
            for file in files {
                file_ops::copy_in(db, sync_root, parent.as_ref(), std::slice::from_ref(file)).map_err(map_drive_err)?;
                copied += 1;
            }
            Ok(())
        });
        // Also when every file is in and the answer was still refused (the session changed during the last
        // copy): an EMPTY list, which the window reads as "nothing to try again" (`None` would send all again).
        self.upload_left = upload_rest(files, copied, result.is_err());
        if result.is_ok() {
            // What was just accepted uploads now: the panel reads it.
            self.uploads.ask();
        }
        result
    }

    fn uploads(&self) -> Option<crate::frames::files::uploads::Uploads> {
        self.uploads.uploads()
    }

    fn downloads(&self) -> Vec<crate::frames::files::keeping::Download> {
        self.downloads.downloads()
    }

    fn ask_uploads(&mut self) {
        self.uploads.ask();
        self.downloads.ask();
    }

    fn retry_upload(&mut self, id: &str) -> Result<(), ActionError> {
        self.uploads.retry(id)
    }

    fn cancel_upload(&mut self, id: &str) -> Result<(), ActionError> {
        self.uploads.cancel(id)
    }

    fn cancels_upload(&self, id: &str) -> bool {
        self.uploads.cancels(id)
    }

    fn pause_uploads(&mut self, pause: bool) -> Result<(), ActionError> {
        self.uploads.pause(pause)
    }

    fn forget_uploads(&mut self) {
        self.uploads.forget();
    }

    fn forget_downloads(&mut self) {
        self.downloads.forget();
    }

    fn upload_left(&mut self) -> Option<Vec<PathBuf>> {
        self.upload_left.take()
    }

    fn answers_later(&self) -> bool {
        true
    }

    fn wake_after(&mut self, after: std::time::Duration) {
        self.copies.wake_after(after);
    }

    fn backups_root(&self) -> Option<String> {
        self.copies.backups_root()
    }

    fn look_for_backups(&mut self) {
        self.copies.look_for_backups();
    }

    fn new_folder(&mut self, parent: Option<&str>, name: &str) -> Result<(), ActionError> {
        // RD-b: every refusal is told in the log (never the name: it is the person's).
        let at_root = parent.is_none();
        let done = self.local(|db, sync_root| {
            let parent_entry = file_ops::resolve_folder(db, parent).map_err(|why| {
                tracing::warn!(at_root, ?why, "new folder: the parent folder was not found");
                map_drive_err(why)
            })?;
            file_ops::create_folder(db, sync_root, parent_entry.as_ref(), name).map_err(|why| {
                tracing::warn!(at_root, ?why, "new folder: refused");
                map_drive_err(why)
            })?;
            Ok(())
        });
        match &done {
            Ok(()) => tracing::info!(at_root, "new folder: made"),
            Err(why) => tracing::warn!(at_root, ?why, "new folder: failed"),
        }
        done
    }

    fn new_file(&mut self, folder: Option<&str>, name: &str) -> Result<(), ActionError> {
        let made = self.local(|db, sync_root| {
            let folder_entry = file_ops::resolve_folder(db, folder).map_err(map_drive_err)?;
            file_ops::create_file(db, sync_root, folder_entry.as_ref(), name, &[]).map_err(map_new_file_err)
        })?;
        // w36: listed and queued now, not when the folder scan next finds it (the window reads the folder again
        // right after this, `frames::files::create_file`). Not queued here: the scan finds it, as before.
        self.engine.list_new_file(&made);
        Ok(())
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

    fn free_up_space(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.copies.ask(files_source::Action::FreeUpSpace, local_items(items))
    }

    fn open_in_file_explorer(&mut self, folder: Option<&str>) -> Result<(), ActionError> {
        let path = self.local(|db, sync_root| match folder {
            Some(id) => {
                let entry = file_ops::resolve(db, id).map_err(map_drive_err)?;
                file_ops::item_path_for(db, sync_root, &entry).map_err(map_drive_err)
            }
            None => Ok(sync_root.to_path_buf()),
        })?;
        let open = self.starts.open;
        self.start_later(files_source::Action::OpenInFileExplorer, vec![path], open)
    }

    /// What Move to Trash, its Undo, a link that was asked for, Keep on this PC and Free up space did, once the
    /// engine answered (not an activity feed).
    fn changes(&mut self) -> Vec<Change> {
        let mut changes = self.trash.take();
        changes.extend(self.uploads.take());
        changes.extend(self.links.take());
        let kept = self.copies.take();
        if !kept.is_empty() {
            // A copy that was kept comes down now: read how far.
            self.downloads.ask();
        }
        self.downloads.take();
        changes.extend(kept);
        changes.extend(self.versions.take());
        changes.extend(self.conflicts.take());
        changes.append(&mut self.start_failed.lock().unwrap_or_else(PoisonError::into_inner));
        // Where the rows stand in the Backups tree: an arrival redraws by its wake.
        self.places.pump();
        // The sidebar's figures are read again after a change (`files_facts`); an arrival redraws by its wake.
        self.facts.pump(!changes.is_empty());
        self.pins.pump();
        changes
    }
}
