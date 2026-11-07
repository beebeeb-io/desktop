//! LD-0113 Task D4: `DesktopBackupHost`, desktop's side of core's backup pass (`beebeeb_backup::BackupHost`).
//!
//! It works in one resolved layout (the Backups root and this device's folder, cached in the state DB): it lists
//! this device's backup of a category, creates and moves folders and files there through the backup client
//! (`X-Beebeeb-Origin: backup`), queues uploads (`origin = 'backup'`, the source read in place), keeps the records
//! (`backup_records`, per account) and turns a backup off when its folder was deleted elsewhere (rule 11).
//!
//! The pass is the one writer the protection layer protects the tree for: nothing here goes through
//! `check_app_mutation` or `check_drive_mutation`. The engine's own placeholder changes that follow a server move
//! pass the drive guard as `by_this_process` (protection lane 2's answer, 2026-11-06).

use std::collections::HashMap;
use std::sync::Arc;

use beebeeb_backup::{BackupHost, DeviceTree, HostError, UploadRequest};
use beebeeb_core::backup::{BackupRecord, FolderPresence, RebuildClock, RecordRef, RemoteEntry};
use beebeeb_types::backup::Category;
use uuid::Uuid;

use crate::engine_bridge::{BackupChild, BackupFolderState, EngineBridge};
use crate::state_db::{BackupEnqueue, BackupLayout, BackupStop};

/// A failure in the host's words: a kind and never a name or a path (core's rule for `HostError`).
fn host_error(kind: &str) -> impl Fn(crate::engine_bridge::EngineError) -> HostError + '_ {
    move |error| {
        tracing::warn!(kind, reason = crate::engine_bridge::error_category(&error), "backup: host step failed");
        HostError(kind.to_string())
    }
}

fn db_error(kind: &str) -> impl Fn(rusqlite::Error) -> HostError + '_ {
    move |_| HostError(format!("{kind}: the state database refused"))
}

fn parse_id(id: &str) -> Option<Uuid> {
    Uuid::parse_str(id).ok()
}

/// LD-0121 seam (spec Amendment 12): why a backup stops. Today the pass knows only rule 11's two reasons; core's
/// `backup_stop(notices, last_pass_at, listing)` (core v0.2.2) replaces this with the notice-based reason and its
/// words (`files_removed`, `backup_deleted`, `unknown`).
pub(crate) fn backup_stop_reason(why: FolderPresence) -> &'static str {
    match why {
        FolderPresence::DeviceDeleted => "device_deleted",
        FolderPresence::CategoryDeleted => "category_deleted",
        FolderPresence::Present => "present",
    }
}

/// Turn the backup of these categories off in the settings (the same switch the person uses). `Ok(false)`: the
/// settings could not be written; the stop notice still stands and no pass runs for a folder whose category folder
/// is forgotten.
fn switch_off_in_settings(keys: &[String]) -> bool {
    crate::config::DesktopConfig::update(|cfg| {
        for key in keys {
            cfg.set_known_folder(key, false);
        }
        (true, ())
    })
    .is_ok()
}

pub(crate) struct DesktopBackupHost {
    bridge: Arc<EngineBridge>,
    layout: BackupLayout,
    /// Every category this device backs up now: a deleted device folder turns all of them off at once.
    enabled: Vec<Category>,
    /// A tree the records' rebuild just listed, handed to the pass that follows it instead of listing again
    /// (review 2026-11-07): taken once.
    listed: std::sync::Mutex<HashMap<String, DeviceTree>>,
}

impl DesktopBackupHost {
    pub(crate) fn new(bridge: Arc<EngineBridge>, layout: BackupLayout, enabled: Vec<Category>) -> Self {
        Self { bridge, layout, enabled, listed: std::sync::Mutex::default() }
    }

    /// [`BackupHost::device_tree`] for the records' rebuild, with the clock its listing measured (LD-0123: core's
    /// `records_from_remote_with_clock` takes it); the pass that follows takes the same listing.
    pub(crate) async fn device_tree_for_rebuild(&self, c: &Category) -> Result<(DeviceTree, RebuildClock), HostError> {
        let (tree, clock) = self.device_tree_clocked(c).await?;
        if let Ok(mut listed) = self.listed.lock() {
            listed.insert(c.key(), tree.clone());
        }
        Ok((tree, clock))
    }

