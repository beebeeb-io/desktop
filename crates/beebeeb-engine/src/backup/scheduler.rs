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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_backup::PassReport;
use beebeeb_core::backup::PassBudget;
use beebeeb_types::backup::Category;

use crate::engine_bridge::EngineBridge;

/// A full pass of every folder that is on.
pub const FULL_PASS_EVERY: Duration = Duration::from_secs(10 * 60);
/// How long a source must be quiet after a change before its pass runs.
pub const DEBOUNCE: Duration = Duration::from_secs(5);
/// What one pass of one folder may queue (core plans a prefix in key order; the next pass takes the rest).
pub const PASS_BUDGET: PassBudget = PassBudget {
    max_uploads: 500,
    max_bytes: 4 << 30,
};

/// A pass is being prepared or run now (set around the runner's pass, cleared when it ends however it ends).
static PASS_RUNNING: AtomicBool = AtomicBool::new(false);
/// "Back up now" was asked: the next tick runs a pass of every folder that is on, without waiting for the cadence.
static RUN_NOW: AtomicBool = AtomicBool::new(false);

/// Holds [`PASS_RUNNING`] while a pass runs; cleared on drop, so a pass that ends early or is cancelled never leaves
/// the card saying "Backing up".
struct RunningGuard;

impl RunningGuard {
    fn start() -> Self {
        PASS_RUNNING.store(true, Ordering::SeqCst);
        Self
    }
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        PASS_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// Whether the runner is passing the backed-up folders now.
pub fn pass_running() -> bool {
    PASS_RUNNING.load(Ordering::SeqCst)
}

/// Ask the runner for a pass of every folder that is on at its next tick ("Back up now"). The runner honours it in
/// [`BackupScheduler::due`]; with no folder on, or no runner, the ask is dropped at the next tick, never kept.
pub fn request_pass_now() {
    RUN_NOW.store(true, Ordering::SeqCst);
}

/// Whether a "run now" is waiting for the runner's next tick.
pub fn pass_requested() -> bool {
    RUN_NOW.load(Ordering::SeqCst)
}

/// The last pass's count of items whose bytes are not on this device (`NotLocal`), per category: the follow-up note
/// of a folder backed up from OneDrive (Amendment 9). In memory: the next pass after a restart sets it again.
static NOT_LOCAL: Mutex<std::collections::BTreeMap<String, u64>> = Mutex::new(std::collections::BTreeMap::new());

/// The last pass's `NotLocal` count of the category `key`, if a pass ran since the app started.
pub fn not_local_count(key: &str) -> Option<u64> {
    NOT_LOCAL.lock().ok().and_then(|counts| counts.get(key).copied())
}

/// One folder this device backs up now and where it is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupSource {
    pub category: Category,
    pub path: PathBuf,
}

/// What one tick did.
#[derive(Debug, Default)]
pub struct TickReport {
    /// The categories passed, with what each pass reported (or why it could not run).
    pub passes: Vec<(Category, Result<PassReport, String>)>,
    /// LD-0139: the notice check failed, so no pass ran (the card says "Couldn't check — will retry.").
    pub check_failed: bool,
}

pub struct BackupScheduler {
    debounce: Duration,
    last_full: Option<Instant>,
    /// When a watched source last changed, until a pass takes it.
    changed: Arc<Mutex<Option<Instant>>>,
    watcher: Option<notify::RecommendedWatcher>,
    watched: Vec<PathBuf>,
    /// LD-0139: the stopped backups' switches were reconciled since this scheduler started.
    reconciled: bool,
}

impl BackupScheduler {
    pub fn new() -> Self {
        Self::with_debounce(DEBOUNCE)
    }

    pub fn with_debounce(debounce: Duration) -> Self {
        // LD-0139: a new session starts with no failed notice check to show.
        super::stop::set_check_failed(false);
        Self {
            debounce,
            last_full: None,
            changed: Arc::default(),
            watcher: None,
            watched: Vec::new(),
            reconciled: false,
        }
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
        tracing::info!(
            watching,
            sources = wanted.len(),
            "backup: watching the backed-up folders"
        );
        self.watcher = Some(watcher);
    }

