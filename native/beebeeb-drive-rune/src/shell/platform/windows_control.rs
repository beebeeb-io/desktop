//! Windows: the control channel's transport, a named pipe (`shell/control.rs` is the protocol).
//!
//! - **The name** is this identity's channel plus the user's SID
//!   (`\\.\pipe\io.beebeeb.app.control.S-1-5-21-…`): one per identity and user, so a test build
//!   and the real app, and two people on one PC, never meet.
//! - **The ACL** admits that user only (`D:P(A;;GA;;;<SID>)`: a protected DACL with one entry),
//!   and remote clients are rejected (`PIPE_REJECT_REMOTE_CLIENTS`).
//! - **Serving it is being the instance**: the pipe is created with
//!   `FILE_FLAG_FIRST_PIPE_INSTANCE`, which fails when the name exists.
//! - **Every wait is bounded.** Reads poll `PeekNamedPipe` against a deadline, so a client that
//!   connects and says nothing costs the server two seconds, and a server that never answers
//!   costs the caller its timeout: neither side can hang the other.
//!
//! What this does not defend against: another process of the SAME user that creates the name
//! first (it would be taken for the running instance, and a launch would hand it one of the
//! three verbs and exit). Such a process can already do anything this app can.

use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, GetLastError,
    HANDLE, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION,
    SECURITY_SQOS_PRESENT, WriteFile,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT, PeekNamedPipe, WaitNamedPipeW,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;

use super::windows::wide;
use super::{ControlHandler, ControlServer, ServeError};
use crate::shell::control::{self, MAX_REPLY, MAX_REQUEST, Reply, Request};
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
        // SAFETY: `self.0` is a handle this value owns (CreateNamedPipeW, CreateFileW or
        // OpenProcessToken returned it, checked valid) and it is closed exactly once, here.
        unsafe { CloseHandle(self.raw()) };
    }
}

/// The current user's SID as text (`S-1-5-21-…`).
fn current_user_sid() -> Result<String, String> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: the pseudo-handle of this process needs no closing; `token` is a live local the
    // call fills on success.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(format!("OpenProcessToken failed: Win32 error {}", last_error()));
    }
    let token = Handle(token as isize);
    let mut needed: u32 = 0;
    // SAFETY: a null buffer of length 0 asks for the size, written to the live local `needed`.
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
    let mut text: *mut u16 = ptr::null_mut();
    // SAFETY: `sid` is the valid SID read above; `text` is a live local that receives a
    // LocalAlloc'd, NUL-terminated string on success.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 || text.is_null() {
        return Err(format!("ConvertSidToStringSidW failed: Win32 error {}", last_error()));
    }
    // SAFETY: `text` is the NUL-terminated string the call above allocated: read up to its
    // terminator, then freed exactly once with LocalFree, as its contract says.
    let sid = unsafe {
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
        LocalFree(text.cast());
        sid
    };
    Ok(sid)
}

/// The pipe's security descriptor in SDDL: a protected DACL whose one entry gives the user
/// everything. Nobody else, not even an administrator's other session, is granted access.
fn sddl_for(sid: &str) -> String {
    format!("D:P(A;;GA;;;{sid})")
}

fn name_for(channel: &str, sid: &str) -> String {
    format!("\\\\.\\pipe\\{channel}.{sid}")
}