    /// This device's backup of `c` as the server has it now, and the clock of the listing that read it. A device or
    /// category folder that is gone lists nothing, so its clock is `RebuildClock::UNKNOWN` (nothing is rebuilt then).
    async fn device_tree_clocked(&self, c: &Category) -> Result<(DeviceTree, RebuildClock), HostError> {
        let device = self
            .bridge
            .backup_folder_state(&self.layout.device_folder_id)
            .await
            .map_err(host_error("device_folder_state"))?;
        if device == BackupFolderState::Gone {
            let tree = DeviceTree {
                device_folder_live: false,
                category_folder_live: None,
                folders: HashMap::new(),
                files: Vec::new(),
                complete: true,
            };
            return Ok((tree, RebuildClock::UNKNOWN));
        }
        let (folder_id, live) = self.category_folder(c).await?;
        if live == Some(false) {
            let tree = DeviceTree {
                device_folder_live: true,
                category_folder_live: Some(false),
                folders: HashMap::new(),
                files: Vec::new(),
                complete: true,
            };
            return Ok((tree, RebuildClock::UNKNOWN));
        }
        let (mut tree, clock) = self.list_category(&folder_id).await?;
        tree.category_folder_live = live;
        Ok((tree, clock))
    }

    /// The category folder's id: the remembered one, else (a first backup, or "Back up again") the live folder of
    /// that name in the device folder (the old mirror's `GLegion/Pictures` is adopted), else a new one.
    async fn category_folder(&self, c: &Category) -> Result<(String, Option<bool>), HostError> {
        let db = self.bridge.db();
        if let Some(id) = db.backup_category_folder(&c.key()).map_err(db_error("category_folder"))? {
            let state = self.bridge.backup_folder_state(&id).await.map_err(host_error("category_folder_state"))?;
            return Ok((id, Some(state == BackupFolderState::Live)));
        }
        let id = self
            .bridge
            .backup_ensure_folder(&self.layout.device_folder_id, c.display_name())
            .await
            .map_err(host_error("category_folder_create"))?;
        db.set_backup_category_folder(&c.key(), Some(&id)).map_err(db_error("category_folder"))?;
        Ok((id, None))
    }

    /// Every live folder and file under `category_folder`, keys joined with `/`, and the tightest clock its pages
    /// measured. A folder whose name cannot be opened is not entered and makes the listing partial; a file's is
    /// listed by id with an empty key.
    async fn list_category(&self, category_folder: &str) -> Result<(DeviceTree, RebuildClock), HostError> {
        let root = parse_id(category_folder).ok_or_else(|| HostError("category folder id is not a uuid".into()))?;
        let mut folders = HashMap::from([(String::new(), root)]);
        let mut files = Vec::new();
        let mut complete = true;
        let mut clock = RebuildClock::UNKNOWN;
        let mut queue = std::collections::VecDeque::from([(String::new(), category_folder.to_string())]);
        while let Some((prefix, folder_id)) = queue.pop_front() {
            let (children, all, folder_clock) =
                self.bridge.backup_list_children_clocked(&folder_id).await.map_err(host_error("listing"))?;
            complete &= all;
            clock = crate::engine_bridge::tighter_clock(clock, folder_clock);
            for BackupChild { id, name, is_folder, size, uploaded_at_ms, .. } in children {
                let Some(remote_id) = parse_id(&id) else {
                    complete = false;
                    continue;
                };
                let key = name.as_deref().map(|name| if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") });
                if is_folder {
                    match key {
                        Some(key) => {
                            folders.insert(key.clone(), remote_id);
                            queue.push_back((key, id));
                        }
                        None => complete = false,
                    }
                } else {
                    files.push(RemoteEntry { key: key.unwrap_or_default(), remote_id, size, uploaded_at_ms });
                }
            }
        }
        Ok((DeviceTree { device_folder_live: true, category_folder_live: None, folders, files, complete }, clock))
    }
}

impl BackupHost for DesktopBackupHost {
    async fn device_tree(&self, c: &Category) -> Result<DeviceTree, HostError> {
        if let Some(tree) = self.listed.lock().ok().and_then(|mut listed| listed.remove(&c.key())) {
            return Ok(tree);
        }
        Ok(self.device_tree_clocked(c).await?.0)
    }

