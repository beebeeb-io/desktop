//! macOS, an app inside the App Sandbox: the helper is the app's XPC service, and the system gives it a sandbox of
//! its own.
//!
//! A process the app starts itself inherits the app's sandbox and can apply no other (the parent module). An XPC
//! service is started by the system instead: "Executables that are launched by the system (your app, any appexes,
//! any XPC services) start with a fresh sandbox. You can configure that sandbox with the entitlements" (Apple DTS,
//! <https://developer.apple.com/forums/thread/749725>). The helper's bundle is signed with one entitlement,
//! `com.apple.security.app-sandbox`, so its sandbox has no network, no file of the user and no write outside its own
//! container. A profile of our own on top is refused there (`sandbox_init` fails with code 1, measured in the
//! service), so that App Sandbox is the confinement.
//!
//! Everything below was measured on macOS 26.6 (Apple silicon) with a throwaway ad-hoc-signed sandboxed app
//! (`io.beebeeb.test.xpcprobe`) and its service:
//!
//! | The sandboxed app asks | Answer |
//! |---|---|
//! | `sandbox_check(service pid, ...)` | answered, and the same as the service's own (so the kernel is asked, as for a child) |
//! | `SecCodeCopyGuestWithAttributes` by pid, `SecCodeCopySigningInformation` | answered: the entitlements of the running service |
//! | `proc_pid_rusage(service pid)` | refused (code 1): the app cannot read the service's memory |
//! | `kill(service pid, ...)` | refused (code 1); `xpc_connection_kill` ends it within 100 ms |
//! | `kevent` `NOTE_EXIT` on the service pid | registered and delivered |
//! | `shm_open` under a name of ours | refused (code 1): a sandboxed app makes no POSIX shared memory outside an app group |
//! | anonymous shared memory handed over as an XPC shared-memory object | mapped by the service; a read-only one cannot be made writable (code 13) |
//!
//! Measured again on 2026-11-27 (dm-82) from inside the probe's service, with the service's own sandbox as the only
//! confinement (the deny-default profile on top is refused there): listing or reading the user's home, its
//! `Documents` and `Library/Containers`, and writing in it fail with code 1; TCP connects (to 1.1.1.1:443 and to two
//! local ports) and a bind fail with code 1; `posix_spawn` of `/bin/sh` fails with code 35 (the process limit of
//! zero: the App Sandbox itself allows it, see [`APP_SANDBOX_ALLOWS`]); a `.docx` opens and its page 1 renders. The
//! first start right after the probe app was signed again was refused (its code did not yet check valid against its
//! signature): that fails closed, and the next start was accepted.
//!
//! What follows from it:
//!
//! - **One document per process.** The system runs one process per service and app, whatever the number of
//!   connections (measured). So one helper runs at a time ([`Slot`]): a second start waits [`SERVICE_WAIT`] for the
//!   first to be closed and is then refused with system code 16; the file falls back to Quick Look. Dropping a
//!   `Child` ends the service's process (`xpc_connection_kill`, then the connection is cancelled) and waits for the
//!   kernel's notice of its exit before the next start, so every document gets a process of its own. The service
//!   also ends itself when its connection is gone (the app closed it, or died). A process not seen to end within that
//!   wait is waited for again by the next start, before it connects ([`Slot::wait_for_previous`]).
//! - **The start's wait** (dm-84, RC15): the system starts the process, so the wait for the service's answer holds
//!   that start: [`SERVICE_HANDSHAKE_BOUND`], 30 s. launchd holds back the start of a service whose last process ran
//!   for less than 10 s, and a cold first launch checks a large executable; RC15 waited 10 s and refused exactly
//!   then. Every start says in the log how long the answer took, or why none came (timed out, interrupted, invalid),
//!   and a refusal names the check that failed.
//! - **Checked before any grant** ([`judge_service`]): the kernel's own answers for the service's process, asked by
//!   the app, and the entitlements of its signature, exactly the one. The kernel denies an App Sandbox 13 of the 22
//!   operations the deny-default profile denies, plus the user's home; the other nine are named at
//!   [`APP_SANDBOX_ALLOWS`]. A service that did apply the profile is held to all 22.
//! - **Memory.** The app cannot read the service's footprint, so the service watches itself and ends above its
//!   limit (`macos_service.rs`); code that no longer runs as written can stop that watch, and is then bounded by the
//!   resource limits the service set on itself and by the caller's own timer followed by [`ServiceChild::kill`].
//! - **Sections** are anonymous shared memory here ([`Shared`]), handed over as XPC shared-memory objects on the
//!   connection: no descriptor exists to send. A read grant carries a second view of the same memory whose highest
//!   protection is read-only; the object made from it cannot carry more.
//!
//! `xpc_connection_kill` has no public header (as `sandbox_check` has none); everything else is `<xpc/xpc.h>`,
//! `<mach/mach_vm.h>` and `Security.framework`, the last one loaded at run time so a helper never links it.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fmt;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::{
    Check, Facts, MUST_BE_ALLOWED, MUST_BE_DENIED, PROFILE_VERSION, REAP_BOUND, STATE_CONFINED, STATE_REFUSED, Status,
    UNKNOWN_EXIT_CODE, denied, errno, lock, owned, pipe, read_facts, sys,
};
use crate::{ExitStatus, KILLED_EXIT_CODE, Limits, Mode, Pipes, SandboxError};

/// The handshake's state of a service whose confinement is the App Sandbox the system gave it.
pub(super) const STATE_APP_SANDBOX: u32 = 3;

/// How long a second start waits for the one running helper to be closed.
pub(super) const SERVICE_WAIT: Duration = Duration::from_secs(3);

/// How long the app waits for its service's answer (dm-84). The system starts the service's process, so the wait
/// holds that start too: a cold first launch checks the code of a large executable, and launchd holds back the next
/// start of a service whose last process ran for less than 10 s ("pushing respawn out by 10 seconds"). RC15 waited
/// 10 s (the bound for a helper the app starts itself, `HANDSHAKE_BOUND`) and refused both first starts exactly
/// then. The answer is still checked in full before anything is handed over.
pub(super) const SERVICE_HANDSHAKE_BOUND: Duration = Duration::from_secs(30);

/// How long a start waits for the process of the document before it (killed, or ending itself) to be gone: the
/// system runs one process per service, so a connection made while that one still runs reaches it, and it serves one
/// document only.
pub(super) const PREVIOUS_EXIT_BOUND: Duration = Duration::from_secs(10);

/// A service the checks refuse while it says it is confined is asked once more after this long: the kernel's view of
/// a process that has just confined itself can lag its own report.
pub(super) const RECHECK_AFTER: Duration = Duration::from_millis(250);

/// The waits of one service start ([`start_service`]); the tests shorten them.
#[derive(Debug, Clone, Copy)]
pub(super) struct Bounds {
    pub(super) handshake: Duration,
    pub(super) reap: Duration,
    pub(super) recheck: Duration,
}

