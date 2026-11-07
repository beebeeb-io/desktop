//! LD-0113 Task D4: when the backup passes run. It replaces the old mirror (task 0797), which copied the known
//! folders into the sync root; a pass reads each folder in place and queues uploads into `Backups/<device>`.
//!
//! - A full pass of every folder that is on, every [`FULL_PASS_EVERY`].
//! - A pass soon after a change: a file-system watcher on each source marks it changed, and the next tick once the
//!   folder has been quiet for the debounce runs the pass.
//! - One pass at a time (the runner's tick waits for it, as it did for the mirror). Uploads run in the normal queue.
//!
//! Logs carry counts and categories, never paths or names.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_backup::PassReport;
use beebeeb_core::backup::PassBudget;
use beebeeb_types::backup::Category;

use crate::engine_bridge::EngineBridge;

/// A full pass of every folder that is on.
pub(crate) const FULL_PASS_EVERY: Duration = Duration::from_secs(10 * 60);
/// How long a source must be quiet after a change before its pass runs.
pub(crate) const DEBOUNCE: Duration = Duration::from_secs(5);
/// What one pass of one folder may queue (core plans a prefix in key order; the next pass takes the rest).
pub(crate) const PASS_BUDGET: PassBudget = PassBudget { max_uploads: 500, max_bytes: 4 << 30 };

/// The last pass's count of items whose bytes are not on this device (`NotLocal`), per category: the follow-up note
/// of a folder backed up from OneDrive (Amendment 9). In memory: the next pass after a restart sets it again.
static NOT_LOCAL: Mutex<std::collections::BTreeMap<String, u64>> = Mutex::new(std::collections::BTreeMap::new());

/// The last pass's `NotLocal` count of the category `key`, if a pass ran since the app started.
pub(crate) fn not_local_count(key: &str) -> Option<u64> {
    NOT_LOCAL.lock().ok().and_then(|counts| counts.get(key).copied())
}

/// One folder this device backs up now and where it is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackupSource {
    pub(crate) category: Category,
    pub(crate) path: PathBuf,
}

/// What one tick did.
#[derive(Debug, Default)]
pub(crate) struct TickReport {
    /// The categories passed, with what each pass reported (or why it could not run).
    pub(crate) passes: Vec<(Category, Result<PassReport, String>)>,
}

pub(crate) struct BackupScheduler {
    debounce: Duration,
    last_full: Option<Instant>,
    /// When a watched source last changed, until a pass takes it.
    changed: Arc<Mutex<Option<Instant>>>,
    watcher: Option<notify::RecommendedWatcher>,
    watched: Vec<PathBuf>,
}

impl BackupScheduler {
    pub(crate) fn new() -> Self {
        Self::with_debounce(DEBOUNCE)
    }

    pub(crate) fn with_debounce(debounce: Duration) -> Self {
        Self { debounce, last_full: None, changed: Arc::default(), watcher: None, watched: Vec::new() }
    }

    /// Watch exactly `sources` (a folder switched on or off changes the set). A source that cannot be watched is
    /// still passed every [`FULL_PASS_EVERY`].
    fn watch(&mut self, sources: &[BackupSource]) {
        let wanted: Vec<PathBuf> = sources.iter().map(|s| s.path.clone()).collect();
        if wanted == self.watched && (self.watcher.is_some() || wanted.is_empty()) {
            return;
        }
        self.watcher = None;
        // A folder just switched on starts its backup on the next tick, not at the next 10-minute pass.
        if wanted.iter().any(|path| !self.watched.contains(path)) {
            self.last_full = None;
        }
        self.watched = wanted.clone();
        if wanted.is_empty() {
            return;
        }
        let changed = Arc::clone(&self.changed);
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if event.is_ok_and(|e| !e.kind.is_access()) {
                if let Ok(mut at) = changed.lock() {
                    *at = Some(Instant::now());
                }
            }
        });
        let Ok(mut watcher) = watcher else {
            tracing::warn!("backup: no file watcher; passes run on the 10-minute cadence only");
            return;
        };
        let mut watching = 0;
        for path in &wanted {
            if notify::Watcher::watch(&mut watcher, path, notify::RecursiveMode::Recursive).is_ok() {
                watching += 1;
            }
        }
        tracing::info!(watching, sources = wanted.len(), "backup: watching the backed-up folders");
        self.watcher = Some(watcher);
    }

    /// Whether a pass is due at `now`: the first one, the 10-minute one, or a change that has settled.
    pub(crate) fn due(&self, now: Instant) -> bool {
        let full = self.last_full.is_none_or(|at| now.duration_since(at) >= FULL_PASS_EVERY);
        let settled = self
            .changed
            .lock()
            .ok()
            .and_then(|at| *at)
            .is_some_and(|at| now.duration_since(at) >= self.debounce);
        full || settled
    }

    /// One runner tick: watch `sources`, and when a pass is due run one pass of each through `DesktopBackupHost`
    /// in the cached layout. `None`: nothing was due, or there is no layout yet.
    pub(crate) async fn tick(
        &mut self,
        bridge: &Arc<EngineBridge>,
        sync_root: &Path,
        sources: Vec<BackupSource>,
        now: Instant,
    ) -> Option<TickReport> {
        self.watch(&sources);
        bridge.abandon_orphaned_backup_sessions().await;
        if sources.is_empty() || !self.due(now) {
            return None;
        }
        if let Ok(mut at) = self.changed.lock() {
            *at = None;
        }
        // A layout that cannot be resolved now is tried again at the next 10-minute pass or after the next change,
        // never on every tick.
        self.last_full = Some(now);
        let layout = match bridge.db().backup_layout() {
            Ok(Some(layout)) => layout,
            Ok(None) => {
                let Some(me) = super::device::device_self() else {
                    tracing::warn!("backup: this device has no usable name or machine id; no pass");
                    return None;
                };
                let enabled: Vec<Category> = sources.iter().map(|s| s.category.clone()).collect();
                match super::layout::ensure_layout(bridge, &me, &enabled).await {
                    Ok(layout) => layout,
                    Err(error) => {
                        tracing::warn!(%error, "backup: the Backups layout could not be resolved; no pass");
                        return None;
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, "backup: the layout could not be read; no pass");
                return None;
            }
        };
        Some(run_passes(bridge, layout, sync_root, &sources).await)
    }
}

