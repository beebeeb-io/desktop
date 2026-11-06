//! LD-0105 W2: the streaming fetch on Windows. Every `FETCH_DATA` of a placeholder whose version is
//! pinned becomes a demand on the process's ONE range session for that file (spec S3), served as
//! core authenticates each chunk (I1), in 4 KiB pieces (D-3), with progress reports (S0). The rules
//! are in `crate::cf_range`; this is the Cloud Files half:
//!
//! - the session registry, and each session's task (preparation, `range_session`, its end: IN_SYNC
//!   and agreed content only on "whole" (I6), the S6 poison after a failure that wrote);
//! - the blocked callback: its demand, the keep-alive, revocation;
//! - each session's watch (every 250 ms, for the session's whole life): every requester's
//!   `OpenProcess(SYNCHRONIZE)` handle (spec S5: a dead opener fails its requests and, when no live
//!   requester has one left, stops the session, between two reads too), and the idle limit (desktop
//!   #225: no request for 30 s ends the session, releasing its lease, ticket and row, "not whole");
//! - the transport gate (`cf_range::GatedIo`): read-ahead reaches Windows only under a pending
//!   request's own keys, never on `CF_REQUEST_KEY_DEFAULT` and never after a close (lead ruling,
//!   2026-11-06);
//! - `CANCEL_FETCH_DATA`, `NOTIFY_FILE_OPEN_COMPLETION` and W3's per-app decline (spec §6.4).
//!
//! Plaintext lives in core's `Zeroizing` buffers and goes straight to `CfExecute`; nothing is written
//! anywhere else, and no name, path or URL is logged.

#![cfg(target_os = "windows")]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use windows::Win32::Foundation::{NTSTATUS, STATUS_SUCCESS, WAIT_OBJECT_0};
use windows::Win32::Storage::CloudFilters::*;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};

use crate::cf_range::{AfterSession, Answer, DemandBook, DbRecords, PieceOut, Promotion, RequestKeys, SessionEnd, StreamSink, after_session};

/// The decline's status (spec §6.4, P1b spike (x2): the open returned in 4 ms, then reads were ranges).
const STATUS_DECLINED: NTSTATUS = NTSTATUS(0xC000_CF1F_u32 as i32);

/// One placeholder's session.
struct Session {
    control: beebeeb_transfer::RangeControl,
    book: Arc<Mutex<DemandBook>>,
    meter: Arc<crate::transfer_progress::ReceiveMeter>,
    /// Set (under the registry lock) when the session ended: a demand can no longer join it.
    ended: std::sync::atomic::AtomicBool,
    /// Every requester the session served, watched until the session ends (desktop #225 review P2).
    requesters: Mutex<Vec<(u32, Requester)>>,
}

impl Session {
    /// Watch `pid` for the rest of the session (opened once, at its first request).
    fn watch(&self, pid: u32) {
        let mut watched = lock(&self.requesters);
        if !watched.iter().any(|(p, _)| *p == pid) {
            watched.push((pid, Requester::open(pid)));
        }
    }
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Arc<Session>>>> = LazyLock::new(Default::default);
/// The kill switch as read at engine start (`cf_range::W2_STREAMING`, desktop #225).
static SWITCH_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Engine start: read the streaming kill switch once (lead ruling 2026-11-06).
pub(crate) fn read_switch_at_engine_start() {
    let on = crate::cf_range::W2_STREAMING;
    SWITCH_ON.store(on, std::sync::atomic::Ordering::SeqCst);
    tracing::info!(streaming = on, "LD-0105 W2 streaming switch read at engine start");
}
/// Files a session handed back to the whole-file path (`UseApi` before any write): their later fetches
/// go there directly, until the process restarts.
static WHOLE_FILE_ONLY: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);
/// W3: whole-remainder fetches armed by an open completion.
static OPENS: LazyLock<Mutex<crate::stream_players::OpenFetches>> = LazyLock::new(Default::default);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What one `FETCH_DATA` asked, copied out of the callback.
pub(super) struct Fetch {
    pub connection: CF_CONNECTION_KEY,
    pub keys: RequestKeys,
    pub file_id: String,
    pub offset: i64,
    pub length: i64,
    pub optional_length: i64,
    pub file_size: i64,
    pub flags: i32,
    pub requester: u32,
    pub image: Option<String>,
    pub package: Option<String>,
}