impl Bounds {
    pub(super) const SERVICE: Bounds = Bounds {
        handshake: SERVICE_HANDSHAKE_BOUND,
        reap: REAP_BOUND,
        recheck: RECHECK_AFTER,
    };
}

/// System code 16: the one service process is taken by another document.
const EBUSY: u32 = 16;

pub(super) type XpcObject = *mut c_void;

/// A block without captured values, laid out as the compiler lays one out (`Block_private.h` of the system's blocks
/// runtime): what `xpc_connection_set_event_handler` takes. No crate and no C file is needed for it.
#[repr(C)]
pub(super) struct Block {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: unsafe extern "C" fn(*const Block, XpcObject),
    descriptor: *const BlockDescriptor,
}

#[repr(C)]
struct BlockDescriptor {
    reserved: usize,
    size: usize,
}

// SAFETY: both hold only addresses of items that live for the whole process and are never written.
unsafe impl Sync for Block {}
// SAFETY: as above.
unsafe impl Sync for BlockDescriptor {}

static BLOCK_DESCRIPTOR: BlockDescriptor = BlockDescriptor {
    reserved: 0,
    size: size_of::<Block>(),
};

/// `BLOCK_IS_GLOBAL`: the block is never copied or freed.
const BLOCK_IS_GLOBAL: c_int = 1 << 28;

impl Block {
    pub(super) const fn new(invoke: unsafe extern "C" fn(*const Block, XpcObject)) -> Self {
        Self {
            isa: (&raw const _NSConcreteGlobalBlock).cast(),
            flags: BLOCK_IS_GLOBAL,
            reserved: 0,
            invoke,
            descriptor: &raw const BLOCK_DESCRIPTOR,
        }
    }
}

unsafe extern "C" {
    static _NSConcreteGlobalBlock: [*const c_void; 32];
    pub(super) static _xpc_type_dictionary: c_void;
    static _xpc_type_shmem: c_void;
    static _xpc_type_error: c_void;
    static _xpc_error_connection_interrupted: c_void;
    static _xpc_error_connection_invalid: c_void;

    fn xpc_connection_create(name: *const c_char, queue: *mut c_void) -> XpcObject;
    pub(super) fn xpc_connection_set_event_handler(connection: XpcObject, handler: *const Block);
    pub(super) fn xpc_connection_resume(connection: XpcObject);
    pub(super) fn xpc_connection_cancel(connection: XpcObject);
    fn xpc_connection_get_pid(connection: XpcObject) -> c_int;
    pub(super) fn xpc_connection_send_message(connection: XpcObject, message: XpcObject);
    fn xpc_connection_send_message_with_reply_sync(connection: XpcObject, message: XpcObject) -> XpcObject;
    /// No public header. Ends the service's process with the signal (measured: within 100 ms).
    fn xpc_connection_kill(connection: XpcObject, signal: c_int);
    fn xpc_dictionary_create(keys: *const *const c_char, values: *const XpcObject, count: usize) -> XpcObject;
    pub(super) fn xpc_dictionary_create_reply(original: XpcObject) -> XpcObject;
    pub(super) fn xpc_dictionary_get_remote_connection(dictionary: XpcObject) -> XpcObject;
    pub(super) fn xpc_dictionary_set_uint64(dictionary: XpcObject, key: *const c_char, value: u64);
    pub(super) fn xpc_dictionary_get_uint64(dictionary: XpcObject, key: *const c_char) -> u64;
    fn xpc_dictionary_set_fd(dictionary: XpcObject, key: *const c_char, fd: c_int);
    pub(super) fn xpc_dictionary_dup_fd(dictionary: XpcObject, key: *const c_char) -> c_int;
    fn xpc_dictionary_set_value(dictionary: XpcObject, key: *const c_char, value: XpcObject);
    pub(super) fn xpc_dictionary_get_value(dictionary: XpcObject, key: *const c_char) -> XpcObject;
    fn xpc_shmem_create(region: *mut c_void, len: usize) -> XpcObject;
    pub(super) fn xpc_shmem_map(shmem: XpcObject, region: *mut *mut c_void) -> usize;
    pub(super) fn xpc_get_type(object: XpcObject) -> *const c_void;
    pub(super) fn xpc_retain(object: XpcObject) -> XpcObject;
    pub(super) fn xpc_release(object: XpcObject);

    fn mach_vm_remap(
        target: u32,
        address: *mut u64,
        size: u64,
        mask: u64,
        flags: c_int,
        source: u32,
        source_address: u64,
        copy: c_int,
        current: *mut c_int,
        maximum: *mut c_int,
        inheritance: u32,
    ) -> c_int;
    fn mach_vm_protect(target: u32, address: u64, size: u64, set_maximum: c_int, protection: c_int) -> c_int;
    fn getpagesize() -> c_int;
    pub(super) fn mprotect(address: *mut c_void, len: usize, protection: c_int) -> c_int;
    fn _NSGetExecutablePath(buffer: *mut c_char, size: *mut u32) -> c_int;
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn getuid() -> u32;
    fn getpwuid(uid: u32) -> *const Passwd;
}

/// `struct passwd` (`pwd.h`), up to the field read here.
#[repr(C)]
struct Passwd {
    name: *const c_char,
    password: *const c_char,
    uid: u32,
    gid: u32,
    change: i64,
    class: *const c_char,
    gecos: *const c_char,
    dir: *const c_char,
}

/// The keys of the messages between the app and its service.
pub(super) const KEY_MEMORY: &CStr = c"memory";
pub(super) const KEY_STDIN: &CStr = c"stdin";
pub(super) const KEY_STDOUT: &CStr = c"stdout";
pub(super) const KEY_STATE: &CStr = c"state";
pub(super) const KEY_PROFILE: &CStr = c"profile";
pub(super) const KEY_GRANT: &CStr = c"grant";
pub(super) const KEY_WRITE: &CStr = c"write";
pub(super) const KEY_REGION: &CStr = c"region";

/// The service a helper path names: `<name>.xpc/Contents/MacOS/<file>` is the executable of the XPC service
/// `<name>` (the bundle carries its identifier as its name; the bundle test holds that). `None` for any other path.
pub(super) fn service_name(helper_exe: &Path) -> Option<CString> {
    let macos = helper_exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    if macos.file_name()? != "MacOS" || contents.file_name()? != "Contents" {
        return None;
    }
    let name = bundle.file_name()?.as_bytes().strip_suffix(b".xpc")?;
    if name.is_empty() {
        return None;
    }
    CString::new(name).ok()
}

/// The service this process is, when its own executable sits in an XPC service's bundle. The path is the one the
/// system started the process from; nothing is read from the environment, an argument or a file.
pub(super) fn own_service() -> bool {
    static OWN: OnceLock<bool> = OnceLock::new();
    *OWN.get_or_init(|| {
        let mut buffer = vec![0u8; 4096];
        let mut size = buffer.len() as u32;
        // SAFETY: `buffer` is writable for `size` bytes; the call writes a terminated path or reports the size needed.
        if unsafe { _NSGetExecutablePath(buffer.as_mut_ptr().cast(), &raw mut size) } != 0 {
            return false;
        }
        let len = buffer.iter().position(|&byte| byte == 0).unwrap_or(0);
        service_name(Path::new(std::ffi::OsStr::from_bytes(&buffer[..len]))).is_some()
    })
}