/// This identity's pipe for the current user.
pub(super) fn pipe_name(identity: AppIdentity) -> Result<String, String> {
    Ok(name_for(identity.control_channel(), &current_user_sid()?))
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
            // existing server's instances are in use.
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

/// Hands one request to whoever serves `name` and waits at most `timeout` in all.
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
    // The running instance may bring its window to the front for this request.
    let mut server: u32 = 0;
    // SAFETY: `pipe` is a live client handle; `server` is a live local.
    if unsafe { GetNamedPipeServerProcessId(pipe.raw(), &mut server) } != 0 && server != 0 {
        // SAFETY: takes a process id by value; failing changes nothing.
        unsafe { AllowSetForegroundWindow(server) };
    }
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

/// Real pipes, real ACL, on this machine. Each test uses a name of its own, so a running app
/// (or another test) is never touched.
#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    const REAL: AppIdentity = AppIdentity::Real;

    fn test_name(what: &str) -> String {
        let sid = current_user_sid().unwrap();
        name_for(
            &format!("io.beebeeb.app.test.harness.{what}.{}", std::process::id()),
            &sid,
        )
    }

    fn recording(reply: Reply) -> (ControlHandler, Arc<Mutex<Vec<Request>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let handler: ControlHandler = Arc::new(move |request| {
            sink.lock().unwrap().push(request);
            reply
        });
        (handler, seen)
    }

    fn raw_exchange(name: &str, bytes: &[u8], wait: Duration) -> Option<Vec<u8>> {
        let pipe = open_client(name).expect("the pipe opens for its own user");
        assert!(write_all(&pipe, bytes));
        read_until(&pipe, MAX_REPLY, Instant::now() + wait, |b| b.contains(&b'\n'))
    }

    #[test]
    fn the_name_and_the_access_list_are_per_identity_and_per_user() {
        let sid = current_user_sid().unwrap();
        assert!(sid.starts_with("S-1-"), "a SID");
        assert_eq!(sddl_for("S-1-5-21-1-2-3-1001"), "D:P(A;;GA;;;S-1-5-21-1-2-3-1001)");
        let (real, test) = (
            pipe_name(AppIdentity::Real).unwrap(),
            pipe_name(AppIdentity::Test).unwrap(),
        );
        assert_eq!(real, format!("\\\\.\\pipe\\io.beebeeb.app.control.{sid}"));
        assert_eq!(test, format!("\\\\.\\pipe\\io.beebeeb.app.test.control.{sid}"));
        assert_ne!(real, test);
    }

    /// P2-4: a real round trip of every verb, with the acknowledgement.
    #[test]
    fn a_request_crosses_a_real_pipe_and_is_acknowledged() {
        let name = test_name("roundtrip");
        let (handler, seen) = recording(Reply::Ok);
        let _server = serve(&name, REAL, handler).unwrap();
        let wait = Duration::from_secs(5);
        assert_eq!(send(&name, &Request::Show, wait), Delivery::Acknowledged);
        assert_eq!(send(&name, &Request::QuitForUpdate, wait), Delivery::Acknowledged);
        let link = Request::Open("beebeeb://settings/security".into());
        assert_eq!(send(&name, &link, wait), Delivery::Acknowledged);
        assert_eq!(*seen.lock().unwrap(), vec![Request::Show, Request::QuitForUpdate, link]);
    }

    /// Serving the name is being the instance: a second server is refused, and the name is
    /// free again once the first is dropped.
    #[test]
    fn only_one_process_serves_a_name() {
        let name = test_name("single");
        let (handler, _) = recording(Reply::Ok);
        let first = serve(&name, REAL, handler.clone()).unwrap();
        assert!(matches!(
            serve(&name, REAL, handler.clone()),
            Err(ServeError::AlreadyRunning)
        ));
        drop(first);
        assert_eq!(
            send(&name, &Request::Show, Duration::from_millis(300)),
            Delivery::Unreachable
        );
        let again = serve(&name, REAL, handler);
        assert!(again.is_ok(), "the name is free after the server is gone");
    }

    /// P2-9: an instance that is there and does not take the request is "no answer", within
    /// the caller's timeout; no instance at all is "unreachable".
    #[test]
    fn a_busy_or_silent_instance_is_no_answer_and_none_is_unreachable() {
        let name = test_name("busy");
        assert_eq!(
            send(&name, &Request::Show, Duration::from_millis(300)),
            Delivery::Unreachable
        );
        let (handler, seen) = recording(Reply::Busy);
        let server = serve(&name, REAL, handler).unwrap();
        assert_eq!(send(&name, &Request::Show, Duration::from_secs(5)), Delivery::NoAnswer);
        assert_eq!(seen.lock().unwrap().len(), 1);
        drop(server);

        // A handler that takes longer than the caller waits.
        let slow_name = test_name("slow");
        let slow: ControlHandler = Arc::new(|_| {
            std::thread::sleep(Duration::from_millis(1500));
            Reply::Ok
        });
        let _server = serve(&slow_name, REAL, slow).unwrap();
        let started = Instant::now();
        assert_eq!(
            send(&slow_name, &Request::Show, Duration::from_millis(300)),
            Delivery::NoAnswer
        );
        assert!(
            started.elapsed() < Duration::from_millis(1400),
            "the caller's timeout holds"
        );
    }

    /// Untrusted input: garbage, an unknown verb, an oversized line and a silent client get no
    /// answer and reach no handler, and the channel still works afterwards.
    #[test]
    fn anything_that_is_not_a_request_is_ignored_and_the_channel_survives() {
        let name = test_name("garbage");
        let (handler, seen) = recording(Reply::Ok);
        let _server = serve(&name, REAL, handler).unwrap();
        let short = Duration::from_millis(400);
        for bytes in [
            &b"hello\n"[..],
            b"BBCTL1 sign-out\n",
            b"BBCTL1 open https://example.com/\n",
            b"BBCTL1 quit-for-update now\n",
            b"\0\0\0\0\n",
        ] {
            assert_eq!(
                raw_exchange(&name, bytes, short),
                None,
                "{:?}",
                String::from_utf8_lossy(bytes)
            );
        }
        let long = vec![b'a'; MAX_REQUEST + 200];
        assert_eq!(raw_exchange(&name, &long, short), None, "too long");
        // Connects and says nothing: the server gives up on it after its own wait.
        let silent = open_client(&name).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        drop(silent);
        assert!(seen.lock().unwrap().is_empty(), "no handler ran");
        assert_eq!(
            send(&name, &Request::Show, Duration::from_secs(5)),
            Delivery::Acknowledged
        );
        assert_eq!(*seen.lock().unwrap(), vec![Request::Show]);
    }

    /// A pipe somebody made before the app: `sddl` is its security descriptor. It takes one
    /// client, keeps what that client wrote, and answers a forged `ok`.
    struct Squatter {
        name: String,
        received: Arc<Mutex<Vec<u8>>>,
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }

    impl Squatter {
        fn received(&self) -> Vec<u8> {
            self.received.lock().unwrap().clone()
        }
    }

    impl Drop for Squatter {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let Some(worker) = self.worker.take() else { return };
            let deadline = Instant::now() + Duration::from_secs(3);
            while !worker.is_finished() && Instant::now() < deadline {
                drop(open_client(&self.name));
                std::thread::sleep(Duration::from_millis(20));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }

    /// `Err`: the Win32 error of creating the pipe (a descriptor this user may not set).
    fn squat(name: &str, sddl: &str) -> Result<Squatter, u32> {
        let sddl_w = wide(sddl);
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `sddl_w` is NUL-terminated and outlives the call; `descriptor` is a live local
        // that receives a LocalAlloc'd descriptor on success, freed below.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl_w.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        assert!(ok != 0 && !descriptor.is_null(), "{sddl} is SDDL");
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let name_w = wide(name);
        // SAFETY: `name_w` is NUL-terminated; `attributes` and its descriptor are alive for the
        // call, which copies the descriptor onto the new pipe object.
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
        // SAFETY: allocated by the conversion above, freed exactly once.
        unsafe { LocalFree(descriptor) };
        if pipe == INVALID_HANDLE_VALUE || pipe.is_null() {
            return Err(error);
        }
        let pipe = Handle(pipe as isize);
        let received = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (sink, thread_stop) = (received.clone(), stop.clone());
        let worker = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                // SAFETY: `pipe` is this thread's live pipe handle; no OVERLAPPED.
                let connected = unsafe { ConnectNamedPipe(pipe.raw(), ptr::null_mut()) } != 0
                    || last_error() == ERROR_PIPE_CONNECTED;
                if connected && !thread_stop.load(Ordering::SeqCst) {
                    let line = read_until(&pipe, MAX_REQUEST, Instant::now() + Duration::from_millis(600), |b| {
                        b.contains(&b'\n')
                    });
                    if let Some(line) = line {
                        sink.lock().unwrap().extend_from_slice(&line);
                        write_all(&pipe, b"ok\n");
                        std::thread::sleep(Duration::from_millis(200));
                    }
                }
                // SAFETY: `pipe` is this thread's live pipe handle.
                unsafe { DisconnectNamedPipe(pipe.raw()) };
            }
        });
        Ok(Squatter {
            name: name.to_owned(),
            received,
            stop,
            worker: Some(worker),
        })
    }

    /// Task 2455, the review's scenario as far as one account can stage it: somebody created
    /// the app's pipe name FIRST, with an access list that lets everyone in, and answers `ok`
    /// to anything. A launch must not take that for the running Beebeeb: it writes nothing (no
    /// verb, no link) and reports the channel unreachable, and the app cannot serve the name.
    #[test]
    fn a_pipe_that_admits_everyone_is_not_the_running_app_and_gets_no_request() {
        let name = test_name("squat-open");
        let sid = current_user_sid().unwrap();
        // Everyone (WD) and this user: the squatter lets its victim connect.
        let squatter = squat(&name, &format!("D:(A;;GA;;;WD)(A;;GA;;;{sid})")).unwrap();
        let (handler, _) = recording(Reply::Ok);
        assert!(
            matches!(serve(&name, REAL, handler), Err(ServeError::AlreadyRunning)),
            "the name is taken: the app is not the instance"
        );
        let link = Request::Open("beebeeb://open/files?folder=Secret_1".into());
        for request in [link, Request::QuitForUpdate, Request::Show] {
            assert_eq!(
                send(&name, &request, Duration::from_secs(2)),
                Delivery::Unreachable,
                "{request:?}"
            );
        }
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            String::from_utf8_lossy(&squatter.received()),
            "",
            "the squatter was handed a request"
        );
    }

    /// A panic in the handler is a "busy" answer, not the end of the channel.
    #[test]
    fn a_panicking_handler_does_not_end_the_channel() {
        let name = test_name("panic");
        let calls = Arc::new(Mutex::new(0));
        let counter = calls.clone();
        let handler: ControlHandler = Arc::new(move |_| {
            let mut calls = counter.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            *calls += 1;
            if *calls == 1 {
                drop(calls);
                panic!("the handler");
            }
            Reply::Ok
        });
        let _server = serve(&name, REAL, handler).unwrap();
        assert_eq!(send(&name, &Request::Show, Duration::from_secs(5)), Delivery::NoAnswer);
        assert_eq!(
            send(&name, &Request::Show, Duration::from_secs(5)),
            Delivery::Acknowledged
        );
    }
}
