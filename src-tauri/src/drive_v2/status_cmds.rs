//! Batch B02b: the commands behind `src/v2/source/extras.ts`, gathering what `status.rs` and `photo_meta.rs` map:
//! the transfer board, the engine's status view, the queue, the backup marks, the disks. View data only (spec 2.1).

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, Manager, State};

use super::DriveError;
use super::commands::{Answer, blocking, local, require_session};
use super::model;
use super::photo_meta::{self, PhotoMeta};
use super::status::{self, DriveStatus, OfflineEntry, SyncProblem, TransferRow};
use crate::AppState;
use crate::popover_data::PopoverRuntime;
use crate::state_db::{FileStatus, PinState, StateDb};

/// `drive://transfers`: the moving rows, about once a second while anything moves, and one empty list when it stops.
pub(crate) const TRANSFERS_EVENT: &str = "drive://transfers";
const TRANSFER_TICK: Duration = Duration::from_secs(1);
/// How many queued changes the Uploads panel and Sync history read.
const QUEUE_LIMIT: u32 = 500;

static TRANSFERS_STARTED: OnceLock<()> = OnceLock::new();

fn now() -> i64 {
    crate::local_state_purge::now_unix_seconds()
}

fn queue(db: &StateDb) -> Vec<crate::state_db::UnsentChange> {
    db.unsent_changes(0, QUEUE_LIMIT, now())
        .map(|page| page.items)
        .unwrap_or_default()
}

fn disks() -> Vec<(std::path::PathBuf, u64)> {
    sysinfo::Disks::new_with_refreshed_list()
        .list()
        .iter()
        .map(|d| (d.mount_point().to_path_buf(), d.available_space()))
        .collect()
}

#[tauri::command]
pub(crate) async fn drive_status(
    state: State<'_, AppState>,
    runtime: State<'_, PopoverRuntime>,
) -> Answer<DriveStatus> {
    let acct = state
        .active_account()
        .map_err(|_| String::from(DriveError::SignedOut))?;
    let unlocked = acct.session.lock().map(|g| g.is_some()).unwrap_or(false);
    let present = state.auth_present.lock().map(|g| *g).unwrap_or(false);
    let signed_out = !(unlocked || present) || acct.auth_health.is_expired();
    let board = runtime.transfers.snapshot_dtos();
    let listing = runtime
        .status
        .lock()
        .ok()
        .and_then(|v| v.startup_check)
        .filter(|c| c.total > 0 && c.done < c.total)
        .map(|c| status::ListingProgress {
            done: c.done,
            total: c.total,
        });
    blocking(move || {
        let (upload_bps, eta_seconds) = status::upload_rate(&board);
        let mut out = DriveStatus {
            signed_out,
            upload_bps,
            eta_seconds,
            listing,
            ..DriveStatus::default()
        };
        let cfg = crate::config::DesktopConfig::load().map_err(|_| DriveError::Failed("config"))?;
        if let Some(root) = cfg.sync_root.as_deref() {
            out.free_space_bytes = status::free_space_for(root, &disks());
        }
        // Names and the list are read only while the vault is unlocked, like every other drive read.
        let Some((db, _)) = local()?.filter(|_| unlocked) else {
            return Ok(out);
        };
        let rows = model::live_rows(&db)?;
        out.file_count = Some(rows.iter().filter(|r| !r.is_dir()).count() as u64);
        out.conflicts = rows
            .iter()
            .filter(|r| r.status == FileStatus::Conflict && !r.is_dir())
            .map(|r| status::ConflictRef {
                id: r.file_id.clone(),
                name: model::leaf_name(r),
            })
            .collect();
        let gave_up = db.queue_hold_summary().map(|q| q.gave_up).unwrap_or(0).max(0) as u32;
        let changes = queue(&db);
        let backup_stuck = changes.iter().filter(|c| c.backup && status::is_problem(c)).count();
        out.stopped_files = gave_up;
        out.sync_problems =
            changes.iter().filter(|c| !c.backup && status::is_problem(c)).count() as u32 + out.conflicts.len() as u32;
        let enabled: Vec<&str> = crate::known_folder::KNOWN_FOLDERS
            .iter()
            .map(|k| k.key)
            .filter(|k| cfg.known_folder_enabled(k))
            .collect();
        let marks: HashMap<String, Option<i64>> = enabled
            .iter()
            .map(|k| {
                (
                    k.to_string(),
                    db.backup_pass_mark(k).ok().and_then(|m| m.last_pass_at_ms),
                )
            })
            .collect();
        let stops = crate::backup::stop::stopped_backups(&cfg).len();
        out.backup = status::backup_view(&enabled, &marks, stops);
        out.backup.device_folder = db
            .backup_layout()
            .ok()
            .flatten()
            .map(|layout| layout.device_folder_name);
        out.backup_problems = (stops + backup_stuck) as u32;
        Ok(out)
    })
    .await
}

