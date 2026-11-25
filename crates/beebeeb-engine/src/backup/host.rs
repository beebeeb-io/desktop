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
use beebeeb_types::backup::{BackupStopReason, Category};
use uuid::Uuid;

use super::tree_cache::{self, KeptListing, OpsVerdict};
use crate::engine_bridge::{BackupChild, BackupFolderState, BackupSnapshot, EngineBridge};

/// RE3-a: a listing page by page (a server with no snapshot) runs this many folder requests at full speed, then one
/// every [`PAGED_LISTING_PACE`].
const PAGED_LISTING_BURST: usize = 20;
const PAGED_LISTING_PACE: std::time::Duration = std::time::Duration::from_millis(250);
/// `/sync/ops` answers at most this many ops a page (the server's default): a full page may not be the head.
const OPS_PAGE: usize = 1_000;

/// Where a full listing reads the folders from.
enum Listing<'a> {
    /// The one snapshot of the account's tree (RE3-a).
    Snapshot(&'a BackupSnapshot),
    /// `GET /files` per folder, paced after a burst (a server with no snapshot, or a test's direct listing).
    Pages { paced: bool },
}

/// One read of the ops after a kept listing's cursor.
#[derive(Clone)]
struct OpsRead {
    ops: Vec<crate::api_client::SyncOp>,
    /// The cursor the ops were read up to.
    to: i64,
    /// The read reached the head: nothing after `to`.
    caught_up: bool,
}
use crate::state_db::{BackupEnqueue, BackupLayout, BackupPassRecord, BackupStopWrite};

/// A failure in the host's words: a kind and never a name or a path (core's rule for `HostError`).
fn host_error(kind: &str) -> impl Fn(crate::engine_bridge::EngineError) -> HostError + '_ {
    move |error| {
        tracing::warn!(
            kind,
            reason = crate::engine_bridge::error_category(&error),
            "backup: host step failed"
        );
        HostError(kind.to_string())
    }
}

fn db_error(kind: &str) -> impl Fn(rusqlite::Error) -> HostError + '_ {
    move |_| HostError(format!("{kind}: the state database refused"))
}

fn parse_id(id: &str) -> Option<Uuid> {
    Uuid::parse_str(id).ok()
}

/// Turn the backup of these categories off in the settings (the same switch the person uses). `Ok(false)`: the
/// settings could not be written; the stop notice still stands and no pass runs for a folder whose category folder
/// is forgotten.
pub fn switch_off_in_settings(identity: &'static crate::identity::EngineIdentity, keys: &[String]) -> bool {
    crate::config::DesktopConfig::update_for(identity, |cfg| {
        for key in keys {
            cfg.set_known_folder(key, false);
        }
        (true, ())
    })
    .is_ok()
}

/// Whether the switch `key` in the PC's settings file is the local data's owner's to turn off (the per-account rule
/// of P3-8): a known folder's switch is the PC's; a chosen folder's (`custom:<name>`) only when the folder is one of
/// `owners_chosen` (`backup_folders::runner_chosen`: tagged with that owner; an untagged one is nobody's). Every
/// account's chosen folders share the one backup set, so a stop or a reconcile here never turns off another
/// account's. A key that names no category: never.
pub fn owners_switch(key: &str, owners_chosen: &[crate::backup::sources::ChosenFolder]) -> bool {
    match Category::from_key(key) {
        Some(Category::Custom(_)) => owners_chosen.iter().any(|folder| folder.key() == key),
        Some(_) => true,
        None => false,
    }
}

/// The tree of a backup whose device or category folder is gone: nothing listed, and complete (there is nothing more).
fn gone_tree(device_folder_live: bool, category_folder_live: Option<bool>) -> DeviceTree {
    DeviceTree {
        device_folder_live,
        category_folder_live,
        folders: HashMap::new(),
        files: Vec::new(),
        complete: true,
    }
}

pub struct DesktopBackupHost {
    bridge: Arc<EngineBridge>,
    layout: BackupLayout,
    /// Every category this device backs up now: a deleted device folder turns all of them off at once.
    enabled: Vec<Category>,
    /// A tree the records' rebuild just listed, handed to the pass that follows it instead of listing again
    /// (review 2026-11-07): taken once.
    listed: std::sync::Mutex<HashMap<String, DeviceTree>>,
    /// RE3-a: this tick's one snapshot (`None` inside: it could not be read; the listing goes page by page).
    snapshot: tokio::sync::Mutex<Option<Option<Arc<BackupSnapshot>>>>,
    /// RE3-a: this tick's ops reads by cursor (`None`: the read failed), so backups kept at one cursor read once.
    ops: tokio::sync::Mutex<HashMap<i64, Option<OpsRead>>>,
}