/// The callback's answer: served here, or left to the whole-file path.
pub(super) enum Served {
    Done,
    WholeFilePath,
}

/// Serve `fetch` through the file's range session. `Served::WholeFilePath` when this file streams
/// not (no progressive policy, a legacy file, a session that gave the file to the API path).
pub(super) fn serve(fetch: &Fetch, bridge: &crate::callback_gate::CallbackLease<super::EngineBridge>, handle: &tokio::runtime::Handle) -> Served {
    let route = crate::cf_range::fetch_route(
        SWITCH_ON.load(std::sync::atomic::Ordering::SeqCst),
        super::registration::progressive_in_effect(),
        lock(&WHOLE_FILE_ONLY).contains(&fetch.file_id),
    );
    if route == crate::cf_range::FetchRoute::WholeFile {
        return Served::WholeFilePath;
    }
    if declined(fetch, bridge, handle) {
        tracing::info!(file_id = %fetch.file_id, "open-time whole-file fetch of a listed player declined (W3)");
        // SAFETY: the keys of this live request; 0 bytes with a non-success status (spec §6.4).
        unsafe { decline(fetch) };
        return Served::Done;
    }
    let (session, answer) = match join_or_start(fetch, bridge, handle) {
        Start::Joined(session, answer) => (session, answer),
        Start::WholeFilePath => return Served::WholeFilePath,
        Start::Answered => return Served::Done,
        Start::Failed => {
            fail(fetch);
            return Served::Done;
        }
    };
    let met = handle.block_on(async {
        tokio::select! {
            biased;
            _ = super::cancelled(bridge) => {
                session.control.close();
                Answer::Failed
            }
            answer = answer => answer.unwrap_or(Answer::Failed),
            never = super::fetch_progress::keep_fetch_alive(fetch.connection, fetch.keys.transfer_key, fetch.file_size, &session.meter) => match never {},
        }
    });
    match met {
        Answer::Met | Answer::Cancelled => Served::Done,
        // Review P2: a session that gave the file to the whole-file path (`UseApi`, a legacy file found
        // by its preparation) hands this still-pending request over instead of failing it.
        Answer::Failed if lock(&WHOLE_FILE_ONLY).contains(&fetch.file_id) => Served::WholeFilePath,
        Answer::Failed => {
            fail(fetch);
            Served::Done
        }
    }
}

/// How a fetch reached its session.
enum Start {
    Joined(Arc<Session>, tokio::sync::oneshot::Receiver<Answer>),
    WholeFilePath,
    /// The fetch was answered already (failed, and for 0783 resolved).
    Answered,
    Failed,
}

