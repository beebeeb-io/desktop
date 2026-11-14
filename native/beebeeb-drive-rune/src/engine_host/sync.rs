//! Sync on the native side of the engine handle (contract
//! `docs/superpowers/specs/2026-11-12-native-sync-start-and-sign-out.md`, §4.3, S1): the start worker, one at a
//! time; the words the status line says for the handle's view; the refusal the modal shows; and the two banner
//! fields S1 feeds (offline, the session ended).
//!
//! The UI thread reads only `Engine::view()` here, which never waits: no `gate()`, no `is_syncing()`, no
//! `is_paused()` (each of those takes a lock a start holds while it registers the folder).
//!
//! Never logged or shown: a path. The folder appears by its name only, in the modal (the person's own screen).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use beebeeb_engine::app::{CredentialStores, Engine, SyncStart, SyncStartRefused};
use beebeeb_engine::engine_status::{EngineStatusView, State as Reported};
use beebeeb_engine::signin::gate::GateState as EngineGate;
use beebeeb_engine::sync_view::{Explorer, Start, SyncView};

use crate::frames::global::samples::OFFLINE;
use crate::frames::global::source::{GlobalSource, GlobalStatus, ListedAt};
use crate::frames::global::sync_off::{Refused, SyncOff};
use crate::shell::tray::{EngineState, TrayInputs};
use crate::ui::status_line as words;

/// At most one `start-sync` worker at a time (contract §4.3). A start asked while one runs is answered by that one:
/// the view says where it got to. The folder's name is kept for the refusal modal.
#[derive(Default)]
pub struct StartWorker {
    busy: Arc<AtomicBool>,
    /// The name of the folder the last start asked for, never its path.
    folder: Arc<Mutex<Option<String>>>,
}

/// Clears the worker's flag however the worker ends (a panic included).
struct Done(Arc<AtomicBool>);

impl Drop for Done {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl StartWorker {
    /// Runs `start` on a `start-sync` thread, unless a start runs: `Ok(true)` when this call started one,
    /// `Ok(false)` when the running one answers it.
    pub fn ask(&self, start: impl FnOnce() + Send + 'static) -> Result<bool, std::io::Error> {
        if self.busy.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }
        let done = Done(self.busy.clone());
        std::thread::Builder::new()
            .name("start-sync".into())
            .spawn(move || {
                let _done = done;
                start();
            })
            .map(|_| true)
    }

    /// The folder's name for the start worker to keep.
    pub fn folder(&self) -> Arc<Mutex<Option<String>>> {
        self.folder.clone()
    }