impl DesktopBackupHost {
    pub fn new(bridge: Arc<EngineBridge>, layout: BackupLayout, enabled: Vec<Category>) -> Self {
        Self {
            bridge,
            layout,
            enabled,
            listed: std::sync::Mutex::default(),
            snapshot: tokio::sync::Mutex::default(),
            ops: tokio::sync::Mutex::default(),
        }
    }

    /// RE3-a: this device's backup of `c` for a pass, and the clock of its listing. The kept listing of an earlier pass
    /// when it can be used ([`Self::kept_tree`]: no listing, one `/sync/ops` read); else a full listing from one
    /// snapshot, kept for the next passes. `rebuild`: the records are about to be rebuilt from it, so it is listed in
    /// full and its clock measured. The pass that follows takes the same tree.
    pub async fn device_tree_for_pass(
        &self,
        c: &Category,
        rebuild: bool,
    ) -> Result<(DeviceTree, RebuildClock), HostError> {
        let kept = if rebuild { None } else { self.kept_tree(c).await? };
        let (tree, clock) = match kept {
            Some(tree) => (tree, RebuildClock::UNKNOWN),
            None => self.full_listing(c, rebuild).await?,
        };
        if let Ok(mut listed) = self.listed.lock() {
            listed.insert(c.key(), tree.clone());
        }
        Ok((tree, clock))
    }

    /// The kept listing of `c`, handed to core as PARTIAL (core marks nothing removed elsewhere from it), when it is
    /// the remembered category folder's, younger than a day, read from a snapshot, and the ops since it touch none of
    /// its folders (nor its files, past [`tree_cache::TOUCHED_LISTING_GAP`]). When the ops could not be read to the
    /// head, the device and category folders are checked by id (two requests) instead. `None`: list in full.
    async fn kept_tree(&self, c: &Category) -> Result<Option<DeviceTree>, HostError> {
        let (device, key) = (self.layout.device_folder_id.as_str(), c.key());
        let Some(kept) = tree_cache::get(device, &key) else {
            return Ok(None);
        };
        let remembered = self
            .bridge
            .db()
            .backup_category_folder(&key)
            .map_err(db_error("category_folder"))?;
        if remembered.as_deref() != Some(kept.category_folder_id.as_str())
            || kept.listed_at.elapsed() >= tree_cache::FULL_LISTING_EVERY
        {
            return Ok(None);
        }
        let Some(cursor) = kept.cursor else {
            return Ok(None);
        };
        let read = self.ops_since(cursor).await;
        let checked = match &read {
            Some(read) => match tree_cache::verdict(&kept, device, &read.ops) {
                OpsVerdict::FolderTouched => return Ok(None),
                OpsVerdict::FileTouched if kept.listed_at.elapsed() >= tree_cache::TOUCHED_LISTING_GAP => {
                    return Ok(None);
                }
                // Not moved on: the touch is seen again, and lists once the gap is over.
                OpsVerdict::FileTouched => read.caught_up,
                OpsVerdict::Quiet => {
                    tree_cache::advance(device, &key, read.to);
                    read.caught_up
                }
            },
            None => false,
        };
        if !checked {
            if self.folder_state(device).await? == BackupFolderState::Gone
                || self.folder_state(&kept.category_folder_id).await? == BackupFolderState::Gone
            {
                return Ok(None);
            }
        }
        let mut tree = kept.tree;
        tree.device_folder_live = true;
        tree.category_folder_live = Some(true);
        tree.complete = false;
        Ok(Some(tree))
    }