/// Whether this process has a sandbox (asked once): such an app makes no POSIX shared-memory object.
pub(super) fn app_is_sandboxed() -> bool {
    static SANDBOXED: OnceLock<bool> = OnceLock::new();
    // SAFETY: takes no pointer.
    *SANDBOXED.get_or_init(|| super::has_sandbox(unsafe { sys::getpid() }))
}

/// The length of the whole pages that hold `len` bytes.
pub(super) fn span(len: usize) -> usize {
    // SAFETY: takes no pointer.
    let page = unsafe { getpagesize() }.max(4096) as usize;
    len.div_ceil(page).saturating_mul(page)
}

const MAP_ANON: c_int = 0x1000;
const VM_FLAGS_ANYWHERE: c_int = 1;
const VM_INHERIT_NONE: u32 = 2;
const VM_PROT_READ: c_int = 1;

/// Anonymous shared memory of `len` bytes, all zero: what a section is made of where `shm_open` is refused.
pub(super) fn anonymous(len: usize) -> Result<*mut u8, SandboxError> {
    // SAFETY: a new anonymous shared mapping at an address the system chooses; its owner unmaps it once.
    let view = unsafe {
        sys::mmap(
            null_mut(),
            len,
            sys::PROT_READ | sys::PROT_WRITE,
            sys::MAP_SHARED | MAP_ANON,
            -1,
            0,
        )
    };
    if view as isize == -1 {
        return Err(super::os("section view"));
    }
    Ok(view.cast())
}

/// A second view of the `len` bytes at `view` (the same memory, not a copy) whose protection and highest protection
/// are read-only: the kernel refuses to make it writable, here and in whatever maps an object made from it
/// (measured: code 13 in the service). Unmapped by its owner.
pub(super) fn sealed(view: *mut u8, len: usize) -> Result<*mut u8, SandboxError> {
    let size = span(len) as u64;
    let failed = |code: c_int| SandboxError::Os {
        step: "section seal",
        code: code as u32,
    };
    let mut address = 0u64;
    let (mut current, mut maximum) = (0 as c_int, 0 as c_int);
    // SAFETY: maps the pages at `view` (mapped for `size` bytes by this process) a second time at an address the
    // system chooses, shared (`copy` is 0); the three out-values are valid places.
    let result = unsafe {
        mach_vm_remap(
            sys::mach_task_self_,
            &raw mut address,
            size,
            0,
            VM_FLAGS_ANYWHERE,
            sys::mach_task_self_,
            view as u64,
            0,
            &raw mut current,
            &raw mut maximum,
            VM_INHERIT_NONE,
        )
    };
    if result != 0 {
        return Err(failed(result));
    }
    for set_maximum in [1, 0] {
        // SAFETY: the mapping just made, for its whole size; lowers its highest, then its current protection.
        let result = unsafe { mach_vm_protect(sys::mach_task_self_, address, size, set_maximum, VM_PROT_READ) };
        if result != 0 {
            // SAFETY: the mapping just made, unmapped once.
            unsafe { sys::munmap(address as *mut c_void, size as usize) };
            return Err(failed(result));
        }
    }
    Ok(address as *mut u8)
}

/// The app's side of one service: what [`ServiceChild`] needs of it. The system's (XPC) in the app, a fake in the
/// tests.
pub(super) trait Link: Send + Sync + fmt::Debug {
    /// Hands the service the helper's ends of the two pipes and its memory limit, and waits at most `bound` for
    /// its answer: what it says about itself, and its process. `None` when no well-formed answer came.
    fn start(&self, stdin: &OwnedFd, stdout: &OwnedFd, memory_bytes: u64, bound: Duration) -> Option<(Status, c_int)>;
    /// What the kernel and the service's signature say about the process `pid`.
    fn facts(&self, pid: c_int, status: Option<Status>) -> ServiceFacts;
    /// Sends one grant: the `span` bytes at `region`, of which the section is the first `len`.
    fn grant(&self, wire: u64, len: u64, writable: bool, region: *mut u8, span: usize) -> Result<(), i32>;
    /// Ends the service's process now. Fine on one that already ended.
    fn kill(&self);
    /// Waits at most `bound` for the service's process to be gone.
    fn ended_within(&self, bound: Duration) -> bool;
}

/// What is read about a service before the app hands out its pipes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ServiceFacts {
    /// The handshake and the kernel's answers for the deny-default profile's list.
    pub(super) facts: Facts,
    /// How many of [`APP_SANDBOX_DENIES`] and of the checks on the user's home the kernel reports as denied.
    pub(super) app_denied: usize,
    /// How many were asked (`0` when the user's home could not be named: nothing is taken for denied then).
    pub(super) app_asked: usize,
    /// The first of those the kernel did not report as denied: its place in [`APP_SANDBOX_DENIES`] followed by
    /// [`HOME_CHECKS`] ([`app_check_name`] names it in the log).
    pub(super) first_allowed: Option<usize>,
    /// The running code is valid against its signature.
    pub(super) signature_valid: bool,
    /// The entitlements of the signature: how many, and whether `com.apple.security.app-sandbox` is true.
    pub(super) entitlements: Option<(usize, bool)>,
}

/// What the kernel denies a process in an App Sandbox without any further entitlement, of the deny-default
/// profile's list (measured in the service, and the same asked from the app).
pub(super) const APP_SANDBOX_DENIES: &[Check] = &[
    Check::Path(c"file-read-data", c"/Users"),
    Check::Path(c"file-read-data", c"/private/var"),
    Check::Path(c"file-read-data", c"/Library/Preferences"),
    Check::Path(c"file-write-data", c"/Users"),
    Check::Path(c"file-write-data", c"/private/tmp"),
    Check::Path(c"file-write-data", c"/private/var/tmp"),
    Check::Path(c"file-write-data", c"/Library/Fonts"),
    Check::Path(c"file-write-create", c"/private/tmp"),
    Check::Path(c"file-write-create", c"/Library/Fonts"),
    Check::Path(c"file-write-unlink", c"/Library/Fonts"),
    Check::Plain(c"network-outbound"),
    Check::Plain(c"network-inbound"),
    Check::Plain(c"network-bind"),
];

/// What an App Sandbox leaves allowed of the deny-default profile's list (measured): reading the system's own
/// files, starting a process (the service takes that away from itself with a process limit of zero before it reads
/// anything: `posix_spawn` and `fork` then fail with code 35, measured), and looking up the system's Mach services.
/// The known difference to a helper under the profile; a test holds that the two lists make up the whole.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const APP_SANDBOX_ALLOWS: &[Check] = &[
    Check::Path(c"file-read-data", c"/private/etc/passwd"),
    Check::Path(c"file-read-data", c"/Volumes"),
    Check::Path(c"file-read-data", c"/Applications"),
    Check::Path(c"process-exec*", c"/bin/sh"),
    Check::Path(c"process-exec*", c"/usr/bin/true"),
    Check::Plain(c"process-fork"),
    Check::Service(c"mach-lookup", c"com.apple.system.notification_center"),
    Check::Service(c"mach-lookup", c"com.apple.windowserver.active"),
    Check::Service(c"mach-lookup", c"com.apple.SecurityServer"),
];

