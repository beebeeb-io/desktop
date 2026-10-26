//! Scheduling and retry state for the single account-scoped S2 job.
use crate::state_db::UnknownCheck;

pub(super) fn failed(record: &mut UnknownCheck, counted: bool, now: i64) {
    if counted {
        record.stops = 0;
    } else {
        record.attempts = record.attempts.saturating_sub(1);
        record.stops = record.stops.saturating_add(1);
    }
    let exponent = if counted { record.attempts } else { record.stops }.clamp(0, 9) as u32;
    let delay = (300i64 * (1i64 << exponent)).min(86_400);
    record.next_attempt_at = now.saturating_add(delay);
    record.state = Some(
        if record.attempts >= 8 {
            "could_not_check"
        } else {
            "waiting_retry"
        }
        .into(),
    );
    record.updated_at = now;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_2191_s2_outages_never_count_but_eight_failures_stop() {
        let mut r = UnknownCheck::default();
        for _ in 0..50 {
            r.attempts += 1;
            failed(&mut r, false, 1000);
        }
        assert_eq!(r.attempts, 0);
        assert_eq!(r.state.as_deref(), Some("waiting_retry"));
        assert_eq!(r.next_attempt_at, 87400);
        r.stops = 0;
        for _ in 0..8 {
            r.attempts += 1;
            failed(&mut r, true, 1000);
        }
        assert_eq!(r.attempts, 8);
        assert_eq!(r.state.as_deref(), Some("could_not_check"));
    }
}

