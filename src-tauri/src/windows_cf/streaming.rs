//! LD-0105 W2: the streaming fetch on Windows. Every `FETCH_DATA` of a placeholder whose version is
//! pinned becomes a demand on the process's ONE range session for that file (spec S3), served as
//! core authenticates each chunk (I1), in 4 KiB pieces (D-3), with progress reports (S0). The rules
//! are in `crate::cf_range`; this is the Cloud Files half:
//!
//! - the session registry, and each session's task (preparation, `range_session`, its end: IN_SYNC
//!   and agreed content only on "whole" (I6), the S6 poison after a failure that wrote);
//! - the blocked callback: its demand, the keep-alive, the requester's `OpenProcess(SYNCHRONIZE)`
//!   handle (spec S5: a dead opener fails its requests and, when no live requester is left, stops
//!   the session), revocation;
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

use crate::cf_range::{AfterSession, DemandBook, DbRecords, PieceOut, RequestKeys, StreamSink, after_session};

/// The decline's status (spec §6.4, P1b spike (x2): the open returned in 4 ms, then reads were ranges).
const STATUS_DECLINED: NTSTATUS = NTSTATUS(0xC000_CF1F_u32 as i32);

/// One placeholder's session.
struct Session {
    control: beebeeb_transfer::RangeControl,
    book: Arc<Mutex<DemandBook>>,
    meter: Arc<crate::transfer_progress::ReceiveMeter>,
    /// Set (under the registry lock) when the session ended: a demand can no longer join it.
    ended: std::sync::atomic::AtomicBool,
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Arc<Session>>>> = LazyLock::new(Default::default);
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
    if !super::registration::progressive_in_effect() || lock(&WHOLE_FILE_ONLY).contains(&fetch.file_id) {
        return Served::WholeFilePath;
    }
    if declined(fetch, bridge) {
        tracing::info!(file_id = %fetch.file_id, "open-time whole-file fetch of a listed player declined (W3)");
        // SAFETY: the keys of this live request; 0 bytes with a non-success status (spec §6.4).
        unsafe { decline(fetch) };
        return Served::Done;
    }
    let (offset, length) = (fetch.offset.max(0) as u64, fetch.length.max(0) as u64);
    let session = match join_or_start(fetch, bridge, handle) {
        Ok(session) => session,
        Err(Served::WholeFilePath) => return Served::WholeFilePath,
        Err(Served::Done) => {
            fail(fetch);
            return Served::Done;
        }
    };
    let (id, answer) = lock(&session.book).add(offset, length, fetch.requester, fetch.keys);
    session.control.demand(id, offset, length);
    let process = Requester::open(fetch.requester);
    let met = handle.block_on(async {
        tokio::select! {
            biased;
            _ = super::cancelled(bridge) => {
                session.control.close();
                false
            }
            answer = answer => answer.unwrap_or(false),
            () = process.exited() => {
                // Spec S5: its requests fail; with no live requester left the session stops.
                let (ids, others) = lock(&session.book).requester_exited(fetch.requester);
                for id in ids {
                    session.control.cancel(id);
                }
                if !others {
                    session.control.requester_gone();
                    session.control.close();
                }
                false
            }
            never = super::fetch_progress::keep_fetch_alive(fetch.connection, fetch.keys.transfer_key, fetch.file_size, &session.meter) => match never {},
        }
    });
    if !met {
        fail(fetch);
    }
    Served::Done
}

