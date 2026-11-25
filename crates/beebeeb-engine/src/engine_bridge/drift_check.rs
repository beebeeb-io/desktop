//! The sync drift check (folder digests, task 13): find a server change no op told this PC about.
//!
//! Sync follows the server's op log. A server write that records no op (an admin restore, a janitor, an erasure, a
//! pool delete) leaves the mirror wrong until a full re-snapshot. This check runs from the idle pull (the delta came
//! back empty at the head), at most once per [`CHECK_EVERY`], in the person's own files only:
//!
//! 1. The walker (`folder_digest::changed_folders`) runs from the root against the digests stored in
//!    `remote_folder_digests`: an unchanged tree costs one request.
//! 2. Each folder whose digest changed is listed (the engine's listing, `prepare_owned_rows` and the stored or
//!    decrypted name) and its direct children compared with the mirror's rows under it: ids, kind, size, version
//!    and object version id (when the listing carries them), and the name against the mirror path's leaf.
//! 3. A child that is uploading (on the server or here), or that has a local op queued, makes the folder in flux:
//!    not stored, not counted. A server folder the mirror lacks that is a system folder (`system_role` or
//!    `managed_by`) is skipped: sync may leave those out.
//! 4. A folder that agrees, whose changed subfolders all agree too, is stored by the after-read rule (its
//!    `digest_now`, asked after everything was read, is still the digest the walk read). A folder whose subtree
//!    holds a disagreeing or in-flux folder is not stored, so the next walk enters it again.
//! 5. A folder that disagrees is drift only when the next check, with no op applied in between (the sync cursor did
//!    not move), still disagrees: then a re-snapshot is asked for (`StateDb::request_resnapshot`) and the stored
//!    digests are cleared, so the check after it reads every folder again.
//!
//! A server without digests (`Unsupported`) is not asked again in this process; `NotReady` and failures wait for the
//! next window. One log line per check, counts only: never an id or a name.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::*;
use crate::folder_digest::{FolderDigests, RequestCount, WalkError, changed_folders, digest_now};
use crate::state_db::folder_digests::DriftChild;

/// How often the check runs at most.
pub(super) const CHECK_EVERY: Duration = Duration::from_secs(10 * 60);

/// More rows than this in one folder listing is refused (a cursor loop, not a folder).
const LISTING_CAP: usize = 200_000;

/// What one check did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DriftAction {
    /// Nothing changed since the stored digests.
    Unchanged,
    /// Every changed folder agreed (or was in flux); what agreed is stored.
    Agreed,
    /// A folder disagrees for the first time: the next check decides.
    Watching,
    /// A folder disagreed twice with no op applied in between: a re-snapshot is asked for.
    Resnapshot,
    /// The server has no digests: no check for the rest of this run.
    Unsupported,
    /// The account's digests are not computed yet: the next window.
    NotReady,
    /// A request or a read failed, or the engine is stopping: the next window.
    Failed,
}

impl DriftAction {
    fn as_str(self) -> &'static str {
        match self {
            DriftAction::Unchanged => "unchanged",
            DriftAction::Agreed => "agreed",
            DriftAction::Watching => "watching",
            DriftAction::Resnapshot => "resnapshot",
            DriftAction::Unsupported => "unsupported",
            DriftAction::NotReady => "not_ready",
            DriftAction::Failed => "failed",
        }
    }
}

/// One check's counts (what its log line says).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DriftReport {
    pub requests: u32,
    pub changed: usize,
    pub in_flux: usize,
    pub disagreeing: usize,
    pub stored: usize,
    pub action: DriftAction,
}

impl DriftReport {
    fn new(action: DriftAction) -> Self {
        DriftReport {
            requests: 0,
            changed: 0,
            in_flux: 0,
            disagreeing: 0,
            stored: 0,
            action,
        }
    }
}

/// The folders that disagreed at the last check, and the sync cursor then.
#[derive(Debug, Clone)]
struct Watching {
    cursor: Option<i64>,
    folders: HashSet<String>,
}

/// The check's memory for one sync root, for the life of this process.
#[derive(Default)]
struct RootState {
    last_started: Option<Instant>,
    running: bool,
    unsupported: bool,
    watching: Option<Watching>,
}