/// The checks on the user's home: reading it, its `Library` and its `Documents`, writing and creating in it.
const HOME_CHECKS: &[(&CStr, &str)] = &[
    (c"file-read-data", ""),
    (c"file-read-data", "/Library"),
    (c"file-read-data", "/Documents"),
    (c"file-write-data", ""),
    (c"file-write-create", ""),
    (c"file-write-create", "/Library"),
];

/// The user's home as the system's user database names it (a sandboxed app's `HOME` is its container, which is not
/// what the service must be kept out of).
fn home() -> Option<Vec<u8>> {
    // SAFETY: the call returns null or a record that stays valid until the next such call on this thread; its
    // directory is a terminated string, copied before anything else runs.
    unsafe {
        let record = getpwuid(getuid());
        if record.is_null() || (*record).dir.is_null() {
            return None;
        }
        let dir = CStr::from_ptr((*record).dir).to_bytes();
        (dir.len() > 1 && dir[0] == b'/').then(|| dir.to_vec())
    }
}

/// The name of one of the App Sandbox checks in the log: its operation and its place (`~` is the user's home).
pub(super) fn app_check_name(index: usize) -> String {
    let text = |value: &CStr| value.to_string_lossy().into_owned();
    if let Some(check) = APP_SANDBOX_DENIES.get(index) {
        return match *check {
            Check::Plain(operation) => text(operation),
            Check::Path(operation, place) | Check::Service(operation, place) => {
                format!("{} {}", text(operation), text(place))
            }
        };
    }
    match HOME_CHECKS.get(index - APP_SANDBOX_DENIES.len().min(index)) {
        Some((operation, below)) => format!("{} ~{below}", text(operation)),
        None => String::from("unknown"),
    }
}

/// Reads what [`judge_service`] judges about the process `pid`.
pub(super) fn read_service_facts(pid: c_int, status: Option<Status>) -> ServiceFacts {
    let mut first_allowed = None;
    let mut app_denied = 0;
    for (index, check) in APP_SANDBOX_DENIES.iter().enumerate() {
        if denied(pid, *check) == Some(true) {
            app_denied += 1;
        } else {
            first_allowed.get_or_insert(index);
        }
    }
    let mut app_asked = 0;
    if let Some(home) = home() {
        app_asked = APP_SANDBOX_DENIES.len() + HOME_CHECKS.len();
        for (index, (operation, below)) in HOME_CHECKS.iter().enumerate() {
            let mut path = home.clone();
            path.extend_from_slice(below.as_bytes());
            let refused = CString::new(path).is_ok_and(|path| super::denied_path(pid, operation, &path) == Some(true));
            if refused {
                app_denied += 1;
            } else {
                first_allowed.get_or_insert(APP_SANDBOX_DENIES.len() + index);
            }
        }
    }
    let signature = signature(pid);
    ServiceFacts {
        facts: read_facts(pid, status),
        app_denied,
        app_asked,
        first_allowed,
        signature_valid: signature.is_some_and(|signature| signature.valid),
        entitlements: signature.and_then(|signature| signature.entitlements),
    }
}

/// Refuses a service whose facts are not those of a process confined by the system: one check per fact, each with
/// its own reason. Pure, so every refusal is tested.
pub(super) fn judge_service(service: &ServiceFacts) -> Result<(), SandboxError> {
    let refused = |why| Err(SandboxError::NotConfined(why));
    let facts = &service.facts;
    let Some(status) = facts.status else {
        return refused("no handshake");
    };
    if status.state == STATE_REFUSED {
        return refused("the service reports no sandbox of its own");
    }
    if status.state != STATE_CONFINED && status.state != STATE_APP_SANDBOX {
        return refused("an unknown handshake");
    }
    if status.profile_version != PROFILE_VERSION {
        return refused("another profile than this app's");
    }
    if !facts.sandboxed {
        return refused("the kernel reports no sandbox");
    }
    if !service.signature_valid {
        return refused("the service's code is not what its signature covers");
    }
    if service.entitlements != Some((1, true)) {
        return refused("the service's signature carries other entitlements than the App Sandbox alone");
    }
    if facts.allowed != MUST_BE_ALLOWED.len() {
        return refused("the kernel's answers are not a confined helper's");
    }
    if status.state == STATE_CONFINED {
        // It says the deny-default profile took: then all of that profile's list is denied.
        if facts.denied != MUST_BE_DENIED.len() {
            return refused("the kernel allows what the profile denies");
        }
        return Ok(());
    }
    if service.app_asked == 0 || service.app_denied != service.app_asked {
        return refused("the kernel allows what the App Sandbox denies");
    }
    Ok(())
}

/// What the system's code-signing service says about a running process.
#[derive(Debug, Clone, Copy)]
struct Signature {
    valid: bool,
    entitlements: Option<(usize, bool)>,
}

/// The calls of `Security.framework` and `CoreFoundation` used to read a signature, looked up at run time: the
/// crate links neither, so a helper (which links this crate too) loads neither.
struct Signing {
    number_create: unsafe extern "C" fn(*const c_void, isize, *const c_void) -> *const c_void,
    dictionary_create: unsafe extern "C" fn(
        *const c_void,
        *const *const c_void,
        *const *const c_void,
        isize,
        *const c_void,
        *const c_void,
    ) -> *const c_void,
    dictionary_count: unsafe extern "C" fn(*const c_void) -> isize,
    dictionary_value: unsafe extern "C" fn(*const c_void, *const c_void) -> *const c_void,
    string_create: unsafe extern "C" fn(*const c_void, *const c_char, u32) -> *const c_void,
    release: unsafe extern "C" fn(*const c_void),
    key_callbacks: *const c_void,
    value_callbacks: *const c_void,
    boolean_true: *const c_void,
    guest_pid: *const c_void,
    info_entitlements: *const c_void,
    copy_guest: unsafe extern "C" fn(*const c_void, *const c_void, u32, *mut *const c_void) -> i32,
    check_validity: unsafe extern "C" fn(*const c_void, u32, *const c_void) -> i32,
    copy_information: unsafe extern "C" fn(*const c_void, u32, *mut *const c_void) -> i32,
}

// SAFETY: addresses of functions and constants of two system libraries that stay loaded for the whole process.
unsafe impl Send for Signing {}
// SAFETY: as above.
unsafe impl Sync for Signing {}