    /// One page of the ops after `cursor`, read once per tick. `None`: the read failed, or the server's head is
    /// behind the cursor (its log was reset).
    async fn ops_since(&self, cursor: i64) -> Option<OpsRead> {
        let mut memo = self.ops.lock().await;
        if let Some(read) = memo.get(&cursor) {
            return read.clone();
        }
        let read = match self.bridge.backup_ops_since(cursor).await {
            Ok(page) => {
                let last = page.ops.iter().map(|op| op.seq_id).max();
                match (last, page.head_seq) {
                    (_, Some(head)) if head < cursor => None,
                    (Some(last), head) => Some(OpsRead {
                        caught_up: head.map_or(page.ops.len() < OPS_PAGE, |head| last >= head),
                        to: last,
                        ops: page.ops,
                    }),
                    (None, head) => Some(OpsRead {
                        ops: Vec::new(),
                        to: head.unwrap_or(cursor),
                        caught_up: true,
                    }),
                }
            }
            Err(error) => {
                tracing::warn!(
                    reason = crate::engine_bridge::error_category(&error),
                    "backup: the ops since the last listing could not be read; the folders are checked by id"
                );
                None
            }
        };
        memo.insert(cursor, read.clone());
        read
    }

    /// This tick's snapshot of the account's tree, read once for every backup the tick lists.
    async fn snapshot(&self) -> Option<Arc<BackupSnapshot>> {
        let mut memo = self.snapshot.lock().await;
        if let Some(snapshot) = memo.as_ref() {
            return snapshot.clone();
        }
        let snapshot = match self.bridge.backup_snapshot().await {
            Ok(snapshot) => Some(Arc::new(snapshot)),
            Err(error) => {
                tracing::warn!(
                    reason = crate::engine_bridge::error_category(&error),
                    "backup: no snapshot; the backup is listed folder by folder, paced"
                );
                None
            }
        };
        *memo = Some(snapshot.clone());
        snapshot
    }

    /// A full listing of `c`: from this tick's snapshot (one request for every backup), else page by page, paced.
    /// A live listing is kept for the next passes. `clocked`: one listing page of the category folder measures the
    /// server's clock for the records' rebuild (a snapshot carries none).
    async fn full_listing(&self, c: &Category, clocked: bool) -> Result<(DeviceTree, RebuildClock), HostError> {
        let device = self
            .bridge
            .backup_folder_state(&self.layout.device_folder_id)
            .await
            .map_err(host_error("device_folder_state"))?;
        if device == BackupFolderState::Gone {
            return Ok((gone_tree(false, None), RebuildClock::UNKNOWN));
        }
        let (folder_id, live) = self.category_folder(c).await?;
        if live == Some(false) {
            return Ok((gone_tree(true, Some(false)), RebuildClock::UNKNOWN));
        }
        // The records' rebuild needs each file's `uploaded_at` and the server's clock, which the snapshot lacks
        // (it has created_at/updated_at only), so a rebuild lists page by page, paced, ONCE per backup (never on
        // every pass); every other full listing is the snapshot. A rebuild's listing is not kept (no cursor): the
        // next pass lists from the snapshot, once, and keeps that.
        let snapshot = if clocked { None } else { self.snapshot().await };
        let (mut tree, clock) = match &snapshot {
            Some(snapshot) => self.list_category(&folder_id, Listing::Snapshot(snapshot)).await?,
            None => self.list_category(&folder_id, Listing::Pages { paced: true }).await?,
        };
        tree.category_folder_live = live;
        let listing = KeptListing {
            category_folder_id: folder_id,
            tree: tree.clone(),
            listed_at: std::time::Instant::now(),
            cursor: snapshot.map(|snapshot| snapshot.seq_id),
        };
        tree_cache::put(&self.layout.device_folder_id, &c.key(), listing);
        tracing::info!(
            category = %c.key(),
            files = tree.files.len(),
            folders = tree.folders.len(),
            complete = tree.complete,
            "backup: listed in full"
        );
        Ok((tree, clock))
    }