fn states() -> &'static Mutex<HashMap<PathBuf, RootState>> {
    static STATES: OnceLock<Mutex<HashMap<PathBuf, RootState>>> = OnceLock::new();
    STATES.get_or_init(Default::default)
}

fn with_state<R>(sync_root: &Path, f: impl FnOnce(&mut RootState) -> R) -> R {
    let mut all = states().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    f(all.entry(sync_root.to_path_buf()).or_default())
}

/// Called from the idle branch of the pull: runs the check when its window is due. Never fails the tick.
pub(super) async fn after_idle_pull(bridge: &EngineBridge, sync_root: &Path) {
    let due = with_state(sync_root, |state| {
        let due =
            !state.unsupported && !state.running && state.last_started.is_none_or(|at| at.elapsed() >= CHECK_EVERY);
        if due {
            state.running = true;
            state.last_started = Some(Instant::now());
        }
        due
    });
    if due {
        check_now(bridge, sync_root).await;
    }
}

/// One check now, whatever the window (the window is [`after_idle_pull`]'s). A server found without digests is
/// not asked again.
pub(crate) async fn check_now(bridge: &EngineBridge, sync_root: &Path) -> DriftReport {
    let (unsupported, watching) = with_state(sync_root, |state| {
        state.running = true;
        (state.unsupported, state.watching.take())
    });
    let (report, watching) = if unsupported {
        (DriftReport::new(DriftAction::Unsupported), None)
    } else {
        match check(bridge, watching.clone()).await {
            Ok(done) => done,
            Err(error) => {
                tracing::warn!(error = %error, "sync drift check: a read failed; the next window tries again");
                (DriftReport::new(DriftAction::Failed), watching)
            }
        }
    };
    with_state(sync_root, |state| {
        state.running = false;
        state.watching = watching;
        if report.action == DriftAction::Unsupported {
            state.unsupported = true;
        }
    });
    if !unsupported {
        tracing::info!(
            requests = report.requests,
            changed = report.changed,
            in_flux = report.in_flux,
            disagreeing = report.disagreeing,
            stored = report.stored,
            action = report.action.as_str(),
            "sync drift check"
        );
    }
    report
}

/// How one listed folder compares with the mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Agrees,
    InFlux,
    Disagrees,
}

async fn check(bridge: &EngineBridge, watching: Option<Watching>) -> anyhow::Result<(DriftReport, Option<Watching>)> {
    let count = RequestCount::default();
    let cursor = bridge.db().get_sync_cursor()?;
    let known = bridge.db().remote_folder_digests()?;
    let ended = |action: DriftAction, count: &RequestCount| {
        let mut report = DriftReport::new(action);
        report.requests = count.get();
        report
    };
    let changed = match changed_folders(bridge.api(), "", &known, &count).await {
        Ok(changed) => changed,
        Err(WalkError::Unsupported) => return Ok((ended(DriftAction::Unsupported, &count), None)),
        Err(WalkError::NotReady { .. }) => return Ok((ended(DriftAction::NotReady, &count), watching)),
        Err(error) => {
            tracing::warn!(error = %error, "sync drift check: the walk failed; the next window tries again");
            return Ok((ended(DriftAction::Failed, &count), watching));
        }
    };
    if changed.is_empty() {
        // Nothing moved since the digests stored: what disagreed then is not seen again, so nothing is watched.
        return Ok((ended(DriftAction::Unchanged, &count), None));
    }

    let mut verdicts = Vec::with_capacity(changed.len());
    for folder in &changed {
        if bridge.is_stopping() {
            return Ok((ended(DriftAction::Failed, &count), watching));
        }
        verdicts.push(compare_folder(bridge, &folder.id, &count).await?);
    }

    // Children before parents (the walk lists parents first): a folder is trusted only when it and every changed
    // folder under it agree.
    let index: HashMap<&str, usize> = changed.iter().enumerate().map(|(i, f)| (f.id.as_str(), i)).collect();
    let mut whole = vec![false; changed.len()];
    for i in (0..changed.len()).rev() {
        whole[i] = verdicts[i] == Verdict::Agrees
            && changed[i]
                .child_folders
                .iter()
                .all(|(child, _)| index.get(child.as_str()).is_none_or(|&j| whole[j]));
    }
    // The after-read rule: what was read under a digest is trusted only when that digest still holds now.
    let mut trusted = Vec::new();
    let mut forget = Vec::new();
    for (folder, whole) in changed.iter().zip(&whole) {
        if *whole && after_read_holds(bridge, folder, &count).await? {
            trusted.push((folder.id.clone(), folder.digest.clone()));
        } else {
            forget.push(folder.id.clone());
        }
    }
    bridge.db().update_remote_folder_digests(&trusted, &forget)?;

    let disagreeing: HashSet<String> = changed
        .iter()
        .zip(&verdicts)
        .filter(|(_, verdict)| **verdict == Verdict::Disagrees)
        .map(|(folder, _)| folder.id.clone())
        .collect();
    let mut report = DriftReport {
        requests: 0,
        changed: changed.len(),
        in_flux: verdicts.iter().filter(|v| **v == Verdict::InFlux).count(),
        disagreeing: disagreeing.len(),
        stored: trusted.len(),
        action: DriftAction::Agreed,
    };
    let next = if disagreeing.is_empty() {
        None
    } else if watching.is_some_and(|seen| seen.cursor == cursor && !seen.folders.is_disjoint(&disagreeing)) {
        report.action = DriftAction::Resnapshot;
        bridge.db().request_resnapshot()?;
        bridge.db().clear_remote_folder_digests()?;
        None
    } else {
        report.action = DriftAction::Watching;
        Some(Watching {
            cursor,
            folders: disagreeing,
        })
    };
    report.requests = count.get();
    Ok((report, next))
}