impl Signing {
    fn get() -> Option<&'static Signing> {
        static SIGNING: OnceLock<Option<Signing>> = OnceLock::new();
        SIGNING.get_or_init(Self::load).as_ref()
    }

    fn load() -> Option<Self> {
        // SAFETY: a terminated path of a system framework; the handle is kept for the whole process.
        let library = unsafe { dlopen(c"/System/Library/Frameworks/Security.framework/Security".as_ptr(), 1) };
        if library.is_null() {
            return None;
        }
        let symbol = |name: &CStr| {
            // SAFETY: a handle `dlopen` returned and a terminated name. The framework's own exports and those of
            // the libraries it depends on (CoreFoundation) are found.
            let address = unsafe { dlsym(library, name.as_ptr()) };
            (!address.is_null()).then_some(address)
        };
        // The address of a constant that holds an object: the object.
        let constant = |name: &CStr| {
            // SAFETY: the symbol is a pointer-sized constant of the library, readable for the whole process.
            symbol(name).map(|address| unsafe { *address.cast::<*const c_void>() })
        };
        // SAFETY: each address is that of the named function of the system's SDK, whose C signature is the type it
        // is given here (`CFNumber.h`, `CFDictionary.h`, `CFString.h`, `CFBase.h`, `SecCode.h`).
        unsafe {
            Some(Self {
                number_create: std::mem::transmute::<*mut c_void, _>(symbol(c"CFNumberCreate")?),
                dictionary_create: std::mem::transmute::<*mut c_void, _>(symbol(c"CFDictionaryCreate")?),
                dictionary_count: std::mem::transmute::<*mut c_void, _>(symbol(c"CFDictionaryGetCount")?),
                dictionary_value: std::mem::transmute::<*mut c_void, _>(symbol(c"CFDictionaryGetValue")?),
                string_create: std::mem::transmute::<*mut c_void, _>(symbol(c"CFStringCreateWithCString")?),
                release: std::mem::transmute::<*mut c_void, _>(symbol(c"CFRelease")?),
                key_callbacks: symbol(c"kCFTypeDictionaryKeyCallBacks")?,
                value_callbacks: symbol(c"kCFTypeDictionaryValueCallBacks")?,
                boolean_true: constant(c"kCFBooleanTrue")?,
                guest_pid: constant(c"kSecGuestAttributePid")?,
                info_entitlements: constant(c"kSecCodeInfoEntitlementsDict")?,
                copy_guest: std::mem::transmute::<*mut c_void, _>(symbol(c"SecCodeCopyGuestWithAttributes")?),
                check_validity: std::mem::transmute::<*mut c_void, _>(symbol(c"SecCodeCheckValidity")?),
                copy_information: std::mem::transmute::<*mut c_void, _>(symbol(c"SecCodeCopySigningInformation")?),
            })
        }
    }
}

/// `kCFNumberIntType`, `kCFStringEncodingUTF8`, and `kSecCSSigningInformation | kSecCSRequirementInformation |
/// kSecCSDynamicInformation`: the entitlements of the code that runs, not of a file.
const NUMBER_INT: isize = 9;
const UTF8: u32 = 0x0800_0100;
const SIGNING_INFORMATION: u32 = (1 << 1) | (1 << 2) | (1 << 3);

/// The signature of the code that runs as the process `pid`: whether the running code is still what the signature
/// covers, and its entitlements. `None` when the system could not name the process's code at all.
fn signature(pid: c_int) -> Option<Signature> {
    let api = Signing::get()?;
    // SAFETY: every object made here is released once, below; every call gets objects of the kind its C signature
    // names, and valid places for what it returns. A null result is checked before it is used.
    unsafe {
        let number = (api.number_create)(null(), NUMBER_INT, (&raw const pid).cast());
        if number.is_null() {
            return None;
        }
        let (keys, values) = ([api.guest_pid], [number]);
        let attributes = (api.dictionary_create)(
            null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            api.key_callbacks,
            api.value_callbacks,
        );
        (api.release)(number);
        if attributes.is_null() {
            return None;
        }
        let mut code: *const c_void = null();
        let found = (api.copy_guest)(null(), attributes, 0, &raw mut code);
        (api.release)(attributes);
        if found != 0 || code.is_null() {
            return None;
        }
        let valid = (api.check_validity)(code, 0, null()) == 0;
        let mut information: *const c_void = null();
        let mut entitlements = None;
        if (api.copy_information)(code, SIGNING_INFORMATION, &raw mut information) == 0 && !information.is_null() {
            let dictionary = (api.dictionary_value)(information, api.info_entitlements);
            if !dictionary.is_null() {
                let key = (api.string_create)(null(), c"com.apple.security.app-sandbox".as_ptr(), UTF8);
                if !key.is_null() {
                    let sandboxed = (api.dictionary_value)(dictionary, key) == api.boolean_true;
                    entitlements = Some(((api.dictionary_count)(dictionary).max(0) as usize, sandboxed));
                    (api.release)(key);
                }
            }
            (api.release)(information);
        }
        (api.release)(code);
        Some(Signature { valid, entitlements })
    }
}

/// The one service process of the app: taken by the helper that runs, free again when that helper is gone.
#[derive(Debug)]
pub(super) struct Slot {
    taken: Mutex<bool>,
    freed: Condvar,
    /// The link of the document before, when its process was not seen to end within its close's wait: the next start
    /// waits for it first ([`Slot::wait_for_previous`]).
    lingering: Mutex<Option<Box<dyn Link>>>,
}