/// The file's live session, or a new one: started under the registry lock, after the poison was
/// checked again there (a session that failed records its poison before it leaves the registry).
fn join_or_start(fetch: &Fetch, bridge: &crate::callback_gate::CallbackLease<super::EngineBridge>, handle: &tokio::runtime::Handle) -> Result<Arc<Session>, Served> {
    let mut sessions = lock(&SESSIONS);
    if let Some(session) = sessions.get(&fetch.file_id)
        && !session.ended.load(std::sync::atomic::Ordering::SeqCst)
    {
        return Ok(session.clone());
    }
    if !matches!(bridge.db().poison(&fetch.file_id), Ok(None)) {
        return Err(Served::Done);
    }
    let path = super::callbacks::db_placeholder_path(bridge, &fetch.file_id).ok_or(Served::Done)?;
    let on_disk = bytes_held(&path);
    // Spec S3: no identity with bytes on disk is a mismatch; core checks it, and an unreadable count
    // counts as bytes (`u64::MAX`): never trusted.
    let prep = handle.block_on(bridge.range_prep(&fetch.file_id, fetch.file_size.max(0) as u64, on_disk.min(fetch.file_size.max(0) as u64)));
    let prep = match prep {
        Ok(prep) => prep,
        Err(crate::engine_bridge::PrepEnd::WholeFilePath) => return Err(Served::WholeFilePath),
        Err(crate::engine_bridge::PrepEnd::ResolveSize(size)) => {
            drop(sessions);
            fail(fetch);
            super::callbacks::resolve_size_mismatch_by_id(bridge, &fetch.file_id, size as i64);
            return Err(Served::Done);
        }
        Err(crate::engine_bridge::PrepEnd::Failed(e)) => {
            tracing::warn!(file_id = %fetch.file_id, error = %e.root_cause(), "range session could not start");
            return Err(Served::Done);
        }
    };
    let session = Arc::new(Session {
        control: beebeeb_transfer::RangeControl::new(),
        book: Arc::new(Mutex::new(DemandBook::default())),
        meter: Arc::new(crate::transfer_progress::ReceiveMeter::default()),
        ended: false.into(),
    });
    sessions.insert(fetch.file_id.clone(), session.clone());
    drop(sessions);
    let (file_id, connection, file_size) = (fetch.file_id.clone(), fetch.connection, fetch.file_size.max(0) as u64);
    let run = session.clone();
    handle.spawn(async move { run_session(run, prep, file_id, path, connection, file_size).await });
    Ok(session)
}

struct Quiet;
impl beebeeb_transfer::Observer for Quiet {}

/// `CfExecute(TRANSFER_DATA)` for one authenticated piece.
struct CfOut(CF_CONNECTION_KEY);

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
        .map_err(|e| std::io::Error::other(format!("CfExecute 0x{:08X}", e.code().0 as u32)))
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
    // A key taken before the first transfer: the promotion hashes the file only while it is unchanged.
    let key_before = super::poison::content_key(&path);
    let board = Arc::new(bridge.transfers().begin(&file_id, crate::transfer_progress::Direction::Down, file_size));
    let records = DbRecords {
        db: bridge.db_arc(),
        file_id: file_id.clone(),
        version_number,
    };
    let progress = (board.clone(), session.meter.clone());
    let mut sink = StreamSink::new(session.book.clone(), CfOut(connection), records, file_size).with_progress(move |done| {
        progress.0.update(done);
        progress.1.keep(done.saturating_sub(progress.1.kept()));
    });
    let io = crate::api_client::range_io::RangeIo::new(bridge.api(), &session.meter, kbps_limit);
    let shared = beebeeb_transfer::DirectStore::global().entry(bridge.api().api_base(), "desktop");
    // Revocation (sign-out, lock): the session stops at once and its lease is released.
    let result = tokio::select! {
        result = beebeeb_transfer::range_session(&io, &shared, beebeeb_transfer::RangeLessor::global(), request, file_key, &session.control, &mut sink, &Quiet) => result,
        () = super::cancelled(&bridge) => Ok(beebeeb_transfer::RangeOutcome::Closed(Default::default())),
    };
    let transferred = sink.transferred;
    drop(sink);
    match after_session(&result) {
        AfterSession::Promote => {
            {
                use std::os::windows::ffi::OsStrExt;
                let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
                // SAFETY: NUL-terminated, alive for the call.
                unsafe { super::callbacks::mark_in_sync(&wide) };
            }
            let hash = hash_if_unchanged(&path, &file_id, key_before);
            bridge.finish_whole_range(&file_id, file_size, version_number, hash, &ticket);
            if let Ok(board) = Arc::try_unwrap(board) {
                board.finish();
            }
        }
        AfterSession::Leave => {
            let _ = bridge.db().set_status_if(&file_id, crate::state_db::FileStatus::Downloading, crate::state_db::FileStatus::CloudOnly);
        }
        AfterSession::Poison(reason) => super::poison::poison_now(bridge.db(), &file_id, &path, reason),
        AfterSession::WholeFilePath => {
            lock(&WHOLE_FILE_ONLY).insert(file_id.clone());
            let _ = bridge.db().set_status_if(&file_id, crate::state_db::FileStatus::Downloading, crate::state_db::FileStatus::CloudOnly);
        }
        AfterSession::ResolveSize | AfterSession::Fail => {
            let _ = bridge.db().set_status_if(&file_id, crate::state_db::FileStatus::Downloading, crate::state_db::FileStatus::Error);
        }
    }
    tracing::info!(file_id = %file_id, transferred, outcome = ?after_session(&result), "range session ended");
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

