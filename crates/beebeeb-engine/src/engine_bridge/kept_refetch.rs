//! S6: a file kept on this PC ("Always keep on this device") that this PC does not hold is fetched again.
//!
//! A kept file can be missing because its one fetch failed (offline, a quit, an ended session) or because the stale
//! pass emptied it for a newer version. "Kept" is exactly the database's rule (`StateDb::kept_files_to_fetch`: its own
//! pin, or one inherited from a folder pinned while the file was known). The step runs at a tick whose pull reached
//! the server, after the stale pass and before the queue step; none is started while sync is paused (the paused tick
//! returns first) or while a sign-out's send phase is armed. It never downloads itself; the fetch goes through the
//! download speed limit:
//! - **Windows:** ONE batch at a time, off the tick, ONE file at a time through `hydrate_placeholder` (its callback is
//!   the paced download), pause and stop read again before each file. A failure backs off: tried again at the next
//!   step run, then +2, +4, +8, … at most every 64 runs, in memory for this engine run.
//! - **Linux:** a `HydrateFile` op (reason [`KEPT_REASON`]) into an empty place; the queue runs it at the limit and
//!   retries it by its own rule.
//! - **macOS:** nothing of the engine's own, and no `HydrateFile` op (dm-71, dm-72): a kept file's one copy is
//!   Finder's, and the system fetches it (the keep asks it to, the item's content policy keeps it, and a new version
//!   comes by the stale pass's signal, `stale_copies.rs`). A fetch queued here was a second copy in the hidden state
//!   folder and a download beside the system's.

use super::*;
use std::sync::Mutex;

/// How many kept files one step starts beyond the stale pass's own; the rest wait for a later tick.
pub const KEPT_REFETCH_PER_TICK: usize = 8;

/// How many of the rule's rows one step reads: more than it acts on, so files that keep failing (backed off) never
/// hold back the files behind them.
pub const KEPT_REFETCH_READ_PER_TICK: usize = 256;

/// The op's reason, so a kept fetch that meets a change, or whose row moved, is dropped, never retried over it.
pub(super) const KEPT_REASON: &str = "kept_refetch";

/// A kept file to fetch and its place in the sync folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptPlace {
    pub file_id: String,
    pub place: PathBuf,
}

/// What one fetch of a kept file did.
#[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptFetch {
    Done,
    Failed,
    /// Nothing at the place yet (a placeholder not minted): fetched at a later step, not counted as a failure.
    NotThere,
}

/// What one batch did.
#[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KeptBatch {
    pub fetched: usize,
    pub failed: usize,
    pub not_there: usize,
    /// Pause or stop ended the batch before its last file.
    pub stopped: bool,
}

/// The step's memory for one engine run: whether a batch runs, how many step runs there were, and each failing
/// file's backoff (its failures, and the step run it may be tried again at). Nothing of it is stored.
#[derive(Debug, Default)]
pub struct KeptRefetch {
    running: AtomicBool,
    round: AtomicU64,
    backoff: Mutex<HashMap<String, (u32, u64)>>,
}

/// A batch is running while this lives; dropped (a panic included), the next step may start one.
#[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
pub struct RunningGuard(Arc<KeptRefetch>);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.running.store(false, Ordering::SeqCst);
    }
}

/// Step runs skipped after the `failures`-th failure: 0, 1, 3, 7, … at most 63, so a failing file is tried again at
/// runs +1, +2, +4, +8, … and at most every 64 runs.
fn skip(failures: u32) -> u64 {
    (1u64 << failures.saturating_sub(1).min(6)) - 1
}

impl KeptRefetch {
    /// `None` while a batch runs (one at a time: whole-file and direct downloads are paced per transfer).
    #[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
    pub fn try_start(self: &Arc<Self>) -> Option<RunningGuard> {
        (!self.running.swap(true, Ordering::SeqCst)).then(|| RunningGuard(Arc::clone(self)))
    }