    /// [`BackupHost::device_tree`] for the records' rebuild, with the clock its listing measured (LD-0123: core's
    /// `records_from_remote_with_clock` takes it); the pass that follows takes the same listing.
    pub async fn device_tree_for_rebuild(&self, c: &Category) -> Result<(DeviceTree, RebuildClock), HostError> {
        self.device_tree_for_pass(c, true).await
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
            return Ok((gone_tree(false, None), RebuildClock::UNKNOWN));
        }
        let (folder_id, live) = self.category_folder(c).await?;
        if live == Some(false) {
            return Ok((gone_tree(true, Some(false)), RebuildClock::UNKNOWN));
        }
        let (mut tree, clock) = self.list_category(&folder_id, Listing::Pages { paced: false }).await?;
        tree.category_folder_live = live;
        Ok((tree, clock))
    }

    /// The category folder's id: the remembered one, else (a first backup, or "Back up again") the live folder of that
    /// name in the device folder (the old mirror's `TestPC/Pictures` is adopted), else a new one. A folder the person
    /// chose: [`Self::chosen_category_folder`].
    async fn category_folder(&self, c: &Category) -> Result<(String, Option<bool>), HostError> {
        if matches!(c, Category::Custom(_)) {
            return self.chosen_category_folder(c).await;
        }
        let db = self.bridge.db();
        if let Some(id) = db
            .backup_category_folder(&c.key())
            .map_err(db_error("category_folder"))?
        {
            let state = self.folder_state(&id).await?;
            return Ok((id, Some(state == BackupFolderState::Live)));
        }
        let id = self
            .bridge
            .backup_ensure_folder(&self.layout.device_folder_id, c.display_name())
            .await
            .map_err(host_error("category_folder_create"))?;
        db.set_backup_category_folder(&c.key(), Some(&id))
            .map_err(db_error("category_folder"))?;
        Ok((id, None))
    }

    async fn folder_state(&self, id: &str) -> Result<BackupFolderState, HostError> {
        self.bridge
            .backup_folder_state(id)
            .await
            .map_err(host_error("category_folder_state"))
    }

    /// A chosen folder's category folder (P3-10, by construction): the folder whose id its settings entry holds
    /// (`ChosenFolder::folder_id`, the local data's owner's entry), never a folder found by its name, which may be a
    /// backup the person kept. The entry outlives a sign-out, which empties the local state.
    /// - Its id, remembered here too (no stop and no sign-out since): live or gone, as every backup's folder (a gone
    ///   one stops the backup, rule 11).
    /// - Its id, not remembered (signed in again, or "Back up again" after a stop): continued while it is a live
    ///   folder in this device's folder (never one elsewhere), and remembered. A listing that is not complete answers
    ///   an error: the folder may be in the part not read, and nothing is made.
    /// - No id (a new add, or a settings file from before the id), or its folder is gone: a new folder. Its id is
    ///   minted and recorded in the entry BEFORE the create, so a create whose answer is lost is found by its id on
    ///   the next pass, never made twice; an id that cannot be recorded makes nothing.
    async fn chosen_category_folder(&self, c: &Category) -> Result<(String, Option<bool>), HostError> {
        let db = self.bridge.db();
        let key = c.key();
        let owner = db
            .owner()
            .map_err(db_error("category_folder"))?
            .ok_or_else(|| HostError("category_folder: no account is bound".into()))?;
        let cfg = crate::config::DesktopConfig::load_for(self.bridge.identity)
            .map_err(|_| HostError("category_folder: the settings could not be read".into()))?;
        let entry = crate::app::bindings::backup_folders::chosen_for(&cfg.backup_chosen_folders, Some(&owner))
            .into_iter()
            .find(|folder| folder.key() == key)
            .ok_or_else(|| HostError("category_folder: not a chosen folder of this account".into()))?;
        let remembered = db.backup_category_folder(&key).map_err(db_error("category_folder"))?;
        let parent = &self.layout.device_folder_id;
        if let Some(id) = entry.folder_id {
            if remembered.as_deref() == Some(id.as_str()) {
                let state = self.folder_state(&id).await?;
                return Ok((id, Some(state == BackupFolderState::Live)));
            }
            let (children, complete) = self
                .bridge
                .backup_list_children(parent)
                .await
                .map_err(host_error("category_folder_state"))?;
            if children.iter().any(|child| child.is_folder && child.id == id) {
                db.set_backup_category_folder(&key, Some(&id))
                    .map_err(db_error("category_folder"))?;
                return Ok((id, None));
            }
            if !complete {
                return Err(HostError(
                    "category_folder: the device folder was not listed completely".into(),
                ));
            }
        }
        let id = self
            .bridge
            .backup_new_folder_id()
            .map_err(host_error("category_folder_create"))?;
        let recorded = crate::config::DesktopConfig::update_for(self.bridge.identity, |cfg| {
            let recorded = crate::app::bindings::backup_folders::record_folder_id(cfg, &key, &owner, &id);
            (recorded, recorded)
        });
        if recorded != Ok(true) {
            return Err(HostError(
                "category_folder: the new folder's id was not recorded".into(),
            ));
        }
        self.bridge
            .backup_create_folder_as(parent, c.display_name(), &id)
            .await
            .map_err(host_error("category_folder_create"))?;
        db.set_backup_category_folder(&key, Some(&id))
            .map_err(db_error("category_folder"))?;
        Ok((id, None))
    }

    /// Every live folder and file under `category_folder`, keys joined with `/`, and the tightest clock its pages
    /// measured. A folder whose name cannot be opened is not entered and makes the listing partial; a file's is
    /// listed by id with an empty key.
    async fn list_category(
        &self,
        category_folder: &str,
        source: Listing<'_>,
    ) -> Result<(DeviceTree, RebuildClock), HostError> {
        let root = parse_id(category_folder).ok_or_else(|| HostError("category folder id is not a uuid".into()))?;
        let mut folders = HashMap::from([(String::new(), root)]);
        let mut files = Vec::new();
        let mut complete = true;
        let mut clock = RebuildClock::UNKNOWN;
        let mut queue = std::collections::VecDeque::from([(String::new(), category_folder.to_string())]);
        let mut requests = 0usize;
        while let Some((prefix, folder_id)) = queue.pop_front() {
            let children = match &source {
                Listing::Snapshot(snapshot) => self.bridge.backup_children_in(snapshot, &folder_id),
                Listing::Pages { paced } => {
                    requests += 1;
                    if *paced && requests > PAGED_LISTING_BURST {
                        tokio::time::sleep(PAGED_LISTING_PACE).await;
                    }
                    let (children, all, folder_clock) = self
                        .bridge
                        .backup_list_children_clocked(&folder_id)
                        .await
                        .map_err(host_error("listing"))?;
                    complete &= all;
                    clock = crate::engine_bridge::tighter_clock(clock, folder_clock);
                    children
                }
            };
            for BackupChild {
                id,
                name,
                is_folder,
                size,
                uploaded_at_ms,
                ..
            } in children
            {
                let Some(remote_id) = parse_id(&id) else {
                    complete = false;
                    continue;
                };
                let key = name.as_deref().map(|name| {
                    if prefix.is_empty() {
                        name.to_string()
                    } else {
                        format!("{prefix}/{name}")
                    }
                });
                if is_folder {
                    match key {
                        Some(key) => {
                            folders.insert(key.clone(), remote_id);
                            queue.push_back((key, id));
                        }
                        None => complete = false,
                    }
                } else {
                    files.push(RemoteEntry {
                        key: key.unwrap_or_default(),
                        remote_id,
                        size,
                        uploaded_at_ms,
                    });
                }
            }
        }
        Ok((
            DeviceTree {
                device_folder_live: true,
                category_folder_live: None,
                folders,
                files,
                complete,
            },
            clock,
        ))
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
        let id = parse_id(&id).ok_or_else(|| HostError("a folder id is not a uuid".into()))?;
        tree_cache::add_folder(parent, name, id);
        Ok(id)
    }

    async fn move_file(&self, id: Uuid, new_parent: Uuid, new_name: &str) -> Result<(), HostError> {
        self.bridge
            .backup_move_file(&id.to_string(), &new_parent.to_string(), new_name)
            .await
            .map_err(host_error("move_file"))
    }

    async fn enqueue_upload(&self, req: UploadRequest) -> Result<(), HostError> {
        match self
            .bridge
            .enqueue_backup_upload(&req)
            .map_err(host_error("enqueue_upload"))?
        {
            BackupEnqueue::Queued(_) | BackupEnqueue::AlreadyQueued(_) | BackupEnqueue::AlreadyBackedUp => Ok(()),
            BackupEnqueue::NoOwner => Err(HostError("enqueue_upload: no account is bound".into())),
        }
    }

    async fn records(&self, c: &Category) -> Result<Vec<BackupRecord>, HostError> {
        self.bridge.db().backup_records(&c.key()).map_err(db_error("records"))
    }

    async fn held_keys(&self, c: &Category) -> Result<Vec<String>, HostError> {
        self.bridge
            .db()
            .backup_held_keys(&c.key())
            .map_err(db_error("held_keys"))
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

    /// Rule 11 in core's pass: the folder was deleted elsewhere after this pass's notice check found nothing (a race;
    /// the check before the pass catches the usual case), so it stops as `unknown`: a folder gone with no notice.
    async fn turn_off(&self, c: &Category, why: FolderPresence) -> Result<(), HostError> {
        let device_gone = why == FolderPresence::DeviceDeleted;
        // No gate decided it: the backups' last pass and acknowledged notices stay as their gate wrote them.
        self.stop_backups(c, BackupStopReason::Unknown, device_gone, device_gone, None)
            .await
    }
}