/// Starts the transfer emitter once per process.
fn ensure_transfers_started(app: &tauri::AppHandle) {
    if TRANSFERS_STARTED.set(()).is_err() {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("drive-transfers".into())
        .spawn(move || {
            let mut was_moving = false;
            loop {
                std::thread::sleep(TRANSFER_TICK);
                let board = app.state::<PopoverRuntime>().transfers.snapshot_dtos();
                let rows = status::transfer_rows(&board, &[]);
                if rows.is_empty() && !was_moving {
                    continue;
                }
                was_moving = !rows.is_empty();
                if let Err(error) = app.emit(TRANSFERS_EVENT, &rows) {
                    tracing::debug!(%error, "transfer event not sent");
                }
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "transfer emitter did not start");
    }
}

/// The Uploads panel (DW-0427): what moves now and what waits or stopped, with live progress after through
/// `drive://transfers`.
#[tauri::command]
pub(crate) async fn drive_transfers(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    runtime: State<'_, PopoverRuntime>,
) -> Answer<Vec<TransferRow>> {
    require_session(&state).map_err(String::from)?;
    ensure_transfers_started(&app);
    let board = runtime.transfers.snapshot_dtos();
    blocking(move || {
        let changes = local()?.map(|(db, _)| queue(&db)).unwrap_or_default();
        Ok(status::transfer_rows(&board, &changes))
    })
    .await
}

/// Sync history's problems (DW-0428), each with why in plain words.
#[tauri::command]
pub(crate) async fn drive_sync_problems(state: State<'_, AppState>) -> Answer<Vec<SyncProblem>> {
    require_session(&state).map_err(String::from)?;
    blocking(move || {
        Ok(local()?
            .map(|(db, _)| status::sync_problems(&queue(&db)))
            .unwrap_or_default())
    })
    .await
}

#[derive(Debug, Serialize)]
pub(crate) struct RetryAnswer {
    retried: u32,
}

/// Try again on Sync history and Uploads rows: each change is made due now (the queue's own `retry_unsent_change`
/// rule; a held, parked or blocked one is left as it is), then a running engine is nudged.
#[tauri::command]
pub(crate) async fn drive_sync_retry(state: State<'_, AppState>, ids: Vec<String>) -> Answer<RetryAnswer> {
    require_session(&state).map_err(String::from)?;
    let retried = blocking(move || {
        let Some((db, _)) = local()? else { return Ok(0) };
        let mut n = 0u32;
        for id in ids.iter().take(QUEUE_LIMIT as usize) {
            if db.retry_unsent_change(id, now()).map_err(DriveError::db)? == crate::state_db::RetryOutcome::Retried {
                n += 1;
            }
        }
        Ok(n)
    })
    .await?;
    if retried > 0 {
        let acct = state.active_account()?;
        let link = acct
            .engine
            .lock()
            .await
            .as_ref()
            .filter(|e| e.is_running())
            .map(|e| e.send_phase());
        if let Some(link) = link {
            link.nudge();
        }
    }
    Ok(RetryAnswer { retried })
}

/// Available offline (DW-0715): the top-most kept items, a folder with its file count and bytes.
#[tauri::command]
pub(crate) async fn drive_offline_list(state: State<'_, AppState>) -> Answer<Vec<OfflineEntry>> {
    require_session(&state).map_err(String::from)?;
    blocking(move || {
        let Some((db, _)) = local()? else { return Ok(Vec::new()) };
        let mut rows = Vec::new();
        for entry in model::live_rows(&db)? {
            let contract = db.get_file_contract_state(&entry.file_id).map_err(DriveError::db)?;
            let kept = contract
                .as_ref()
                .is_some_and(|c| c.effective_pin_state() == PinState::Pinned);
            let item = model::item_from(&entry, contract.as_ref(), None);
            rows.push((entry, kept, item));
        }
        Ok(status::offline_entries(&rows))
    })
    .await
}

/// Photo metadata for the ids asked (at most a page), from the copies already on this PC; an online-only file gets
/// only `uploading`.
#[tauri::command]
pub(crate) async fn drive_photo_meta(
    state: State<'_, AppState>,
    runtime: State<'_, PopoverRuntime>,
    ids: Vec<String>,
) -> Answer<HashMap<String, PhotoMeta>> {
    require_session(&state).map_err(String::from)?;
    let uploading: Vec<String> = runtime
        .transfers
        .snapshot_dtos()
        .into_iter()
        .filter(|t| t.direction == crate::transfer_progress::Direction::Up && !t.failed)
        .map(|t| t.file_id)
        .collect();
    blocking(move || {
        let Some((db, root)) = local()? else {
            return Ok(HashMap::new());
        };
        let roots: Vec<String> = db
            .backups_roots()
            .map_err(DriveError::db)?
            .into_iter()
            .map(|(_, path)| path)
            .collect();
        let mut out = HashMap::new();
        for id in ids.iter().take(model::PHOTOS_PAGE) {
            let Ok(entry) = model::resolve(&db, id) else { continue };
            let name = model::leaf_name(&entry);
            let mut meta = PhotoMeta::default();
            if entry.status == FileStatus::Local {
                let path = root.join(entry.path.trim_start_matches('/'));
                let mut head = Vec::with_capacity(photo_meta::HEAD_BYTES);
                if std::fs::File::open(&path)
                    .and_then(|f| f.take(photo_meta::HEAD_BYTES as u64).read_to_end(&mut head))
                    .is_ok()
                {
                    let video = model::mime_for_name(&name).is_some_and(|m| m.starts_with("video/"));
                    meta = photo_meta::from_head(&head, video);
                }
            }
            meta.uploading = entry.status == FileStatus::Uploading || uploading.iter().any(|u| u == id);
            meta.device = photo_meta::backup_device(&roots, &entry.path);
            out.insert(id.clone(), meta);
        }
        Ok(out)
    })
    .await
}

/// Where Photos' pictures and videos are: each folder that holds any directly, with how many.
#[derive(Debug, Serialize)]
pub(crate) struct PhotoSource {
    id: Option<String>,
    name: String,
    path: String,
    count: u64,
}

#[tauri::command]
pub(crate) async fn drive_photo_sources(state: State<'_, AppState>) -> Answer<Vec<PhotoSource>> {
    require_session(&state).map_err(String::from)?;
    blocking(move || {
        let Some((db, _)) = local()? else { return Ok(Vec::new()) };
        let rows = model::live_rows(&db)?;
        let mut counts: HashMap<Option<String>, u64> = HashMap::new();
        for r in rows
            .iter()
            .filter(|r| !r.is_dir() && model::is_photo_or_video(&model::leaf_name(r)))
        {
            *counts.entry(r.parent_id.clone()).or_default() += 1;
        }
        let mut out: Vec<PhotoSource> = counts
            .into_iter()
            .map(|(id, count)| {
                let folder = id.as_deref().and_then(|p| rows.iter().find(|r| r.file_id == p));
                PhotoSource {
                    name: folder.map_or_else(|| model::ROOT_NAME.to_string(), model::leaf_name),
                    path: folder.map_or_else(String::new, |f| f.path.trim_matches('/').to_string()),
                    id,
                    count,
                }
            })
            .collect();
        out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.path.cmp(&b.path)));
        Ok(out)
    })
    .await
}

/// The folder path of each id ("Files › Docs › 2026") for search results (B05).
#[tauri::command]
pub(crate) async fn drive_paths(state: State<'_, AppState>, ids: Vec<String>) -> Answer<HashMap<String, String>> {
    require_session(&state).map_err(String::from)?;
    blocking(move || {
        let Some((db, _)) = local()? else {
            return Ok(HashMap::new());
        };
        let mut out = HashMap::new();
        for id in ids.iter().take(500) {
            let Ok(entry) = model::resolve(&db, id) else { continue };
            let parent = match entry.parent_id.as_deref() {
                Some(p) => db.get_file(p).map_err(DriveError::db)?,
                None => None,
            };
            let crumbs = model::crumbs(&db, parent.as_ref())?;
            out.insert(
                id.clone(),
                crumbs.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(" › "),
            );
        }
        Ok(out)
    })
    .await
}