    /// Fetch `batch`, one file at a time; `go_on()` is asked before each file (pause, stop), and `false` ends it. An
    /// attempt counts as done only when the row reads `local` afterwards; anything else backs the file off.
    #[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
    pub fn run_batch(
        &self,
        db: &StateDb,
        batch: Vec<KeptPlace>,
        fetch: &dyn Fn(&Path) -> KeptFetch,
        go_on: &dyn Fn() -> bool,
    ) -> KeptBatch {
        let mut done = KeptBatch::default();
        for kept in batch {
            if !go_on() {
                done.stopped = true;
                break;
            }
            match fetch(&kept.place) {
                KeptFetch::NotThere => done.not_there += 1,
                KeptFetch::Done if Self::reads_local(db, &kept.file_id) => {
                    self.lock_backoff().remove(&kept.file_id);
                    done.fetched += 1;
                }
                KeptFetch::Done | KeptFetch::Failed => {
                    self.failed(&kept.file_id);
                    done.failed += 1;
                }
            }
        }
        if done != KeptBatch::default() {
            tracing::info!(
                fetched = done.fetched,
                failed = done.failed,
                not_there = done.not_there,
                stopped = done.stopped,
                "kept files: fetched again"
            );
        }
        done
    }

    #[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
    fn reads_local(db: &StateDb, file_id: &str) -> bool {
        db.get_file(file_id)
            .ok()
            .flatten()
            .is_some_and(|row| row.status == FileStatus::Local)
    }

    /// One more failure: the file waits [`skip`] step runs.
    fn failed(&self, file_id: &str) {
        let round = self.round.load(Ordering::SeqCst);
        let mut backoff = self.lock_backoff();
        let entry = backoff.entry(file_id.to_string()).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = round + 1 + skip(entry.0);
    }

