//! The flyout of the running app on the real engine (`frames::tray::on_engine` puts it in place of the
//! board's fixture).
//!
//! [`EngineTraySource`] gathers what the flyout is drawn from ([`Facts`]): the session gate and the two
//! switches from the handle; the engine's last report of itself from the host's fold (`LiveEngine::drain`,
//! taken first on every read, so the flyout is current also while the main window is closed); the counts
//! and the history from the state database on this PC, only while the account's keys are in memory; the
//! account's storage from Beebeeb, through #322's `EngineDriveSource::storage` on a thread of its own, at
//! most once a minute. The words are `frames::tray::live`'s.
//!
//! Must not: read the board's fixture; wait for the network on the window's thread; log a name, a path
//! or anything a report carries.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::backup_protect::BackupsTree;
use beebeeb_engine::drive_v2::model as drive_model;
use beebeeb_engine::engine_status::{EngineStatusView, REASON_AUTH, REASON_ENGINE_ERROR};
use beebeeb_engine::signin::gate::GateState as EngineGate;
use beebeeb_engine::state_db::{FileStatus, LocalActivityKind, StateDb};

use super::{EngineDriveSource, EngineFilesSource, LiveEngine, log};
use crate::frames::files::source::FilesSource;
use crate::frames::tray::live::{self, Facts, Gate, Local, Progress, Report, Row, RowKind};
use crate::frames::tray::source::{Action, MAX_ROWS, Snapshot, TraySource};
use crate::shell::identity::AppIdentity;
use crate::shell::tray::{EngineState, TrayInputs};
use crate::source::{DriveItem, DriveSource};

#[cfg(test)]
mod tests;

/// The account's storage is read again from Beebeeb at most this often.
const STORAGE_EVERY: Duration = Duration::from_secs(60);

/// How many rows past [`MAX_ROWS`] a history read takes, so the rows LD-0113 leaves out (a device's
/// manifest in Backups, rewritten by every backup pass) do not shorten the list. The engine's own tray
/// list reads the same way (`status_views`).
const HIDDEN_SLACK: usize = 16;

/// The flyout's [`TraySource`] on the real engine.
pub struct EngineTraySource<C: CredentialStores> {
    live: Arc<LiveEngine<C>>,
    /// Beebeeb is in File Explorer, as the shell's tray state says ([`TraySource::follow`]).
    explorer_ready: bool,
    storage: Arc<Mutex<StorageRead>>,
}

/// The account's storage as last read, and whose it is.
#[derive(Default)]
struct StorageRead {
    /// The account it was read for (`account_of`): another account never shows it.
    account: Option<String>,
    used_and_quota: Option<(u64, u64)>,
    /// When the last read began; `None` before the first.
    began: Option<Instant>,
    reading: bool,
}

impl<C: CredentialStores> EngineTraySource<C> {
    pub fn new(live: Arc<LiveEngine<C>>) -> Self {
        Self {
            live,
            explorer_ready: false,
            storage: Arc::default(),
        }
    }

    /// What the flyout is drawn from, read now. The engine's queued reports are taken first.
    pub fn facts(&self) -> Facts {
        self.live.drain();
        let engine = &self.live.engine;
        let gate = gate_of(engine.gate());
        let running = engine.is_syncing();
        let paused = engine.is_paused();
        let view = self.live.status_view();
        let report = report_of(&view);
        let ready = gate == Gate::Ready;
        // A file's "uploading" or "downloading" is a file on its way only while sync runs and reaches Beebeeb.
        let moving = running && !paused && matches!(report, Report::Syncing | Report::Idle);
        let local = if ready {
            engine.state_db().ok().map(|db| local_of(&db, now(), moving))
        } else {
            None
        };
        Facts {
            gate,
            session_ended: session_ended(engine),
            running,
            paused,
            report,
            progress: progress_of(&view),
            explorer_ready: self.explorer_ready,
            local,
            storage: if ready { self.storage() } else { None },
        }
    }