/// One pass of each source, in order. A source at or under the sync root is refused (and reported), and every pass
/// excludes the sync root; a sync root with no clean spelling runs no pass at all.
pub(crate) async fn run_passes(
    bridge: &Arc<EngineBridge>,
    layout: crate::state_db::BackupLayout,
    sync_root: &Path,
    sources: &[BackupSource],
) -> TickReport {
    let mut report = TickReport::default();
    let Some(exclude) = super::sources::pass_excludes(sync_root) else {
        tracing::warn!("backup: the sync root has no clean absolute spelling; no pass");
        return report;
    };
    let enabled: Vec<Category> = sources.iter().map(|s| s.category.clone()).collect();
    let host = super::host::DesktopBackupHost::new(Arc::clone(bridge), layout, enabled);
    for source in sources {
        let category = source.category.clone();
        if let Err(refusal) = super::sources::check_source(&source.path, sync_root) {
            tracing::warn!(category = %category.key(), ?refusal, "backup: source refused");
            report.passes.push((category, Err(format!("source refused: {refusal:?}"))));
            continue;
        }
        if let Err(error) = rebuild_records_if_empty(&host, &source.path, &exclude, &category).await {
            tracing::warn!(category = %category.key(), %error, "backup: records not rebuilt; the pass plans without them");
        }
        let pass = beebeeb_backup::run_pass(&host, &source.path, &exclude, &category, PASS_BUDGET).await;
        match &pass {
            Ok(done) => tracing::info!(
                category = %category.key(),
                queued = done.queued,
                moved = done.moved,
                unchanged = done.unchanged,
                held = done.held,
                turned_off = done.turned_off.is_some(),
                "backup: pass done"
            ),
            Err(error) => tracing::warn!(category = %category.key(), %error, "backup: pass failed"),
        }
        if let Ok(done) = &pass {
            let not_local = done.skipped.get(&beebeeb_core::backup::SkipReason::NotLocal).copied().unwrap_or(0);
            if let Ok(mut counts) = NOT_LOCAL.lock() {
                counts.insert(category.key(), not_local as u64);
            }
        }
        report.passes.push((category, pass.map_err(|e| e.to_string())));
        if bridge.db().backup_layout().ok().flatten().is_none() {
            // The device folder was deleted elsewhere: every backup turned off, nothing more to pass.
            break;
        }
    }
    report
}

/// Spec §4.2 rule 10: a folder with no records yet (a first backup into an adopted folder, the old mirror's
/// `GLegion/Pictures`, or a reinstall) gets its records rebuilt from what the server already holds, so a copy made
/// after the file last changed is not uploaded a second time. Core's `records_from_remote_with_clock` decides, with
/// the clock the listing measured from the server's `Date` header (LD-0123): a copy counts when the server stamped it
/// later than the file's last change plus the device's offset and the measurement's margin. With no clock measured
/// core applies its 10-minute floor. The offset is measured now: a clock that was behind when a file changed and has
/// been corrected since is not covered (core's documented limit; a sealed source mtime, decision LD-0122, would be).
/// Nothing is uploaded or changed on the server here.
pub(crate) async fn rebuild_records_if_empty(
    host: &super::host::DesktopBackupHost,
    source: &Path,
    exclude: &[PathBuf],
    category: &Category,
) -> Result<usize, beebeeb_backup::HostError> {
    use beebeeb_backup::BackupHost;
    if !host.records(category).await?.is_empty() {
        return Ok(0);
    }
    let (tree, clock) = host.device_tree_for_rebuild(category).await?;
    if !tree.device_folder_live || tree.category_folder_live == Some(false) || tree.files.is_empty() {
        return Ok(0);
    }
    let (root, exclude) = (source.to_path_buf(), exclude.to_vec());
    let walk = tokio::task::spawn_blocking(move || beebeeb_backup::walk_source(&root, &exclude))
        .await
        .map_err(|_| beebeeb_backup::HostError("the rebuild walk stopped before it finished".into()))?
        .map_err(|e| beebeeb_backup::HostError(format!("cannot read the backed-up folder ({:?})", e.kind())))?;
    let records = beebeeb_core::backup::records_from_remote_with_clock(&tree.files, &walk.entries, clock);
    let mut stored = 0;
    for record in records {
        let expected = beebeeb_core::backup::RecordRef { key: record.key.clone(), remote_id: record.remote_id };
        if host.put_record(category, &expected, record).await? {
            stored += 1;
        }
    }
    tracing::info!(
        category = %category.key(),
        stored,
        remote = tree.files.len(),
        clock_offset_ms = ?clock.offset_ms,
        clock_margin_ms = clock.margin_ms,
        "backup: records rebuilt from the server"
    );
    Ok(stored)
}