/// The file's live session, or a new one. The new one is in the registry before its preparation (so
/// a concurrent fetch joins it, and its demand waits in the session's mailbox), and the registry lock
/// is never held across the preparation's network call. The poison is checked again under the lock:
/// a session that failed records its poison before it leaves the registry.
fn join_or_start(fetch: &Fetch, bridge: &crate::callback_gate::CallbackLease<super::EngineBridge>, handle: &tokio::runtime::Handle) -> Start {
    // The demand joins under the registry lock, which `end` also takes: it is in the book before the
    // session can end, so `end`'s `fail_all` answers it (review P1: a demand added after `end` waited
    // for ever).
    let demand = |session: &Arc<Session>| {
        // Spec S5: from its first request on (the preparation included), the requester is watched; its
        // exit fails its demands (the callback's answer) and, with no request left, stops the session.
        session.watch(fetch.requester);
        let (offset, length) = (fetch.offset.max(0) as u64, fetch.length.max(0) as u64);
        let (id, answer) = lock(&session.book).add(offset, length, fetch.requester, fetch.keys);
        session.control.demand(id, offset, length);
        answer
    };
    let mut waits = 0;
    let (session, answer) = loop {
        let mut sessions = lock(&SESSIONS);
        if let Some(session) = sessions.get(&fetch.file_id) {
            if !session.ended.load(std::sync::atomic::Ordering::SeqCst) {
                return Start::Joined(session.clone(), demand(session));
            }
            // Ending (its poison may not be recorded yet): wait for it to leave, then start afresh
            // (self-review: an idle or a withheld end must not fail the next read); 2 s at most.
            drop(sessions);
            waits += 1;
            if waits > 100 {
                return Start::Failed;
            }
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        if !matches!(bridge.db().poison(&fetch.file_id), Ok(None)) {
            return Start::Failed;
        }
        let session = Arc::new(Session {
            control: beebeeb_transfer::RangeControl::new(),
            book: Arc::new(Mutex::new(DemandBook::default())),
            meter: Arc::new(crate::transfer_progress::ReceiveMeter::default()),
            ended: false.into(),
            requesters: Mutex::new(Vec::new()),
        });
        sessions.insert(fetch.file_id.clone(), session.clone());
        let answer = demand(&session);
        handle.spawn(watch(session.clone()));
        break (session, answer);
    };
    let Some(path) = super::callbacks::db_placeholder_path(bridge, &fetch.file_id) else {
        end(&session, &fetch.file_id);
        return Start::Failed;
    };
    let file_size = fetch.file_size.max(0) as u64;
    // Spec S3: no identity with bytes on disk is a mismatch (core checks it); an unreadable view counts
    // as bytes, so it is never trusted.
    let on_disk = bytes_held(&path).min(file_size);
    // Review P3 (streaming.rs:182): a sign-out or lock during the preparation's metadata call ends it.
    let prepared = handle.block_on(async {
        tokio::select! {
            biased;
            _ = super::cancelled(bridge) => Err(crate::engine_bridge::PrepEnd::Failed(anyhow::anyhow!("signed out or locked"))),
            prep = bridge.range_prep(&fetch.file_id, file_size, on_disk) => prep,
        }
    });
    match prepared {
        Ok(prep) => {
            let (run, file_id, connection) = (session.clone(), fetch.file_id.clone(), fetch.connection);
            handle.spawn(async move { run_session(run, prep, file_id, path, connection, file_size).await });
            Start::Joined(session, answer)
        }
        Err(end_why) => {
            if matches!(end_why, crate::engine_bridge::PrepEnd::WholeFilePath) {
                // Review P3: a legacy file is remembered, so its later fetches skip the preparation; the
                // fetches that joined meanwhile take the whole-file path too.
                lock(&WHOLE_FILE_ONLY).insert(fetch.file_id.clone());
            }
            end(&session, &fetch.file_id);
            match end_why {
                crate::engine_bridge::PrepEnd::WholeFilePath => Start::WholeFilePath,
                crate::engine_bridge::PrepEnd::ResolveSize(size) => {
                    // Task 0783: fail this fetch first (the placeholder cannot be resized mid-fetch).
                    fail(fetch);
                    super::callbacks::resolve_size_mismatch_by_id(bridge, &fetch.file_id, size as i64);
                    Start::Answered
                }
                crate::engine_bridge::PrepEnd::Failed(e) => {
                    tracing::warn!(file_id = %fetch.file_id, error = %e.root_cause(), "range session could not start");
                    Start::Failed
                }
            }
        }
    }
}

/// Close `session`: no fetch joins it any more. It stays in the registry until `end` (after any
/// poison is recorded, review P2), and the next fetch after that starts a new session, which trusts
/// the bytes this one transferred (one recorded identity).
fn retire(session: &Arc<Session>) {
    {
        let _sessions = lock(&SESSIONS);
        session.ended.store(true, std::sync::atomic::Ordering::SeqCst);
        // Nothing fetched is released to a closing session (`GatedIo`): no transfer after a close.
        lock(&session.book).close();
    }
    session.control.close();
}

/// The session's watch, every 250 ms for its whole life: a requester that exited (spec S5, §9: a
/// killed player stops the GETs within a chunk, between two reads too), and the idle limit.
async fn watch(session: Arc<Session>) {
    let mut every = tokio::time::interval(Duration::from_millis(250));
    loop {
        every.tick().await;
        if session.ended.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let tick = {
            let mut watched = lock(&session.requesters);
            let mut book = lock(&session.book);
            crate::cf_range::session_tick(&mut watched, &mut book, crate::cf_range::IDLE_LIMIT)
        };
        for id in tick.cancel {
            session.control.cancel(id);
        }
        match tick.end {
            Some(SessionEnd::RequesterGone) => {
                tracing::info!("range session: its requester exited; the session stops");
                session.control.requester_gone();
                retire(&session);
                return;
            }
            Some(SessionEnd::Idle) => {
                tracing::info!("range session: no request for the idle limit; the session ends, not whole");
                retire(&session);
                return;
            }
            None => {}
        }
    }
}

struct Quiet;
impl beebeeb_transfer::Observer for Quiet {}

/// `CfExecute(TRANSFER_DATA)` for one authenticated piece; the last refusal's HRESULT, for the log.
struct CfOut(CF_CONNECTION_KEY, Option<u32>);

impl PieceOut for CfOut {
    fn transfer(&mut self, keys: RequestKeys, offset: u64, plain: &[u8]) -> std::io::Result<()> {
        // SAFETY: `plain` outlives the call, and CfExecute copies it before it returns.
        unsafe {
            super::callbacks::transfer_one(
                self.0,
                keys.transfer_key,
                keys.request_key,
                plain.as_ptr().cast(),
                offset as i64,
                plain.len() as i64,
                STATUS_SUCCESS,
            )
        }
        .map_err(|e| {
            let code = e.code().0 as u32;
            self.1 = Some(code);
            // QA logging gap (desktop #225): every refusal names its status.
            tracing::warn!(hresult = %format!("0x{code:08X}"), offset, len = plain.len(), request_key = keys.request_key, gone = crate::cf_range::request_gone(code), "TRANSFER_DATA refused");
            // A cancelled request or a key no longer valid ends that request, never the file (#225).
            if crate::cf_range::request_gone(code) { crate::cf_range::gone_error() } else { std::io::Error::other(format!("CfExecute 0x{code:08X}")) }
        })
    }
}

/// One session, from start to its end (spec S6, S9).
async fn run_session(
    session: Arc<Session>,
    prep: crate::engine_bridge::RangePrep,
    file_id: String,
    path: std::path::PathBuf,
    connection: CF_CONNECTION_KEY,
    file_size: u64,
) {
    let Some(bridge) = super::bridge() else {
        end(&session, &file_id);
        return;
    };
    let crate::engine_bridge::RangePrep { request, file_key, version_number, kbps_limit, ticket } = prep;
    // A key taken before the first transfer: the promotion hashes the file only while it is unchanged,
    // and a poison keeps it only while the file still has it (review P2, poison.rs:538).
    let key_before = super::poison::content_key(&path);
    let board = Arc::new(bridge.transfers().begin(&file_id, crate::transfer_progress::Direction::Down, file_size));
    let records = DbRecords {
        db: bridge.db_arc(),
        file_id: file_id.clone(),
        version_number,
    };
    let progress = (board.clone(), session.meter.clone());
    let closing = session.clone();
    let mut sink = StreamSink::new(session.book.clone(), CfOut(connection, None), records, file_size)
        .with_progress(move |done| {
            progress.0.update(done);
            progress.1.keep(done.saturating_sub(progress.1.kept()));
        })
        .on_withheld(move || retire(&closing));
    let io = crate::cf_range::GatedIo::new(crate::api_client::range_io::RangeIo::new(bridge.api(), &session.meter, kbps_limit), session.book.clone());
    let shared = beebeeb_transfer::DirectStore::global().entry(bridge.api().api_base(), "desktop");
    // Revocation (sign-out, lock): the session stops at once and its lease is released.
    let result = tokio::select! {
        result = beebeeb_transfer::range_session(&io, &shared, beebeeb_transfer::RangeLessor::global(), request, file_key, &session.control, &mut sink, &Quiet) => result,
        () = super::cancelled(&bridge) => Ok(beebeeb_transfer::RangeOutcome::Closed(Default::default())),
    };
    let (transferred, withheld, refused) = (sink.transferred, sink.withheld, sink.out.1.map(|c| format!("0x{c:08X}")));
    drop(sink);
    // No demand joins a session that returned (review P1: one joining while the promotion hashes).
    {
        let _sessions = lock(&SESSIONS);
        session.ended.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    match after_session(&result) {
        // I6 and the lead's rulings (desktop #225): every byte ON DISK (the range query, never "sent"),
        // then the read-back's hash under the content-key guard, and only then IN_SYNC and agreed content.
        AfterSession::Promote => {
            let (check_path, check_id) = (path.clone(), file_id.clone());
            // The range query and a whole-file read: off the runtime's workers.
            let (on_disk_whole, read_back) = tokio::task::spawn_blocking(move || {
                let whole = !withheld && on_disk_whole(&check_path, file_size);
                (whole, whole.then(|| read_back(&check_path, &check_id)).flatten())
            })
            .await
            .unwrap_or((false, None));
            match crate::cf_range::promotion(withheld, on_disk_whole, key_before, read_back) {
                Promotion::InSync(hash) => {
                    use std::os::windows::ffi::OsStrExt;
                    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
                    // SAFETY: NUL-terminated, alive for the call.
                    unsafe { super::callbacks::mark_in_sync(&wide) };
                    // Self-review: a write that landed between the read-back and the mark is not ours.
                    if super::poison::content_key(&path) == key_before {
                        bridge.finish_whole_range(&file_id, file_size, version_number, Some(hash), &ticket);
                        if let Ok(board) = Arc::try_unwrap(board) {
                            board.finish();
                        }
                    } else {
                        let _ = super::placeholders::mark_not_in_sync_path(&path);
                        tracing::warn!(file_id = %file_id, "streamed whole, but the file changed while it was marked: not in sync");
                        let _ = bridge.db().set_status(&file_id, crate::state_db::FileStatus::Local);
                    }
                }
                Promotion::Changed => {
                    tracing::warn!(file_id = %file_id, "streamed whole, but the file changed meanwhile: not marked in sync");
                    let _ = bridge.db().set_status(&file_id, crate::state_db::FileStatus::Local);
                }
                Promotion::NotWhole => {
                    tracing::warn!(file_id = %file_id, withheld, on_disk_whole, "streamed whole, but not every byte is on disk: not promoted");
                    let _ = bridge.db().set_status_if(&file_id, crate::state_db::FileStatus::Downloading, crate::state_db::FileStatus::CloudOnly);
                }
            }
        }
        AfterSession::Leave => {
            let _ = bridge.db().set_status_if(&file_id, crate::state_db::FileStatus::Downloading, crate::state_db::FileStatus::CloudOnly);
        }
        AfterSession::Poison(reason) => {
            tracing::warn!(file_id = %file_id, reason, last_refusal = ?refused, error = ?result.as_ref().err().map(ToString::to_string), "range session failed after a write: poisoning");
            super::poison::poison_now(bridge.db(), &file_id, &path, reason, key_before)
        }
        // Nothing written: the whole-file path serves it (and resolves a size, task 0783, from the
        // authenticated length: review P2).
        AfterSession::WholeFilePath | AfterSession::ResolveSize => {
            lock(&WHOLE_FILE_ONLY).insert(file_id.clone());
            let _ = bridge.db().set_status_if(&file_id, crate::state_db::FileStatus::Downloading, crate::state_db::FileStatus::CloudOnly);
        }
        AfterSession::Fail => {
            let _ = bridge.db().set_status_if(&file_id, crate::state_db::FileStatus::Downloading, crate::state_db::FileStatus::Error);
        }
    }
    tracing::info!(file_id = %file_id, transferred, withheld, last_refusal = ?refused, outcome = ?after_session(&result), "range session ended");
    drop(ticket);
    end(&session, &file_id);
}

/// Spec S3's ONDISK, as far as core needs it (more than 0 or not): an unreadable view counts as bytes,
/// so it is never trusted.
fn bytes_held(path: &std::path::Path) -> u64 {
    match super::placeholders::platform_view(path) {
        crate::unknown_check::PlatformView::Placeholder { on_disk: crate::unknown_check::OnDisk::Nothing, .. } => 0,
        _ => u64::MAX,
    }
}

/// Desktop #225 review P2: is every byte of the placeholder at `path` on disk (ONDISK covers
/// `[0, size)`)? The on-disk range query, through an attribute-only handle opened as the reparse point
/// (nothing is fetched). Any error is "no": a session that ended whole is promoted only on what the
/// platform holds, never on what was sent.
pub(crate) fn on_disk_whole(path: &std::path::Path, size: u64) -> bool {
    // The 0.8.15 open (`placeholders::open_for_state`), never one that could ask for the file.
    super::placeholders::open_for_state(path)
        .ok()
        .and_then(|file| super::placeholders::on_disk_ranges(&file, size).ok())
        .is_some_and(|ranges| crate::cf_range::covers_whole(&ranges, size))
}

/// The session left the registry (after any poison was recorded) and no demand is left waiting.
fn end(session: &Arc<Session>, file_id: &str) {
    let mut sessions = lock(&SESSIONS);
    session.ended.store(true, std::sync::atomic::Ordering::SeqCst);
    {
        let mut book = lock(&session.book);
        book.close();
        book.fail_all();
    }
    if sessions.get(file_id).is_some_and(|s| Arc::ptr_eq(s, session)) {
        sessions.remove(file_id);
    }
}

/// Task 1640 after "whole" (contract Amendment 5: the read-back replaces S9's incremental hash): the
/// sha256 of the file as it is on disk, read under a no-fetch scope (a hole is a read error, never our
/// own fetch), and the content key right after the read. `None`: the read failed. The promotion keeps
/// the hash only while that key is the one taken before the first transfer (a write since, or during
/// the read, means the bytes are not only ours: no agreed content, no IN_SYNC, the candidate stays).
fn read_back(path: &std::path::Path, file_id: &str) -> Option<(String, Option<(i64, i64)>)> {
    use sha2::Digest;
    use std::io::Read;
    let _scope = crate::no_fetch_scope::enter(&[file_id]);
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = zeroize::Zeroizing::new(vec![0u8; 1 << 20]);
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    drop(file);
    Some((hasher.finalize().iter().map(|b| format!("{b:02x}")).collect(), super::poison::content_key(path)))
}

/// W3 (spec §6.4): decline this fetch? Only the first whole-remainder fetch after a listed player's
/// open of a media file of no excluded origin; never ours, never an explicit hydration.
fn declined(fetch: &Fetch, bridge: &crate::callback_gate::CallbackLease<super::EngineBridge>, handle: &tokio::runtime::Handle) -> bool {
    use crate::stream_players as sp;
    if !sp::is_whole_remainder(fetch.offset, fetch.length, fetch.optional_length, fetch.file_size) {
        return false;
    }
    let first = lock(&OPENS).first_after_open(fetch.requester, &fetch.file_id);
    let Ok(Some(entry)) = bridge.db().get_file(&fetch.file_id) else {
        return false;
    };
    // Review P3 (streaming.rs:384): a database error counts as excluded, as the poison check counts one
    // as poisoned (a wrong decline costs availability, so it fails toward the whole file).
    let shared = match bridge.db().get_file_contract_state(&fetch.file_id) {
        Ok(state) => state.is_some_and(|c| c.namespace == crate::state_db::Namespace::SharedWithMe),
        Err(_) => true,
    };
    let image = fetch.image.as_deref().and_then(|nt| sp::dos_image_path(nt, devices()));
    let requester = sp::Requester {
        ours: fetch.requester == std::process::id(),
        explicit_hydration: fetch.flags & CF_CALLBACK_FETCH_DATA_FLAG_EXPLICIT_HYDRATION.0 != 0,
        package: fetch.package.as_deref(),
        image: image.as_deref(),
    };
    let name = crate::transfer_progress::display_name(&entry.path);
    let target = sp::Target {
        name: &name,
        mime: None,
        excluded_origin: shared || bridge.is_request_row(&fetch.file_id),
    };
    // Review P1: asked last, only when all else says decline (it may make the discovery call).
    // Bounded, and cut short by a sign-out (self-review): the open waits on it.
    let can_stream = || {
        handle.block_on(async {
            tokio::select! {
                biased;
                _ = super::cancelled(bridge) => false,
                streams = tokio::time::timeout(Duration::from_secs(5), bridge.can_stream(&fetch.file_id)) => streams.unwrap_or(false),
            }
        })
    };
    sp::decline_open_fetch(sp::STREAMING_ENABLED, &requester, program_files(), target, first, can_stream)
}

/// `%ProgramFiles%` and `%ProgramFiles(x86)%`, lower case.
fn program_files() -> &'static [String] {
    static ROOTS: LazyLock<Vec<String>> = LazyLock::new(|| {
        ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .map(|p| p.to_ascii_lowercase())
            .collect()
    });
    &ROOTS
}

/// `(drive, NT device)` pairs for `dos_image_path` (Amendment 1: Windows sends NT paths).
fn devices() -> &'static [(String, String)] {
    static DEVICES: LazyLock<Vec<(String, String)>> = LazyLock::new(|| {
        use windows::Win32::Storage::FileSystem::QueryDosDeviceW;
        (b'A'..=b'Z')
            .filter_map(|letter| {
                let drive = format!("{}:", letter as char);
                let wide: Vec<u16> = drive.encode_utf16().chain(Some(0)).collect();
                let mut out = [0u16; 512];
                // SAFETY: `wide` is NUL-terminated; `out` is writable for its length.
                let n = unsafe { QueryDosDeviceW(windows::core::PCWSTR(wide.as_ptr()), Some(&mut out)) } as usize;
                let first = out[..n].split(|c| *c == 0).next()?;
                (!first.is_empty()).then(|| (drive, String::from_utf16_lossy(first)))
            })
            .collect()
    });
    &DEVICES
}