/// LD-0139 (spec Amendment 12d): a stop a pass's gate decided: the notices check it read, and the acknowledged set core
/// answered for the backup it decided.
pub struct GateStop<'a> {
    pub check: &'a crate::backup::stop::NoticeCheck,
    pub acknowledged: Vec<String>,
}

impl DesktopBackupHost {
    /// A backup turns off on this device with core's reason (spec Amendment 12): it says why (the stop row the card
    /// shows with "Back up again") and never uploads again by itself. With `every_backup` (a removal of the account's
    /// files, a device folder that is gone, or a notice naming it) every backup of this device turns off at once; a
    /// device folder that is gone also forgets the layout.
    ///
    /// LD-0139 (Amendment 12d): each backup's queued uploads are dropped first (idempotent); then its stop row, its
    /// forgotten category folder, the forgotten layout and, for a stop `gate` decided, each stopped backup's new
    /// `last_pass_at` and acknowledged set go in ONE state-DB transaction. A crash before it commits leaves no stop and
    /// no acknowledgement (the next pass decides the same stop again); after it, both (the stop is never applied
    /// twice).
    pub async fn stop_backups(
        &self,
        c: &Category,
        reason: BackupStopReason,
        device_gone: bool,
        every_backup: bool,
        gate: Option<GateStop<'_>>,
    ) -> Result<(), HostError> {
        let db = self.bridge.db();
        let categories: Vec<Category> = if every_backup || device_gone || reason.files_removed() {
            // Every backup still on; one already off keeps the reason it stopped for.
            let stopped: Vec<String> = db
                .backup_stops()
                .map_err(db_error("turn_off"))?
                .into_iter()
                .map(|s| s.category)
                .collect();
            // Review (2026-11-07): every folder switched on, the ones this tick did not pass included (a folder
            // waiting for the OneDrive answer), so none keeps going after its backup was removed.
            let mut on: Vec<Category> = self.enabled.clone();
            if let Ok(cfg) = crate::config::DesktopConfig::load_for(self.bridge.identity) {
                // The local data's owner's switches alone (P3-8): another account's chosen folder, or an untagged one,
                // is not this device's backup of this account, and its switch is not this stop's to turn off.
                let owners = crate::app::bindings::backup_folders::runner_chosen(db, &cfg);
                for category in cfg
                    .known_folder_backup
                    .iter()
                    .filter(|key| owners_switch(key, &owners))
                    .filter_map(|key| Category::from_key(key))
                {
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
        } else {
            vec![c.clone()]
        };
        let at_ms = chrono::Utc::now().timestamp_millis();
        let keys: Vec<String> = categories.iter().map(Category::key).collect();
        // The queued uploads go first, before the marks are read (review 2026-11-08): a failed read below must not
        // leave a backup that turns off with uploads still queued.
        for key in &keys {
            self.bridge
                .turn_off_backup_uploads(key)
                .await
                .map_err(host_error("turn_off"))?;
        }
        // Amendment 12d: every stopped backup stores core's set for it in the stop's write: the decided one its gate's
        // answer, each other one (a stop that covers every backup) its own window's ids.
        let mut marks = Vec::new();
        if let Some(gate) = gate {
            for category in &categories {
                let acknowledged = if category == c {
                    gate.acknowledged.clone()
                } else {
                    let mark = db.backup_pass_mark(&category.key()).map_err(db_error("turn_off"))?;
                    crate::backup::stop::acknowledged_after(gate.check, &mark, category)
                };
                marks.push(BackupPassRecord {
                    category: category.key(),
                    last_pass_at_ms: gate.check.server_now_ms,
                    acknowledged,
                });
            }
        }
        db.record_backup_stops(&BackupStopWrite {
            reason: reason.as_str(),
            at_ms,
            categories: &keys,
            clear_layout: device_gone,
            marks: &marks,
        })
        .map_err(db_error("turn_off"))?;
        let switched = switch_off_in_settings(self.bridge.identity, &keys);
        tracing::info!(
            turned_off = keys.len(),
            switched,
            reason = reason.as_str(),
            "backup: turned off"
        );
        Ok(())
    }
}
