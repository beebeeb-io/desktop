//! `drive://event`: what the window hears after a change. The engine applies changes on its own
//! schedule (a pull, a watcher event, a Cloud Files callback), so one poller started by the first
//! drive command watches the state DB's change cursor and indexed counts, and emits `changed` and
//! `sync` when they move. The commands that change something also emit `changed` for the folder
//! at once. Payloads are `DriveEvent` view data only.

use std::sync::OnceLock;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tauri::{Emitter, Manager};

use super::dto::{DriveEvent, SyncSummary};
use super::model;
use crate::AppState;

pub(crate) const EVENT: &str = "drive://event";
const TICK: Duration = Duration::from_secs(2);

static STARTED: OnceLock<()> = OnceLock::new();

pub(crate) fn emit(app: &tauri::AppHandle, event: &DriveEvent) {
    if let Err(error) = app.emit(EVENT, event) {
        tracing::debug!(%error, "drive event not sent");
    }
}

pub(crate) fn changed(app: &tauri::AppHandle, folder_id: Option<String>) {
    emit(app, &DriveEvent::Changed { folder_id });
}

/// The engine's mirrored state and the pause switch of the active account; `None` when nobody is
/// unlocked.
pub(crate) fn engine_view(state: &AppState) -> Option<(String, bool)> {
    let acct = state.active_account().ok()?;
    if !acct.session.lock().map(|g| g.is_some()).unwrap_or(false) {
        return None;
    }
    let raw = acct.engine_state.lock().map(|g| g.clone()).unwrap_or_default();
    Some((raw, acct.sync_paused.load(Ordering::Relaxed)))
}

/// Starts the poller once per process.
pub(crate) fn ensure_started(app: &tauri::AppHandle) {
    if STARTED.set(()).is_err() {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("drive-events".into())
        .spawn(move || run(app));
    if let Err(error) = spawned {
        tracing::warn!(%error, "drive event poller did not start");
    }
}

#[derive(PartialEq)]
struct Seen {
    anchor: Option<Vec<u8>>,
    counts: (u32, u32, u32, u32, u32),
}

fn run(app: tauri::AppHandle) {
    let mut last_seen: Option<Seen> = None;
    let mut last_sync: Option<SyncSummary> = None;
    loop {
        std::thread::sleep(TICK);
        let Some((raw, paused)) = engine_view(&app.state::<AppState>()) else {
            last_seen = None;
            continue;
        };
        let Ok(Some(db)) =
            crate::config::DesktopConfig::load().and_then(|cfg| crate::local_state_purge::state_db_for_config(&cfg))
        else {
            continue;
        };
        let counts = model::sync_counts(&db);
        let seen = Seen {
            anchor: db.fp_last_anchor().ok().flatten(),
            counts: (
                counts.on_device,
                counts.online_only,
                counts.moving,
                counts.conflicts,
                counts.on_hold,
            ),
        };
        let moved = last_seen.as_ref() != Some(&seen);
        if moved && last_seen.is_some() {
            changed(&app, None);
        }
        let phase = model::sync_phase(true, &raw, paused, &counts);
        let offline = if moved || last_sync.is_none() {
            model::available_offline_count(&db)
        } else {
            last_sync.as_ref().map_or(0, |s| s.available_offline)
        };
        let sync = model::sync_summary(phase, &counts, offline);
        if last_sync.as_ref() != Some(&sync) {
            emit(&app, &DriveEvent::Sync { sync: sync.clone() });
            last_sync = Some(sync);
        }
        last_seen = Some(seen);
    }
}