/// `OpenProcess(SYNCHRONIZE)` on the requester (spec S5), so a reused PID cannot fool the wait. The
/// handle's value is kept as a number, so the session (and its watch task) can hold it.
struct Requester(Option<isize>);

impl Requester {
    fn open(pid: u32) -> Self {
        // SAFETY: a plain open; the handle is closed in Drop.
        let handle = (pid != std::process::id() && pid != 0).then(|| unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }.ok()).flatten();
        Requester(handle.map(|h| h.0 as isize))
    }
}

impl crate::cf_range::Exits for Requester {
    /// Never for our own process or one that could not be opened.
    fn exited(&self) -> bool {
        // SAFETY: the handle is open until Drop.
        self.0.is_some_and(|h| unsafe { WaitForSingleObject(windows::Win32::Foundation::HANDLE(h as *mut _), 0) } == WAIT_OBJECT_0)
    }
}

impl Drop for Requester {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            // SAFETY: opened in `open`, closed once.
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(windows::Win32::Foundation::HANDLE(handle as *mut _)) };
        }
    }
}

fn fail(fetch: &Fetch) {
    // QA logging gap (desktop #225): every failed request, with the status we answer it with.
    tracing::info!(file_id = %fetch.file_id, offset = fetch.offset, length = fetch.length, requester = fetch.requester, status = "0xC0000001", "fetch request failed");
    // SAFETY: the keys of this live request.
    unsafe { super::callbacks::fail_transfer(fetch.connection, fetch.keys.transfer_key, fetch.keys.request_key, fetch.offset, fetch.length) };
}