/// Proof that the service is this helper's; dropping it lets the next start in.
#[derive(Debug)]
pub(super) struct SlotGuard(&'static Slot);

impl Slot {
    pub(super) const fn new() -> Self {
        Self {
            taken: Mutex::new(false),
            freed: Condvar::new(),
            lingering: Mutex::new(None),
        }
    }

    /// Waits at most `bound` for the service to be free, and takes it.
    pub(super) fn take(&'static self, bound: Duration) -> Option<SlotGuard> {
        let deadline = Instant::now() + bound;
        let mut taken = lock(&self.taken);
        while *taken {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            taken = self
                .freed
                .wait_timeout(taken, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        *taken = true;
        Some(SlotGuard(self))
    }

    /// Before a new connection: waits at most `bound` for the process of the document before to be gone, when its
    /// close could not see it end. `true` when there is no such process (any more). Logged with how long it took.
    pub(super) fn wait_for_previous(&self, bound: Duration) -> bool {
        let Some(previous) = lock(&self.lingering).take() else {
            return true;
        };
        let since = Instant::now();
        let gone = previous.ended_within(bound);
        if gone {
            note(&format!(
                "office helper: waited {} ms for the previous document's service process to end",
                since.elapsed().as_millis()
            ));
        } else {
            note(&format!(
                "office helper: the previous document's service process still runs after {} s; this start may reach it",
                bound.as_secs()
            ));
        }
        gone
    }
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        *lock(&self.0.taken) = false;
        self.0.freed.notify_one();
    }
}

static SLOT: Slot = Slot::new();

/// A helper that runs as the app's XPC service. Dropping it ends the service's process.
#[derive(Debug)]
pub(crate) struct ServiceChild {
    link: Box<dyn Link>,
    pid: c_int,
    /// How long dropping it waits for its process to be gone before the next start waits for that instead.
    reap: Duration,
    ended: Mutex<Option<ExitStatus>>,
    /// Set before the app's own kill, so the end it causes is told apart from any other.
    killed: AtomicBool,
    next_wire: AtomicU64,
    granted: Arc<AtomicU64>,
    /// Last, so the service is free for the next document only after this one's process is gone.
    _slot: SlotGuard,
}

impl ServiceChild {
    pub(super) fn id(&self) -> u32 {
        self.pid.max(0) as u32
    }

    /// The system reports the end of a process that is not the app's child, not how it ended: the code is the app's
    /// own kill or "not reported".
    pub(super) fn wait(&self, bound: Duration) -> Result<Option<ExitStatus>, SandboxError> {
        let mut ended = lock(&self.ended);
        if ended.is_none() && self.link.ended_within(bound) {
            let code = if self.killed.load(Ordering::SeqCst) {
                KILLED_EXIT_CODE
            } else {
                UNKNOWN_EXIT_CODE
            };
            *ended = Some(ExitStatus { code });
        }
        Ok(*ended)
    }

    pub(super) fn kill(&self) -> Result<(), SandboxError> {
        if lock(&self.ended).is_none() {
            self.killed.store(true, Ordering::SeqCst);
            self.link.kill();
        }
        Ok(())
    }

    pub(super) fn granted_bytes(&self) -> &Arc<AtomicU64> {
        &self.granted
    }

    /// Hands the service the section's memory as an XPC shared-memory object: the view that can never be written
    /// for a read grant.
    pub(super) fn grant(
        &self,
        section: &super::SectionInner,
        access: crate::Access,
    ) -> Result<crate::Granted, SandboxError> {
        let wire = self.next_wire.fetch_add(1, Ordering::SeqCst);
        let writable = access == crate::Access::ReadWrite;
        let region = if writable { section.view } else { section.sealed_view()? };
        let len = section.len as u64;
        self.link
            .grant(wire, len, writable, region, span(section.len))
            .map_err(|code| SandboxError::Os {
                step: "grant",
                code: code as u32,
            })?;
        Ok(crate::Granted { wire, len })
    }
}

impl Drop for ServiceChild {
    fn drop(&mut self) {
        let _ = self.kill();
        // Gone before the service is free again: the next document's connection reaches a new process. One that is
        // not seen to end in time is handed to the slot (before the guard below frees it), and the next start waits
        // for it before it connects.
        if matches!(self.wait(self.reap), Ok(None)) {
            let link = std::mem::replace(&mut self.link, Box::new(Ended));
            *lock(&self._slot.0.lingering) = Some(link);
        }
    }
}

/// What a [`ServiceChild`] holds once its link went to the slot: a process that is already let go.
#[derive(Debug)]
struct Ended;

impl Link for Ended {
    fn start(&self, _: &OwnedFd, _: &OwnedFd, _: u64, _: Duration) -> Option<(Status, c_int)> {
        None
    }
    fn facts(&self, pid: c_int, status: Option<Status>) -> ServiceFacts {
        read_service_facts(pid, status)
    }
    fn grant(&self, _: u64, _: u64, _: bool, _: *mut u8, _: usize) -> Result<(), i32> {
        Err(sys::EBADMSG)
    }
    fn kill(&self) {}
    fn ended_within(&self, _: Duration) -> bool {
        true
    }
}

/// Starts the helper behind `link` and checks it: the caller gets its pipes only when [`judge_service`] accepts what
/// the kernel and its signature say. Any failure on the way ends the service (the returned error drops the child).
pub(super) fn start_service(
    link: Box<dyn Link>,
    slot: SlotGuard,
    limits: Limits,
    bounds: Bounds,
) -> Result<(ServiceChild, Pipes), SandboxError> {
    // The app's launch lock, for as long as the helper's ends of the pipes exist in the app.
    let launch = crate::launch_lock();
    let (helper_stdin, to_helper) = pipe()?;
    let (from_helper, helper_stdout) = pipe()?;
    let asked = Instant::now();
    let answer = link.start(&helper_stdin, &helper_stdout, limits.memory_bytes, bounds.handshake);
    let waited = asked.elapsed();
    drop(helper_stdin);
    drop(helper_stdout);
    drop(launch);
    // How long the system took to start the service and the service to answer: what tells a slow start from a
    // refusal in the next report (dm-84).
    note(&match answer {
        Some((_, pid)) => format!(
            "office helper: XPC service pid {pid} answered in {:.1} s",
            waited.as_secs_f32()
        ),
        None => format!(
            "office helper: no handshake from the XPC service after {:.1} s (the bound is {} s)",
            waited.as_secs_f32(),
            bounds.handshake.as_secs()
        ),
    });
    let child = ServiceChild {
        link,
        pid: answer.map_or(0, |(_, pid)| pid),
        reap: bounds.reap,
        ended: Mutex::new(None),
        killed: AtomicBool::new(false),
        next_wire: AtomicU64::new(1),
        granted: Arc::new(AtomicU64::new(0)),
        _slot: slot,
    };
    // The service has been sent its two pipe ends and its memory limit, and nothing on them. What it says is checked
    // against what the kernel and its signature say; only then does the caller get the pipes.
    let status = answer.map(|(status, _)| status);
    let mut facts = child.link.facts(child.pid, status);
    let mut judged = judge_service(&facts);
    // A service that says it is confined and is refused on what the kernel or its signature say is asked once more,
    // a moment later, with every check again: never fewer.
    if judged.is_err()
        && status.is_some_and(|status| status.state == STATE_APP_SANDBOX || status.state == STATE_CONFINED)
    {
        note(&confinement_line(child.pid, &facts, judged));
        std::thread::sleep(bounds.recheck);
        facts = child.link.facts(child.pid, status);
        judged = judge_service(&facts);
    }
    note(&confinement_line(child.pid, &facts, judged));
    if judged.is_ok() {
        note(&confined_line(&facts));
    }
    judged?;
    Ok((
        child,
        Pipes {
            to_helper: std::fs::File::from(to_helper),
            from_helper: std::fs::File::from(from_helper),
        },
    ))
}

/// The line the app writes when it has checked a service: which confinement it runs under and what was checked, or
/// why it was refused. Counts and fixed text only.
pub(super) fn confinement_line(pid: c_int, service: &ServiceFacts, judged: Result<(), SandboxError>) -> String {
    let confinement = match service.facts.status.map(|status| status.state) {
        Some(STATE_CONFINED) => "the App Sandbox and the deny-default profile",
        Some(STATE_APP_SANDBOX) => "the App Sandbox alone (the system refused the deny-default profile on top)",
        _ => "none it could name",
    };
    let checked = format!(
        "kernel asked from the app: {}/{} profile operations denied, {}/{} App Sandbox checks denied, {}/{} allowed; \
         signature valid: {}, entitlements: {}",
        service.facts.denied,
        MUST_BE_DENIED.len(),
        service.app_denied,
        service.app_asked,
        service.facts.allowed,
        MUST_BE_ALLOWED.len(),
        service.signature_valid,
        match service.entitlements {
            Some((count, sandbox)) => format!("{count} (app-sandbox: {sandbox})"),
            None => "unread".into(),
        },
    );
    match judged {
        Ok(()) => format!("office helper: XPC service pid {pid} accepted, confined by {confinement}; {checked}"),
        Err(error) => format!(
            "office helper: XPC service pid {pid} refused, failed check: {} ({error}), it is ended; {checked}",
            failed_check(service, error)
        ),
    }
}

/// The name of the check that refused a service, as the log says it: the judge's reason, and for the App Sandbox's
/// own list the first operation the kernel did not deny.
pub(super) fn failed_check(service: &ServiceFacts, error: SandboxError) -> String {
    let reason = match error {
        SandboxError::NotConfined(reason) => reason,
        _ => "not a refusal",
    };
    match (reason, service.first_allowed) {
        ("the kernel allows what the App Sandbox denies", Some(index)) => {
            format!("app-sandbox {}", app_check_name(index))
        }
        ("the kernel allows what the App Sandbox denies", None) => {
            String::from("app-sandbox (the user's home unnamed)")
        }
        ("no handshake", _) => String::from("handshake (no answer)"),
        _ => reason.to_string(),
    }
}

/// The one line that says a service runs confined, in the words the app's log is searched for (dm-84):
/// "office helper: confined (app sandbox, 19/19 checks)".
pub(super) fn confined_line(service: &ServiceFacts) -> String {
    match service.facts.status.map(|status| status.state) {
        Some(STATE_CONFINED) => format!(
            "office helper: confined (app sandbox and profile, {}/{} checks)",
            service.facts.denied,
            MUST_BE_DENIED.len()
        ),
        _ => format!(
            "office helper: confined (app sandbox, {}/{} checks)",
            service.app_denied, service.app_asked
        ),
    }
}

/// Where the app's own log takes a line of this module (dm-84): set once by the app
/// ([`beebeeb_office_sandbox_set_note_hook`]). Without it a line goes to the system log and standard error only.
static NOTE_HOOK: OnceLock<extern "C" fn(*const c_char)> = OnceLock::new();

/// The app hands this module the function that writes a line to its own log. Exported by name because the crate's
/// public surface is the same on every system and this is a Mac's alone. The line is a terminated string of fixed
/// text and numbers, alive for the call only. The first hook stays.
#[unsafe(no_mangle)]
pub extern "C" fn beebeeb_office_sandbox_set_note_hook(hook: extern "C" fn(*const c_char)) {
    let _ = NOTE_HOOK.set(hook);
}

/// Writes `line` to the system log, standard error and the app's own log: what the log shows of a service's start
/// and confinement.
fn note(line: &str) {
    eprintln!("{line}");
    let Ok(line) = CString::new(line) else {
        return;
    };
    if let Some(hook) = NOTE_HOOK.get() {
        hook(line.as_ptr());
    }
    #[cfg(not(test))]
    {
        unsafe extern "C" {
            fn syslog(priority: c_int, format: *const c_char, ...);
        }
        // SAFETY: a constant format with one string argument, both terminated, alive for the call. 5 is LOG_NOTICE.
        unsafe { syslog(5, c"%s".as_ptr(), line.as_ptr()) };
    }
}

/// Starts the app's XPC service `name` as the helper. `launch` is released at once: nothing a process could inherit
/// exists yet, and the wait for the service can be long.
pub(super) fn spawn_service(
    name: &CStr,
    limits: Limits,
    mode: Mode,
    launch: crate::LaunchGuard,
) -> Result<(ServiceChild, Pipes), SandboxError> {
    drop(launch);
    if mode != Mode::Plain {
        return Err(SandboxError::NotConfined(
            "a helper that is an XPC service takes no argument, so this mode has no confined start on macOS",
        ));
    }
    let Some(slot) = SLOT.take(SERVICE_WAIT) else {
        return Err(SandboxError::Os {
            step: "service busy",
            code: EBUSY,
        });
    };
    // The process of the document before is gone first, so this connection reaches a new one.
    SLOT.wait_for_previous(PREVIOUS_EXIT_BOUND);
    start_service(Box::new(XpcLink::connect(name)?), slot, limits, Bounds::SERVICE)
}

unsafe extern "C" fn ignore_event(_block: *const Block, _event: XpcObject) {}

/// The app's handler of its connection: nothing arrives on it but the notice that the service is gone, which the
/// kernel's own notice ([`Link::ended_within`]) already tells.
static IGNORE: Block = Block::new(ignore_event);

/// An XPC object kept by one owner and released once.
struct Held(XpcObject);

// SAFETY: XPC objects may be used and released from any thread.
unsafe impl Send for Held {}

impl Drop for Held {
    fn drop(&mut self) {
        // SAFETY: the one reference this value holds.
        unsafe { xpc_release(self.0) };
    }
}

/// The app's connection to its service, and the kernel's notice of the service's exit.
struct XpcLink {
    connection: XpcObject,
    /// A kernel queue with the exit of the service's process on it, once the process is known.
    exit: Mutex<Option<OwnedFd>>,
    pid: AtomicI32,
    gone: AtomicBool,
}

// SAFETY: a connection may be used from any thread; the rest is locked or atomic.
unsafe impl Send for XpcLink {}
// SAFETY: as above.
unsafe impl Sync for XpcLink {}

impl fmt::Debug for XpcLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("XpcLink")
            .field("pid", &self.pid.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl XpcLink {
    fn connect(name: &CStr) -> Result<Self, SandboxError> {
        // SAFETY: a terminated name; a null queue is the system's default. The connection is released in `drop`.
        let connection = unsafe { xpc_connection_create(name.as_ptr(), null_mut()) };
        if connection.is_null() {
            return Err(SandboxError::Os {
                step: "service connection",
                code: 0,
            });
        }
        // SAFETY: the new connection; the handler is a constant block; a connection must be given one before it is
        // resumed.
        unsafe {
            xpc_connection_set_event_handler(connection, &raw const IGNORE);
            xpc_connection_resume(connection);
        }
        Ok(Self {
            connection,
            exit: Mutex::new(None),
            pid: AtomicI32::new(0),
            gone: AtomicBool::new(false),
        })
    }

    /// Registers the kernel's notice of the exit of `pid`. A process that is already gone is noted as such.
    fn watch_exit(&self, pid: c_int) {
        // SAFETY: takes no pointer.
        let queue = unsafe { sys::kqueue() };
        if queue == -1 {
            return;
        }
        let queue = owned(queue);
        let change = sys::kevent {
            ident: pid as usize,
            filter: sys::EVFILT_PROC,
            flags: sys::EV_ADD,
            fflags: sys::NOTE_EXIT,
            data: 0,
            udata: null_mut(),
        };
        // SAFETY: one valid change, the count passed; no event is asked for, so nothing is written.
        if unsafe { sys::kevent(queue.as_raw_fd(), &raw const change, 1, null_mut(), 0, null()) } == -1 {
            if errno() == sys::ESRCH {
                self.gone.store(true, Ordering::SeqCst);
            }
            return;
        }
        *lock(&self.exit) = Some(queue);
    }
}

impl Link for XpcLink {
    fn start(&self, stdin: &OwnedFd, stdout: &OwnedFd, memory_bytes: u64, bound: Duration) -> Option<(Status, c_int)> {
        // SAFETY: a new empty dictionary; each key is a terminated constant; the two descriptors are open, and the
        // message takes its own reference to each. The connection gets one more reference for the thread below.
        let (connection, message) = unsafe {
            let message = xpc_dictionary_create(null(), null(), 0);
            if message.is_null() {
                return None;
            }
            xpc_dictionary_set_uint64(message, KEY_MEMORY.as_ptr(), memory_bytes);
            xpc_dictionary_set_fd(message, KEY_STDIN.as_ptr(), stdin.as_raw_fd());
            xpc_dictionary_set_fd(message, KEY_STDOUT.as_ptr(), stdout.as_raw_fd());
            (Held(xpc_retain(self.connection)), Held(message))
        };
        // The system's call for a message and its answer has no time limit, so it waits on a thread of its own: a
        // service that never answers is ended below, which ends the call.
        let (answered, answer) = std::sync::mpsc::channel();
        let asked = std::thread::Builder::new()
            .name("office-service-start".into())
            .spawn(move || {
                // The two holders themselves move to this thread, not the addresses inside them.
                let (connection, message) = (connection, message);
                // SAFETY: both objects are held by this thread until it ends; the answer is released once. The two
                // error objects are the system's constants, compared by address only.
                let status = unsafe {
                    let reply = Held(xpc_connection_send_message_with_reply_sync(connection.0, message.0));
                    if reply.0.is_null() {
                        Err("no reply object")
                    } else if xpc_get_type(reply.0) == &raw const _xpc_type_dictionary {
                        Ok(Status {
                            state: xpc_dictionary_get_uint64(reply.0, KEY_STATE.as_ptr()) as u32,
                            profile_version: xpc_dictionary_get_uint64(reply.0, KEY_PROFILE.as_ptr()) as u32,
                        })
                    } else if reply.0.cast_const() == &raw const _xpc_error_connection_interrupted {
                        Err("the connection was interrupted (the service's process ended before it answered)")
                    } else if reply.0.cast_const() == &raw const _xpc_error_connection_invalid {
                        Err("the connection is invalid (the system did not start the service, or the service turned it away)")
                    } else if xpc_get_type(reply.0) == &raw const _xpc_type_error {
                        Err("another connection error")
                    } else {
                        Err("a reply that is no dictionary")
                    }
                };
                let _ = answered.send(status);
            });
        let status = match asked.map(|_| answer.recv_timeout(bound)) {
            Ok(Ok(Ok(status))) => Some(status),
            Ok(Ok(Err(why))) => {
                note(&format!("office helper: the XPC service gave no handshake: {why}"));
                None
            }
            Ok(Err(_)) => {
                note(&format!(
                    "office helper: the XPC service gave no handshake within {} s (the handshake timed out)",
                    bound.as_secs()
                ));
                None
            }
            Err(_) => None,
        };
        // SAFETY: the connection is alive until `drop`. Zero until the service has answered once.
        let pid = unsafe { xpc_connection_get_pid(self.connection) };
        if pid > 0 {
            self.pid.store(pid, Ordering::SeqCst);
            self.watch_exit(pid);
        }
        let Some(status) = status else {
            self.kill();
            return None;
        };
        (pid > 0).then_some((status, pid))
    }

    fn facts(&self, pid: c_int, status: Option<Status>) -> ServiceFacts {
        read_service_facts(pid, status)
    }

    fn grant(&self, wire: u64, len: u64, writable: bool, region: *mut u8, span: usize) -> Result<(), i32> {
        // SAFETY: `region` is mapped for `span` bytes by the section that asks, for as long as it lives; the object
        // made from it carries no more access than that mapping allows. The message holds the object; both
        // references made here are released once.
        unsafe {
            let memory = xpc_shmem_create(region.cast(), span);
            if memory.is_null() {
                return Err(sys::EBADMSG);
            }
            let memory = Held(memory);
            let message = xpc_dictionary_create(null(), null(), 0);
            if message.is_null() {
                return Err(sys::EBADMSG);
            }
            let message = Held(message);
            xpc_dictionary_set_uint64(message.0, KEY_GRANT.as_ptr(), wire);
            xpc_dictionary_set_uint64(message.0, KEY_MEMORY.as_ptr(), len);
            xpc_dictionary_set_uint64(message.0, KEY_WRITE.as_ptr(), u64::from(writable));
            xpc_dictionary_set_value(message.0, KEY_REGION.as_ptr(), memory.0);
            xpc_connection_send_message(self.connection, message.0);
        }
        Ok(())
    }

    fn kill(&self) {
        // SAFETY: the connection is alive until `drop`. The signal goes to the service's process through the system
        // (the app itself may not signal it, measured); cancelling afterwards is what a service that still runs as
        // written ends itself on.
        unsafe {
            xpc_connection_kill(self.connection, sys::SIGKILL);
            xpc_connection_cancel(self.connection);
        }
    }

    fn ended_within(&self, bound: Duration) -> bool {
        if self.gone.load(Ordering::SeqCst) {
            return true;
        }
        let exit = lock(&self.exit);
        // No process was ever known (the service did not answer): there is nothing to wait for.
        let Some(queue) = exit.as_ref() else {
            return true;
        };
        let wait = Timespec {
            seconds: bound.as_secs().min(i64::MAX as u64) as i64,
            nanoseconds: i64::from(bound.subsec_nanos()),
        };
        let mut event = sys::kevent {
            ident: 0,
            filter: 0,
            flags: 0,
            fflags: 0,
            data: 0,
            udata: null_mut(),
        };
        // SAFETY: `event` is a valid place for one event, the count passed; `wait` is a valid time.
        let got = unsafe {
            sys::kevent(
                queue.as_raw_fd(),
                null(),
                0,
                &raw mut event,
                1,
                (&raw const wait).cast(),
            )
        };
        if got == 1 {
            self.gone.store(true, Ordering::SeqCst);
        }
        got == 1
    }
}

impl Drop for XpcLink {
    fn drop(&mut self) {
        // SAFETY: the connection made in `connect`, cancelled (fine a second time) and released once.
        unsafe {
            xpc_connection_cancel(self.connection);
            xpc_release(self.connection);
        }
    }
}

/// `struct timespec`.
#[repr(C)]
struct Timespec {
    seconds: i64,
    nanoseconds: i64,
}

/// An XPC shared-memory object of the `span` bytes at `region`: what a grant carries. The caller releases it, or
/// hands it to the service's side, which does.
#[cfg(test)]
pub(super) fn shared_object(region: *mut u8, span: usize) -> XpcObject {
    // SAFETY: `region` is mapped for `span` bytes by the calling test.
    let memory = unsafe { xpc_shmem_create(region.cast(), span) };
    // SAFETY: the object just made.
    assert!(unsafe { xpc_get_type(memory) } == &raw const _xpc_type_shmem);
    memory
}

#[cfg(test)]
#[path = "macos_xpc_tests.rs"]
mod tests;