    async fn ensure_folder(&self, parent: Uuid, name: &str) -> Result<Uuid, HostError> {
        let id = self
            .bridge
            .backup_ensure_folder(&parent.to_string(), name)
            .await
            .map_err(host_error("ensure_folder"))?;
        parse_id(&id).ok_or_else(|| HostError("a folder id is not a uuid".into()))
    }

    async fn move_file(&self, id: Uuid, new_parent: Uuid, new_name: &str) -> Result<(), HostError> {
        self.bridge
            .backup_move_file(&id.to_string(), &new_parent.to_string(), new_name)
            .await
            .map_err(host_error("move_file"))
    }

    async fn enqueue_upload(&self, req: UploadRequest) -> Result<(), HostError> {
        match self.bridge.enqueue_backup_upload(&req).map_err(host_error("enqueue_upload"))? {
            BackupEnqueue::Queued(_) | BackupEnqueue::AlreadyQueued(_) | BackupEnqueue::AlreadyBackedUp => Ok(()),
            BackupEnqueue::NoOwner => Err(HostError("enqueue_upload: no account is bound".into())),
        }
    }

    async fn records(&self, c: &Category) -> Result<Vec<BackupRecord>, HostError> {
        self.bridge.db().backup_records(&c.key()).map_err(db_error("records"))
    }

    async fn held_keys(&self, c: &Category) -> Result<Vec<String>, HostError> {
        self.bridge.db().backup_held_keys(&c.key()).map_err(db_error("held_keys"))
    }

    async fn put_record(&self, c: &Category, expected: &RecordRef, record: BackupRecord) -> Result<bool, HostError> {
        self.bridge
            .db()
            .put_backup_record_cas(&c.key(), expected, &record)
            .map_err(db_error("put_record"))
    }

    async fn mark_removed_elsewhere(&self, c: &Category, records: &[RecordRef]) -> Result<(), HostError> {
        let marked = self
            .bridge
            .db()
            .mark_backup_records_removed_elsewhere(&c.key(), records)
            .map_err(db_error("mark_removed_elsewhere"))?;
        tracing::info!(marked, asked = records.len(), "backup: records removed elsewhere");
        Ok(())
    }

    /// Rule 11: the folder was deleted elsewhere, so this backup turns off on this device, says why (the stop
    /// notice the Backups screen shows with "Back up again") and never uploads again by itself (Amendment 12). A
    /// deleted device folder turns every backup of this device off and forgets the layout.
    async fn turn_off(&self, c: &Category, why: FolderPresence) -> Result<(), HostError> {
        let db = self.bridge.db();
        let categories: Vec<Category> = match why {
            FolderPresence::DeviceDeleted => {
                // Every backup still on; one already off keeps the reason it stopped for.
                let stopped: Vec<String> =
                    db.backup_stops().map_err(db_error("turn_off"))?.into_iter().map(|s| s.category).collect();
                // Review (2026-11-07): every folder switched on, the ones this tick did not pass included (a folder
                // waiting for the OneDrive answer), so none keeps a category folder inside the deleted device folder.
                let mut on: Vec<Category> = self.enabled.clone();
                if let Ok(cfg) = crate::config::DesktopConfig::load() {
                    for category in cfg.known_folder_backup.iter().filter_map(|key| Category::from_key(key)) {
                        if !on.contains(&category) {
                            on.push(category);
                        }
                    }
                }
                let mut all: Vec<Category> = on.into_iter().filter(|e| !stopped.contains(&e.key())).collect();
                if !all.contains(c) {
                    all.push(c.clone());
                }
                all
            }
            _ => vec![c.clone()],
        };
        let at_ms = chrono::Utc::now().timestamp_millis();
        let keys: Vec<String> = categories.iter().map(Category::key).collect();
        for key in &keys {
            self.bridge.turn_off_backup_uploads(key).await.map_err(host_error("turn_off"))?;
            db.set_backup_category_folder(key, None).map_err(db_error("turn_off"))?;
            db.record_backup_stop(&BackupStop { category: key.clone(), reason: backup_stop_reason(why).into(), at_ms })
                .map_err(db_error("turn_off"))?;
        }
        if why == FolderPresence::DeviceDeleted {
            db.clear_backup_layout().map_err(db_error("turn_off"))?;
        }
        let switched = switch_off_in_settings(&keys);
        tracing::info!(turned_off = keys.len(), switched, reason = backup_stop_reason(why), "backup: turned off");
        Ok(())
    }
}