async fn after_read_holds(bridge: &EngineBridge, folder: &FolderDigests, count: &RequestCount) -> anyhow::Result<bool> {
    match digest_now(bridge.api(), &folder.id, count).await {
        Ok(now) => Ok(now == folder.digest),
        Err(WalkError::FolderGone(_)) => Ok(false),
        Err(WalkError::Failed(error)) => Err(error),
        Err(other) => Err(anyhow::anyhow!("{other}")),
    }
}

/// Every non-trashed child row of `folder_id` (`""` = the root), all pages, each page counted.
/// `None`: the folder is gone (its parent's digest moved, so it is not trusted either way).
async fn list_folder(
    bridge: &EngineBridge,
    folder_id: &str,
    count: &RequestCount,
) -> anyhow::Result<Option<Vec<serde_json::Value>>> {
    let mut rows = Vec::new();
    let mut cursor: Option<String> = None;
    let mut cursors = HashSet::new();
    loop {
        count.add(1);
        let page = if folder_id.is_empty() {
            bridge.api().list_root_page(cursor.as_deref()).await
        } else {
            bridge.api().list_owned_folder_page(folder_id, cursor.as_deref()).await
        };
        let page = match page {
            Ok(page) => page,
            Err(error) if !folder_id.is_empty() && error_http_status(&error) == Some(404) => return Ok(None),
            Err(error) => return Err(error),
        };
        let files = page["files"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("a folder listing without its rows"))?;
        rows.extend(
            files
                .iter()
                .filter(|row| row["is_trashed"].as_bool() != Some(true))
                .cloned(),
        );
        anyhow::ensure!(rows.len() <= LISTING_CAP, "a folder listing past {LISTING_CAP} rows");
        match &page["next_cursor"] {
            serde_json::Value::String(next) if !next.is_empty() && cursors.insert(next.clone()) => {
                cursor = Some(next.clone());
            }
            serde_json::Value::Null => return Ok(Some(rows)),
            _ => anyhow::bail!("a folder listing whose cursor does not move"),
        }
    }
}