/// Serializes cancellation with state promotion. Hashing/network run outside this lock.
pub(crate) struct Scope {
    live: std::sync::Mutex<bool>,
    pub bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
    pub paused: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub wake: tokio::sync::Notify,
    started: std::sync::atomic::AtomicBool,
}
impl Scope {
    pub fn new(
        bridge: std::sync::Arc<crate::engine_bridge::EngineBridge>,
        paused: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            live: std::sync::Mutex::new(true),
            bridge,
            paused,
            wake: tokio::sync::Notify::new(),
            started: std::sync::atomic::AtomicBool::new(false),
        }
    }
    pub fn alive(&self) -> bool {
        *self.live.lock().unwrap() && !self.bridge.is_stopping()
    }
    pub fn active(&self) -> bool {
        self.alive() && !self.paused.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn commit<T>(&self, operation: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
        let live = self.live.lock().unwrap();
        if !*live || self.bridge.is_stopping() || self.paused.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(super::compare::Halt::Paused.into());
        }
        operation()
    }
    pub fn cancel(&self) {
        *self.live.lock().unwrap() = false;
        self.wake.notify_waiters();
    }
    pub async fn before(&self, bytes: u64, oversize: bool) -> anyhow::Result<u64> {
        loop {
            if !self.active() {
                return Err(super::compare::Halt::Paused.into());
            }
            if self.bridge.unknown_foreground_busy() {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
            let now = chrono::Utc::now().timestamp_millis();
            let day = chrono::Utc::now().timestamp().div_euclid(86400);
            let limit = super::budget::allowance(super::cost::unmetered());
            let allowed = self.commit(|| {
                self.bridge
                    .db()
                    .update_unknown_budget(|l| l.reserve(now, day, bytes, limit, oversize))
            })?;
            match allowed {
                Ok(()) => return Ok(limit),
                Err(super::budget::Stop::Daily) => return Err(super::compare::Halt::Daily.into()),
                Err(super::budget::Stop::Until(at)) => {
                    tokio::time::sleep(std::time::Duration::from_millis((at - now).clamp(1, 1000) as u64)).await;
                }
            }
        }
    }
    pub async fn request<T>(&self, future: impl std::future::Future<Output = anyhow::Result<T>>) -> anyhow::Result<T> {
        tokio::pin!(future);
        loop {
            if !self.active() {
                return Err(super::compare::Halt::Paused.into());
            }
            tokio::select! {result=&mut future=>return result,_=tokio::time::sleep(std::time::Duration::from_millis(50))=>{}}
        }
    }
    pub async fn json(&self, url: &str) -> anyhow::Result<serde_json::Value> {
        self.before(0, false).await?;
        self.request(self.bridge.api().unknown_json(url)).await
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    #[test]
    fn task_2191_s2_pause_and_generation_cancel_prevent_next_promotion() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(crate::api_client::ApiClient::new(
            "http://127.0.0.1:9".into(),
            "test".into(),
            [0; 32],
        ));
        let paused = Arc::new(AtomicBool::new(false));
        let scope = Scope::new(
            Arc::new(crate::engine_bridge::EngineBridge::new(db, api)),
            paused.clone(),
        );
        let mut calls = 0;
        scope
            .commit(|| {
                calls += 1;
                Ok(())
            })
            .unwrap();
        paused.store(true, Ordering::SeqCst);
        assert!(!scope.active());
        assert!(
            scope
                .commit(|| {
                    calls += 1;
                    Ok(())
                })
                .is_err()
        );
        paused.store(false, Ordering::SeqCst);
        scope.cancel();
        assert!(!scope.active());
        assert!(
            scope
                .commit(|| {
                    calls += 1;
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(calls, 1);
    }
}

#[cfg(test)]
mod throughput_tests {
    #[test]
    fn task_2191_s2_twenty_thousand_files_drain_without_tick_delay() {
        let mut pending = 20_000usize;
        let mut quanta = 0;
        while pending > 0 {
            let n = super::quantum_len(pending);
            assert!(n > 0 && n <= 4);
            pending -= n;
            quanta += 1;
        }
        assert_eq!(quanta, 5000);
        assert_eq!(super::idle_delay(true), std::time::Duration::ZERO);
    }
}
fn quantum_len(pending: usize) -> usize {
    pending.min(4)
}
fn idle_delay(work: bool) -> std::time::Duration {
    if work {
        std::time::Duration::ZERO
    } else {
        std::time::Duration::from_secs(1)
    }
}

use crate::state_db::UnknownRow;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

fn wholly_local(path: &Path, id: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        use super::{OnDisk, PlatformView};
        matches!(
            crate::windows_cf::placeholders::platform_view(path),
            PlatformView::PlainFile
        ) || matches!(crate::windows_cf::placeholders::platform_view(path),PlatformView::Placeholder{on_disk:OnDisk::Wholly,identity:Some(ref found),..} if found==id)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = id;
        path.is_file()
    }
}
fn path(root: &Path, row: &UnknownRow) -> anyhow::Result<PathBuf> {
    crate::engine_bridge::local_file_path_under_sync_root(root, &row.path)
}
fn save(scope: &Scope, row: &UnknownRow) -> anyhow::Result<bool> {
    scope.commit(|| {
        Ok(scope
            .bridge
            .db()
            .put_unknown_check(&row.file_id, &row.path, &row.record)?)
    })
}
/// Local work has its own task and disk allowance. No network wait can hold this loop.
fn record_quantum(scope: &Scope, root: &Path) -> anyhow::Result<usize> {
    if !scope.active() {
        return Ok(0);
    }
    let db = scope.bridge.db();
    let now = chrono::Utc::now().timestamp();
    for mut row in db.list_unknown_local_waiting(16)? {
        if row.record.state.as_deref() != Some("waiting") {
            continue;
        }
        if wholly_local(&path(root, &row)?, &row.file_id) {
            row.record.state = None;
        }
        row.record.updated_at = now;
        save(scope, &row)?;
    }
    let mut bytes = 0u64;
    let rows = db.list_unknown_to_record(4)?;
    let mut work = 0;
    for mut row in rows {
        if !scope.active() {
            break;
        }
        let path = path(root, &row)?;
        if !wholly_local(&path, &row.file_id) {
            row.record.state = Some("waiting".into());
            row.record.updated_at = now;
            save(scope, &row)?;
            work += 1;
            continue;
        }
        let len = std::fs::metadata(&path)?.len();
        if bytes > 0 && bytes.saturating_add(len) > 1024 * 1024 * 1024 {
            break;
        }
        bytes = bytes.saturating_add(len);
        match super::record::record_checked(db, &row, &path, || {}, Some(scope)) {
            Ok(true) => work += 1,
            _ => {
                // An open/changing file rotates behind untouched files; never monopolizes recording.
                row.record.updated_at = now;
                save(scope, &row)?;
            }
        }
    }
    Ok(work)
}

async fn check_quantum(scope: &Scope, root: &Path) -> anyhow::Result<usize> {
    if !scope.active() {
        return Ok(0);
    }
    let now = chrono::Utc::now().timestamp();
    let rows = scope.bridge.db().list_unknown_to_check(now, 4)?;
    let count = quantum_len(rows.len());
    let mut checked = 0usize;
    let mut matched = 0usize;
    let mut diverged = 0usize;
    for mut row in rows.into_iter().take(count) {
        if !scope.active() {
            break;
        }
        let path = path(root, &row)?;
        if !wholly_local(&path, &row.file_id) {
            row.record.state = Some("waiting".into());
            row.record.updated_at = now;
            save(scope, &row)?;
            continue;
        }
        row.record.attempts += 1;
        if !save(scope, &row)? {
            continue;
        }
        checked += 1;
        let mut source = super::source::Remote::new(scope, &row.file_id);
        let result = super::compare::compare(&mut source, &mut row.record, row.current_version, &path).await;
        use super::compare::{Comparison, Halt, HttpFailure};
        let handled = match result {
            Ok(Comparison::Matched {
                version,
                hash,
                hash_mode,
            }) => {
                match super::resolve::matched_checked(
                    scope.bridge.db(),
                    &row,
                    &path,
                    version,
                    &hash,
                    hash_mode,
                    Some(scope),
                ) {
                    Ok(true) => {
                        matched += 1;
                        continue;
                    }
                    result => result
                        .map(|_| ())
                        .and_then(|_| Err(anyhow::anyhow!("local file changed"))),
                }
            }
            Ok(Comparison::NoMatch) => {
                // Persist divergence first. Recovery is entry-driven if copying or deletion stops.
                row.record.state = Some("diverged".into());
                row.record.updated_at = now;
                save(scope, &row)?;
                let result = super::resolve::diverged_checked(scope.bridge.db(), root, &row, now, Some(scope));
                if result.is_ok() {
                    diverged += 1;
                    continue;
                }
                result.map(|_| ())
            }
            Ok(Comparison::FileGone) => {
                // The normal sync tick owns the server deletion; keep local bytes until it arrives.
                row.record.state = Some("waiting".into());
                row.record.updated_at = now;
                save(scope, &row)?;
                continue;
            }
            Ok(Comparison::Undecided) => Err(anyhow::anyhow!("could not compare")),
            Err(e) => Err(e),
        };
        if let Err(e) = handled {
            let http = e.downcast_ref::<HttpFailure>();
            let counted = e.downcast_ref::<Halt>().is_none()
                && !http.is_some_and(|h| h.0 >= 500 || matches!(h.0, 404 | 409 | 429));
            failed(&mut row.record, counted, now);
            row.record.last_error = Some(if counted { "comparison" } else { "deferred" }.into());
            if let Some(HttpFailure(429, retry)) = http {
                row.record.next_attempt_at = row
                    .record
                    .next_attempt_at
                    .max(now.saturating_add((*retry).min(i64::MAX as u64) as i64));
                scope.commit(|| {
                    scope.bridge.db().update_unknown_budget(|l| {
                        l.rate_limited(chrono::Utc::now().timestamp_millis(), *retry, row.record.stops as u32)
                    })
                })?;
            }
            // Pause may restore attempts but cannot promote content. A cancelled generation writes nothing.
            let live = scope.live.lock().unwrap();
            if *live && !scope.bridge.is_stopping() {
                scope
                    .bridge
                    .db()
                    .put_unknown_check(&row.file_id, &row.path, &row.record)?;
            }
            drop(live);
            if !counted {
                break;
            }
        }
    }
    if checked > 0 {
        tracing::info!(checked, matched, diverged, "unknown content check quantum");
    }
    Ok(checked)
}

/// One owner is kept in the runner stack. Forced runner cancellation drops it and closes
/// the generation gate, including for blocking hashing already in flight.
pub(crate) struct Job {
    scope: Arc<Scope>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Job {
    pub fn start(
        bridge: Arc<crate::engine_bridge::EngineBridge>,
        root: PathBuf,
        paused: Arc<AtomicBool>,
    ) -> anyhow::Result<Self> {
        bridge.db().clear_removing_states()?;
        bridge.db().rearm_could_not_check()?;
        bridge.db().update_unknown_budget(|l| {
            let day = chrono::Utc::now().timestamp().div_euclid(86400);
            if day > l.day {
                l.day = day;
                l.bytes = l.in_flight;
            }
            // Charge unfinished reservations; no refund after a crash. No old request is live in this generation.
            l.finished();
        })?;
        let scope = Arc::new(Scope::new(bridge, paused));
        let mut tasks = Vec::new();
        let s = scope.clone();
        let r = root.clone();
        tasks.push(tokio::spawn(async move {
            while s.alive() {
                let s2 = s.clone();
                let r2 = r.clone();
                let work = tokio::task::spawn_blocking(move || record_quantum(&s2, &r2))
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or(0)
                    > 0;
                let delay = idle_delay(work);
                if delay.is_zero() {
                    s.wake.notify_one();
                    tokio::task::yield_now().await;
                } else {
                    tokio::time::sleep(delay).await;
                }
            }
        }));
        let s = scope.clone();
        tasks.push(tokio::spawn(async move {
            // Start after the runner's first stamp/retry. Wakes thereafter never spawn another worker.
            while !s.started.load(std::sync::atomic::Ordering::SeqCst) {
                if !s.alive() {
                    return;
                }
                tokio::select! {_=s.wake.notified()=>{},_=tokio::time::sleep(std::time::Duration::from_millis(100))=>{}}
            }
            let runtime = tokio::runtime::Handle::current();
            let mut retry_offset = 0;
            while s.alive() {
                let s2 = s.clone();
                let r = root.clone();
                let runtime = runtime.clone();
                let outcome = tokio::task::spawn_blocking(move || {
                    if !s2.active() {
                        return Ok(0);
                    }
                    let entries = s2
                        .bridge
                        .db()
                        .list_kept_copies()?
                        .into_iter()
                        .filter(|c| !c.original_removed && c.reason == crate::state_db::KeptReason::PreFixChange)
                        .count();
                    retry_offset %= entries.max(1);
                    let _ = super::resolve::retry_checked(
                        s2.bridge.db(),
                        &r,
                        chrono::Utc::now().timestamp(),
                        Some(&s2),
                        retry_offset,
                    );
                    // A failed copy before an entry existed still retries from its durable divergence state.
                    for row in s2.bridge.db().list_unknown_waiting(16)? {
                        if row.record.state.as_deref() == Some("diverged")
                            && s2.bridge.db().pending_kept_original(&row.path)?.is_none()
                        {
                            let _ = super::resolve::diverged_checked(
                                s2.bridge.db(),
                                &r,
                                &row,
                                chrono::Utc::now().timestamp(),
                                Some(&s2),
                            );
                        }
                        s2.commit(|| {
                            Ok(s2
                                .bridge
                                .db()
                                .touch_unknown_waiting(&row.file_id, chrono::Utc::now().timestamp())?)
                        })?;
                    }
                    runtime.block_on(check_quantum(&s2, &r))
                })
                .await;
                retry_offset = retry_offset.saturating_add(16);
                let work = outcome.ok().and_then(Result::ok).unwrap_or(0) > 0;
                let delay = idle_delay(work);
                if delay.is_zero() {
                    tokio::task::yield_now().await;
                } else {
                    tokio::select! {_=s.wake.notified()=>{},_=tokio::time::sleep(delay)=>{}}
                }
            }
        }));
        Ok(Self { scope, tasks })
    }
    pub fn wake(&self) {
        self.scope.started.store(true, std::sync::atomic::Ordering::SeqCst);
        self.scope.wake.notify_one();
    }
    pub async fn stop(&mut self) {
        self.scope.cancel();
        for task in self.tasks.drain(..) {
            let _ = task.await;
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.scope.cancel();
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
    use std::io::{Read, Write};
    fn setup() -> (tempfile::TempDir, Arc<StateDb>) {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        for n in 0..8 {
            let id = format!("u{n}");
            let name = format!("{n}.txt");
            std::fs::write(dir.path().join(&name), b"same").unwrap();
            db.upsert_file(&FileEntry {
                file_id: id,
                path: name,
                status: FileStatus::Local,
                size_bytes: 4,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        }
        (dir, db)
    }
    #[tokio::test]
    async fn task_2191_s2_real_decryptor_current_match_clears_four_then_next_four() {
        let (dir, db) = setup();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let server = std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let mut calls = Vec::new();
            while !stop2.load(std::sync::atomic::Ordering::SeqCst) && start.elapsed().as_secs() < 15 {
                let Ok((mut socket, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                };
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0; 8192];
                let len = socket.read(&mut request).unwrap();
                let raw = String::from_utf8_lossy(&request[..len]);
                let path = raw.split_whitespace().nth(1).unwrap();
                calls.push(path.to_string());
                let id = path.split('/').nth(4).unwrap().split('?').next().unwrap();
                let body = if path.contains("/chunks/") {
                    let key = beebeeb_core::kdf::derive_file_key(
                        &beebeeb_core::kdf::MasterKey::from_bytes([0; 32]),
                        id.as_bytes(),
                    );
                    beebeeb_core::encrypt::encrypt_chunk_raw(&key, b"same").unwrap()
                } else {
                    assert!(!path.contains("/versions"), "current match must not fetch history");
                    let mut manifest =
                        crate::download_pin::tests::manifest("11111111-1111-4111-8111-111111111111", 4, &[4]);
                    manifest["version_number"] = serde_json::json!(2);
                    serde_json::to_vec(&serde_json::json!({"download_version":manifest})).unwrap()
                };
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                socket.write_all(&body).unwrap();
            }
            calls
        });
        let bridge = Arc::new(crate::engine_bridge::EngineBridge::new(
            db.clone(),
            Arc::new(crate::api_client::ApiClient::new(url, "test".into(), [0; 32])),
        ));
        let scope = Scope::new(bridge, Arc::new(AtomicBool::new(false)));
        assert_eq!(record_quantum(&scope, dir.path()).unwrap(), 4);
        assert_eq!(record_quantum(&scope, dir.path()).unwrap(), 4);
        assert_eq!(check_quantum(&scope, dir.path()).await.unwrap(), 4);
        assert_eq!(db.list_unknown_to_check(i64::MAX, 100).unwrap().len(), 4);
        assert_eq!(check_quantum(&scope, dir.path()).await.unwrap(), 4);
        assert!(db.list_unknown_to_check(i64::MAX, 100).unwrap().is_empty());
        for n in 0..8 {
            assert_eq!(
                db.get_file_contract_state(&format!("u{n}"))
                    .unwrap()
                    .unwrap()
                    .local_base_version,
                2
            );
        }
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let calls = server.join().unwrap();
        assert_eq!(calls.len(), 16);
        assert_eq!(calls.iter().filter(|p| p.contains("/versions")).count(), 0);
        let ledger = db.update_unknown_budget(|l| l.clone()).unwrap();
        assert_eq!((ledger.bytes, ledger.in_flight), (8 * 32, 0));
    }
    #[test]
    fn task_2191_s2_eight_failures_share_one_non_dismissable_review_entry() {
        let (_dir, db) = setup();
        for n in 0..3 {
            db.put_unknown_check(
                &format!("u{n}"),
                &format!("{n}.txt"),
                &UnknownCheck {
                    state: Some("could_not_check".into()),
                    attempts: 8,
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let feed = crate::engine_bridge::version_conflict_feed_from_db(&db).unwrap();
        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].kind, "could_not_check");
        assert_eq!(feed[0].action, "none");
        assert!(feed[0].detail.starts_with("3 files"));
    }
    #[test]
    fn task_2191_s2_stale_record_cannot_replace_first_recording() {
        let (dir, db) = setup();
        let row = db.list_unknown_to_record(1).unwrap().remove(0);
        let path = path(dir.path(), &row).unwrap();
        assert!(super::super::record::record_one(&db, &row, &path, || {}).unwrap());
        std::fs::write(&path, b"later edit").unwrap();
        assert!(!super::super::record::record_one(&db, &row, &path, || {}).unwrap());
        assert_eq!(db.list_unknown_to_check(0, 1).unwrap()[0].record.recorded_size, Some(4));
    }
    #[tokio::test]
    async fn task_2191_s2_foreground_and_global_cooldown_gate_every_request() {
        let (_dir, db) = setup();
        let transfers = Arc::new(crate::transfer_progress::TransferBoard::default());
        let api = Arc::new(crate::api_client::ApiClient::new(
            "http://127.0.0.1:9".into(),
            "test".into(),
            [0; 32],
        ));
        let bridge =
            Arc::new(crate::engine_bridge::EngineBridge::new(db.clone(), api).with_transfers(transfers.clone()));
        let scope = Scope::new(bridge, Arc::new(AtomicBool::new(false)));
        let guard = transfers.begin("foreground", crate::transfer_progress::Direction::Down, 100);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), scope.before(0, false))
                .await
                .is_err()
        );
        assert_eq!(db.update_unknown_budget(|l| l.requests.len()).unwrap(), 0);
        drop(guard);
        db.update_unknown_budget(|l| l.rate_limited(chrono::Utc::now().timestamp_millis(), 700, 0))
            .unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), scope.before(0, false))
                .await
                .is_err()
        );
        assert_eq!(db.update_unknown_budget(|l| l.requests.len()).unwrap(), 0);
        db.update_unknown_budget(|l| l.cooldown_until = 0).unwrap();
        scope.before(0, false).await.unwrap();
        assert_eq!(db.update_unknown_budget(|l| l.requests.len()).unwrap(), 1);
        scope.cancel();
        assert!(scope.before(0, false).await.is_err());
    }
    #[tokio::test]
    async fn task_2191_s2_job_cancel_before_first_wake_joins_both_tasks() {
        let (dir, db) = setup();
        let api = Arc::new(crate::api_client::ApiClient::new(
            "http://127.0.0.1:9".into(),
            "test".into(),
            [0; 32],
        ));
        let bridge = Arc::new(crate::engine_bridge::EngineBridge::new(db.clone(), api));
        let mut job = Job::start(bridge, dir.path().to_path_buf(), Arc::new(AtomicBool::new(true))).unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1500), job.stop())
                .await
                .is_ok()
        );
        assert_eq!(db.list_unknown_to_record(100).unwrap().len(), 8);
        assert_eq!(db.update_unknown_budget(|l| l.requests.len()).unwrap(), 0);
    }
}
