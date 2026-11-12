//! The `#[tauri::command]`s. Each one checks the session, does its work off the async executor and
//! answers with a DTO from `dto.rs`. Spec 3.1 names them; `src/v2/source/desktop.ts` calls them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde::Deserialize;
use tauri::State;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use super::DriveError;
use super::dto::*;
use super::events;
use super::fsops;
use super::model::{self, Sort};
use crate::state_db::{FileEntry, FileStatus, StateDb};
use crate::{AppState, api_client};

pub(super) type Answer<T> = Result<T, String>;

/// The unlocked session's account: a locked vault is `locked`, no account `signed-out`.
pub(super) fn require_session(state: &AppState) -> Result<(), DriveError> {
    let acct = state.active_account().map_err(|_| DriveError::SignedOut)?;
    let unlocked = acct.session.lock().map(|g| g.is_some()).unwrap_or(false);
    if unlocked {
        return Ok(());
    }
    let present = state.auth_present.lock().map(|g| *g).unwrap_or(false);
    Err(if present {
        DriveError::Locked
    } else {
        DriveError::SignedOut
    })
}

fn api(state: &State<'_, AppState>) -> Result<api_client::ApiClient, DriveError> {
    crate::account_commands::api_client_from_session(state).map_err(|e| DriveError::from_text(&e))
}

/// The state DB and the sync root. No sync root or no database yet (before the first sync) is
/// `None`: a read shows an empty, stale listing, an action fails.
pub(super) fn local() -> Result<Option<(StateDb, PathBuf)>, DriveError> {
    let cfg = crate::config::DesktopConfig::load_for(crate::APP).map_err(|_| DriveError::Failed("config"))?;
    let Some(root) = cfg.sync_root.clone() else {
        return Ok(None);
    };
    let db = crate::local_state_purge::state_db_for_config(&cfg).map_err(DriveError::db)?;
    Ok(db.map(|db| (db, root)))
}

fn local_required() -> Result<(StateDb, PathBuf), DriveError> {
    local()?.ok_or(DriveError::Failed("not synced yet"))
}

pub(super) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, DriveError> + Send + 'static,
) -> Answer<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| String::from(DriveError::Failed("task")))?
        .map_err(String::from)
}

fn stale(state: &AppState) -> bool {
    events::engine_view(state).is_none_or(|(raw, _)| matches!(raw.as_str(), "offline" | "error"))
}