    /// Whether a pass is due at `now`: the first one, the 10-minute one, or a change that has settled.
    pub fn due(&self, now: Instant) -> bool {
        let full = pass_requested()
            || self
                .last_full
                .is_none_or(|at| now.duration_since(at) >= FULL_PASS_EVERY);
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
    pub async fn tick(
        &mut self,
        bridge: &Arc<EngineBridge>,
        sync_root: &Path,
        sources: Vec<BackupSource>,
        now: Instant,
    ) -> Option<TickReport> {
        // LD-0139 (review 2026-11-08): when the scheduler starts, every stopped backup's switch is off, persisted (a
        // kill between a stop's commit and its switch-off, or a settings write that failed); a read that fails is
        // tried again on the next tick. Each pass reconciles again (`run_passes`).
        if !self.reconciled && super::stop::reconcile_stopped_switches(bridge.identity, bridge.db()).is_ok() {
            self.reconciled = true;
        }
        self.watch(&sources);
        bridge.abandon_orphaned_backup_sessions().await;
        if sources.is_empty() {
            // Nothing is on: a "Back up now" has nothing to run and is not kept for a folder switched on later.
            RUN_NOW.store(false, Ordering::SeqCst);
            return None;
        }
        if !self.due(now) {
            return None;
        }
        RUN_NOW.store(false, Ordering::SeqCst);
        if let Ok(mut at) = self.changed.lock() {
            *at = None;
        }
        // A layout that cannot be resolved now is tried again at the next 10-minute pass or after the next change,
        // never on every tick.
        self.last_full = Some(now);
        // LD-0139: a pass that cannot start (no layout, no device name) attempts no check, so none is shown as failed.
        super::stop::set_check_failed(false);
        let layout = match bridge.db().backup_layout() {
            Ok(Some(layout)) => layout,
            Ok(None) => {
                let Some(me) = super::device::device_self(bridge.identity) else {
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
        let running = RunningGuard::start();
        let report = run_passes(bridge, layout, sync_root, &sources).await;
        drop(running);
        super::stop::set_check_failed(report.check_failed);
        Some(report)
    }
}

/// One pass of each source, in order. A source at or under the sync root is refused (and reported), and every pass
/// excludes the sync root; a sync root with no clean spelling runs no pass at all.
///
/// LD-0139 (spec Amendments 12, 12c and 12d): the tick first reads the account's backup notices once, every page,
/// from core's `notices_since` of the oldest `last_pass_at` among the sources. Each source then goes to core's
/// `backup_pass_gate_v2` with its `last_pass_at` and the notice ids it acknowledged, before anything is planned:
/// - a check that failed on any page is core's `CheckFailed`: no pass (nothing is uploaded or turned off; the next
///   pass retries; the card says core's "Couldn't check — will retry.");
/// - a backup core turns off is stopped with core's reason, its stop and core's acknowledged set in one write;
/// - one that stays on records the check's `Date` as its `last_pass_at` with core's acknowledged set, in one write.
pub async fn run_passes(
    bridge: &Arc<EngineBridge>,
    layout: crate::state_db::BackupLayout,
    sync_root: &Path,
    sources: &[BackupSource],
) -> TickReport {
    use super::stop::PassDecision;
    let mut report = TickReport::default();
    let Some(exclude) = super::sources::pass_excludes(sync_root) else {
        tracing::warn!("backup: the sync root has no clean absolute spelling; no pass");
        return report;
    };
    // Review (2026-11-07): a backup with a stop row is off until "Back up again", even when its switch could not be
    // turned off; it is never passed. Review (2026-11-08): the stops are read first (and any switch still on next to a
    // stop row goes off); a read that fails runs no pass, since it cannot tell a stopped backup from one that is on.
    let stopped = match super::stop::reconcile_stopped_switches(bridge.identity, bridge.db()) {
        Ok(stopped) => stopped,
        Err(error) => {
            tracing::warn!(%error, "backup: the stopped backups could not be read; no pass (the next pass retries)");
            report.check_failed = true;
            for source in sources {
                report
                    .passes
                    .push((source.category.clone(), Err("stops not read".into())));
            }
            return report;
        }
    };
    let enabled: Vec<Category> = sources.iter().map(|s| s.category.clone()).collect();
    // Review (2026-11-08): which backups this tick gates is decided ONCE, with each one's marks read once, and the
    // check's `since` comes from exactly those marks: the loop gates this set and nothing else, so no backup is decided
    // on a listing that started after its window (a source refused here and accepted a moment later waits for the
    // next tick). `decide` also refuses a check that does not cover a window (`since_ms`).
    let mut gated: Vec<(&BackupSource, crate::state_db::BackupPassMark)> = Vec::new();
    for source in sources {
        let category = source.category.clone();
        if stopped.contains(&category.key()) {
            report
                .passes
                .push((category, Err("stopped: waiting for Back up again".into())));
            continue;
        }
        if let Err(refusal) = super::sources::check_source(&source.path, sync_root) {
            tracing::warn!(category = %category.key(), ?refusal, "backup: source refused");
            report
                .passes
                .push((category, Err(format!("source refused: {refusal:?}"))));
            continue;
        }
        // Marks that cannot be read are the default (no pass yet, nothing acknowledged): the check then asks from 0
        // and every notice in the window counts, which errs toward stopping, never toward an upload.
        let mark = bridge.db().backup_pass_mark(&category.key()).unwrap_or_default();
        gated.push((source, mark));
    }
    let check = if gated.is_empty() {
        None
    } else {
        let since = super::stop::check_since(gated.iter().map(|(_, mark)| mark));
        match bridge.backup_notice_check(since).await {
            Ok(check) => Some(check),
            Err(error) => {
                tracing::warn!(%error, "backup: the notices could not be checked; no pass (the next pass retries)");
                report.check_failed = true;
                None
            }
        }
    };
    let check_clock = check
        .as_ref()
        .map_or(beebeeb_core::backup::RebuildClock::UNKNOWN, |check| check.clock);
    let device_folder_id = uuid::Uuid::parse_str(&layout.device_folder_id).ok();
    let host = super::host::DesktopBackupHost::new(Arc::clone(bridge), layout, enabled);
    for (source, mark) in gated {
        let category = source.category.clone();
        let last_pass_at = super::stop::last_pass_at(&mark, check_clock);
        let folder_id = || {
            let id = bridge.db().backup_category_folder(&category.key()).ok().flatten();
            id.and_then(|id| uuid::Uuid::parse_str(&id).ok())
        };
        // Decided twice: from the notices alone before the listing (which can create the category folder, a server
        // write), then with the folders as the listing found them (step 3).
        let mut listed = super::stop::ListedBackup {
            category: &category,
            device_folder_id,
            device_folder_live: true,
            category_folder_id: folder_id(),
            category_folder_live: None,
        };
        let mut decision = super::stop::decide(check.as_ref(), last_pass_at, &mark.acknowledged, &listed);
        let mut tree_and_clock = None;
        if matches!(decision, PassDecision::Run { .. }) {
            // One listing per pass: the stop decision, the records' rebuild and (cached) the pass all read it.
            let (tree, clock) = match host.device_tree_for_rebuild(&category).await {
                Ok(listed) => listed,
                Err(error) => {
                    tracing::warn!(category = %category.key(), %error, "backup: not listed; no pass");
                    report.passes.push((category, Err(error.to_string())));
                    continue;
                }
            };
            listed.device_folder_live = tree.device_folder_live;
            listed.category_folder_id = folder_id();
            listed.category_folder_live = tree.category_folder_live;
            decision = super::stop::decide(check.as_ref(), last_pass_at, &mark.acknowledged, &listed);
            tree_and_clock = Some((tree, clock));
        }
        let (tree, clock, acknowledged) = match (decision, tree_and_clock) {
            (PassDecision::CheckFailed, _) => {
                report.check_failed = true;
                tracing::warn!(category = %category.key(), "backup: the notices were not read completely; no pass");
                report.passes.push((category, Err("notices not checked".into())));
                continue;
            }
            (
                PassDecision::Stop {
                    reason,
                    every_backup,
                    acknowledged,
                },
                _,
            ) => {
                let device_gone = !listed.device_folder_live;
                let gate = check
                    .as_ref()
                    .map(|check| super::host::GateStop { check, acknowledged });
                let stopped = host
                    .stop_backups(&category, reason, device_gone, every_backup, gate)
                    .await;
                let ok = stopped.is_ok();
                tracing::info!(category = %category.key(), reason = reason.as_str(), every_backup, ok, "backup: stopped");
                report
                    .passes
                    .push((category, Err(format!("turned off: {}", reason.as_str()))));
                if every_backup {
                    break; // every backup of this device is off now
                }
                continue;
            }
            (PassDecision::Run { acknowledged }, Some((tree, clock))) => (tree, clock, acknowledged),
            (PassDecision::Run { .. }, None) => continue, // not reached: a run is decided after the listing
        };
        if let Some(check) = &check {
            let record = crate::state_db::BackupPassRecord {
                category: category.key(),
                last_pass_at_ms: check.server_now_ms,
                acknowledged,
            };
            if let Err(error) = bridge.db().record_backup_pass(&record) {
                tracing::warn!(category = %category.key(), %error, "backup: last_pass_at not recorded");
            }
        }
        if let Err(error) = rebuild_records_if_empty(&host, &source.path, &exclude, &category, &tree, clock).await {
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
            let not_local = done
                .skipped
                .get(&beebeeb_core::backup::SkipReason::NotLocal)
                .copied()
                .unwrap_or(0);
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
pub async fn rebuild_records_if_empty(
    host: &super::host::DesktopBackupHost,
    source: &Path,
    exclude: &[PathBuf],
    category: &Category,
    tree: &beebeeb_backup::DeviceTree,
    clock: beebeeb_core::backup::RebuildClock,
) -> Result<usize, beebeeb_backup::HostError> {
    use beebeeb_backup::BackupHost;
    if !host.records(category).await?.is_empty() {
        return Ok(0);
    }
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
        let expected = beebeeb_core::backup::RecordRef {
            key: record.key.clone(),
            remote_id: record.remote_id,
        };
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