    /// The account's storage as last read, while it is the account signed in now. Starts a read on a thread
    /// of its own when the last one began [`STORAGE_EVERY`] ago or longer and none is under way; a read that
    /// fails keeps the last value.
    fn storage(&self) -> Option<(u64, u64)> {
        let account = account_of(&self.live.engine);
        let mut read = self.storage.lock().unwrap_or_else(PoisonError::into_inner);
        if !read.reading && read.began.is_none_or(|began| began.elapsed() >= STORAGE_EVERY) {
            read.reading = true;
            read.began = Some(Instant::now());
            let (engine, shared, whose) = (self.live.engine.clone(), self.storage.clone(), account.clone());
            let started = std::thread::Builder::new().name("tray-storage".into()).spawn(move || {
                let summary = EngineDriveSource::new(engine).storage();
                let mut read = shared.lock().unwrap_or_else(PoisonError::into_inner);
                read.reading = false;
                if let Some(summary) = summary {
                    read.account = whose;
                    read.used_and_quota = Some((
                        u64::try_from(summary.used_bytes).unwrap_or(0),
                        u64::try_from(summary.quota_bytes).unwrap_or(0),
                    ));
                }
            });
            if started.is_err() {
                read.reading = false;
            }
        }
        read.used_and_quota
            .filter(|_| account.is_some() && read.account == account)
    }
}

impl<C: CredentialStores> TraySource for EngineTraySource<C> {
    fn snapshot(&self) -> Snapshot {
        live::snapshot(&self.facts(), AppIdentity::CURRENT.product_name(), chrono::Local::now())
    }

    /// Pause and Resume reach the engine through the shell, Review…, Show and Sign in move the window
    /// (`frames::tray::Intent`): nothing is left for the source to do.
    fn act(&mut self, _: Action) {}

    /// Shows the file in File Explorer, selected in its folder: #322's `show_in_file_explorer`, with its
    /// guard (refused while sync does not run on this PC). A row without an id does nothing.
    fn reveal(&mut self, id: &str) {
        if id.is_empty() {
            return;
        }
        let mut files = EngineFilesSource::new(self.live.engine.clone());
        if let Err(error) = files.show_in_file_explorer(&[row_item(id)]) {
            log(format!("the flyout could not show a file in File Explorer ({error:?})"));
        }
    }

    fn follow(&mut self, shell: &TrayInputs) {
        self.explorer_ready = shell.explorer_ready;
    }
}