    fn lock_backoff(&self) -> std::sync::MutexGuard<'_, HashMap<String, (u32, u64)>> {
        self.backoff.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl EngineBridge {
    /// One step run's kept files: `extra` first (the stale pass's kept files, already capped by its own pass), then at
    /// most [`KEPT_REFETCH_PER_TICK`] of the rule's rows that are not backed off, by file id once each.
    pub fn kept_refetch_plan(
        &self,
        sync_root: &Path,
        state: &KeptRefetch,
        extra: Vec<KeptPlace>,
    ) -> Result<Vec<KeptPlace>, EngineError> {
        let round = state.round.fetch_add(1, Ordering::SeqCst) + 1;
        let mut plan: Vec<KeptPlace> = Vec::new();
        for kept in extra {
            if !plan.iter().any(|planned| planned.file_id == kept.file_id) {
                plan.push(kept);
            }
        }
        let rows = self
            .db
            .kept_files_to_fetch(KEPT_REFETCH_READ_PER_TICK, cfg!(target_os = "windows"))?;
        let backoff = state.lock_backoff();
        let mut new = 0;
        for row in rows {
            if new == KEPT_REFETCH_PER_TICK {
                break;
            }
            let backed_off = backoff.get(&row.file_id).is_some_and(|(_, next)| *next > round);
            if backed_off || plan.iter().any(|planned| planned.file_id == row.file_id) {
                continue;
            }
            let Ok(place) = local_file_path_under_sync_root(sync_root, &row.path) else {
                continue;
            };
            plan.push(KeptPlace {
                file_id: row.file_id,
                place,
            });
            new += 1;
        }
        Ok(plan)
    }

    /// Linux: queue a fetch (`HydrateFile`, reason [`KEPT_REASON`]) for each kept file the plan names whose place holds
    /// nothing at all. Anything there (an empty file is the person's too), or a place that cannot be read, is skipped
    /// and backed off, so it never holds back the files behind it. It never downloads. Returns how many it queued.
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    pub fn queue_kept_fetches(&self, sync_root: &Path, state: &KeptRefetch) -> Result<usize, EngineError> {
        // Dm-72: where the system's own fetch brings a kept file's bytes (a Mac), none is queued here.
        if kept_bytes_come_by_the_systems_fetch() {
            return Ok(0);
        }
        let mut queued = 0;
        for kept in self.kept_refetch_plan(sync_root, state, Vec::new())? {
            let empty =
                matches!(std::fs::symlink_metadata(&kept.place), Err(e) if e.kind() == std::io::ErrorKind::NotFound);
            let Some(entry) = self.db.get_file(&kept.file_id)?.filter(|_| empty) else {
                state.failed(&kept.file_id);
                continue;
            };
            self.enqueue_hydrate_operation(&entry, now_secs(), KEPT_REASON)?;
            queued += 1;
        }
        if queued > 0 {
            tracing::info!(queued, "kept files: fetches queued");
        }
        Ok(queued)
    }
}

/// Windows: the fetch of one kept file, `hydrate_placeholder` (it blocks until the bytes are on disk, through the
/// paced callback, and never writes over bytes on disk).
#[cfg(target_os = "windows")]
pub(crate) fn fetch_kept_placeholder(place: &Path) -> KeptFetch {
    if std::fs::symlink_metadata(place).is_err() {
        return KeptFetch::NotThere;
    }
    match crate::windows_cf::placeholders::hydrate_placeholder(place) {
        Ok(()) => KeptFetch::Done,
        Err(error) => {
            tracing::warn!(%error, "a kept file was not fetched again; a later tick tries again");
            KeptFetch::Failed
        }
    }
}

/// Windows, the tick's step: start ONE batch off the tick unless one runs. It plans with the stale pass's kept files
/// first and fetches one file at a time while sync is not paused and the engine not stopping. Not awaited: the
/// thread is detached, as the pin sweep's is; an in-flight fetch ends when the callbacks are revoked at stop.
#[cfg(target_os = "windows")]
pub(crate) fn spawn_kept_batch(
    bridge: Arc<EngineBridge>,
    sync_root: PathBuf,
    state: Arc<KeptRefetch>,
    extra: Vec<KeptPlace>,
    sync_paused: Arc<AtomicBool>,
) {
    let Some(running) = state.try_start() else {
        return;
    };
    drop(tokio::task::spawn_blocking(move || {
        let _running = running;
        let plan = match bridge.kept_refetch_plan(&sync_root, &state, extra) {
            Ok(plan) => plan,
            Err(error) => return tracing::warn!(%error, "kept files: the plan did not complete; next tick retries"),
        };
        let go_on = || !sync_paused.load(Ordering::Relaxed) && !bridge.is_stopping();
        state.run_batch(&bridge.db, plan, &fetch_kept_placeholder, &go_on);
    }));
}

/// Linux, the tick's step: queue this tick's kept fetches off the async workers (the plan reads the database and
/// each place); this tick's queue step runs them.
#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
pub(crate) async fn queue_kept_step(bridge: &Arc<EngineBridge>, sync_root: &Path, state: &Arc<KeptRefetch>) {
    let (bridge, sync_root, state) = (Arc::clone(bridge), sync_root.to_path_buf(), Arc::clone(state));
    match tokio::task::spawn_blocking(move || bridge.queue_kept_fetches(&sync_root, &state)).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => tracing::warn!(%error, "kept files: the step did not complete; next tick retries"),
        Err(error) => tracing::warn!(%error, "kept files: the step panicked; next tick retries"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// The status a failed fetch leaves on a row the plan selects again: Windows `error` (R1). Elsewhere the batch
    /// does not run in production and the plan never selects `error`, so the row stays `cloud_only`.
    const FAILED: FileStatus = if cfg!(target_os = "windows") {
        FileStatus::Error
    } else {
        FileStatus::CloudOnly
    };

    fn bridge() -> (tempfile::TempDir, EngineBridge) {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(
            "https://api.beebeeb.io".into(),
            "token".into(),
            [7u8; 32],
        ));
        (dir, EngineBridge::new(&crate::identity::TEST, db, api))
    }

    /// A kept (own pin) cloud-only file `<id>.txt`.
    fn kept(bridge: &EngineBridge, id: &str) {
        let entry = FileEntry {
            file_id: id.into(),
            path: format!("{id}.txt"),
            status: FileStatus::CloudOnly,
            size_bytes: 10,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        bridge
            .db
            .upsert_file_with_contract(&entry, |contract| contract.item_kind = ItemKind::File)
            .unwrap();
        bridge.db.set_recursive_pin(id, true, 1).unwrap();
    }

    fn ids(plan: &[KeptPlace]) -> Vec<String> {
        plan.iter().map(|kept| kept.file_id.clone()).collect()
    }

    fn id_of(place: &Path) -> String {
        place.file_stem().unwrap().to_string_lossy().into_owned()
    }

    /// Runs `rounds` step runs with `fetch` (given the round and the file id); returns each file's attempted rounds.
    fn run_rounds(
        bridge: &EngineBridge,
        root: &Path,
        rounds: u64,
        fetch: impl Fn(&str) -> KeptFetch,
    ) -> HashMap<String, Vec<u64>> {
        let state = KeptRefetch::default();
        let attempts = RefCell::new(HashMap::<String, Vec<u64>>::new());
        let round = Cell::new(0);
        for r in 1..=rounds {
            round.set(r);
            let plan = bridge.kept_refetch_plan(root, &state, Vec::new()).unwrap();
            let fake = |place: &Path| {
                let id = id_of(place);
                attempts.borrow_mut().entry(id.clone()).or_default().push(round.get());
                fetch(&id)
            };
            state.run_batch(&bridge.db, plan, &fake, &|| true);
        }
        attempts.into_inner()
    }

    /// S6 R3: a step starts at most the cap of the rule's files; the stale pass's kept files come first, once, and do
    /// not count against it.
    #[test]
    fn s6_kept_a_tick_starts_at_most_the_cap() {
        let (dir, bridge) = bridge();
        for n in 0..20 {
            kept(&bridge, &format!("k{n:02}"));
        }
        let plan = bridge
            .kept_refetch_plan(dir.path(), &KeptRefetch::default(), Vec::new())
            .unwrap();
        let extra = KeptPlace {
            file_id: "k05".into(),
            place: dir.path().join("k05.txt"),
        };
        let with_extra = bridge
            .kept_refetch_plan(dir.path(), &KeptRefetch::default(), vec![extra.clone(), extra.clone()])
            .unwrap();
        println!(
            "s6 kept cap: {} planned; with a stale extra {:?}",
            plan.len(),
            ids(&with_extra)
        );
        assert_eq!(plan.len(), KEPT_REFETCH_PER_TICK, "at most the cap");
        assert_eq!(plan[0].place, dir.path().join("k00.txt"), "the file's place");
        assert_eq!(with_extra.len(), KEPT_REFETCH_PER_TICK + 1, "the extra beside the cap");
        assert_eq!(with_extra[0], extra, "the extra first");
        assert_eq!(ids(&with_extra).iter().filter(|id| *id == "k05").count(), 1, "once");
    }

    /// Dm-72: by the Mac rule (the system's fetch brings a kept file's bytes) the step queues no fetch of its own.
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    #[test]
    fn dm72_kept_by_the_mac_rule_the_step_queues_no_engine_fetch() {
        let (dir, bridge) = bridge();
        kept(&bridge, "a");
        system_fetch_for_test(true);
        let on_a_mac = bridge.queue_kept_fetches(dir.path(), &KeptRefetch::default());
        system_fetch_for_test(false);
        assert_eq!(on_a_mac.unwrap(), 0, "nothing queued");
        assert!(bridge.db.list_operations_for_file("a").unwrap().is_empty());
        let elsewhere = bridge.queue_kept_fetches(dir.path(), &KeptRefetch::default());
        assert_eq!(elsewhere.unwrap(), 1, "Linux keeps its queued fetch");
    }

    /// S6 R6: a file whose fetch failed once is fetched at the next step run; a fetch that lands clears its backoff.
    #[test]
    fn s6_kept_fails_once_then_the_next_tick_fetches_it() {
        let (dir, bridge) = bridge();
        kept(&bridge, "a");
        let state = KeptRefetch::default();
        let first = bridge.kept_refetch_plan(dir.path(), &state, Vec::new()).unwrap();
        let failing = |_: &Path| {
            bridge.db.set_status("a", FAILED).unwrap();
            KeptFetch::Failed
        };
        let one = state.run_batch(&bridge.db, first, &failing, &|| true);
        let second = bridge.kept_refetch_plan(dir.path(), &state, Vec::new()).unwrap();
        println!("s6 kept retry: round 1 {one:?}, round 2 plans {:?}", ids(&second));
        assert_eq!(one.failed, 1);
        assert_eq!(ids(&second), ["a"], "the next step run fetches it again");
        let landing = |_: &Path| {
            bridge.db.set_status("a", FileStatus::Local).unwrap();
            KeptFetch::Done
        };
        let two = state.run_batch(&bridge.db, second, &landing, &|| true);
        assert_eq!(two.fetched, 1);
        assert!(
            state.lock_backoff().is_empty(),
            "a fetch that landed clears the backoff"
        );
    }

    /// S6 R6: files that keep failing back off (runs 1, 2, 4, 8, 16), and never hold back the file behind them.
    #[test]
    fn s6_kept_repeated_failures_back_off_and_never_starve_the_rest() {
        let (dir, bridge) = bridge();
        for n in 0..8 {
            kept(&bridge, &format!("bad{n}"));
        }
        kept(&bridge, "good");
        let attempts = run_rounds(&bridge, dir.path(), 20, |id| {
            if id == "good" {
                bridge.db.set_status(id, FileStatus::Local).unwrap();
                KeptFetch::Done
            } else {
                bridge.db.set_status(id, FAILED).unwrap();
                KeptFetch::Failed
            }
        });
        println!(
            "s6 kept backoff: bad0 {:?}, good {:?}",
            attempts.get("bad0"),
            attempts.get("good")
        );
        for n in 0..8 {
            assert_eq!(attempts[&format!("bad{n}")], [1, 2, 4, 8, 16], "bad{n} backs off");
        }
        assert_eq!(attempts["good"], [3], "the good file is fetched by run 3, once");
    }

    /// S6 R6: an attempt counts only when the row reads `local` afterwards; a place not there yet is no failure.
    #[test]
    fn s6_kept_an_attempt_counts_only_when_the_row_reads_local() {
        let (dir, bridge) = bridge();
        kept(&bridge, "stuck");
        kept(&bridge, "absent");
        let attempts = run_rounds(&bridge, dir.path(), 4, |id| {
            if id == "stuck" {
                bridge.db.set_status(id, FAILED).unwrap();
                KeptFetch::Done
            } else {
                KeptFetch::NotThere
            }
        });
        println!(
            "s6 kept counts: stuck {:?}, absent {:?}",
            attempts["stuck"], attempts["absent"]
        );
        assert_eq!(
            attempts["stuck"],
            [1, 2, 4],
            "`Done` with the row not local backs off like a failure"
        );
        assert_eq!(
            attempts["absent"],
            [1, 2, 3, 4],
            "not there: planned again at the next run"
        );
    }

    /// S6 R2 / L3: pause or stop, read before each file, ends the batch; the file being fetched finishes.
    #[test]
    fn s6_kept_the_batch_stops_at_pause_or_stop() {
        let (dir, bridge) = bridge();
        let batch: Vec<KeptPlace> = ["a", "b", "c"]
            .iter()
            .map(|id| KeptPlace {
                file_id: id.to_string(),
                place: dir.path().join(format!("{id}.txt")),
            })
            .collect();
        let fetches = Cell::new(0);
        let fetch = |_: &Path| {
            fetches.set(fetches.get() + 1);
            KeptFetch::NotThere
        };
        let done = KeptRefetch::default().run_batch(&bridge.db, batch, &fetch, &|| fetches.get() == 0);
        println!("s6 kept pause: {} fetches, {done:?}", fetches.get());
        assert_eq!(fetches.get(), 1, "paused after the first file");
        assert!(done.stopped);
    }

    /// S6 R5: one batch at a time; the guard frees the next one when dropped, after a panic too.
    #[test]
    fn s6_kept_one_batch_at_a_time() {
        let state = Arc::new(KeptRefetch::default());
        let running = state.try_start();
        assert!(running.is_some(), "the first batch starts");
        assert!(state.try_start().is_none(), "none while it runs");
        drop(running);
        assert!(state.try_start().is_some(), "the next one after it ended");
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _running = state.try_start().unwrap();
            panic!("s6: a batch panicked");
        }));
        assert!(panicked.is_err());
        assert!(state.try_start().is_some(), "and after one panicked");
    }
}
