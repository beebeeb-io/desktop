//! Windows: the control channel's transport, a named pipe (`shell/control.rs` is the protocol).
//!
//! - **The name** is this identity's channel plus the user's SID and the Windows session
//!   (`\\.\pipe\io.beebeeb.app.control.S-1-5-21-….<session id>`): one per identity, user and
//!   session, so a test build and the real app, two people on one PC, and one account signed
//!   in twice (the console and a remote desktop) never meet.
//! - **The security descriptor** names that user as the owner and admits that user only
//!   (`O:<SID>D:P(A;;GA;;;<SID>)`: a protected DACL with one entry), and remote clients are
//!   rejected (`PIPE_REJECT_REMOTE_CLIENTS`).
//! - **Serving it is being the instance**: the pipe is created with
//!   `FILE_FLAG_FIRST_PIPE_INSTANCE`, which fails when the name exists.
//! - **Every wait is bounded.** Reads poll `PeekNamedPipe` against a deadline, so a client that
//!   connects and says nothing costs the server two seconds, and a server that never answers
//!   costs the caller its timeout: neither side can hang the other.
//!
//! - **A launch checks the server before it writes** (task 2455). The pipe namespace is shared
//!   by everybody on the PC and the name is no secret, so ANY local user can create this name
//!   first, with an access list that lets the victim in. [`send`] therefore asks the OS three
//!   things about the pipe it has opened ([`server_facts`]) and hands them to
//!   `control::check_server`: the server process runs as this user, the pipe object is owned
//!   by this user, and its access list admits this user only. On any mismatch, and whenever
//!   one of them cannot be read, it writes NOTHING (no verb, no link) and reports the channel
//!   unreachable: the launch says that it could not check and exits 1; it never starts a
//!   second instance and never talks to a stranger. Starting up beside such a pipe fails the
//!   same way: `FILE_FLAG_FIRST_PIPE_INSTANCE` refuses to create ours.
//!
//! What this does not defend against: another process of the SAME user that creates the name
//! first with the same owner and access list (it would be taken for the running instance, and
//! a launch would hand it one of the three verbs and exit). That includes a LOW-INTEGRITY
//! process of this user (a sandboxed one): it passes all three facts, although it cannot do
//! everything this app can; the check does not read the server's integrity label. Refusing a
//! server whose label is below medium is a follow-up. And an administrator, who can set any
//! owner on any object.

use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_SUCCESS, GENERIC_READ,
    GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
    SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, GetAce, GetLengthSid, GetTokenInformation,
    IsValidSid, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenSessionId, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION,
    SECURITY_SQOS_PRESENT, WriteFile,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT, PeekNamedPipe, WaitNamedPipeW,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;

use super::windows::wide;
use super::{ControlHandler, ControlServer, ServeError};
use crate::shell::control::{self, AccessEntry, MAX_REPLY, MAX_REQUEST, Reply, Request, ServerFacts};
use crate::shell::identity::AppIdentity;
use crate::shell::instance::Delivery;

/// How long the server waits for a connected client's request line.
const REQUEST_WAIT: Duration = Duration::from_secs(2);
/// How long the server waits for the client to take its answer before it hangs up.
const REPLY_TAKEN_WAIT: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(5);

fn last_error() -> u32 {
    // SAFETY: reads this thread's last-error value; takes no pointer.
    unsafe { GetLastError() }
}

/// A pipe or file handle, closed on drop. `HANDLE` is a raw pointer, so this is what crosses
/// to the server thread.
struct Handle(isize);

// SAFETY: a kernel handle is a process-wide name for a kernel object, valid on any thread; the
// only user of a `Handle` at a time is the thread that owns the value.
unsafe impl Send for Handle {}

impl Handle {
    fn raw(&self) -> HANDLE {
        self.0 as HANDLE
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a handle this value owns (CreateNamedPipeW, CreateFileW,
        // OpenProcess or OpenProcessToken returned it, checked valid) and it is closed exactly
        // once, here.
        unsafe { CloseHandle(self.raw()) };
    }
}

/// Memory the OS allocated with `LocalAlloc` and handed over, freed on drop (on every path).
struct LocalMem(*mut core::ffi::c_void);

impl Drop for LocalMem {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a non-null block a Win32 call allocated with LocalAlloc for this
        // value alone (checked where the value is made); it is freed exactly once, here.
        unsafe { LocalFree(self.0) };
    }
}