/// The name of a listed row this PC reads: the stored owner-key record's, else core's reader over
/// `name_encrypted`. `None`: no name this PC can read (blocked, unreadable): the name is not compared.
fn server_name(bridge: &EngineBridge, id: &str, row: &serde_json::Value) -> Option<String> {
    let master = bridge.api().master_key();
    let name = match crate::owner_keys::stored_name(bridge.db(), id, master).ok()? {
        crate::owner_keys::StoredName::Name(name) => name,
        crate::owner_keys::StoredName::Blocked => return None,
        crate::owner_keys::StoredName::None => {
            let text = row["name_encrypted"].as_str().filter(|text| !text.trim().is_empty())?;
            let master = beebeeb_core::kdf::MasterKey::from_bytes(*master);
            beebeeb_core::encrypt::decrypt_name(&master, id, text).ok()?
        }
    };
    let name = name.trim().trim_matches('/').to_string();
    (!name.is_empty()).then_some(name)
}

/// The mirror path ends in `name` (its leaf; an old plaintext name may itself hold a `/`).
fn path_ends_in(path: &str, name: &str) -> bool {
    let path = path.trim_start_matches('/');
    path == name || path.strip_suffix(name).is_some_and(|parent| parent.ends_with('/'))
}

fn is_system_folder(row: &serde_json::Value) -> bool {
    !row["system_role"].is_null() || !row["managed_by"].is_null()
}

/// Compare one folder's server children with the mirror's rows under it.
async fn compare_folder(bridge: &EngineBridge, folder_id: &str, count: &RequestCount) -> anyhow::Result<Verdict> {
    let Some(rows) = list_folder(bridge, folder_id, count).await? else {
        return Ok(Verdict::InFlux);
    };
    if rows.iter().any(|row| row["is_uploading"].as_bool() == Some(true)) {
        return Ok(Verdict::InFlux);
    }
    let mirror = bridge.db().drift_children(folder_id)?;
    if mirror.iter().any(|child| {
        matches!(
            child.status,
            FileStatus::Uploading | FileStatus::Error | FileStatus::Conflict | FileStatus::Trashing
        )
    }) || bridge.db().local_op_queued_in_folder(folder_id)?
    {
        return Ok(Verdict::InFlux);
    }
    let rows: Vec<serde_json::Value> = bridge
        .prepare_owned_rows(&rows)
        .await?
        .into_iter()
        .map(|p| p.row)
        .collect();
    let mut mirror: HashMap<&str, &DriftChild> = mirror.iter().map(|c| (c.file_id.as_str(), c)).collect();
    for row in &rows {
        let Some(id) = row["id"].as_str().filter(|id| !id.is_empty()) else {
            continue;
        };
        let is_folder = row["is_folder"].as_bool().unwrap_or(false);
        let Some(child) = mirror.remove(id) else {
            if is_folder && is_system_folder(row) {
                continue;
            }
            return Ok(Verdict::Disagrees);
        };
        if child.is_folder != is_folder {
            return Ok(Verdict::Disagrees);
        }
        if !is_folder {
            let size = row["size_bytes"].as_i64().or_else(|| row["size"].as_i64());
            let version = row["version_number"]
                .as_i64()
                .or_else(|| row["current_version"].as_i64());
            let object = row["current_object_version_id"].as_str();
            if size.is_some_and(|size| size != child.size_bytes)
                || version.is_some_and(|version| version != child.current_version)
                || object.is_some_and(|object| child.current_object_version_id.as_deref() != Some(object))
            {
                return Ok(Verdict::Disagrees);
            }
        }
        if !child.path.is_empty()
            && !child.name_unreadable
            && server_name(bridge, id, row).is_some_and(|name| !path_ends_in(&child.path, &name))
        {
            return Ok(Verdict::Disagrees);
        }
    }
    // A mirror row the server no longer lists.
    Ok(if mirror.is_empty() {
        Verdict::Agrees
    } else {
        Verdict::Disagrees
    })
}