    /// The name of the folder the last start asked for.
    pub fn folder_name(&self) -> Option<String> {
        self.folder.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// Start sync into the folder the settings name, else the identity's suggestion (`Engine::sync_root`), and keep its
/// name for the modal. Blocks: a `start-sync` worker's. The log line names the refusal's kind only.
pub fn start_sync_now<C: CredentialStores>(engine: &Engine<C>, folder: &Mutex<Option<String>>) {
    let sync_root = match engine.sync_root() {
        Ok(sync_root) => sync_root,
        Err(_) => return super::log("sync was not started: the settings could not be read"),
    };
    *folder.lock().unwrap_or_else(PoisonError::into_inner) = name_of(&sync_root);
    match engine.start_sync(sync_root) {
        Ok(SyncStart::Started) => super::log("sync started"),
        Ok(SyncStart::AlreadyRunning) => {}
        Ok(SyncStart::NoSession) => super::log("sync was not started: no session"),
        Err(refused) => super::log(format!("sync was not started: {}", refused.kind())),
    }
}

fn name_of(folder: &Path) -> Option<String> {
    folder.file_name().map(|name| name.to_string_lossy().into_owned())
}

/// The status line's word for the view (DW-0401's `.mk-foot`); `None` while the window shows no Files (signed out,
/// locked, the phrase). Never "Up to date", "Syncing" or "Paused" without a running engine (invariant I12), and "Up
/// to date" only after the running engine finished a check that reached Beebeeb (DW-1212: never a guess).
pub fn sync_word(view: &SyncView) -> Option<&'static str> {
    if view.gate != EngineGate::Ready {
        return None;
    }
    if view.session_ended {
        return Some(words::SIGN_IN_AGAIN);
    }
    Some(match view.start {
        Start::NotAsked | Start::Refused(_) => words::SYNC_IS_OFF,
        Start::Starting => words::STARTING,
        Start::Running if view.paused => words::PAUSED,
        Start::Running => match view.reported {
            Some(Reported::Offline) => OFFLINE,
            Some(Reported::Syncing) => words::SYNCING,
            Some(Reported::Idle) if view.last_check_at.is_some() => words::UP_TO_DATE,
            Some(Reported::Error) => words::SYNC_STOPPED,
            _ => words::CHECKING,
        },
    })
}

/// The b's inputs for the handle's view and the engine's last report (`LiveEngine::status_view`, the flyout reads the
/// same): not running is the b's "not syncing" (a refused start included); running, before a finished check, its
/// "checking"; "up to date" only after the engine reported a finished check.
pub fn tray_inputs_of(sync: &SyncView, status: &EngineStatusView) -> TrayInputs {
    let gate = sync.gate;
    let session_expired = gate != EngineGate::SignedOut && sync.session_ended;
    let running = sync.start == Start::Running;
    TrayInputs {
        signed_in: gate != EngineGate::SignedOut && !session_expired,
        session_expired,
        locked: matches!(gate, EngineGate::Locked | EngineGate::ScreenLocked),
        paused: sync.paused,
        explorer_ready: running && sync.explorer == Explorer::Added,
        engine: if running {
            super::tray::engine_state(status)
        } else {
            EngineState::NotRunning
        },
        files_remaining: status
            .files_remaining
            .filter(|_| running && !sync.paused && status.is_syncing()),
        ..TrayInputs::default()
    }
}

/// The refused attempt the modal shows (Q3), with the folder's name: while the view says the last start refused.
pub fn sync_refusal(view: &SyncView, folder: Option<String>) -> Option<SyncOff> {
    let Start::Refused(kind) = view.start else {
        return None;
    };
    Some(SyncOff {
        attempt: view.attempt,
        kind: refused_of(kind),
        folder,
    })
}

/// The engine's refusal in the modal's terms, one for one.
pub fn refused_of(kind: SyncStartRefused) -> Refused {
    match kind {
        SyncStartRefused::FolderOfAnotherProvider => Refused::FolderOfAnotherProvider,
        SyncStartRefused::FolderUnreadable => Refused::FolderUnreadable,
        SyncStartRefused::FolderMissing => Refused::FolderMissing,
        SyncStartRefused::RegistrationFailed { code } => Refused::RegistrationFailed { code },
        SyncStartRefused::OtherAccountsData => Refused::OtherAccountsData,
        SyncStartRefused::IdentityUnknown => Refused::IdentityUnknown,
        SyncStartRefused::StopUnconfirmed => Refused::StopUnconfirmed,
        SyncStartRefused::StateInUse => Refused::StateInUse,
        SyncStartRefused::Other => Refused::Other,
    }
}

/// The global banner's source on the real engine (S1 feeds two fields: the session ended, DW-0105; offline,
/// DW-0101). Reads the view only.
pub struct EngineGlobalSource<C: CredentialStores> {
    engine: Arc<Engine<C>>,
}

impl<C: CredentialStores> EngineGlobalSource<C> {
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        Self { engine }
    }
}

impl<C: CredentialStores> GlobalSource for EngineGlobalSource<C> {
    fn status(&self) -> GlobalStatus {
        global_status(&self.engine.view(), chrono::Local::now())
    }

    /// What the engine says now: a check happens on its own ticks, never from the window's thread.
    fn check_connection(&mut self) -> bool {
        !self.engine.view().offline
    }
}

/// The banner's two fields for `view`. Offline only while an engine runs and said so, with the time of its last
/// finished check ("This list is from today at 09:12"); before its first one the status line says it alone.
pub fn global_status(view: &SyncView, now: chrono::DateTime<chrono::Local>) -> GlobalStatus {
    let ready = view.gate == EngineGate::Ready;
    let running = ready && view.start == Start::Running;
    GlobalStatus {
        signed_out: ready && view.session_ended,
        offline: (running && view.offline)
            .then_some(view.last_check_at)
            .flatten()
            .and_then(|at| listed_at(at, now)),
        ..GlobalStatus::default()
    }
}

/// "today" / "yesterday" / "12 Oct", and the 24-hour time, in this PC's time zone.
fn listed_at(at: i64, now: chrono::DateTime<chrono::Local>) -> Option<ListedAt> {
    use chrono::TimeZone;
    let then = chrono::Local.timestamp_opt(at, 0).single()?;
    let day = match (now.date_naive() - then.date_naive()).num_days() {
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        _ => then.format("%-d %b").to_string(),
    };
    Some(ListedAt {
        day,
        time: then.format("%H:%M").to_string(),
    })
}

#[cfg(test)]
#[path = "sync_tests.rs"]
mod tests;