/// The ACE type of "this SID is granted these rights" (`ACCESS_ALLOWED_ACE_TYPE` in winnt.h;
/// its module in `windows-sys` is not one this crate builds).
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
/// Where the SID starts in an ACCESS_ALLOWED_ACE: after the 4-byte header and the 4-byte mask.
const ACE_SID_OFFSET: u16 = 8;
/// The smallest such entry: those 8 bytes and a SID with no sub-authority (8 bytes).
const MIN_ALLOWED_ACE_SIZE: u16 = 16;

/// A SID as text (`S-1-5-21-…`).
///
/// # Safety
/// `sid` must be null or point at memory that holds a SID and stays alive for the call.
unsafe fn sid_text(sid: PSID) -> Result<String, String> {
    // SAFETY: the caller's contract; IsValidSid reads only the SID's own header and
    // sub-authorities, and a null pointer is refused before it.
    if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
        return Err("not a valid SID".into());
    }
    let mut text: *mut u16 = ptr::null_mut();
    // SAFETY: `sid` is a valid SID (checked above) and alive by the caller's contract; `text`
    // is a live local that receives a LocalAlloc'd, NUL-terminated string on success.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 || text.is_null() {
        return Err(format!("ConvertSidToStringSidW failed: Win32 error {}", last_error()));
    }
    let text = LocalMem(text.cast());
    // SAFETY: `text.0` is the NUL-terminated UTF-16 string the call above allocated; it is
    // read up to its terminator and stays alive until `text` drops at the end of this function.
    let sid = unsafe {
        let start = text.0.cast::<u16>();
        let mut len = 0;
        while *start.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(start, len))
    };
    Ok(sid)
}

/// This process's token, for queries.
fn own_token() -> Result<Handle, String> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: the pseudo-handle of this process needs no closing; `token` is a live local the
    // call fills on success.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 || token.is_null() {
        return Err(format!("OpenProcessToken failed: Win32 error {}", last_error()));
    }
    Ok(Handle(token as isize))
}

/// The SID of a token's user, as text.
fn token_user_sid(token: &Handle) -> Result<String, String> {
    let mut needed: u32 = 0;
    // SAFETY: `token` is a live token handle; a null buffer of length 0 asks for the size,
    // written to the live local `needed`.
    unsafe { GetTokenInformation(token.raw(), TokenUser, ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        return Err(format!("GetTokenInformation failed: Win32 error {}", last_error()));
    }
    // u64 elements: TOKEN_USER holds pointers and needs their alignment.
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: `buffer` is at least `needed` bytes, aligned for TOKEN_USER, and outlives the
    // call and the reads of it below.
    let ok = unsafe { GetTokenInformation(token.raw(), TokenUser, buffer.as_mut_ptr().cast(), needed, &mut needed) };
    if ok == 0 {
        return Err(format!("GetTokenInformation failed: Win32 error {}", last_error()));
    }
    // SAFETY: on success the buffer starts with a TOKEN_USER whose SID pointer points into the
    // same buffer, which is alive until the end of this function.
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    // SAFETY: `sid` points into `buffer`, alive for the call.
    unsafe { sid_text(sid) }
}

/// The current user's SID as text (`S-1-5-21-…`).
fn current_user_sid() -> Result<String, String> {
    token_user_sid(&own_token()?)
}

/// The Windows session this process runs in (the console, or one remote desktop session).
fn current_session_id() -> Result<u32, String> {
    let token = own_token()?;
    let (mut session, mut written): (u32, u32) = (0, 0);
    // SAFETY: `token` is a live token handle; `session` is a live u32, which is exactly what
    // TokenSessionId writes (its size is passed); `written` is a live local.
    let ok = unsafe {
        GetTokenInformation(
            token.raw(),
            TokenSessionId,
            ptr::from_mut(&mut session).cast(),
            size_of::<u32>() as u32,
            &mut written,
        )
    };
    if ok == 0 || written as usize != size_of::<u32>() {
        return Err(format!(
            "GetTokenInformation(session) failed: Win32 error {}",
            last_error()
        ));
    }
    Ok(session)
}

/// The user a running process belongs to: the SID of its token's user.
fn process_user_sid(pid: u32) -> Result<String, String> {
    // SAFETY: takes its arguments by value; the returned handle (if any) is owned by the
    // `Handle` below and closed on every path.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(format!("OpenProcess failed: Win32 error {}", last_error()));
    }
    let process = Handle(process as isize);
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: `process` is a live process handle with the right to query; `token` is a live
    // local the call fills on success.
    if unsafe { OpenProcessToken(process.raw(), TOKEN_QUERY, &mut token) } == 0 || token.is_null() {
        return Err(format!("OpenProcessToken(server) failed: Win32 error {}", last_error()));
    }
    token_user_sid(&Handle(token as isize))
}