/// The row as #322's file actions take it: they read the id alone.
fn row_item(id: &str) -> DriveItem {
    DriveItem {
        id: id.to_string(),
        parent_id: None,
        name: String::new(),
        is_folder: false,
        size: None,
        child_count: None,
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Whose storage a read is: the user id the server named, else the stored email.
fn account_of<C: CredentialStores>(engine: &Engine<C>) -> Option<String> {
    engine.user_id().or_else(|| engine.account_email())
}

fn gate_of(gate: EngineGate) -> Gate {
    match gate {
        EngineGate::SignedOut => Gate::SignedOut,
        EngineGate::Locked => Gate::Locked,
        EngineGate::NeedsRecoveryPhrase => Gate::NeedsRecoveryPhrase,
        EngineGate::NoPlan => Gate::NoPlan,
        EngineGate::KeyStoreUnavailable => Gate::KeyStoreUnavailable,
        EngineGate::Ready => Gate::Ready,
    }
}

/// Beebeeb refused this account's session again and again (`AuthHealth`): it ended (DW-1219).
pub(super) fn session_ended<C: CredentialStores>(engine: &Engine<C>) -> bool {
    engine
        .accounts()
        .active_account()
        .is_ok_and(|account| account.auth_health.is_expired())
}

/// The engine's last report, in the flyout's terms. An `error` with the `auth` reason is Beebeeb
/// answering 401, not a way to Beebeeb that failed: the session decides ([`session_ended`]), and until
/// it does the state is not known. Before the first report, and after a resume, nothing is known.
pub(super) fn report_of(view: &EngineStatusView) -> Report {
    match (view.state.as_str(), view.reason_code.as_deref()) {
        ("running", _) => Report::Starting,
        ("idle", _) => Report::Idle,
        ("syncing", _) => Report::Syncing,
        ("offline", _) => Report::Offline,
        ("error", Some(REASON_AUTH)) => Report::Unknown,
        ("error", Some(REASON_ENGINE_ERROR)) => Report::Failed,
        ("error", _) => Report::ServerError,
        ("paused" | "stopped", _) => Report::Quiet,
        _ => Report::Unknown,
    }
}

/// The b's engine state from the same report: the flyout's "Can't reach" is the b's offline, an error on
/// this PC its stopped.
pub(super) fn engine_state(view: &EngineStatusView) -> EngineState {
    match report_of(view) {
        Report::Syncing => EngineState::Syncing,
        Report::Offline | Report::ServerError => EngineState::Offline,
        Report::Failed => EngineState::Error,
        Report::Unknown | Report::Starting | Report::Idle | Report::Quiet => EngineState::Idle,
    }
}

fn progress_of(view: &EngineStatusView) -> Progress {
    Progress {
        files_remaining: view.files_remaining,
        bytes_total: view.bytes_total,
        bytes_done: view.bytes_done,
    }
}

/// What the state database on this PC says, for the flyout, at `now` (unix seconds). `moving`: sync runs,
/// so a file the database has as uploading or downloading is listed as on its way. Every read is bounded:
/// counts, and at most `MAX_ROWS + HIDDEN_SLACK` rows of each kind. A read that fails counts as nothing.
pub(super) fn local_of(db: &StateDb, now: i64, moving: bool) -> Local {
    let counts = drive_model::sync_counts(db);
    let hold = db.queue_hold_summary().ok();
    let count = |n: i64| u32::try_from(n.max(0)).unwrap_or(u32::MAX);
    let gave_up = hold.as_ref().map_or(0, |hold| count(hold.gave_up));
    let backups = BackupsTree::read(db);
    let listed = |path: &str| !backups.is_hidden(path);
    let read = MAX_ROWS + HIDDEN_SLACK;

    // On its way now: the newest files the database has as moving.
    let mut rows: Vec<Row> = Vec::new();
    if moving {
        let newest = db.newest_file_rows(read).unwrap_or_default();
        rows.extend(newest.into_iter().filter_map(|(entry, _)| {
            let bytes = u64::try_from(entry.size_bytes).unwrap_or(0);
            let kind = match entry.status {
                FileStatus::Uploading => RowKind::Uploading { bytes },
                FileStatus::Downloading => RowKind::Downloading { bytes },
                _ => return None,
            };
            listed(&entry.path).then_some(Row {
                id: Some(entry.file_id),
                path: entry.path,
                at: now,
                kind,
            })
        }));
    }
    // Given up: what Beebeeb stopped retrying, newest first. A backup's file is on this PC outside the
    // sync folder (LD-0113): the status counts it, the list leaves it out.
    if gave_up > 0 {
        let unsent = db
            .unsent_changes(0, u32::try_from(read).unwrap_or(u32::MAX), now)
            .map(|page| page.items)
            .unwrap_or_default();
        let mut failed: Vec<Row> = unsent
            .into_iter()
            .filter(|change| change.group == "gave_up" && !change.backup && listed(&change.path))
            .map(|change| Row {
                id: change.file_id,
                path: change.path,
                at: change.queued_at,
                kind: RowKind::Failed,
            })
            .collect();
        failed.sort_by_key(|row| std::cmp::Reverse(row.at));
        rows.extend(failed);
    }
    // What happened: the finished transfers and the moves to Trash, newest first.
    let mut happened: Vec<Row> = Vec::new();
    let transfers = db.list_recent_transfer_activity(read).unwrap_or_default();
    happened.extend(transfers.into_iter().filter_map(|done| {
        let kind = match done.direction.as_str() {
            "up" => RowKind::Uploaded,
            "down" => RowKind::Downloaded,
            _ => return None,
        };
        let path = done.rel_path.filter(|path| !path.is_empty()).unwrap_or(done.file_name);
        listed(&path).then_some(Row {
            id: done.file_id,
            path,
            at: done.occurred_at,
            kind,
        })
    }));
    let events = db.list_recent_local_activity(read).unwrap_or_default();
    happened.extend(events.into_iter().filter_map(|event| {
        if !matches!(event.event_type, LocalActivityKind::MovedToTrash) {
            return None;
        }
        let path = event
            .rel_path
            .filter(|path| !path.is_empty())
            .unwrap_or(event.file_name);
        listed(&path).then_some(Row {
            id: event.file_id,
            path,
            at: event.occurred_at,
            kind: RowKind::MovedToTrash,
        })
    }));
    happened.sort_by_key(|row| std::cmp::Reverse(row.at));
    rows.extend(happened);
    rows.truncate(MAX_ROWS);

    Local {
        // Not wired: the engine counts files and folders together (`sync_counts`); counting the files alone
        // reads every row, too slow for each refresh of the flyout.
        files: None,
        conflicts: counts.conflicts,
        gave_up,
        unsent: hold.as_ref().map_or(0, |hold| count(hold.waiting)),
        held_for_quota: hold.as_ref().map_or(0, |hold| {
            hold.held
                .iter()
                .filter(|group| group.reason == "quota")
                .fold(0, |held, group| held.saturating_add(count(group.count)))
        }),
        moving: counts.moving,
        rows,
    }
}