/// SAFETY: the keys of a live request.
unsafe fn decline(fetch: &Fetch) {
    // SAFETY: the caller's contract; 0 bytes, so no buffer.
    if let Err(e) = unsafe {
        super::callbacks::transfer_one(fetch.connection, fetch.keys.transfer_key, fetch.keys.request_key, std::ptr::null(), fetch.offset, fetch.length.max(0), STATUS_DECLINED)
    } {
        tracing::warn!(hresult = %format!("0x{:08X}", e.code().0 as u32), "decline: CfExecute failed");
    }
}

/// `CANCEL_FETCH_DATA`: the demands it names are cancelled (their callers return without a
/// transfer); with no demand left, core stops read-ahead.
pub unsafe extern "system" fn cancel_fetch_data_callback(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    if info.is_null() || params.is_null() {
        return;
    }
    // SAFETY: valid for the duration of the callback.
    let (info, params) = unsafe { (&*info, &*params) };
    let Some(file_id) = super::callbacks::file_id_of(info) else { return };
    // SAFETY: the cancel arm of a CANCEL_FETCH_DATA callback.
    let cancel = unsafe { params.Anonymous.Cancel.Anonymous.FetchData };
    let Some(session) = lock(&SESSIONS).get(&file_id).cloned() else { return };
    let ids = lock(&session.book).cancel(cancel.FileOffset.max(0) as u64, cancel.Length.max(0) as u64, info.RequestKey);
    for id in ids {
        session.control.cancel(id);
    }
}