/// The owner and the access list of the pipe object behind `pipe` (either end's handle).
fn pipe_security(pipe: &Handle) -> Result<(String, Option<Vec<AccessEntry>>), String> {
    let mut owner: PSID = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `pipe` is a live handle; the three out-pointers are live locals. On success
    // `descriptor` receives a LocalAlloc'd security descriptor and `owner` and `dacl` point
    // INTO it (or stay null), so they are valid exactly as long as `descriptor` is.
    let status = unsafe {
        GetSecurityInfo(
            pipe.raw(),
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    // Whatever the call allocated is freed when this function returns, on every path (a
    // failure too); `owner` and `dacl` are not used after that.
    let _descriptor = (!descriptor.is_null()).then(|| LocalMem(descriptor));
    if status != ERROR_SUCCESS || descriptor.is_null() {
        return Err(format!("GetSecurityInfo failed: Win32 error {status}"));
    }
    // SAFETY: `owner` is null or points into the descriptor, alive until the function returns.
    let owner = unsafe { sid_text(owner) }.map_err(|e| format!("the pipe's owner: {e}"))?;
    if dacl.is_null() {
        return Ok((owner, None));
    }
    // SAFETY: `dacl` is non-null and points at the ACL inside the live descriptor.
    let count = unsafe { (*dacl).AceCount };
    let mut entries = Vec::with_capacity(usize::from(count));
    for index in 0..u32::from(count) {
        let mut ace: *mut core::ffi::c_void = ptr::null_mut();
        // SAFETY: `dacl` is the live ACL; `index` is below its entry count; `ace` is a live
        // local that receives a pointer into the ACL.
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 || ace.is_null() {
            return Err(format!("GetAce failed: Win32 error {}", last_error()));
        }
        // SAFETY: GetAce returned a pointer to an entry inside the live ACL; every entry
        // starts with an ACE_HEADER (read unaligned: the ACL is a byte layout).
        let header = unsafe { ptr::read_unaligned(ace.cast::<ACE_HEADER>()) };
        if header.AceType == ACCESS_ALLOWED_ACE_TYPE {
            // The entry must hold its header, its mask and at least the smallest SID, and the
            // SID must end inside the entry: nothing past the entry's own size is read.
            if header.AceSize < MIN_ALLOWED_ACE_SIZE {
                return Err("an access list entry is too short for a SID".into());
            }
            // SAFETY: an entry of this type and at least 16 bytes is an ACCESS_ALLOWED_ACE
            // whose SID starts at its `SidStart` field; only that field's address is taken.
            let sid: PSID = unsafe { ptr::addr_of_mut!((*ace.cast::<ACCESS_ALLOWED_ACE>()).SidStart) }.cast();
            // SAFETY: the entry holds at least 8 bytes at `sid` (checked above), and
            // GetLengthSid reads only the SID's second byte (its sub-authority count).
            let sid_len = unsafe { GetLengthSid(sid) };
            if sid_len > u32::from(header.AceSize - ACE_SID_OFFSET) {
                return Err("an access list entry's SID does not fit in the entry".into());
            }
            // SAFETY: `sid` points into the live descriptor and its stated length lies inside
            // its entry (checked above); `sid_text` validates it before converting.
            let sid = unsafe { sid_text(sid) }.map_err(|e| format!("an access list entry: {e}"))?;
            entries.push(AccessEntry { allows: true, sid });
        } else {
            entries.push(AccessEntry {
                allows: false,
                sid: String::new(),
            });
        }
    }
    Ok((owner, Some(entries)))
}

/// What the OS says about who serves the pipe behind the client handle `pipe`, and the
/// server's process id. `Err`: one of the facts could not be read (the caller refuses).
///
/// The process id is read first and its process opened afterwards; if the server exits in
/// between and the id is reused, the token read is another process's. The pipe object's own
/// owner and access list do not have that gap: they are read from the handle itself.
fn server_facts(pipe: &Handle) -> Result<(ServerFacts, u32), String> {
    let mut server: u32 = 0;
    // SAFETY: `pipe` is a live client handle; `server` is a live local.
    if unsafe { GetNamedPipeServerProcessId(pipe.raw(), &mut server) } == 0 || server == 0 {
        return Err(format!(
            "GetNamedPipeServerProcessId failed: Win32 error {}",
            last_error()
        ));
    }
    let process_user = process_user_sid(server)?;
    let (owner, access) = pipe_security(pipe)?;
    Ok((
        ServerFacts {
            process_user,
            owner,
            access,
        },
        server,
    ))
}

/// Whether the pipe behind `pipe` is served by this user's own Beebeeb (`control::check_server`).
/// `Ok`: the server's process id. `Err`: why not, in fixed words and error numbers (no SID).
fn verify_server(pipe: &Handle) -> Result<u32, String> {
    let own = current_user_sid()?;
    let (facts, server) = server_facts(pipe)?;
    control::check_server(&own, &facts).map_err(|not_ours| not_ours.words().to_owned())?;
    Ok(server)
}

/// The pipe's security descriptor in SDDL: the user is the owner (stated, not left to the
/// token's default, which is the Administrators group in an elevated process), and a protected
/// DACL whose one entry gives the user everything. Nobody else, not even an administrator's
/// other session, is granted access. A launch checks exactly this (`control::check_server`).
fn sddl_for(sid: &str) -> String {
    format!("O:{sid}D:P(A;;GA;;;{sid})")
}

fn name_for(channel: &str, sid: &str, session: u32) -> String {
    format!("\\\\.\\pipe\\{channel}.{sid}.{session}")
}

/// This identity's pipe for the current user in the current Windows session.
pub(super) fn pipe_name(identity: AppIdentity) -> Result<String, String> {
    Ok(name_for(
        identity.control_channel(),
        &current_user_sid()?,
        current_session_id()?,
    ))
}

/// Creates the pipe, first instance only, open to the current user only.
fn create_pipe(name: &str) -> Result<Handle, ServeError> {
    let sid = current_user_sid().map_err(ServeError::Os)?;
    let sddl = wide(&sddl_for(&sid));
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `sddl` is NUL-terminated and outlives the call; `descriptor` is a live local that
    // receives a LocalAlloc'd descriptor on success, freed below.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    };
    if ok == 0 || descriptor.is_null() {
        return Err(ServeError::Os(format!(
            "the pipe's access list could not be built: Win32 error {}",
            last_error()
        )));
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let name_w = wide(name);
    // SAFETY: `name_w` is NUL-terminated; `attributes` and the descriptor it points at are
    // alive for the call, which copies the descriptor onto the new pipe object.
    let pipe = unsafe {
        CreateNamedPipeW(
            name_w.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            1024,
            1024,
            0,
            &attributes,
        )
    };
    let error = last_error();
    // SAFETY: `descriptor` was allocated by the conversion above and is freed exactly once; the
    // pipe (if created) holds its own copy.
    unsafe { LocalFree(descriptor) };
    if pipe == INVALID_HANDLE_VALUE || pipe.is_null() {
        return Err(match error {
            // FILE_FLAG_FIRST_PIPE_INSTANCE: the name exists (access denied), or all of the
            // existing server's instances are in use. Whose pipe it is, nobody knows yet: the
            // launch that follows finds out before it writes (`send`), and refuses to start
            // when it is not this user's Beebeeb.
            ERROR_ACCESS_DENIED | ERROR_PIPE_BUSY => ServeError::AlreadyRunning,
            error => ServeError::Os(format!("CreateNamedPipeW failed: Win32 error {error}")),
        });
    }
    Ok(Handle(pipe as isize))
}

/// How many bytes can be read without waiting. `None`: the other end is gone.
fn available(pipe: &Handle) -> Option<u32> {
    let mut available: u32 = 0;
    // SAFETY: a null buffer of length 0 reads nothing; `available` is a live local.
    let ok = unsafe {
        PeekNamedPipe(
            pipe.raw(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            &mut available,
            ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

/// Reads until `complete` says so, the cap is passed, the other end goes, or the deadline.
/// Never blocks: it reads only what `PeekNamedPipe` says is already there.
fn read_until(pipe: &Handle, cap: usize, deadline: Instant, complete: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    loop {
        match available(pipe) {
            None => return complete(&bytes).then_some(bytes),
            Some(0) => {}
            Some(waiting) => {
                // One byte past the cap is enough to know the line is too long.
                let want = (waiting as usize).min(cap + 1 - bytes.len());
                let mut chunk = vec![0u8; want];
                let mut read: u32 = 0;
                // SAFETY: `chunk` is `want` bytes and outlives the call; `read` is a live local.
                // At least `want` bytes are already in the pipe, so the call does not wait.
                let ok = unsafe { ReadFile(pipe.raw(), chunk.as_mut_ptr(), want as u32, &mut read, ptr::null_mut()) };
                if ok == 0 {
                    return None;
                }
                bytes.extend_from_slice(&chunk[..read as usize]);
                if complete(&bytes) || bytes.len() > cap {
                    return Some(bytes);
                }
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(POLL);
    }
}

fn write_all(pipe: &Handle, bytes: &[u8]) -> bool {
    let mut written: u32 = 0;
    // SAFETY: `bytes` is valid for its length for the call; `written` is a live local. The
    // messages are far smaller than the pipe's buffer, so the write does not wait.
    let ok = unsafe {
        WriteFile(
            pipe.raw(),
            bytes.as_ptr(),
            bytes.len() as u32,
            &mut written,
            ptr::null_mut(),
        )
    };
    ok != 0 && written as usize == bytes.len()
}

/// One client: read its line, hand a valid request to the handler, answer. Anything that is
/// not a request gets no answer.
fn serve_one(pipe: &Handle, identity: AppIdentity, handler: &ControlHandler) {
    let line = read_until(
        pipe,
        MAX_REQUEST,
        Instant::now() + REQUEST_WAIT,
        control::request_is_complete,
    );
    let Some(request) = line.and_then(|line| Request::decode(&line, identity)) else {
        return;
    };
    // A panic in the handler must not end the channel.
    let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(request))).unwrap_or(Reply::Busy);
    if write_all(pipe, reply.encode()) {
        // Hanging up discards what the client has not read yet: wait (bounded) for it to go.
        let deadline = Instant::now() + REPLY_TAKEN_WAIT;
        while available(pipe).is_some() && Instant::now() < deadline {
            std::thread::sleep(POLL);
        }
    }
}

fn server_thread(pipe: Handle, identity: AppIdentity, handler: ControlHandler, stop: Arc<AtomicBool>) {
    loop {
        // SAFETY: `pipe` is this thread's live pipe handle; no OVERLAPPED, so the call waits
        // for a client (Drop wakes it by connecting).
        let connected =
            unsafe { ConnectNamedPipe(pipe.raw(), ptr::null_mut()) } != 0 || last_error() == ERROR_PIPE_CONNECTED;
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if connected {
            serve_one(&pipe, identity, &handler);
        } else {
            // Do not spin if connecting keeps failing.
            std::thread::sleep(Duration::from_millis(50));
        }
        // SAFETY: `pipe` is this thread's live pipe handle; disconnecting an unconnected pipe
        // fails harmlessly.
        unsafe { DisconnectNamedPipe(pipe.raw()) };
    }
    // `pipe` drops here: the name is free again.
}

struct PipeServer {
    name: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl ControlServer for PipeServer {}

impl Drop for PipeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let Some(worker) = self.worker.take() else { return };
        // Wake the thread out of ConnectNamedPipe by connecting; it may be busy with a client
        // for a few seconds first. Bounded: after that the thread is left to end with the process.
        let deadline = Instant::now() + Duration::from_secs(6);
        while !worker.is_finished() && Instant::now() < deadline {
            drop(open_client(&self.name));
            std::thread::sleep(Duration::from_millis(20));
        }
        if worker.is_finished() {
            let _ = worker.join();
        }
    }
}

/// Serves `name`. `Err(AlreadyRunning)`: another process does.
pub(super) fn serve(
    name: &str,
    identity: AppIdentity,
    handler: ControlHandler,
) -> Result<Box<dyn ControlServer>, ServeError> {
    let pipe = create_pipe(name)?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let worker = std::thread::Builder::new()
        .name("beebeeb-control".into())
        .spawn(move || server_thread(pipe, identity, handler, thread_stop))
        .map_err(|e| ServeError::Os(format!("starting the control thread failed: {e}")))?;
    Ok(Box::new(PipeServer {
        name: name.to_owned(),
        stop,
        worker: Some(worker),
    }))
}

/// Opens the client end. `Err`: the Win32 error.
fn open_client(name: &str) -> Result<Handle, u32> {
    let name_w = wide(name);
    // SAFETY: `name_w` is NUL-terminated and outlives the call; no security attributes, no
    // template. SECURITY_IDENTIFICATION: the server may learn who connected, and cannot act
    // as this process.
    let pipe = unsafe {
        CreateFileW(
            name_w.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            ptr::null(),
            OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            ptr::null_mut(),
        )
    };
    if pipe == INVALID_HANDLE_VALUE || pipe.is_null() {
        Err(last_error())
    } else {
        Ok(Handle(pipe as isize))
    }
}

/// Hands one request to this user's Beebeeb serving `name` and waits at most `timeout` in all.
/// A server that is not this user's Beebeeb is handed nothing: `Delivery::Unreachable`.
pub(super) fn send(name: &str, request: &Request, timeout: Duration) -> Delivery {
    let deadline = Instant::now() + timeout;
    let pipe = loop {
        match open_client(name) {
            Ok(pipe) => break pipe,
            // The server is talking to another client: wait for our turn.
            Err(ERROR_PIPE_BUSY) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Delivery::NoAnswer;
                }
                let name_w = wide(name);
                // SAFETY: `name_w` is NUL-terminated and outlives the call, which only waits.
                unsafe { WaitNamedPipeW(name_w.as_ptr(), left.as_millis().clamp(1, 250) as u32) };
            }
            // Gone, or not ours to open.
            Err(_) => return Delivery::Unreachable,
        }
    };
    // Before a byte is written: is this this user's own Beebeeb (task 2455)? Anything else,
    // and anything that cannot be checked, gets no verb and no link.
    let server = match verify_server(&pipe) {
        Ok(server) => server,
        Err(why) => {
            crate::shell::host::log(format!(
                "the control channel is not served by this user's Beebeeb ({why}); nothing was sent"
            ));
            return Delivery::Unreachable;
        }
    };
    // The running instance may bring its window to the front for this request.
    // SAFETY: takes a process id by value; failing changes nothing.
    unsafe { AllowSetForegroundWindow(server) };
    if !write_all(&pipe, &request.encode()) {
        return Delivery::Unreachable;
    }
    let reply = read_until(&pipe, MAX_REPLY, deadline, |bytes| bytes.contains(&b'\n'));
    match reply.as_deref().and_then(Reply::decode) {
        Some(Reply::Ok) => Delivery::Acknowledged,
        // Busy, no answer in time, hung up without one, or not our protocol.
        _ => Delivery::NoAnswer,
    }
}

#[cfg(test)]
#[path = "windows_control_tests.rs"]
mod tests;