/// The session left the registry (after any poison was recorded) and no demand is left waiting.
fn end(session: &Arc<Session>, file_id: &str) {
    let mut sessions = lock(&SESSIONS);
    session.ended.store(true, std::sync::atomic::Ordering::SeqCst);
    lock(&session.book).fail_all();
    if sessions.get(file_id).is_some_and(|s| Arc::ptr_eq(s, session)) {
        sessions.remove(file_id);
    }
}

/// Task 1640 after "whole": the sha256 of the file as it is on disk, read under a no-fetch scope,
/// and only while its content key is the one it had before the first transfer (a write since, or
/// during the read, means the bytes are not only ours: no agreed content, the candidate stays).
fn hash_if_unchanged(path: &std::path::Path, file_id: &str, before: Option<(i64, i64)>) -> Option<String> {
    use sha2::Digest;
    use std::io::Read;
    let before = before?;
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
    (super::poison::content_key(path) == Some(before)).then(|| hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// W3 (spec §6.4): decline this fetch? Only the first whole-remainder fetch after a listed player's
/// open of a media file of no excluded origin; never ours, never an explicit hydration.
fn declined(fetch: &Fetch, bridge: &crate::callback_gate::CallbackLease<super::EngineBridge>) -> bool {
    use crate::stream_players as sp;
    if !sp::is_whole_remainder(fetch.offset, fetch.length, fetch.optional_length, fetch.file_size) {
        return false;
    }
    let first = lock(&OPENS).first_after_open(fetch.requester, &fetch.file_id);
    let Ok(Some(entry)) = bridge.db().get_file(&fetch.file_id) else {
        return false;
    };
    let shared = bridge
        .db()
        .get_file_contract_state(&fetch.file_id)
        .ok()
        .flatten()
        .is_some_and(|c| c.namespace == crate::state_db::Namespace::SharedWithMe);
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
    sp::decline_open_fetch(sp::STREAMING_ENABLED, &requester, program_files(), target, first)
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

/// `OpenProcess(SYNCHRONIZE)` on the requester (spec S5), so a reused PID cannot fool the wait.
struct Requester(Option<windows::Win32::Foundation::HANDLE>);

impl Requester {
    fn open(pid: u32) -> Self {
        // SAFETY: a plain open; the handle is closed in Drop.
        Requester((pid != std::process::id() && pid != 0).then(|| unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }.ok()).flatten())
    }

    /// Resolves when the process exited; never for our own process or one that could not be opened.
    async fn exited(&self) {
        let Some(handle) = self.0 else {
            return std::future::pending().await;
        };
        loop {
            // SAFETY: the handle is open until Drop.
            if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

impl Drop for Requester {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            // SAFETY: opened in `open`, closed once.
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(handle) };
        }
    }
}

fn fail(fetch: &Fetch) {
    // SAFETY: the keys of this live request.
    unsafe { super::callbacks::fail_transfer(fetch.connection, fetch.keys.transfer_key, fetch.keys.request_key, fetch.offset, fetch.length) };
}

/// SAFETY: the keys of a live request.
unsafe fn decline(fetch: &Fetch) {
    // SAFETY: the caller's contract; 0 bytes, so no buffer.
    if let Err(e) = unsafe {
        super::callbacks::transfer_one(fetch.connection, fetch.keys.transfer_key, fetch.keys.request_key, std::ptr::null(), fetch.offset, fetch.length.max(0), STATUS_DECLINED)
    } {
        tracing::debug!(error = %e, "decline: CfExecute failed");
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
    let pid = unsafe { (*info.ProcessInfo).ProcessId };
    lock(&OPENS).opened(pid, &file_id);
}

/// A close completion: a session with no request left (the opener closed its handle) ends.
pub(super) fn after_close(file_id: &str) {
    if let Some(session) = lock(&SESSIONS).get(file_id).cloned()
        && lock(&session.book).is_idle()
    {
        session.control.close();
    }
}