/// Against the fake server (`test_api`): the account's tree, its digests and its op log, through the real tick.
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
    use crate::test_api::{FakeApi, Handler, Reply, Request};

    fn uid(kind: u32, n: u32) -> String {
        format!("d{kind:07x}-0013-4000-8000-{n:012x}")
    }

    /// The fake's own handler, with `GET /api/v1/files` answered as the real server pages it (`files`,
    /// `next_cursor`): the fake's listing says `items`, which the engine's listing does not read.
    fn handler(account: Arc<FakeAccount>) -> Handler {
        let inner = account_handler(account);
        Arc::new(move |req: &Request| {
            if req.method == "GET" && req.path.split('?').next() == Some("/api/v1/files") {
                let parent = req.query("parent_id").map(str::to_string);
                let mut snapshot = req.clone();
                snapshot.path = "/api/v1/sync/snapshot".into();
                let reply = inner(&snapshot);
                if reply.status != 200 {
                    return reply;
                }
                let body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
                let files: Vec<serde_json::Value> = body["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|node| node["parent_id"].as_str().map(str::to_string) == parent)
                    .cloned()
                    .collect();
                return Reply::ok(json!({ "files": files, "next_cursor": null }));
            }
            inner(req)
        })
    }

    struct Fixture {
        account: Arc<FakeAccount>,
        fake: FakeApi,
        bridge: EngineBridge,
        root: PathBuf,
        _dir: tempfile::TempDir,
    }

    impl Fixture {
        /// Root: "Docs" (two files, "Deep" under it with one file) and a file at the top.
        fn new() -> Self {
            let (account, _) = FakeAccount::register("drift@example.invalid", "an invented password", TEST_PHRASE);
            let account = Arc::new(account);
            account.add_item(&uid(1, 1), None, "Docs", true, 0);
            account.add_item(&uid(2, 1), Some(&uid(1, 1)), "a.txt", false, 10);
            account.add_item(&uid(2, 2), Some(&uid(1, 1)), "b.txt", false, 20);
            account.add_item(&uid(1, 2), Some(&uid(1, 1)), "Deep", true, 0);
            account.add_item(&uid(2, 3), Some(&uid(1, 2)), "c.txt", false, 30);
            account.add_file_with_content(&uid(2, 4), None, "top.txt", b"top content");
            let fake = FakeApi::start(handler(account.clone()));
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("Beebeeb");
            std::fs::create_dir_all(&root).unwrap();
            let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
            let api = Arc::new(ApiClient::new(
                fake.base.clone(),
                account.session_token.clone(),
                account.master_key,
            ));
            let bridge = EngineBridge::new(&crate::identity::TEST, db, api);
            Fixture {
                account,
                fake,
                bridge,
                root,
                _dir: dir,
            }
        }

        async fn tick(&self) {
            sync_tick(&self.bridge, &self.root).await.unwrap();
        }

        fn digest_requests(&self) -> usize {
            self.fake
                .routes()
                .iter()
                .filter(|route| route.contains("folder-digests"))
                .count()
        }

        /// A re-snapshot is asked for (read without taking it).
        fn resnapshot_asked(&self) -> bool {
            self.bridge.db().peek_resnapshot_request().unwrap().is_some()
        }

        /// Bootstrap, then the first check (the table is empty: every folder is read and stored).
        async fn synced_and_checked(&self) -> DriftReport {
            self.tick().await;
            assert!(self.bridge.db().get_sync_cursor().unwrap().is_some(), "bootstrapped");
            let first = check_now(&self.bridge, &self.root).await;
            println!("drift first check: {first:?}");
            assert_eq!(first.action, DriftAction::Agreed, "{first:?}");
            assert_eq!(first.disagreeing, 0);
            // Root, Docs, Deep.
            assert_eq!(first.stored, 3, "{first:?}");
            assert_eq!(self.bridge.db().remote_folder_digests().unwrap().len(), 3);
            assert!(!self.resnapshot_asked());
            first
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_three_idle_checks_after_the_first_ask_nothing() {
        let fx = Fixture::new();
        fx.synced_and_checked().await;
        for n in 0..3 {
            let report = check_now(&fx.bridge, &fx.root).await;
            println!("drift idle check {n}: {report:?}");
            assert_eq!(report.action, DriftAction::Unchanged);
            assert!(report.requests <= 2, "{report:?}");
            assert!(!fx.resnapshot_asked());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_a_change_without_an_op_is_found_in_two_checks() {
        let fx = Fixture::new();
        fx.synced_and_checked().await;
        // The server changes c.txt (two levels down) and records no op.
        fx.account.add_item(&uid(2, 3), Some(&uid(1, 2)), "c.txt", false, 31);
        fx.tick().await;
        assert!(!fx.resnapshot_asked(), "the tick alone sees nothing");

        let first = check_now(&fx.bridge, &fx.root).await;
        println!("drift check 1 after the change: {first:?}");
        assert_eq!(first.action, DriftAction::Watching);
        assert_eq!(first.disagreeing, 1);
        assert!(!fx.resnapshot_asked());

        let second = check_now(&fx.bridge, &fx.root).await;
        println!("drift check 2 after the change: {second:?}");
        assert_eq!(second.action, DriftAction::Resnapshot);
        assert!(fx.resnapshot_asked());
        assert!(fx.bridge.db().remote_folder_digests().unwrap().is_empty());

        // The re-snapshot brings the mirror back; the next check reads every folder and agrees.
        fx.tick().await;
        assert!(!fx.resnapshot_asked());
        let after = check_now(&fx.bridge, &fx.root).await;
        println!("drift check after the re-snapshot: {after:?}");
        assert_eq!(after.action, DriftAction::Agreed);
        assert_eq!(after.disagreeing, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_a_change_with_its_op_is_applied_by_sync_and_asks_nothing() {
        let fx = Fixture::new();
        fx.synced_and_checked().await;
        fx.account.push_newer_version(&uid(2, 4), b"a newer version").unwrap();
        fx.tick().await;
        for n in 0..2 {
            let report = check_now(&fx.bridge, &fx.root).await;
            println!("drift check {n} after a change with its op: {report:?}");
            assert_eq!(report.disagreeing, 0, "{report:?}");
            assert!(!fx.resnapshot_asked());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_an_uploading_row_is_in_flux() {
        let fx = Fixture::new();
        fx.synced_and_checked().await;
        fx.account.add_item(&uid(2, 1), Some(&uid(1, 1)), "a.txt", false, 11);
        fx.bridge.db().set_status(&uid(2, 1), FileStatus::Uploading).unwrap();
        for n in 0..3 {
            let report = check_now(&fx.bridge, &fx.root).await;
            println!("drift check {n} with an uploading row: {report:?}");
            assert_eq!(report.in_flux, 1, "{report:?}");
            assert_eq!(report.disagreeing, 0);
            assert!(!fx.resnapshot_asked());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_old_server_is_not_asked_again() {
        let fx = Fixture::new();
        fx.account.digests_as_old_server(true);
        fx.tick().await;
        // The first idle tick runs the check through the tick itself.
        fx.tick().await;
        assert_eq!(fx.digest_requests(), 1, "{:?}", fx.fake.routes());
        let before = fx.fake.routes().len();
        let report = check_now(&fx.bridge, &fx.root).await;
        assert_eq!(report.action, DriftAction::Unsupported);
        assert_eq!(report.requests, 0);
        fx.tick().await;
        assert_eq!(fx.digest_requests(), 1);
        assert!(
            !fx.fake.routes()[before..]
                .iter()
                .any(|route| route.contains("/api/v1/files"))
        );
        assert!(!fx.resnapshot_asked());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_runs_from_the_idle_tick_once_per_window() {
        let fx = Fixture::new();
        fx.tick().await;
        assert_eq!(fx.digest_requests(), 0, "the bootstrap tick runs no check");
        fx.tick().await;
        let after_first = fx.digest_requests();
        assert!(after_first >= 3, "the first idle tick reads every folder");
        assert_eq!(fx.bridge.db().remote_folder_digests().unwrap().len(), 3);
        fx.tick().await;
        assert_eq!(fx.digest_requests(), after_first, "within the window: no check");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drift_check_not_ready_waits_for_the_next_window() {
        let fx = Fixture::new();
        fx.account.digests_not_ready(Some(30));
        fx.tick().await;
        let report = check_now(&fx.bridge, &fx.root).await;
        assert_eq!(report.action, DriftAction::NotReady);
        fx.account.digests_not_ready(None);
        let report = check_now(&fx.bridge, &fx.root).await;
        assert_eq!(report.action, DriftAction::Agreed, "{report:?}");
    }
}