/// The item at a path this PC just wrote: the watcher mints its row within moments; until it has,
/// a provisional `local:<path>` id stands in (every command resolves it).
fn item_at_path(db: &StateDb, rel: &str, is_folder: bool, parent_id: Option<String>) -> Result<DriveItem, DriveError> {
    for _ in 0..15 {
        if let Some(entry) = db.get_file_by_path(rel).map_err(DriveError::db)?
            && entry.status != FileStatus::Trashing
        {
            return model::item_for_entry(db, &entry);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
    let mime = (!is_folder)
        .then(|| model::mime_for_name(&name).map(str::to_string))
        .flatten();
    Ok(DriveItem {
        id: format!("{}{rel}", model::LOCAL_ID_PREFIX),
        parent_id,
        is_folder,
        size: (!is_folder).then_some(0),
        child_count: is_folder.then_some(0),
        modified_at: model::now_iso(),
        mime,
        shared: SharedState::None,
        local: if is_folder { None } else { Some(LocalState::Syncing) },
        read_only: None,
        has_thumbnail: false,
        backup: Default::default(),
        name,
    })
}

// ── Reading ─────────────────────────────────────────────────────────────────────────────────────

#[tauri::command]
pub(crate) async fn drive_list(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    folder_id: Option<String>,
    sort: Option<Sort>,
) -> Answer<Listing> {
    require_session(&state)?;
    events::ensure_started(&app);
    let stale = stale(&state);
    blocking(move || match local()? {
        Some((db, _)) => model::list(&db, folder_id.as_deref(), sort.unwrap_or_default(), stale),
        None if folder_id.is_none() => {
            let root = Crumb {
                id: None,
                name: model::ROOT_NAME.to_string(),
            };
            Ok(Listing {
                folder: root.clone(),
                path: vec![root],
                items: Vec::new(),
                as_of: model::now_iso(),
                stale: true,
            })
        }
        None => Err(DriveError::NotFound),
    })
    .await
}

#[tauri::command]
pub(crate) async fn drive_details(state: State<'_, AppState>, id: String) -> Answer<ItemDetails> {
    require_session(&state)?;
    blocking(move || model::details(&local_required()?.0, &id)).await
}

/// The decrypted thumbnail as raw IPC bytes (an empty answer when there is none); the web view
/// shows it through a blob URL. Fetched and decrypted by the engine bridge, which keeps the file
/// key in Rust and refuses while the item's own write is still queued.
#[tauri::command]
pub(crate) async fn drive_thumbnail(
    state: State<'_, AppState>,
    id: String,
    size: Option<String>,
) -> Answer<tauri::ipc::Response> {
    require_session(&state)?;
    let api = Arc::new(api(&state)?);
    let variant = match size.as_deref() {
        Some("large") => "large",
        _ => "small",
    };
    let (db, _) = local_required()?;
    let entry = model::resolve(&db, &id)?;
    if entry.is_dir() {
        return Ok(tauri::ipc::Response::new(Vec::new()));
    }
    let bridge = crate::engine_bridge::EngineBridge::new(crate::APP, Arc::new(db), api);
    match bridge.finder_thumbnail(&entry.file_id, variant).await {
        Ok(bytes) => Ok(tauri::ipc::Response::new(bytes.to_vec())),
        Err(_) => Ok(tauri::ipc::Response::new(Vec::new())),
    }
}

#[tauri::command]
pub(crate) async fn drive_versions(state: State<'_, AppState>, id: String) -> Answer<Vec<VersionEntry>> {
    require_session(&state)?;
    let body = crate::settings_commands::list_file_versions(state, id)
        .await
        .map_err(|e| DriveError::from_text(&e))?;
    Ok(model::versions_from_payload(&body)?)
}

#[tauri::command]
pub(crate) async fn drive_search(state: State<'_, AppState>, query: String) -> Answer<Vec<DriveItem>> {
    require_session(&state)?;
    blocking(move || match local()? {
        Some((db, _)) => model::search(&db, &query),
        None => Ok(Vec::new()),
    })
    .await
}

#[tauri::command]
pub(crate) async fn drive_recent(state: State<'_, AppState>, limit: Option<usize>) -> Answer<Vec<DriveItem>> {
    require_session(&state)?;
    blocking(move || match local()? {
        Some((db, _)) => model::recent(&db, limit.unwrap_or(20)),
        None => Ok(Vec::new()),
    })
    .await
}

/// Shared with me (the mirror's shared roots) and shared by me (the server's sent invites, named
/// from the mirror). When the sent list cannot be fetched, the "with me" half still answers.
#[tauri::command]
pub(crate) async fn drive_shared(state: State<'_, AppState>) -> Answer<Vec<ShareEntry>> {
    require_session(&state)?;
    let sent = api(&state)?.sent_folder_invites().await.ok();
    blocking(move || {
        let Some((db, _)) = local()? else { return Ok(Vec::new()) };
        let mut out = model::shared_with_me(&db)?;
        if let Some(sent) = sent {
            out.extend(model::shared_by_me(&db, &sent)?);
        }
        Ok(out)
    })
    .await
}

#[tauri::command]
pub(crate) async fn drive_photos(state: State<'_, AppState>, cursor: Option<String>) -> Answer<PhotoPage> {
    require_session(&state)?;
    blocking(move || match local()? {
        Some((db, _)) => model::photos(&db, cursor.as_deref()),
        None => Ok(PhotoPage {
            entries: Vec::new(),
            next: None,
        }),
    })
    .await
}

#[tauri::command]
pub(crate) async fn drive_trash(state: State<'_, AppState>) -> Answer<Vec<TrashEntry>> {
    require_session(&state)?;
    let raw = crate::account_commands::desktop_trash_list(state)
        .await
        .map_err(|e| DriveError::from_text(&e))?;
    blocking(move || {
        let db = local().ok().flatten().map(|(db, _)| db);
        Ok(model::trash_entries(db.as_ref(), raw))
    })
    .await
}

#[tauri::command]
pub(crate) async fn drive_storage(state: State<'_, AppState>) -> Answer<StorageSummary> {
    require_session(&state)?;
    let summary = crate::file_open_commands::desktop_storage_summary(state)
        .await
        .map_err(|e| DriveError::from_text(&e))?;
    let (files, photos, backups) = blocking(move || match local()? {
        Some((db, _)) => model::storage_split(&db),
        None => Ok((0, 0, 0)),
    })
    .await?;
    Ok(StorageSummary {
        used_bytes: summary.used_bytes,
        quota_bytes: summary.quota_bytes,
        files_bytes: files,
        photos_bytes: photos,
        backups_bytes: backups,
    })
}

#[tauri::command]
pub(crate) async fn drive_sync_summary(app: tauri::AppHandle, state: State<'_, AppState>) -> Answer<SyncSummary> {
    events::ensure_started(&app);
    let view = events::engine_view(&state);
    blocking(move || {
        let Some((db, _)) = local()? else {
            let phase = if view.is_some() {
                SyncPhase::Checking
            } else {
                SyncPhase::Stopped
            };
            return Ok(SyncSummary {
                phase,
                on_device: 0,
                available_offline: 0,
                online_only: 0,
                pending: 0,
            });
        };
        let counts = model::sync_counts(&db);
        let (raw, paused) = view.clone().unwrap_or_default();
        let phase = model::sync_phase(view.is_some(), &raw, paused, &counts);
        Ok(model::sync_summary(phase, &counts, model::available_offline_count(&db)))
    })
    .await
}

// ── Acting ──────────────────────────────────────────────────────────────────────────────────────

#[tauri::command]
pub(crate) async fn drive_create_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    parent_id: Option<String>,
    name: String,
) -> Answer<DriveItem> {
    require_session(&state)?;
    let item = blocking(move || {
        let (db, root) = local_required()?;
        let parent = model::resolve_folder(&db, parent_id.as_deref())?;
        let rel = fsops::create_folder(&db, &root, parent.as_ref(), &name)?;
        item_at_path(&db, &rel, true, parent.map(|p| p.file_id))
    })
    .await?;
    events::changed(&app, item.parent_id.clone());
    Ok(item)
}

/// A new file in a folder (New file, DW-1522; the editors' "Keep both"). The body is a frame whose header is
/// `{ "parentId": string | null, "name" }` and whose bytes are the content (raw IPC bytes, never base64 JSON). The
/// file is written into the sync root and the engine uploads it; a taken name is refused, never overwritten.
#[tauri::command]
pub(crate) async fn drive_create_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> Answer<DriveItem> {
    require_session(&state)?;
    let tauri::ipc::InvokeBody::Raw(body) = request.body() else {
        return Err(DriveError::Failed("frame").into());
    };
    let body = body.clone();
    let item = blocking(move || {
        let (header, bytes) = unframe(&body)?;
        let name = header
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or(DriveError::Failed("frame"))?;
        let parent_id = header.get("parentId").and_then(|v| v.as_str());
        let (db, root) = local_required()?;
        let parent = model::resolve_folder(&db, parent_id)?;
        let rel = fsops::create_file(&db, &root, parent.as_ref(), name, bytes)?;
        item_at_path(&db, &rel, false, parent.map(|p| p.file_id))
    })
    .await?;
    events::changed(&app, item.parent_id.clone());
    Ok(item)
}

#[tauri::command]
pub(crate) async fn drive_rename(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> Answer<DriveItem> {
    require_session(&state)?;
    let item = blocking(move || {
        let (db, root) = local_required()?;
        let entry = model::resolve(&db, &id)?;
        let new_rel = fsops::rename(&db, &root, &entry, &name)?;
        // The row keeps its id; the watcher updates its path. Answer with the new name now.
        let mut item = model::item_for_entry(&db, &entry)?;
        item.name = new_rel.rsplit('/').next().unwrap_or(&new_rel).to_string();
        if !item.is_folder {
            item.mime = model::mime_for_name(&item.name).map(str::to_string);
        }
        Ok(item)
    })
    .await?;
    events::changed(&app, item.parent_id.clone());
    Ok(item)
}

fn resolve_all(db: &StateDb, ids: &[String]) -> Result<Vec<FileEntry>, DriveError> {
    if ids.is_empty() || ids.len() > 10_000 {
        return Err(DriveError::Failed("selection"));
    }
    ids.iter().map(|id| model::resolve(db, id)).collect()
}

#[tauri::command]
pub(crate) async fn drive_move(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ids: Vec<String>,
    target_id: Option<String>,
) -> Answer<()> {
    require_session(&state)?;
    let target = target_id.clone();
    let sources = blocking(move || {
        let (db, root) = local_required()?;
        let entries = resolve_all(&db, &ids)?;
        let target = model::resolve_folder(&db, target_id.as_deref())?;
        fsops::move_items(&db, &root, &entries, target.as_ref())?;
        Ok(entries.into_iter().map(|e| e.parent_id).collect::<Vec<_>>())
    })
    .await?;
    for parent in sources {
        events::changed(&app, parent);
    }
    events::changed(&app, target);
    Ok(())
}

#[tauri::command]
pub(crate) async fn drive_duplicate(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Answer<Vec<DriveItem>> {
    require_session(&state)?;
    let items = blocking(move || {
        let (db, root) = local_required()?;
        let entries = resolve_all(&db, &ids)?;
        let mut out = Vec::new();
        for entry in &entries {
            let rel = fsops::duplicate(&db, &root, entry)?;
            out.push(item_at_path(&db, &rel, entry.is_dir(), entry.parent_id.clone())?);
        }
        Ok(out)
    })
    .await?;
    if let Some(first) = items.first() {
        events::changed(&app, first.parent_id.clone());
    }
    Ok(items)
}

/// The undo table: token → the ids one Move to Trash sent to the server. Process memory only; a
/// token names no file and does nothing outside this process.
static UNDO: LazyLock<Mutex<HashMap<String, Vec<String>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
const UNDO_KEEP: usize = 64;

pub(crate) fn remember_undo(ids: Vec<String>) -> String {
    let token = uuid::Uuid::new_v4().to_string();
    if let Ok(mut table) = UNDO.lock() {
        if table.len() >= UNDO_KEEP {
            table.clear();
        }
        table.insert(token.clone(), ids);
    }
    token
}

pub(crate) fn take_undo(token: &str) -> Option<Vec<String>> {
    UNDO.lock().ok()?.remove(token)
}

/// What may go to Trash: a row the server knows, that is not mid-upload or in conflict (its newest
/// content is not on the server yet), outside the protected Backups tree, and not read-only shared.
pub(crate) fn check_trashable(db: &StateDb, entry: &FileEntry) -> Result<(), DriveError> {
    if entry.remote_updated_at == 0 || matches!(entry.status, FileStatus::Uploading | FileStatus::Conflict) {
        return Err(DriveError::Conflict);
    }
    let own = crate::backup_protect::own_backups();
    if crate::backup_protect::protected_reason(
        db,
        &own,
        entry.path.trim_matches('/'),
        beebeeb_core::backup::Mutation::Trash,
    )
    .is_some()
    {
        return Err(DriveError::ReadOnly);
    }
    let contract = db.get_file_contract_state(&entry.file_id).map_err(DriveError::db)?;
    if contract.is_some_and(|c| {
        c.namespace == crate::state_db::Namespace::SharedWithMe
            && c.permission_bits & crate::state_db::PERMISSION_WRITE == 0
    }) {
        return Err(DriveError::ReadOnly);
    }
    Ok(())
}

/// Move to Trash, no question asked (DW-0405): the server trashes, the engine's next pull removes
/// the local copies. Answers the token Undo hands back.
#[tauri::command]
pub(crate) async fn drive_trash_items(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Answer<UndoTicket> {
    require_session(&state)?;
    let api = api(&state)?;
    let entries = blocking(move || {
        let (db, _) = local_required()?;
        let entries = resolve_all(&db, &ids)?;
        for entry in &entries {
            check_trashable(&db, entry)?;
        }
        Ok(entries)
    })
    .await?;
    let mut done = Vec::new();
    for entry in &entries {
        if let Err(error) = api.trash_file(&entry.file_id).await {
            // What already went stays undoable; the rest is reported.
            if !done.is_empty() {
                remember_undo(done.clone());
            }
            return Err(DriveError::from_text(&error.to_string()).into());
        }
        done.push(entry.file_id.clone());
    }
    resnapshot();
    for entry in &entries {
        events::emit(
            &app,
            &DriveEvent::Removed {
                id: entry.file_id.clone(),
            },
        );
    }
    events::changed(&app, entries.first().and_then(|e| e.parent_id.clone()));
    Ok(UndoTicket {
        undo_token: remember_undo(done),
    })
}

fn resnapshot() {
    if let Ok(Some((db, _))) = local() {
        let _ = db.request_resnapshot();
    }
}

async fn restore_ids(api: &api_client::ApiClient, ids: &[String]) -> Result<(), DriveError> {
    for id in ids {
        api.restore_file(id)
            .await
            .map_err(|e| DriveError::from_text(&e.to_string()))?;
    }
    resnapshot();
    Ok(())
}

#[tauri::command]
pub(crate) async fn drive_undo_trash(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    undo_token: String,
) -> Answer<()> {
    require_session(&state)?;
    let api = api(&state)?;
    let ids = take_undo(&undo_token).ok_or(DriveError::NotFound)?;
    restore_ids(&api, &ids).await?;
    events::changed(&app, None);
    Ok(())
}

/// Restore from Trash, for a selection (the existing `desktop_trash_restore` takes one id).
#[tauri::command]
pub(crate) async fn drive_restore_from_trash(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Answer<()> {
    require_session(&state)?;
    if ids.is_empty() || ids.iter().any(|id| id.parse::<uuid::Uuid>().is_err()) {
        return Err(DriveError::NotFound.into());
    }
    restore_ids(&api(&state)?, &ids).await?;
    events::changed(&app, None);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PickMode {
    Files,
    Folder,
    Photos,
}

/// Upload through the native picker (Rust opens it; no path crosses into the web view): the
/// picked files or folder are copied into the folder and the engine uploads them.
#[tauri::command]
pub(crate) async fn drive_upload_pick(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    parent_id: Option<String>,
    mode: PickMode,
) -> Answer<UploadStarted> {
    require_session(&state)?;
    let picker_app = app.clone();
    let started = blocking(move || {
        let (db, root) = local_required()?;
        let parent = model::resolve_folder(&db, parent_id.as_deref())?;
        let dialog = picker_app.dialog().file().set_title("Upload to Beebeeb");
        let picked: Vec<PathBuf> = match mode {
            PickMode::Folder => dialog
                .blocking_pick_folder()
                .and_then(|p| p.into_path().ok())
                .into_iter()
                .collect(),
            PickMode::Files => dialog
                .blocking_pick_files()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|p| p.into_path().ok())
                .collect(),
            PickMode::Photos => dialog
                .add_filter(
                    "Photos and videos",
                    &[
                        "jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "avif", "dng", "tif", "tiff", "mp4",
                        "mov", "m4v", "webm",
                    ],
                )
                .blocking_pick_files()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|p| p.into_path().ok())
                .collect(),
        };
        let started = fsops::copy_in(&db, &root, parent.as_ref(), &picked)?;
        Ok((started, parent.map(|p| p.file_id)))
    })
    .await?;
    if started.0 > 0 {
        events::changed(&app, started.1);
    }
    Ok(UploadStarted { started: started.0 })
}

/// Upload what was dropped on the window from File Explorer: the OS hands the window those paths.
/// Only existing files and folders outside the Beebeeb folder are copied; nothing else is read.
#[tauri::command]
pub(crate) async fn drive_upload_dropped(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    parent_id: Option<String>,
    paths: Vec<String>,
) -> Answer<UploadStarted> {
    require_session(&state)?;
    if paths.len() > 10_000 {
        return Err(DriveError::Failed("selection").into());
    }
    let started = blocking(move || {
        let (db, root) = local_required()?;
        let parent = model::resolve_folder(&db, parent_id.as_deref())?;
        let sources: Vec<PathBuf> = paths
            .into_iter()
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .collect();
        let started = fsops::copy_in(&db, &root, parent.as_ref(), &sources)?;
        Ok((started, parent.map(|p| p.file_id)))
    })
    .await?;
    if started.0 > 0 {
        events::changed(&app, started.1);
    }
    Ok(UploadStarted { started: started.0 })
}

fn local_file(id: &str) -> Result<(StateDb, FileEntry, PathBuf), DriveError> {
    let (db, root) = local_required()?;
    let entry = model::resolve(&db, id)?;
    let path = fsops::full_path(&root, &entry.path)?;
    Ok((db, entry, path))
}

/// Open in its app: reading the file through the sync root downloads and decrypts it on this PC
/// first. Executable types are never launched from here (the `reveal_and_open_file` rule).
#[tauri::command]
pub(crate) async fn drive_open(app: tauri::AppHandle, state: State<'_, AppState>, id: String) -> Answer<()> {
    require_session(&state)?;
    blocking(move || {
        let (_, entry, path) = local_file(&id)?;
        if !entry.is_dir() && crate::file_open_commands::is_dangerous_executable(&path) {
            return Err(DriveError::Unsupported);
        }
        app.opener()
            .open_path(path.to_string_lossy().into_owned(), None::<&str>)
            .map_err(|_| DriveError::Failed("open"))
    })
    .await
}

#[tauri::command]
pub(crate) async fn drive_reveal(app: tauri::AppHandle, state: State<'_, AppState>, id: String) -> Answer<()> {
    require_session(&state)?;
    blocking(move || {
        let (_, _, path) = local_file(&id)?;
        app.opener()
            .reveal_item_in_dir(&path)
            .map_err(|_| DriveError::Failed("reveal"))
    })
    .await
}

fn current_version(db: &StateDb, entry: &FileEntry) -> Result<i64, DriveError> {
    Ok(db
        .get_file_contract_state(&entry.file_id)
        .map_err(DriveError::db)?
        .map_or(0, |c| c.current_version))
}

#[tauri::command]
pub(crate) async fn drive_read_text(state: State<'_, AppState>, id: String) -> Answer<TextDocument> {
    require_session(&state)?;
    blocking(move || {
        let (db, entry, path) = local_file(&id)?;
        let bytes = fsops::read_capped(&path, fsops::TEXT_LIMIT)?;
        let version_id = fsops::version_tag(current_version(&db, &entry)?, &bytes);
        let text = String::from_utf8(bytes).map_err(|_| DriveError::Unsupported)?;
        Ok(TextDocument { text, version_id })
    })
    .await
}

#[tauri::command]
pub(crate) async fn drive_save_text(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    text: String,
    base_version_id: String,
) -> Answer<TextDocument> {
    require_session(&state)?;
    let (doc, parent) = blocking(move || {
        let (db, root) = local_required()?;
        let entry = model::resolve(&db, &id)?;
        let version_id = fsops::save_checked(&db, &root, &entry, &base_version_id, text.as_bytes(), fsops::TEXT_LIMIT)?;
        Ok((TextDocument { text, version_id }, entry.parent_id))
    })
    .await?;
    events::changed(&app, parent);
    Ok(doc)
}

/// Binary frames: `[u32 LE header length][header JSON][bytes]`. Content travels as raw IPC bytes,
/// never as base64 JSON; the header carries only the version tag (and, inbound, the id).
pub(crate) fn frame(header: &serde_json::Value, bytes: &[u8]) -> Vec<u8> {
    let head = header.to_string().into_bytes();
    let mut out = Vec::with_capacity(4 + head.len() + bytes.len());
    out.extend_from_slice(&(head.len() as u32).to_le_bytes());
    out.extend_from_slice(&head);
    out.extend_from_slice(bytes);
    out
}

pub(crate) fn unframe(body: &[u8]) -> Result<(serde_json::Value, &[u8]), DriveError> {
    let bad = DriveError::Failed("frame");
    let len = u32::from_le_bytes(body.get(..4).ok_or(bad.clone())?.try_into().map_err(|_| bad.clone())?) as usize;
    let head = body.get(4..4 + len).ok_or(bad.clone())?;
    let header = serde_json::from_slice(head).map_err(|_| bad)?;
    Ok((header, &body[4 + len..]))
}

#[tauri::command]
pub(crate) async fn drive_read_bytes(state: State<'_, AppState>, id: String) -> Answer<tauri::ipc::Response> {
    require_session(&state)?;
    let framed = blocking(move || {
        let (db, entry, path) = local_file(&id)?;
        let bytes = zeroize::Zeroizing::new(fsops::read_capped(&path, fsops::BYTES_LIMIT)?);
        let version_id = fsops::version_tag(current_version(&db, &entry)?, &bytes);
        Ok(frame(&serde_json::json!({ "versionId": version_id }), &bytes))
    })
    .await?;
    Ok(tauri::ipc::Response::new(framed))
}

/// The body is a frame whose header is `{ "id", "baseVersionId" }`. Answers `{ versionId }`.
#[tauri::command]
pub(crate) async fn drive_save_bytes(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> Answer<serde_json::Value> {
    require_session(&state)?;
    let tauri::ipc::InvokeBody::Raw(body) = request.body() else {
        return Err(DriveError::Failed("frame").into());
    };
    let body = body.clone();
    let (version_id, parent) = blocking(move || {
        let (header, bytes) = unframe(&body)?;
        let id = header
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or(DriveError::Failed("frame"))?;
        let base = header
            .get("baseVersionId")
            .and_then(|v| v.as_str())
            .ok_or(DriveError::Failed("frame"))?;
        let (db, root) = local_required()?;
        let entry = model::resolve(&db, id)?;
        let version_id = fsops::save_checked(&db, &root, &entry, base, bytes, fsops::BYTES_LIMIT)?;
        Ok((version_id, entry.parent_id))
    })
    .await?;
    events::changed(&app, parent);
    Ok(serde_json::json!({ "versionId": version_id }))
}