/// `NOTIFY_FILE_OPEN_COMPLETION`: arms W3's "first whole-remainder fetch after an open" for the opener.
pub unsafe extern "system" fn notify_file_open_completion_callback(info: *const CF_CALLBACK_INFO, _params: *const CF_CALLBACK_PARAMETERS) {
    if info.is_null() {
        return;
    }
    // SAFETY: valid for the duration of the callback.
    let info = unsafe { &*info };
    if info.ProcessInfo.is_null() {
        return;
    }
    let Some(file_id) = super::callbacks::file_id_of(info) else { return };
    // SAFETY: as above.
    let process = unsafe { &*info.ProcessInfo };
    // Review P2: only an open by a listed player is armed, so other openers never crowd it out.
    // SAFETY: NUL-terminated wide strings valid for the callback.
    let image = unsafe { process.ImagePath.to_string() }.ok().and_then(|nt| crate::stream_players::dos_image_path(&nt, devices()));
    let package = unsafe { process.PackageName.to_string() }.ok().filter(|p| !p.is_empty());
    let requester = crate::stream_players::Requester {
        package: package.as_deref(),
        image: image.as_deref(),
        ..Default::default()
    };
    if crate::stream_players::is_listed_player(&requester, program_files()) {
        lock(&OPENS).opened(process.ProcessId, &file_id);
    }
}

/// A close completion: a session with no request left (the opener closed its handle) ends.
pub(super) fn after_close(file_id: &str) {
    let session = lock(&SESSIONS).get(file_id).cloned();
    // Review P2: never one in preparation or between a player's reads, only one quiet for a while
    // (the watch ends it at the same limit without a close).
    if let Some(session) = session
        && lock(&session.book).idle_for(crate::cf_range::IDLE_LIMIT)
    {
        retire(&session);
    }
}
