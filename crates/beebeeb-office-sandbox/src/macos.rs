//! macOS: the helper confines itself with a deny-default Seatbelt profile, and the app asks the kernel whether it did.
//!
//! | Layer | What it takes away |
//! |---|---|
//! | Seatbelt profile [`PROFILE`], applied by the helper itself before it reads its first byte of input ([`confine_or_exit`]) | the network, every file but the two font folders (read-only), every write, starting a process, every Mach service, every named shared-memory object |
//! | `posix_spawn` with `POSIX_SPAWN_CLOEXEC_DEFAULT`, an empty environment | every descriptor of the app except the two pipe ends, `/dev/null` as standard error and the control socket; every variable of the app |
//! | Kernel read-back ([`read_facts`], judged by [`judge`]) before the app hands out the pipes | a helper that says it is confined and is not |
//! | Memory watch ([`Watch`]): the helper's physical footprint, every 100 ms | unbounded memory |
//! | Resource limits set by the helper before the profile (a second line, see Memory) | core files, file growth, new processes, address space past its memory limit and two full sections |
//! | A kernel event on the app's exit, registered by the helper before the profile | outliving the app |
//!
//! **How the helper is confined.** macOS has no way to create a process inside a sandbox of the creator's choosing
//! (what an AppContainer token is on Windows), so the helper applies the profile to itself, first thing in `main`, the
//! way Chromium's helpers do (`Seatbelt::Init` in sandbox/mac/seatbelt.cc,
//! <https://chromium.googlesource.com/chromium/src/+/main/sandbox/mac/seatbelt.cc>; the profile language and the
//! deny-default shape as in sandbox/policy/mac/common.sb of the same tree). The call is
//! `sandbox_init_with_parameters`, which Apple does not document; its behaviour here is measured (macOS 26.6, Apple
//! silicon), not taken from a header. A profile, once applied, cannot be changed or removed by the process: a second
//! `sandbox_init` fails with "Operation not permitted" (measured; Apple's wording, "A process is not allowed to change
//! its sandbox", <https://developer.apple.com/forums/thread/706390>).
//!
//! **How the app knows.** The helper's word is not the proof. After its handshake on the control socket the app asks
//! the kernel about the helper's process with `sandbox_check(pid, operation, ...)`: that it has a sandbox at all, that
//! each operation in [`MUST_BE_DENIED`] is denied, and that the one in [`MUST_BE_ALLOWED`] is allowed (so an answer of
//! "denied" to everything, as for a process that is gone, is not taken for confinement). Only then are the pipes
//! handed out. A helper that sends the handshake without having applied the profile is ended; a test does exactly
//! that.
//!
//! **An app that is itself sandboxed cannot use this.** A process started by an App Sandbox app inherits the app's
//! sandbox ("the child process always inherits its sandbox from the parent", the same forum thread; the two
//! entitlements of such a child, `com.apple.security.app-sandbox` and `com.apple.security.inherit`, are described in
//! <https://developer.apple.com/library/archive/documentation/Miscellaneous/Reference/EntitlementKeyReference/Chapters/EnablingAppSandbox.html>),
//! and a process that has a sandbox cannot apply another. The helper would then hold what the app holds (its network
//! entitlements, its container), which is far more than this crate promises. So [`spawn`] starts nothing when the
//! app has a sandbox, and a helper whose profile is refused ends itself: fail closed, never the inherited sandbox.
//! The same Apple page names the way out: an XPC service, which the system starts outside the app's sandbox.
//!
//! **Shared memory.** A [`crate::Section`] is a POSIX shared-memory object made under a random name that is removed
//! at once, so nothing can open it by name; the app keeps two descriptors of it, one opened read-write and one opened
//! read-only. A grant sends one of them over the control socket (`SCM_RIGHTS`): the read-only one for
//! [`Access::Read`]. That is the seal: a mapping of the read-only descriptor cannot be made writable, the kernel
//! refuses a writable mapping of it (system code 1), refuses to re-protect a read-only mapping (13), and a write
//! through one is a bus error (all three measured, and held by tests). Mapping a descriptor the helper already holds
//! needs no rule in the profile (measured), so the profile allows no shared-memory operation and the helper can open
//! no object by name. Dropping a section writes zeros over all of it before it is unmapped.
//!
//! **Memory.** `RLIMIT_AS` and `RLIMIT_DATA` take only a value at or above the address space the process already has
//! (about 415 GiB on Apple silicon, measured), bound only new address space, and the allocator serves several GiB from
//! ranges it reserved earlier (measured). So they are a second line (one allocation of many GiB fails inside the
//! helper: its limit plus room for two full sections); the bound that holds is the watch, which ends the helper once its footprint passes its limit plus the bytes of
//! the sections granted to it. Known residual, as on Windows: a helper can touch memory faster than one poll.
//!
//! **Gone with the app.** The app ends the helper when its `Child` is dropped. If the app dies first, the helper's
//! input pipe ends, and a thread the helper started before it confined itself ends the process on the kernel's
//! notice of the app's exit. Unlike a Windows job this second part runs inside the helper: a helper whose code no
//! longer runs as written could outlive the app, still confined.
//!
//! This is the only module with `unsafe` on macOS. Every block is one call into the system, or one access to memory
//! the system mapped, with its invariant stated.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::{
    Access, DEFAULT_MEMORY_LIMIT_BYTES, ExitStatus, Granted, KILLED_EXIT_CODE, LaunchGuard, Limits, MAX_SECTION_BYTES,
    Pipes, Profile, SandboxError, over_memory,
};

/// The helper's whole confinement, in the system's profile language. Everything not named is denied.
///
/// - The two font folders, read-only: the Office engine reads the system's fonts, as the Windows helper reads the
///   system font folder. Nothing of the user is under either.
/// - The metadata of the folders above them, so those paths can be walked.
/// - Five numbers about the machine (processor counts, page size).
/// - A signal to itself (how `abort` ends the process).
///
/// The helper's own executable and the system libraries are not named: they are mapped before the profile is applied,
/// and a mapped image needs no rule (measured).
pub(crate) const PROFILE: &str = r#"(version 1)
(deny default)
(allow file-read* (subpath "/System/Library/Fonts") (subpath "/Library/Fonts"))
(allow file-read-metadata (literal "/") (literal "/System") (literal "/System/Library") (literal "/Library"))
(allow sysctl-read (sysctl-name "hw.ncpu") (sysctl-name "hw.activecpu") (sysctl-name "hw.logicalcpu") (sysctl-name "hw.physicalcpu") (sysctl-name "hw.pagesize"))
(allow signal (target self))
"#;

/// The number the helper's handshake carries for [`PROFILE`]. Raised with every change of the text; a test pins the
/// text's fingerprint to it.
pub(crate) const PROFILE_VERSION: u32 = 1;

/// The exit code of a helper that could not confine itself ([`confine_or_exit`]).
pub(crate) const UNCONFINED_EXIT_CODE: u8 = 78;

/// The exit code of a helper that ended itself because the app was gone.
const PARENT_GONE_EXIT_CODE: c_int = 79;

/// Added to the signal's number: the exit code of a helper that a signal ended, other than the app's own kill.
const SIGNALLED: u32 = 0x0001_0000;

/// The exit code of a helper whose end the system did not report (another part of the app reaped it).
const UNKNOWN_EXIT_CODE: u32 = 0x0002_0000;

/// The descriptor number of the control socket in the helper.
const CONTROL_FD: RawFd = 3;

/// How long the app waits for a new helper's handshake.
const HANDSHAKE_BOUND: Duration = Duration::from_secs(10);

/// How long the helper waits for the app's first message.
const HELLO_BOUND_MS: c_int = 10_000;

/// How long a grant waits for room in the control socket before it is refused.
const GRANT_BOUND: Duration = Duration::from_secs(1);

/// How long dropping a `Child` waits for the killed helper to be gone.
const REAP_BOUND: Duration = Duration::from_secs(5);

/// How often a helper's footprint is read.
const POLL: Duration = Duration::from_millis(100);

/// Room for grants the helper has not mapped yet, in bytes of the control socket.
const CONTROL_BUFFER_BYTES: c_int = 256 * 1024;

const HELLO_MAGIC: &[u8; 8] = b"BBSBHEL1";
const STATUS_MAGIC: &[u8; 8] = b"BBSBCNF1";
const GRANT_MAGIC: &[u8; 8] = b"BBSBGRT1";
const HELLO_LEN: usize = 16;
const STATUS_LEN: usize = 24;
const GRANT_LEN: usize = 32;

/// The handshake's states.
const STATE_CONFINED: u32 = 1;
const STATE_REFUSED: u32 = 2;

/// The declarations of the system calls used, by hand: the crate links nothing but the system library (the rule in
/// `Cargo.toml`), and four other crates' lockfiles name this one. Layouts and numbers are those of the macOS SDK
/// (`sys/socket.h`, `sys/fcntl.h`, `sys/mman.h`, `spawn.h`, `sys/resource.h`, `sys/event.h`, `libproc.h`).
#[allow(non_camel_case_types, dead_code)]
mod sys {
    use std::ffi::{c_char, c_int, c_void};

    pub const O_RDONLY: c_int = 0;
    pub const O_WRONLY: c_int = 1;
    pub const O_RDWR: c_int = 2;
    pub const O_CREAT: c_int = 0x200;
    pub const O_EXCL: c_int = 0x800;
    pub const F_GETFD: c_int = 1;
    pub const F_SETFD: c_int = 2;
    pub const F_DUPFD_CLOEXEC: c_int = 67;
    pub const FD_CLOEXEC: c_int = 1;
    pub const AF_UNIX: c_int = 1;
    pub const SOCK_DGRAM: c_int = 2;
    pub const SOL_SOCKET: c_int = 0xffff;
    pub const SO_SNDBUF: c_int = 0x1001;
    pub const SO_RCVBUF: c_int = 0x1002;
    pub const SO_TYPE: c_int = 0x1008;
    pub const SO_NOSIGPIPE: c_int = 0x1022;
    pub const SCM_RIGHTS: c_int = 1;
    pub const MSG_TRUNC: c_int = 0x10;
    pub const MSG_CTRUNC: c_int = 0x20;
    pub const MSG_DONTWAIT: c_int = 0x80;
    pub const POLLIN: i16 = 1;
    pub const PROT_READ: c_int = 1;
    pub const PROT_WRITE: c_int = 2;
    pub const MAP_SHARED: c_int = 1;
    pub const POSIX_SPAWN_SETSIGDEF: i16 = 0x04;
    pub const POSIX_SPAWN_SETSIGMASK: i16 = 0x08;
    pub const POSIX_SPAWN_CLOEXEC_DEFAULT: i16 = 0x4000;
    pub const SIGKILL: c_int = 9;
    pub const WNOHANG: c_int = 1;
    pub const RLIMIT_FSIZE: c_int = 1;
    pub const RLIMIT_DATA: c_int = 2;
    pub const RLIMIT_CORE: c_int = 4;
    pub const RLIMIT_AS: c_int = 5;
    pub const RLIMIT_NPROC: c_int = 7;
    pub const EVFILT_PROC: i16 = -5;
    pub const EV_ADD: u16 = 1;
    pub const NOTE_EXIT: u32 = 0x8000_0000;
    pub const RUSAGE_INFO_V0: c_int = 0;
    pub const PROC_PIDLISTFDS: c_int = 1;
    pub const TASK_VM_INFO: u32 = 22;
    pub const EINTR: i32 = 4;
    pub const ECHILD: i32 = 10;
    pub const EAGAIN: i32 = 35;
    pub const ENOBUFS: i32 = 55;
    pub const EEXIST: i32 = 17;
    pub const ESRCH: i32 = 3;
    pub const ENOENT: i32 = 2;
    pub const EBADMSG: i32 = 94;
    /// `sandbox_check`'s filter types (the private `sandbox.h`, as Chromium's seatbelt.cc declares them).
    pub const SANDBOX_FILTER_NONE: c_int = 0;
    pub const SANDBOX_FILTER_PATH: c_int = 1;
    pub const SANDBOX_FILTER_GLOBAL_NAME: c_int = 2;

    #[repr(C)]
    pub struct iovec {
        pub base: *mut c_void,
        pub len: usize,
    }

    #[repr(C)]
    pub struct msghdr {
        pub name: *mut c_void,
        pub name_len: u32,
        pub iov: *mut iovec,
        pub iov_len: c_int,
        pub control: *mut c_void,
        pub control_len: u32,
        pub flags: c_int,
    }

    /// One control message that carries one descriptor: the header (12 bytes) and the descriptor right after it
    /// (`CMSG_DATA` of a 12-byte header is at 12; `CMSG_LEN(4)` and `CMSG_SPACE(4)` are both 16).
    #[repr(C)]
    pub struct one_descriptor {
        pub len: u32,
        pub level: c_int,
        pub kind: c_int,
        pub descriptor: c_int,
    }

    #[repr(C)]
    pub struct pollfd {
        pub fd: c_int,
        pub events: i16,
        pub revents: i16,
    }

    #[repr(C)]
    pub struct rlimit {
        pub current: u64,
        pub maximum: u64,
    }

    #[repr(C)]
    pub struct kevent {
        pub ident: usize,
        pub filter: i16,
        pub flags: u16,
        pub fflags: u32,
        pub data: isize,
        pub udata: *mut c_void,
    }

    /// `struct rusage_info_v0`.
    #[repr(C)]
    #[derive(Default)]
    pub struct rusage_info_v0 {
        pub uuid: [u8; 16],
        pub user_time: u64,
        pub system_time: u64,
        pub pkg_idle_wkups: u64,
        pub interrupt_wkups: u64,
        pub pageins: u64,
        pub wired_size: u64,
        pub resident_size: u64,
        pub phys_footprint: u64,
        pub proc_start_abstime: u64,
        pub proc_exit_abstime: u64,
    }

    /// `struct proc_fdinfo`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct proc_fdinfo {
        pub fd: i32,
        pub kind: u32,
    }

    unsafe extern "C" {
        pub fn pipe(fds: *mut c_int) -> c_int;
        pub fn socketpair(domain: c_int, kind: c_int, protocol: c_int, fds: *mut c_int) -> c_int;
        pub fn fcntl(fd: c_int, command: c_int, ...) -> c_int;
        pub fn setsockopt(fd: c_int, level: c_int, name: c_int, value: *const c_void, len: u32) -> c_int;
        pub fn getsockopt(fd: c_int, level: c_int, name: c_int, value: *mut c_void, len: *mut u32) -> c_int;
        pub fn sendmsg(fd: c_int, message: *const msghdr, flags: c_int) -> isize;
        pub fn recvmsg(fd: c_int, message: *mut msghdr, flags: c_int) -> isize;
        pub fn poll(fds: *mut pollfd, count: u32, timeout_ms: c_int) -> c_int;
        pub fn shm_open(name: *const c_char, flags: c_int, ...) -> c_int;
        pub fn shm_unlink(name: *const c_char) -> c_int;
        pub fn ftruncate(fd: c_int, len: i64) -> c_int;
        pub fn mmap(
            address: *mut c_void,
            len: usize,
            protection: c_int,
            flags: c_int,
            fd: c_int,
            offset: i64,
        ) -> *mut c_void;
        pub fn munmap(address: *mut c_void, len: usize) -> c_int;
        pub fn getentropy(buffer: *mut c_void, len: usize) -> c_int;
        pub fn posix_spawn(
            pid: *mut c_int,
            path: *const c_char,
            actions: *const *mut c_void,
            attributes: *const *mut c_void,
            arguments: *const *mut c_char,
            environment: *const *mut c_char,
        ) -> c_int;
        pub fn posix_spawnattr_init(attributes: *mut *mut c_void) -> c_int;
        pub fn posix_spawnattr_destroy(attributes: *mut *mut c_void) -> c_int;
        pub fn posix_spawnattr_setflags(attributes: *mut *mut c_void, flags: i16) -> c_int;
        pub fn posix_spawnattr_setsigdefault(attributes: *mut *mut c_void, signals: *const u32) -> c_int;
        pub fn posix_spawnattr_setsigmask(attributes: *mut *mut c_void, signals: *const u32) -> c_int;
        pub fn posix_spawn_file_actions_init(actions: *mut *mut c_void) -> c_int;
        pub fn posix_spawn_file_actions_destroy(actions: *mut *mut c_void) -> c_int;
        pub fn posix_spawn_file_actions_adddup2(actions: *mut *mut c_void, fd: c_int, new_fd: c_int) -> c_int;
        pub fn posix_spawn_file_actions_addopen(
            actions: *mut *mut c_void,
            fd: c_int,
            path: *const c_char,
            flags: c_int,
            mode: u16,
        ) -> c_int;
        pub fn kill(pid: c_int, signal: c_int) -> c_int;
        pub fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
        pub fn getpid() -> c_int;
        pub fn getppid() -> c_int;
        pub fn _exit(code: c_int) -> !;
        pub fn setrlimit(resource: c_int, limit: *const rlimit) -> c_int;
        pub fn kqueue() -> c_int;
        pub fn kevent(
            queue: c_int,
            changes: *const kevent,
            change_count: c_int,
            events: *mut kevent,
            event_count: c_int,
            timeout: *const c_void,
        ) -> c_int;
        pub fn proc_pid_rusage(pid: c_int, flavor: c_int, buffer: *mut c_void) -> c_int;
        pub fn proc_pidinfo(pid: c_int, flavor: c_int, argument: u64, buffer: *mut c_void, size: c_int) -> c_int;
        pub static mach_task_self_: u32;
        pub fn task_info(task: u32, flavor: u32, info: *mut u32, count: *mut u32) -> c_int;

        // The sandbox library's calls that Apple ships without a public header (libsystem_sandbox, part of the
        // system library). Declared as Chromium's sandbox/mac/seatbelt.cc declares them.
        pub fn sandbox_init_with_parameters(
            profile: *const c_char,
            flags: u64,
            parameters: *const *const c_char,
            error: *mut *mut c_char,
        ) -> c_int;
        pub fn sandbox_free_error(error: *mut c_char);
        pub fn sandbox_check(pid: c_int, operation: *const c_char, filter: c_int, ...) -> c_int;
        /// Added to a filter type: the check itself is not written to the system log.
        pub static SANDBOX_CHECK_NO_REPORT: c_int;
    }
}

/// The calling thread's last system error code.
fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn os(step: &'static str) -> SandboxError {
    SandboxError::Os {
        step,
        code: errno() as u32,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Takes ownership of a descriptor the system just returned.
fn owned(fd: c_int) -> OwnedFd {
    // SAFETY: every caller passes a descriptor that a system call returned as open and that nothing else owns; it is
    // closed exactly once, by the returned value.
    unsafe { OwnedFd::from_raw_fd(fd) }
}

/// Marks a descriptor close-on-exec: no process the app starts keeps it.
fn close_on_exec(fd: &OwnedFd, step: &'static str) -> Result<(), SandboxError> {
    // SAFETY: `fd` is open; the command takes one integer.
    if unsafe { sys::fcntl(fd.as_raw_fd(), sys::F_SETFD, sys::FD_CLOEXEC) } == -1 {
        return Err(os(step));
    }
    Ok(())
}

/// A close-on-exec duplicate of `fd` numbered 10 or higher, so no file action of the spawn can overwrite it.
fn lifted(fd: OwnedFd, step: &'static str) -> Result<OwnedFd, SandboxError> {
    // SAFETY: `fd` is open; the command takes the lowest number wanted.
    let high = unsafe { sys::fcntl(fd.as_raw_fd(), sys::F_DUPFD_CLOEXEC, 10 as c_int) };
    if high == -1 {
        return Err(os(step));
    }
    Ok(owned(high))
}

/// An unnamed pipe: (read end, write end), both close-on-exec.
fn pipe() -> Result<(OwnedFd, OwnedFd), SandboxError> {
    let mut ends = [-1 as c_int; 2];
    // SAFETY: `ends` is a valid place for two descriptors.
    if unsafe { sys::pipe(ends.as_mut_ptr()) } == -1 {
        return Err(os("pipe"));
    }
    let (read, write) = (owned(ends[0]), owned(ends[1]));
    close_on_exec(&read, "pipe")?;
    close_on_exec(&write, "pipe")?;
    Ok((read, write))
}

/// The control socket: a connected pair of datagram sockets, (the app's end, the helper's end), both close-on-exec.
/// One message is one datagram, so a grant and its descriptor never come apart.
fn control_pair() -> Result<(OwnedFd, OwnedFd), SandboxError> {
    let mut ends = [-1 as c_int; 2];
    // SAFETY: `ends` is a valid place for two descriptors.
    if unsafe { sys::socketpair(sys::AF_UNIX, sys::SOCK_DGRAM, 0, ends.as_mut_ptr()) } == -1 {
        return Err(os("control socket"));
    }
    let (app, helper) = (owned(ends[0]), owned(ends[1]));
    close_on_exec(&app, "control socket")?;
    close_on_exec(&helper, "control socket")?;
    let set = |fd: &OwnedFd, name: c_int, value: c_int| {
        // SAFETY: `fd` is an open socket; `value` is one integer, the size passed. A refusal only leaves the
        // system's default in place.
        unsafe { sys::setsockopt(fd.as_raw_fd(), sys::SOL_SOCKET, name, (&raw const value).cast(), 4) };
    };
    set(&app, sys::SO_SNDBUF, CONTROL_BUFFER_BYTES);
    set(&helper, sys::SO_RCVBUF, CONTROL_BUFFER_BYTES);
    // A grant to a helper that is gone is an error code, never a signal to the app.
    set(&app, sys::SO_NOSIGPIPE, 1);
    Ok((app, helper))
}

/// Sends one datagram, with `descriptor` attached if there is one. Never waits.
fn send(socket: RawFd, bytes: &[u8], descriptor: Option<RawFd>) -> Result<(), i32> {
    let mut part = sys::iovec {
        base: bytes.as_ptr().cast_mut().cast(),
        len: bytes.len(),
    };
    let mut attached = sys::one_descriptor {
        len: 16,
        level: sys::SOL_SOCKET,
        kind: sys::SCM_RIGHTS,
        descriptor: descriptor.unwrap_or(-1),
    };
    let message = sys::msghdr {
        name: null_mut(),
        name_len: 0,
        iov: &raw mut part,
        iov_len: 1,
        control: if descriptor.is_some() {
            (&raw mut attached).cast()
        } else {
            null_mut()
        },
        control_len: if descriptor.is_some() { 16 } else { 0 },
        flags: 0,
    };
    loop {
        // SAFETY: `message` points at one readable buffer of `bytes.len()` bytes and, if a descriptor goes along,
        // at one complete control message of 16 bytes; all of it lives past the call. The call only reads them.
        let sent = unsafe { sys::sendmsg(socket, &raw const message, sys::MSG_DONTWAIT) };
        if sent == bytes.len() as isize {
            return Ok(());
        }
        if sent >= 0 {
            return Err(sys::EBADMSG);
        }
        match errno() {
            sys::EINTR => {}
            code => return Err(code),
        }
    }
}

/// What one datagram held.
struct Received {
    len: usize,
    descriptor: Option<OwnedFd>,
}

/// Receives one datagram into `bytes`, with at most one descriptor. `Ok(None)` when none is waiting. Never waits. A
/// datagram longer than `bytes`, or one whose descriptors did not fit, is an error.
fn receive(socket: RawFd, bytes: &mut [u8]) -> Result<Option<Received>, i32> {
    let mut part = sys::iovec {
        base: bytes.as_mut_ptr().cast(),
        len: bytes.len(),
    };
    let mut attached = sys::one_descriptor {
        len: 0,
        level: 0,
        kind: 0,
        descriptor: -1,
    };
    loop {
        let mut message = sys::msghdr {
            name: null_mut(),
            name_len: 0,
            iov: &raw mut part,
            iov_len: 1,
            control: (&raw mut attached).cast(),
            control_len: 16,
            flags: 0,
        };
        // SAFETY: `message` points at one writable buffer of `bytes.len()` bytes and at 16 writable bytes for one
        // control message, both alive past the call.
        let got = unsafe { sys::recvmsg(socket, &raw mut message, sys::MSG_DONTWAIT) };
        if got < 0 {
            match errno() {
                sys::EINTR => continue,
                sys::EAGAIN => return Ok(None),
                code => return Err(code),
            }
        }
        // A descriptor the system placed here is owned from this line on, whatever else is wrong with the message.
        let carried = message.control_len >= 16
            && attached.len == 16
            && attached.level == sys::SOL_SOCKET
            && attached.kind == sys::SCM_RIGHTS
            && attached.descriptor >= 0;
        let descriptor = carried.then(|| owned(attached.descriptor));
        if message.flags & (sys::MSG_TRUNC | sys::MSG_CTRUNC) != 0 {
            return Err(sys::EBADMSG);
        }
        if let Some(descriptor) = &descriptor {
            // SAFETY: `descriptor` is open; the command takes one integer. A failure leaves a descriptor that the
            // profile's ban on starting a process covers anyway.
            unsafe { sys::fcntl(descriptor.as_raw_fd(), sys::F_SETFD, sys::FD_CLOEXEC) };
        }
        return Ok(Some(Received {
            len: got as usize,
            descriptor,
        }));
    }
}

/// Waits at most `timeout_ms` for a datagram on `socket`. False when none came.
fn readable_within(socket: RawFd, timeout_ms: c_int) -> bool {
    let mut wait = sys::pollfd {
        fd: socket,
        events: sys::POLLIN,
        revents: 0,
    };
    // SAFETY: `wait` is one valid entry, the count passed.
    unsafe { sys::poll(&raw mut wait, 1, timeout_ms) == 1 && wait.revents & sys::POLLIN != 0 }
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from(u32_at(bytes, at)) | (u64::from(u32_at(bytes, at + 4)) << 32)
}

/// The app's first message to a new helper: its memory limit.
fn hello(memory_bytes: u64) -> [u8; HELLO_LEN] {
    let mut out = [0u8; HELLO_LEN];
    out[..8].copy_from_slice(HELLO_MAGIC);
    out[8..].copy_from_slice(&memory_bytes.to_le_bytes());
    out
}

/// The helper's handshake: what it says about itself. On the wire two more 32-bit words follow the state, both zero
/// (kept so the message can grow without changing its length).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Status {
    pub(crate) state: u32,
    pub(crate) profile_version: u32,
}

impl Status {
    fn encode(self) -> [u8; STATUS_LEN] {
        let mut out = [0u8; STATUS_LEN];
        out[..8].copy_from_slice(STATUS_MAGIC);
        out[8..12].copy_from_slice(&self.state.to_le_bytes());
        out[20..24].copy_from_slice(&self.profile_version.to_le_bytes());
        out
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != STATUS_LEN || &bytes[..8] != STATUS_MAGIC {
            return None;
        }
        Some(Self {
            state: u32_at(bytes, 8),
            profile_version: u32_at(bytes, 20),
        })
    }
}

/// One grant on the wire: which grant, how long the section is, and whether it may be written.
fn grant_message(wire: u64, len: u64, access: Access) -> [u8; GRANT_LEN] {
    let mut out = [0u8; GRANT_LEN];
    out[..8].copy_from_slice(GRANT_MAGIC);
    out[8..16].copy_from_slice(&wire.to_le_bytes());
    out[16..24].copy_from_slice(&len.to_le_bytes());
    out[24..28].copy_from_slice(&u32::from(access == Access::ReadWrite).to_le_bytes());
    out
}

/// What the kernel says about a new helper, read before the app hands out its pipes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Facts {
    /// The helper's own handshake, if a well-formed one arrived.
    pub(crate) status: Option<Status>,
    /// The kernel reports a sandbox on the process.
    pub(crate) sandboxed: bool,
    /// How many of [`MUST_BE_DENIED`] the kernel reports as denied.
    pub(crate) denied: usize,
    /// How many of [`MUST_BE_ALLOWED`] the kernel reports as allowed.
    pub(crate) allowed: usize,
}

/// One operation the app asks the kernel about: its name in the profile language, and what it is tried on.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Check {
    /// No object.
    Plain(&'static CStr),
    /// On a path.
    Path(&'static CStr, &'static CStr),
    /// On a Mach service's name.
    Service(&'static CStr, &'static CStr),
}

/// What a confined helper must not be able to do, as the kernel answers for its process: read or write where the
/// user's and the system's files are (a font folder it may read included, for writing), use the network in any
/// direction, start a process, reach a Mach service.
///
/// Every path here exists on every Mac and is one the kernel allows an unconfined process (a test holds that): the
/// kernel answers "denied" for a path that does not exist and for one the system protects from everyone, and such
/// an answer would say nothing about the helper.
pub(crate) const MUST_BE_DENIED: &[Check] = &[
    Check::Path(c"file-read-data", c"/Users"),
    Check::Path(c"file-read-data", c"/private/etc/passwd"),
    Check::Path(c"file-read-data", c"/private/var"),
    Check::Path(c"file-read-data", c"/Volumes"),
    Check::Path(c"file-read-data", c"/Applications"),
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
    Check::Path(c"process-exec*", c"/bin/sh"),
    Check::Path(c"process-exec*", c"/usr/bin/true"),
    Check::Plain(c"process-fork"),
    Check::Service(c"mach-lookup", c"com.apple.system.notification_center"),
    Check::Service(c"mach-lookup", c"com.apple.windowserver.active"),
    Check::Service(c"mach-lookup", c"com.apple.SecurityServer"),
];

/// What the profile allows, asked too: a process the kernel answers "denied" for everything (one that is gone, an
/// identifier that names nothing) is not taken for a confined helper.
pub(crate) const MUST_BE_ALLOWED: &[Check] = &[Check::Path(c"file-read-data", c"/System/Library/Fonts")];

/// The kernel's answer for one operation of the process `pid`: `Some(true)` denied, `Some(false)` allowed, `None`
/// when the kernel gave no answer.
fn denied(pid: c_int, check: Check) -> Option<bool> {
    // SAFETY: reads a constant the sandbox library exports.
    let quiet = unsafe { sys::SANDBOX_CHECK_NO_REPORT };
    // SAFETY: each call passes a terminated operation name and, for a filter that takes one, one terminated string,
    // all constants of this module. The call only reads them and changes nothing in either process.
    let answer = unsafe {
        match check {
            Check::Plain(operation) => sys::sandbox_check(pid, operation.as_ptr(), sys::SANDBOX_FILTER_NONE | quiet),
            Check::Path(operation, path) => {
                sys::sandbox_check(pid, operation.as_ptr(), sys::SANDBOX_FILTER_PATH | quiet, path.as_ptr())
            }
            Check::Service(operation, name) => sys::sandbox_check(
                pid,
                operation.as_ptr(),
                sys::SANDBOX_FILTER_GLOBAL_NAME | quiet,
                name.as_ptr(),
            ),
        }
    };
    match answer {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

/// Whether the kernel reports a sandbox on the process `pid`.
fn has_sandbox(pid: c_int) -> bool {
    // SAFETY: a null operation asks only whether the process has a sandbox; nothing is read through a pointer.
    unsafe { sys::sandbox_check(pid, null(), sys::SANDBOX_FILTER_NONE) == 1 }
}

/// Reads what [`judge`] judges about the process `pid`.
pub(crate) fn read_facts(pid: c_int, status: Option<Status>) -> Facts {
    Facts {
        status,
        sandboxed: has_sandbox(pid),
        denied: MUST_BE_DENIED
            .iter()
            .filter(|check| denied(pid, **check) == Some(true))
            .count(),
        allowed: MUST_BE_ALLOWED
            .iter()
            .filter(|check| denied(pid, **check) == Some(false))
            .count(),
    }
}

/// Refuses a helper whose facts are not those of a process under [`PROFILE`]: one check per fact, each with its own
/// reason. Pure, so every refusal is tested.
pub(crate) fn judge(facts: &Facts) -> Result<(), SandboxError> {
    let Some(status) = facts.status else {
        return Err(SandboxError::NotConfined("no handshake"));
    };
    if status.state == STATE_REFUSED {
        return Err(SandboxError::NotConfined("the system refused the helper's profile"));
    }
    if status.state != STATE_CONFINED {
        return Err(SandboxError::NotConfined("an unknown handshake"));
    }
    if status.profile_version != PROFILE_VERSION {
        return Err(SandboxError::NotConfined("another profile than this app's"));
    }
    if !facts.sandboxed {
        return Err(SandboxError::NotConfined("the kernel reports no sandbox"));
    }
    if facts.denied != MUST_BE_DENIED.len() {
        return Err(SandboxError::NotConfined("the kernel allows what the profile denies"));
    }
    if facts.allowed != MUST_BE_ALLOWED.len() {
        return Err(SandboxError::NotConfined("the kernel's answers are not the profile's"));
    }
    Ok(())
}

/// The helper's process, shared by its `Child`, and its memory watch. The one place that waits for it and signals
/// it, under one lock: once its end has been collected the process identifier may name another process, and nothing
/// here signals it again.
#[derive(Debug)]
struct Process {
    pid: c_int,
    ended: Mutex<Option<ExitStatus>>,
    /// Set before the app's own kill, so the end it causes is told apart from any other.
    killed: AtomicBool,
}

impl Process {
    /// `Some` when the helper has ended, without waiting.
    fn poll(&self) -> Result<Option<ExitStatus>, SandboxError> {
        let mut ended = lock(&self.ended);
        if ended.is_some() {
            return Ok(*ended);
        }
        loop {
            let mut status: c_int = 0;
            // SAFETY: `pid` is a child of this process that has not been collected (checked under the lock above);
            // `status` is a valid place for its status. Never waits.
            let got = unsafe { sys::waitpid(self.pid, &raw mut status, sys::WNOHANG) };
            if got == 0 {
                return Ok(None);
            }
            if got == self.pid {
                let code = self.code(status);
                *ended = Some(ExitStatus { code });
                return Ok(*ended);
            }
            match errno() {
                sys::EINTR => {}
                // Collected elsewhere (an app that ignores child signals): it has ended, how is not known.
                sys::ECHILD => {
                    let code = if self.killed.load(Ordering::SeqCst) {
                        KILLED_EXIT_CODE
                    } else {
                        UNKNOWN_EXIT_CODE
                    };
                    *ended = Some(ExitStatus { code });
                    return Ok(*ended);
                }
                _ => return Err(os("wait")),
            }
        }
    }

    /// The exit code for a wait status: the helper's own, [`KILLED_EXIT_CODE`] for the app's kill, or
    /// [`SIGNALLED`] plus the signal.
    fn code(&self, status: c_int) -> u32 {
        let signal = status & 0x7f;
        if signal == 0 {
            return ((status >> 8) & 0xff) as u32;
        }
        if signal == sys::SIGKILL && self.killed.load(Ordering::SeqCst) {
            return KILLED_EXIT_CODE;
        }
        SIGNALLED + signal as u32
    }

    fn wait(&self, bound: Duration) -> Result<Option<ExitStatus>, SandboxError> {
        let deadline = Instant::now().checked_add(bound);
        let mut pause = Duration::from_millis(1);
        loop {
            if let Some(status) = self.poll()? {
                return Ok(Some(status));
            }
            let left = match deadline {
                Some(deadline) => deadline.saturating_duration_since(Instant::now()),
                None => pause,
            };
            if left.is_zero() {
                return Ok(None);
            }
            std::thread::sleep(pause.min(left));
            pause = (pause * 2).min(Duration::from_millis(10));
        }
    }

    /// Ends the helper now. Fine on one that already ended.
    fn kill(&self) -> Result<(), SandboxError> {
        let ended = lock(&self.ended);
        if ended.is_some() {
            return Ok(());
        }
        self.killed.store(true, Ordering::SeqCst);
        // SAFETY: under the lock and not yet collected, so `pid` still names the helper (running, or ended and
        // waiting to be collected) and nothing else.
        if unsafe { sys::kill(self.pid, sys::SIGKILL) } == 0 || errno() == sys::ESRCH {
            return Ok(());
        }
        Err(os("kill"))
    }
}

/// A confined helper: its process, the app's end of its control socket and its memory watch. Dropping it ends the
/// helper.
#[derive(Debug)]
pub(crate) struct ChildInner {
    process: Arc<Process>,
    control: OwnedFd,
    /// The number of the next grant.
    next_wire: AtomicU64,
    /// The bytes of the sections granted to this helper that are still alive in the app (`crate::GrantedTo`).
    granted: Arc<AtomicU64>,
    /// Set by the memory watch before it ends the helper.
    killed_for_memory: Arc<AtomicBool>,
    watch: Option<Watch>,
}

impl ChildInner {
    pub(crate) fn id(&self) -> u32 {
        self.process.pid as u32
    }

    pub(crate) fn wait(&self, bound: Duration) -> Result<Option<ExitStatus>, SandboxError> {
        self.process.wait(bound)
    }

    pub(crate) fn kill(&self) -> Result<(), SandboxError> {
        self.process.kill()
    }

    pub(crate) fn granted_bytes(&self) -> &Arc<AtomicU64> {
        &self.granted
    }

    pub(crate) fn killed_for_memory(&self) -> bool {
        self.killed_for_memory.load(Ordering::SeqCst)
    }

    /// Sends the helper one of the section's two descriptors: the one opened read-only for [`Access::Read`], which
    /// cannot be mapped writable, the read-write one otherwise.
    pub(crate) fn grant(&self, section: &SectionInner, access: Access) -> Result<Granted, SandboxError> {
        let wire = self.next_wire.fetch_add(1, Ordering::SeqCst);
        let descriptor = match access {
            Access::Read => section.read_only.as_raw_fd(),
            Access::ReadWrite => section.read_write.as_raw_fd(),
        };
        let message = grant_message(wire, section.len as u64, access);
        let started = Instant::now();
        loop {
            match send(self.control.as_raw_fd(), &message, Some(descriptor)) {
                Ok(()) => {
                    return Ok(Granted {
                        wire,
                        len: section.len as u64,
                    });
                }
                // No room: the helper holds grants it has not mapped. It gets a moment to map one, not for ever.
                Err(sys::EAGAIN | sys::ENOBUFS) if started.elapsed() < GRANT_BOUND => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(code) => {
                    return Err(SandboxError::Os {
                        step: "grant",
                        code: code as u32,
                    });
                }
            }
        }
    }

    /// How many descriptors the helper process has open now.
    pub(crate) fn handle_count(&self) -> Result<u32, SandboxError> {
        let mut entries = vec![sys::proc_fdinfo::default(); 4096];
        // SAFETY: `entries` is writable for the size passed; the call writes whole entries only.
        let bytes = unsafe {
            sys::proc_pidinfo(
                self.process.pid,
                sys::PROC_PIDLISTFDS,
                0,
                entries.as_mut_ptr().cast(),
                size_of_val(entries.as_slice()) as c_int,
            )
        };
        if bytes <= 0 {
            return Err(os("handle count"));
        }
        Ok((bytes as usize / size_of::<sys::proc_fdinfo>()) as u32)
    }
}

impl Drop for ChildInner {
    fn drop(&mut self) {
        let _ = self.process.kill();
        // Collected here, so no ended helper is left behind for the app's lifetime.
        let _ = self.process.wait(REAP_BOUND);
        if let Some(watch) = &mut self.watch {
            watch.finish();
        }
    }
}

/// The helper's physical footprint in bytes (what Activity Monitor shows as its memory), as the kernel counts it.
fn footprint(pid: c_int) -> Option<u64> {
    let mut usage = sys::rusage_info_v0::default();
    // SAFETY: `usage` is writable and is the structure of the flavor passed.
    if unsafe { sys::proc_pid_rusage(pid, sys::RUSAGE_INFO_V0, (&raw mut usage).cast()) } != 0 {
        return None;
    }
    Some(usage.phys_footprint)
}

/// One helper's memory watch: a thread that reads its footprint every [`POLL`]. Ending the watch (or dropping it)
/// stops the thread and waits for it.
#[derive(Debug)]
struct Watch {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Watch {
    fn start(
        process: Arc<Process>,
        limit: u64,
        granted: Arc<AtomicU64>,
        killed: Arc<AtomicBool>,
    ) -> Result<Self, SandboxError> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("helper-memory-watch".into())
            .spawn(move || watch(&process, limit, &granted, &killed, &stopping))
            .map_err(|error| SandboxError::Os {
                step: "watch thread",
                code: error.raw_os_error().unwrap_or(0) as u32,
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    fn finish(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.finish();
    }
}

/// The watch thread: ends the helper when it is over ([`crate::over_memory`]) or when its footprint can no longer be
/// read while it runs. Never stops watching a helper that still runs, except when asked to (`stop`).
fn watch(process: &Process, limit: u64, granted: &AtomicU64, killed: &AtomicBool, stop: &AtomicBool) {
    loop {
        std::thread::park_timeout(POLL);
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let over = match process.poll() {
            Ok(Some(_)) => return,
            Ok(None) => match footprint(process.pid) {
                Some(bytes) => over_memory(bytes, 0, limit, granted.load(Ordering::SeqCst)),
                // A helper that ended by itself between the two reads was not ended for its memory.
                None => !matches!(process.poll(), Ok(Some(_))),
            },
            // The wait itself failed: the helper is not left unwatched.
            Err(_) => true,
        };
        if over {
            // Before the kill, so whoever sees the helper's end also sees why.
            killed.store(true, Ordering::SeqCst);
            let _ = process.kill();
            return;
        }
    }
}

/// A shared-memory object without a name, the app's read-write view of all of it, and its two descriptors.
pub(crate) struct SectionInner {
    read_write: OwnedFd,
    /// Opened read-only: what a read grant sends. A mapping of it cannot be written.
    read_only: OwnedFd,
    view: *mut u8,
    len: usize,
}

// SAFETY: the view is memory of this process that only this value unmaps; a shared reference lends it for reading
// and an exclusive one for writing, as a `Vec<u8>` would.
unsafe impl Send for SectionInner {}
// SAFETY: as above.
unsafe impl Sync for SectionInner {}

/// A shared-memory name nothing can guess: 104 random bits. The system's limit is 31 bytes.
fn random_name() -> Result<CString, SandboxError> {
    let mut random = [0u8; 13];
    // SAFETY: `random` is writable for the 13 bytes asked.
    if unsafe { sys::getentropy(random.as_mut_ptr().cast(), random.len()) } != 0 {
        return Err(os("random"));
    }
    let mut name = String::from("/bb.");
    for byte in random {
        name.push_str(&format!("{byte:02x}"));
    }
    CString::new(name).map_err(|_| SandboxError::Os {
        step: "random",
        code: 0,
    })
}

impl SectionInner {
    /// `len` is 1 byte to 4 GiB (checked by the caller).
    pub(crate) fn new(len: usize) -> Result<Self, SandboxError> {
        let (read_write, read_only) = Self::descriptors()?;
        close_on_exec(&read_write, "section")?;
        close_on_exec(&read_only, "section")?;
        // SAFETY: `read_write` is the open, still empty object just made.
        if unsafe { sys::ftruncate(read_write.as_raw_fd(), len as i64) } != 0 {
            return Err(os("section size"));
        }
        // SAFETY: a new shared mapping of `len` bytes of the object, at an address the system chooses; it is
        // unmapped once, in `drop`.
        let view = unsafe {
            sys::mmap(
                null_mut(),
                len,
                sys::PROT_READ | sys::PROT_WRITE,
                sys::MAP_SHARED,
                read_write.as_raw_fd(),
                0,
            )
        };
        if view as isize == -1 {
            return Err(os("section view"));
        }
        Ok(Self {
            read_write,
            read_only,
            view: view.cast(),
            len,
        })
    }

    /// Makes the object under a random name, opens it a second time read-only, and removes the name: from then on
    /// only the two descriptors reach it. The name exists for the three calls; it is only this user's to open.
    fn descriptors() -> Result<(OwnedFd, OwnedFd), SandboxError> {
        for _ in 0..8 {
            let name = random_name()?;
            // SAFETY: `name` is terminated; the flags create a new object or fail; the mode is one promoted integer.
            let made =
                unsafe { sys::shm_open(name.as_ptr(), sys::O_RDWR | sys::O_CREAT | sys::O_EXCL, 0o600 as c_int) };
            if made == -1 {
                if errno() == sys::EEXIST {
                    continue;
                }
                return Err(os("section"));
            }
            let read_write = owned(made);
            // SAFETY: `name` is the terminated name of the object just made.
            let again = unsafe { sys::shm_open(name.as_ptr(), sys::O_RDONLY, 0 as c_int) };
            let failure = (again == -1).then(|| os("section read-only"));
            // SAFETY: as above. The name goes whatever happened.
            let unlinked = unsafe { sys::shm_unlink(name.as_ptr()) };
            let read_only = (again != -1).then(|| owned(again));
            if let Some(failure) = failure {
                return Err(failure);
            }
            if unlinked != 0 {
                return Err(os("section name"));
            }
            return read_only
                .map(|read_only| (read_write, read_only))
                .ok_or(SandboxError::Os {
                    step: "section read-only",
                    code: 0,
                });
        }
        Err(SandboxError::Os {
            step: "section",
            code: sys::EEXIST as u32,
        })
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        // SAFETY: the view is mapped read-write for `len` bytes until `drop`; `&self` lends it for reading only.
        unsafe { std::slice::from_raw_parts(self.view, self.len) }
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: as above, and `&mut self` makes this the only reference into the view.
        unsafe { std::slice::from_raw_parts_mut(self.view, self.len) }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Copies the first `out.len()` bytes of the view into `out`, read from the view's address with volatile reads.
    /// `Section::copy_to` hands over exactly `len` bytes; a longer `out` is a bug of ours and panics here before
    /// anything is read.
    pub(crate) fn copy_into(&self, out: &mut [u8]) {
        assert!(out.len() <= self.len, "a copy longer than the section");
        let source = self.view.cast_const();
        let (words, tail) = out.as_chunks_mut::<8>();
        let done = words.len() * 8;
        for (index, word_out) in words.iter_mut().enumerate() {
            // SAFETY: the view is mapped readable for `self.len` bytes until `drop`, and `index * 8 + 8 <= out.len()
            // <= self.len`; the view starts on a page boundary, so each word read is aligned. The read goes through
            // the raw address: no reference to the shared bytes is made, so a helper writing them at this moment can
            // leave a torn value in `out` but never changes memory a reference points to. Volatile, so each word is
            // read exactly once and nothing is assumed about bytes another process may change.
            let word = unsafe { std::ptr::read_volatile(source.add(index * 8).cast::<u64>()) };
            *word_out = word.to_ne_bytes();
        }
        for (index, byte) in tail.iter_mut().enumerate() {
            // SAFETY: as above, one byte inside the view: `done + index < out.len() <= self.len`.
            *byte = unsafe { std::ptr::read_volatile(source.add(done + index)) };
        }
    }
}

impl Drop for SectionInner {
    fn drop(&mut self) {
        zeroize(self.as_mut_slice());
        #[cfg(test)]
        tests::record_zeroed(self.as_slice());
        // SAFETY: the address `mmap` returned in `new`, unmapped once, here. The two descriptors close right after.
        unsafe { sys::munmap(self.view.cast(), self.len) };
    }
}

/// Writes zero over every byte with volatile writes, which the compiler may not leave out.
fn zeroize(bytes: &mut [u8]) {
    // SAFETY: every bit pattern is a valid `u64`, so the aligned middle of the bytes may be written as words.
    let (head, words, tail) = unsafe { bytes.align_to_mut::<u64>() };
    for word in words {
        // SAFETY: `word` is a valid, aligned and exclusive reference.
        unsafe { std::ptr::write_volatile(word, 0) };
    }
    for byte in head.iter_mut().chain(tail) {
        // SAFETY: `byte` is a valid and exclusive reference.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(Ordering::SeqCst);
}

/// A granted section, mapped in the helper. Unmapped on drop; the grant's descriptor is closed as soon as it is
/// mapped.
pub(crate) struct MappedInner {
    view: *mut u8,
    len: usize,
    writable: bool,
}

// SAFETY: as for `SectionInner`: the view is memory of this process that only this value unmaps.
unsafe impl Send for MappedInner {}
// SAFETY: as above.
unsafe impl Sync for MappedInner {}

impl MappedInner {
    pub(crate) fn as_slice(&self) -> &[u8] {
        // SAFETY: the view is mapped readable for `len` bytes until `drop`; `&self` lends it for reading only.
        unsafe { std::slice::from_raw_parts(self.view, self.len) }
    }

    pub(crate) fn as_mut_slice(&mut self) -> Option<&mut [u8]> {
        if !self.writable {
            return None;
        }
        // SAFETY: a view mapped read-write for `len` bytes until `drop`; `&mut self` makes this the only reference.
        Some(unsafe { std::slice::from_raw_parts_mut(self.view, self.len) })
    }
}

impl Drop for MappedInner {
    fn drop(&mut self) {
        // SAFETY: the address `mmap` returned in `map`, unmapped once, here.
        unsafe { sys::munmap(self.view.cast(), self.len) };
    }
}

/// A grant the helper has received and not mapped yet.
struct Waiting {
    wire: u64,
    descriptor: OwnedFd,
}

/// The helper's side of the control socket: set once by [`confine_or_exit`], and the grants received so far.
struct Control {
    socket: Option<RawFd>,
    waiting: Vec<Waiting>,
}

static CONTROL: Mutex<Control> = Mutex::new(Control {
    socket: None,
    waiting: Vec::new(),
});

/// The helper's side: maps the section of the grant numbered `wire`. The app sent the grant before it named it on
/// the pipe, so it is waiting on the control socket or already taken off it; nothing here waits. The grant's
/// descriptor is closed as soon as it is mapped, or at once when mapping fails, so a grant maps once.
pub(crate) fn map(wire: u64, len: usize, access: Access) -> Result<MappedInner, SandboxError> {
    let descriptor = {
        let mut control = lock(&CONTROL);
        let Some(socket) = control.socket else {
            return Err(SandboxError::Os {
                step: "map",
                code: sys::ENOENT as u32,
            });
        };
        loop {
            if let Some(at) = control.waiting.iter().position(|grant| grant.wire == wire) {
                break control.waiting.swap_remove(at).descriptor;
            }
            let mut bytes = [0u8; GRANT_LEN];
            match receive(socket, &mut bytes) {
                Ok(Some(Received {
                    len: GRANT_LEN,
                    descriptor: Some(descriptor),
                })) if &bytes[..8] == GRANT_MAGIC => control.waiting.push(Waiting {
                    wire: u64_at(&bytes, 8),
                    descriptor,
                }),
                // Not a grant: dropped, and with it any descriptor it carried.
                Ok(Some(_)) => {}
                Ok(None) => {
                    return Err(SandboxError::Os {
                        step: "map",
                        code: sys::ENOENT as u32,
                    });
                }
                Err(code) => {
                    return Err(SandboxError::Os {
                        step: "map",
                        code: code as u32,
                    });
                }
            }
        }
    };
    let protection = if access == Access::ReadWrite {
        sys::PROT_READ | sys::PROT_WRITE
    } else {
        sys::PROT_READ
    };
    // SAFETY: a new shared mapping of `len` bytes of the granted object, at an address the system chooses. The
    // system refuses a writable mapping of a descriptor opened read-only and a length past the object's; the view is
    // unmapped once, in `drop`.
    let view = unsafe { sys::mmap(null_mut(), len, protection, sys::MAP_SHARED, descriptor.as_raw_fd(), 0) };
    if view as isize == -1 {
        return Err(os("map"));
    }
    Ok(MappedInner {
        view: view.cast(),
        len,
        writable: access == Access::ReadWrite,
    })
}

/// Ends the helper at once, with nothing run on the way out.
fn end(code: c_int) -> ! {
    // SAFETY: ends the calling process; takes no pointer.
    unsafe { sys::_exit(code) }
}

/// Whether descriptor 3 is a datagram socket: what a helper started through this crate is handed.
fn control_socket() -> Option<RawFd> {
    let mut kind: c_int = 0;
    let mut len = 4u32;
    // SAFETY: `kind` is a valid place for one integer, the size passed. A descriptor that is closed or no socket is
    // an error, which is the answer wanted.
    let ok = unsafe {
        sys::getsockopt(
            CONTROL_FD,
            sys::SOL_SOCKET,
            sys::SO_TYPE,
            (&raw mut kind).cast(),
            &raw mut len,
        )
    };
    (ok == 0 && kind == sys::SOCK_DGRAM).then_some(CONTROL_FD)
}

/// The helper's first step: confines this process, or ends it. It returns only in a process under [`PROFILE`].
///
/// 1. The app's first message on the control socket (the memory limit). A process that was not started through this
///    crate has no control socket; it is confined all the same, with the default limit, and can map no grant.
/// 2. A thread that ends this process when the process that started it is gone.
/// 3. Resource limits: no core file, no file growth, no new process, address space.
/// 4. The profile. Refused by the system (as for a process that already has a sandbox): the app is told, and this
///    process ends with [`UNCONFINED_EXIT_CODE`].
/// 5. The kernel is asked whether this process now has a sandbox.
/// 6. The handshake to the app, which asks the kernel itself before it sends anything.
pub(crate) fn confine_or_exit() {
    let control = control_socket();
    let refuse = || refuse(control);

    let memory_bytes = match control {
        None => DEFAULT_MEMORY_LIMIT_BYTES,
        Some(socket) => {
            let mut bytes = [0u8; HELLO_LEN];
            if !readable_within(socket, HELLO_BOUND_MS) {
                refuse();
            }
            match receive(socket, &mut bytes) {
                Ok(Some(Received {
                    len: HELLO_LEN,
                    descriptor: None,
                })) if &bytes[..8] == HELLO_MAGIC => u64_at(&bytes, 8),
                _ => refuse(),
            }
        }
    };
    if end_with_parent().is_err() {
        refuse();
    }
    apply_limits(memory_bytes);
    if !apply_profile() {
        refuse();
    }
    // SAFETY: takes no pointer.
    if !has_sandbox(unsafe { sys::getpid() }) {
        refuse();
    }
    if let Some(socket) = control {
        let status = Status {
            state: STATE_CONFINED,
            profile_version: PROFILE_VERSION,
        };
        if send(socket, &status.encode(), None).is_err() {
            end(c_int::from(UNCONFINED_EXIT_CODE));
        }
    }
    lock(&CONTROL).socket = control;
}

/// Tells the app, if there is one to tell, that this process is not confined, and ends it.
fn refuse(control: Option<RawFd>) -> ! {
    if let Some(socket) = control {
        let status = Status {
            state: STATE_REFUSED,
            profile_version: PROFILE_VERSION,
        };
        let _ = send(socket, &status.encode(), None);
    }
    end(c_int::from(UNCONFINED_EXIT_CODE))
}

/// Applies [`PROFILE`] to this process. False when the system refused it (the call's error code is not dependable,
/// measured, so none is passed on).
fn apply_profile() -> bool {
    let Ok(profile) = CString::new(PROFILE) else {
        return false;
    };
    let parameters: [*const c_char; 1] = [null()];
    let mut error: *mut c_char = null_mut();
    // SAFETY: `profile` is terminated; the parameter list is one null entry (no parameters); `error` receives an
    // allocation of the sandbox library's or stays null.
    let result = unsafe { sys::sandbox_init_with_parameters(profile.as_ptr(), 0, parameters.as_ptr(), &raw mut error) };
    if !error.is_null() {
        // SAFETY: the sandbox library allocated `error` in the call above; it is freed once and not read (its text is
        // not ours to pass on).
        unsafe { sys::sandbox_free_error(error) };
    }
    result == 0
}

/// The limits a helper sets on itself before the profile. Each is a second line behind the profile or the watch, so
/// one the system does not take is not a refusal.
fn apply_limits(memory_bytes: u64) {
    let set = |resource: c_int, bytes: u64| {
        let limit = sys::rlimit {
            current: bytes,
            maximum: bytes,
        };
        // SAFETY: `limit` is a valid structure; the call only reads it.
        unsafe { sys::setrlimit(resource, &raw const limit) == 0 }
    };
    // No core file (a document's bytes would be in it), no file growth, no new process: each also denied by the
    // profile.
    set(sys::RLIMIT_CORE, 0);
    set(sys::RLIMIT_FSIZE, 0);
    set(sys::RLIMIT_NPROC, 0);
    // The address space this process already has, plus its memory limit for what it allocates, plus room for two
    // sections of the largest size mapped at once (a document and a page): mapping what the app grants is never
    // what this limit stops, as on Windows the job's cap does not count the app's sections. The system takes no
    // value below what the process already has.
    let Some(now) = address_space() else {
        return;
    };
    let ceiling = now
        .saturating_add(memory_bytes)
        .saturating_add(MAX_SECTION_BYTES.saturating_mul(2));
    set(sys::RLIMIT_DATA, ceiling);
    set(sys::RLIMIT_AS, ceiling);
}

/// The size of this process's address space, as the kernel counts it.
fn address_space() -> Option<u64> {
    // Longer than every revision of the structure; the kernel fills what it knows and says how much.
    let mut info = [0u32; 256];
    let mut count = info.len() as u32;
    // SAFETY: `info` is writable for `count` 32-bit units; the task is this process's own.
    let result = unsafe {
        sys::task_info(
            sys::mach_task_self_,
            sys::TASK_VM_INFO,
            info.as_mut_ptr(),
            &raw mut count,
        )
    };
    if result != 0 || count < 2 {
        return None;
    }
    // The structure's first field: the virtual size, 64 bits.
    Some(u64::from(info[0]) | (u64::from(info[1]) << 32))
}

/// Starts the thread that ends this process when the process that started it is gone. The kernel's notice of that
/// exit is registered here, before the profile; the thread only waits for it.
fn end_with_parent() -> Result<(), ()> {
    // SAFETY: takes no pointer.
    let parent = unsafe { sys::getppid() };
    // SAFETY: takes no pointer.
    let queue = unsafe { sys::kqueue() };
    if queue == -1 {
        return Err(());
    }
    let change = sys::kevent {
        ident: parent as usize,
        filter: sys::EVFILT_PROC,
        flags: sys::EV_ADD,
        fflags: sys::NOTE_EXIT,
        data: 0,
        udata: null_mut(),
    };
    // SAFETY: one valid change, the count passed; no event is asked for, so nothing is written.
    if unsafe { sys::kevent(queue, &raw const change, 1, null_mut(), 0, null()) } == -1 {
        return Err(());
    }
    // Gone between the two calls above: the process that started this one is no longer its parent.
    // SAFETY: takes no pointer.
    if unsafe { sys::getppid() } != parent {
        end(PARENT_GONE_EXIT_CODE);
    }
    std::thread::Builder::new()
        .name("ends-with-the-app".into())
        .spawn(move || {
            loop {
                let mut event = sys::kevent {
                    ident: 0,
                    filter: 0,
                    flags: 0,
                    fflags: 0,
                    data: 0,
                    udata: null_mut(),
                };
                // SAFETY: `event` is a valid place for one event, the count passed; a null timeout waits.
                let got = unsafe { sys::kevent(queue, null(), 0, &raw mut event, 1, null()) };
                if got == -1 && errno() == sys::EINTR {
                    continue;
                }
                // The exit, or a wait that failed: either way this process does not go on without its watch.
                end(PARENT_GONE_EXIT_CODE);
            }
        })
        .map(|_| ())
        .map_err(|_| ())
}

pub(crate) fn remove_left_over_profiles() {}

/// The helper path as the system takes it: absolute, a file, no NUL. Nothing is searched.
fn checked_path(helper_exe: &Path) -> Result<CString, SandboxError> {
    if helper_exe.as_os_str().is_empty() {
        return Err(SandboxError::HelperPath("empty"));
    }
    if !helper_exe.is_absolute() {
        return Err(SandboxError::HelperPath("not an absolute path"));
    }
    let path = CString::new(helper_exe.as_os_str().as_bytes()).map_err(|_| SandboxError::HelperPath("holds a NUL"))?;
    if !std::fs::metadata(helper_exe).map(|m| m.is_file()).unwrap_or(false) {
        return Err(SandboxError::HelperPath("not a file"));
    }
    Ok(path)
}

/// The spawn's attributes and file actions, destroyed when dropped.
struct SpawnSetup {
    attributes: *mut c_void,
    actions: *mut c_void,
}

impl SpawnSetup {
    /// Every signal at its default and none blocked; no descriptor of the app but the four named here: the two pipe
    /// ends as standard input and output, `/dev/null` as standard error, the control socket as descriptor 3.
    fn new(stdin: &OwnedFd, stdout: &OwnedFd, control: &OwnedFd) -> Result<Self, SandboxError> {
        let mut setup = Self {
            attributes: null_mut(),
            actions: null_mut(),
        };
        let step = |result: c_int| {
            if result == 0 {
                Ok(())
            } else {
                Err(SandboxError::Os {
                    step: "spawn setup",
                    code: result as u32,
                })
            }
        };
        let (every_signal, no_signal) = (u32::MAX, 0u32);
        // SAFETY: both objects are initialised before use and destroyed once, in `drop`; each call gets the object
        // it initialised, open descriptors, and pointers to values that live past the call.
        unsafe {
            step(sys::posix_spawnattr_init(&raw mut setup.attributes))?;
            step(sys::posix_spawn_file_actions_init(&raw mut setup.actions))?;
            step(sys::posix_spawnattr_setflags(
                &raw mut setup.attributes,
                sys::POSIX_SPAWN_CLOEXEC_DEFAULT | sys::POSIX_SPAWN_SETSIGDEF | sys::POSIX_SPAWN_SETSIGMASK,
            ))?;
            step(sys::posix_spawnattr_setsigdefault(
                &raw mut setup.attributes,
                &raw const every_signal,
            ))?;
            step(sys::posix_spawnattr_setsigmask(
                &raw mut setup.attributes,
                &raw const no_signal,
            ))?;
            step(sys::posix_spawn_file_actions_adddup2(
                &raw mut setup.actions,
                stdin.as_raw_fd(),
                0,
            ))?;
            step(sys::posix_spawn_file_actions_adddup2(
                &raw mut setup.actions,
                stdout.as_raw_fd(),
                1,
            ))?;
            step(sys::posix_spawn_file_actions_addopen(
                &raw mut setup.actions,
                2,
                c"/dev/null".as_ptr(),
                sys::O_WRONLY,
                0,
            ))?;
            step(sys::posix_spawn_file_actions_adddup2(
                &raw mut setup.actions,
                control.as_raw_fd(),
                CONTROL_FD,
            ))?;
        }
        Ok(setup)
    }
}

impl Drop for SpawnSetup {
    fn drop(&mut self) {
        // SAFETY: each object, if it was initialised (not null), is destroyed once.
        unsafe {
            if !self.attributes.is_null() {
                sys::posix_spawnattr_destroy(&raw mut self.attributes);
            }
            if !self.actions.is_null() {
                sys::posix_spawn_file_actions_destroy(&raw mut self.actions);
            }
        }
    }
}

/// Waits for the helper's handshake. `None` when it ended, said nothing in time, or sent something else.
fn handshake(process: &Process, control: &OwnedFd) -> Option<Status> {
    let started = Instant::now();
    while started.elapsed() < HANDSHAKE_BOUND {
        if readable_within(control.as_raw_fd(), 50) {
            let mut bytes = [0u8; STATUS_LEN];
            return match receive(control.as_raw_fd(), &mut bytes) {
                Ok(Some(Received { len, descriptor: None })) => Status::decode(&bytes[..len.min(STATUS_LEN)]),
                _ => None,
            };
        }
        // A helper that is gone will not say anything.
        if !matches!(process.poll(), Ok(None)) {
            return None;
        }
    }
    None
}

/// `launch` is the app's launch lock, taken by the caller before anything here. Every descriptor made here is
/// close-on-exec from its first moment but for the gap between making it and marking it; no other launch of the app
/// runs in that gap. It is released as soon as the helper's ends are closed in the app.
pub(crate) fn spawn(
    helper_exe: &Path,
    limits: Limits,
    _profile: Profile,
    mode: crate::Mode,
    launch: LaunchGuard,
) -> Result<(ChildInner, Pipes), SandboxError> {
    let path = checked_path(helper_exe)?;
    // A process this one starts inherits this one's sandbox and cannot apply the helper's profile on top (the module
    // documentation). Nothing is started: the helper would hold what this app holds.
    // SAFETY: takes no pointer.
    if has_sandbox(unsafe { sys::getpid() }) {
        return Err(SandboxError::NotConfined(
            "the app has a sandbox its helper would inherit and cannot tighten",
        ));
    }
    let (helper_stdin, to_helper) = pipe()?;
    let (from_helper, helper_stdout) = pipe()?;
    let (control, helper_control) = control_pair()?;
    let helper_stdin = lifted(helper_stdin, "pipe")?;
    let helper_stdout = lifted(helper_stdout, "pipe")?;
    let helper_control = lifted(helper_control, "control socket")?;
    let setup = SpawnSetup::new(&helper_stdin, &helper_stdout, &helper_control)?;

    let word = mode.argument().map(|word| CString::new(word).unwrap_or_default());
    let mut arguments: Vec<*mut c_char> = vec![path.as_ptr().cast_mut()];
    if let Some(word) = &word {
        arguments.push(word.as_ptr().cast_mut());
    }
    arguments.push(null_mut());
    // Nothing of the app's environment, and nothing else.
    let environment: [*mut c_char; 1] = [null_mut()];

    let mut pid: c_int = 0;
    // SAFETY: `path` and every argument are terminated strings that live past the call (the call does not write
    // through them); both lists end with a null entry; `setup` holds initialised attributes and file actions; `pid`
    // receives the new process's identifier.
    let result = unsafe {
        sys::posix_spawn(
            &raw mut pid,
            path.as_ptr(),
            &raw const setup.actions,
            &raw const setup.attributes,
            arguments.as_ptr(),
            environment.as_ptr(),
        )
    };
    // The helper's ends are closed in the app whatever happened: after this, only the helper holds them, so a read
    // from a dead helper ends instead of waiting for ever.
    drop(helper_stdin);
    drop(helper_stdout);
    drop(helper_control);
    drop(setup);
    drop(launch);
    if result != 0 {
        return Err(SandboxError::Os {
            step: "spawn",
            code: result as u32,
        });
    }

    // From here every early return drops `child`, which ends the helper and collects it.
    let mut child = ChildInner {
        process: Arc::new(Process {
            pid,
            ended: Mutex::new(None),
            killed: AtomicBool::new(false),
        }),
        control,
        next_wire: AtomicU64::new(1),
        granted: Arc::new(AtomicU64::new(0)),
        killed_for_memory: Arc::new(AtomicBool::new(false)),
        watch: None,
    };
    send(child.control.as_raw_fd(), &hello(limits.memory_bytes), None).map_err(|code| SandboxError::Os {
        step: "hello",
        code: code as u32,
    })?;
    let status = handshake(&child.process, &child.control);
    // The helper has been sent its memory limit and nothing else. What it says is checked against what the kernel
    // says about its process; only then does the caller get its pipes.
    judge(&read_facts(pid, status))?;
    // A system that cannot report the helper's footprint cannot watch it: the helper is ended.
    if footprint(pid).is_none() {
        return Err(os("memory read"));
    }
    child.watch = Some(Watch::start(
        Arc::clone(&child.process),
        limits.memory_bytes,
        Arc::clone(&child.granted),
        Arc::clone(&child.killed_for_memory),
    )?);
    Ok((
        child,
        Pipes {
            to_helper: File::from(to_helper),
            from_helper: File::from(from_helper),
        },
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    thread_local! {
        /// How many bytes of the last section dropped on this thread were zero right before it was unmapped.
        static ZEROED_AT_DROP: Cell<Option<usize>> = const { Cell::new(None) };
    }

    /// The test-only hook of `SectionInner::drop`.
    pub(super) fn record_zeroed(view: &[u8]) {
        ZEROED_AT_DROP.set(Some(view.iter().filter(|&&byte| byte == 0).count()));
    }

    #[test]
    fn a_dropped_section_is_zeroed() {
        // Neither a multiple of eight nor of the page size, so the zeroing has a tail to cover.
        let len = 3 * 16384 + 5;
        let mut section = crate::Section::new(len).unwrap();
        section.as_mut_slice().fill(0xA5);
        assert_eq!(
            section.as_slice().iter().filter(|&&byte| byte == 0).count(),
            0,
            "the instrument can fail: before the drop no byte is zero"
        );
        ZEROED_AT_DROP.set(None);
        drop(section);
        assert_eq!(
            ZEROED_AT_DROP.get(),
            Some(len),
            "every byte was zero when the memory was released"
        );
    }

    /// A new section is all zero, holds what is written, and copies out exactly.
    #[test]
    fn a_section_starts_zeroed_and_holds_what_is_written() {
        let mut section = crate::Section::new(40_000).unwrap();
        assert_eq!(section.len(), 40_000);
        assert!(section.as_slice().iter().all(|&byte| byte == 0));
        for (index, byte) in section.as_mut_slice().iter_mut().enumerate() {
            *byte = (index % 251) as u8;
        }
        let mut out = vec![0u8; 40_000];
        section.copy_to(&mut out).unwrap();
        assert_eq!(
            out.iter().enumerate().filter(|(i, b)| **b == (i % 251) as u8).count(),
            40_000
        );
    }

    /// The app's own two descriptors of a section: neither survives the start of a process, and the read-only one
    /// cannot be mapped writable even here, in the unconfined app.
    #[test]
    fn a_sections_read_only_descriptor_refuses_a_writable_mapping() {
        let section = SectionInner::new(16384).unwrap();
        for fd in [&section.read_write, &section.read_only] {
            // SAFETY: `fd` is open; the command takes no further argument.
            let flags = unsafe { sys::fcntl(fd.as_raw_fd(), sys::F_GETFD) };
            assert_eq!(flags & sys::FD_CLOEXEC, sys::FD_CLOEXEC, "close-on-exec");
        }
        let map = |fd: &OwnedFd, protection: c_int| {
            // SAFETY: a new shared mapping of the section's 16384 bytes, unmapped right below.
            let view = unsafe { sys::mmap(null_mut(), 16384, protection, sys::MAP_SHARED, fd.as_raw_fd(), 0) };
            let failed = view as isize == -1;
            let code = if failed { errno() } else { 0 };
            if !failed {
                // SAFETY: the mapping just made.
                unsafe { sys::munmap(view, 16384) };
            }
            code
        };
        assert_eq!(map(&section.read_only, sys::PROT_READ), 0, "read-only maps for reading");
        assert_eq!(
            map(&section.read_only, sys::PROT_READ | sys::PROT_WRITE),
            1,
            "a writable mapping of the read-only descriptor is refused (system code 1)"
        );
        assert_eq!(
            map(&section.read_write, sys::PROT_READ | sys::PROT_WRITE),
            0,
            "the instrument can fail: the read-write descriptor maps writable"
        );
    }

    /// The profile's text is pinned to its version: a changed text with the old version turns this red.
    #[test]
    fn the_profile_text_is_pinned_to_its_version() {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in PROFILE.bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        assert_eq!((PROFILE_VERSION, hash), (1, PROFILE_FINGERPRINT), "{hash:#x}");
        assert!(PROFILE.starts_with("(version 1)\n(deny default)\n"));
        // Every rule after the first two lines allows; this is the whole list, by operation.
        let allowed: Vec<&str> = PROFILE
            .lines()
            .skip(2)
            .map(|line| line.split_whitespace().nth(1).unwrap_or(""))
            .collect();
        assert_eq!(allowed, ["file-read*", "file-read-metadata", "sysctl-read", "signal"]);
        for word in ["network", "process", "mach", "ipc", "file-write", "iokit", "import"] {
            assert!(!PROFILE.contains(word), "the profile names {word}");
        }
    }

    const PROFILE_FINGERPRINT: u64 = 0x246b_f2af_9719_e465;

    fn confined() -> Facts {
        Facts {
            status: Some(Status {
                state: STATE_CONFINED,
                profile_version: PROFILE_VERSION,
            }),
            sandboxed: true,
            denied: MUST_BE_DENIED.len(),
            allowed: MUST_BE_ALLOWED.len(),
        }
    }

    /// One row per fact: each missing one refuses the helper with its own reason; the full set is accepted.
    #[test]
    fn judge_refuses_each_missing_fact() {
        let full = confined();
        assert_eq!(judge(&full), Ok(()));
        let status = full.status.unwrap();
        let rows: [(Facts, &str); 8] = [
            (Facts { status: None, ..full }, "no handshake"),
            (
                Facts {
                    status: Some(Status {
                        state: STATE_REFUSED,
                        ..status
                    }),
                    ..full
                },
                "the system refused the helper's profile",
            ),
            (
                Facts {
                    status: Some(Status { state: 7, ..status }),
                    ..full
                },
                "an unknown handshake",
            ),
            (
                Facts {
                    status: Some(Status {
                        profile_version: PROFILE_VERSION + 1,
                        ..status
                    }),
                    ..full
                },
                "another profile than this app's",
            ),
            (
                Facts {
                    sandboxed: false,
                    ..full
                },
                "the kernel reports no sandbox",
            ),
            (
                Facts {
                    denied: MUST_BE_DENIED.len() - 1,
                    ..full
                },
                "the kernel allows what the profile denies",
            ),
            (Facts { denied: 0, ..full }, "the kernel allows what the profile denies"),
            (
                Facts { allowed: 0, ..full },
                "the kernel's answers are not the profile's",
            ),
        ];
        let mut refused = 0;
        for (facts, why) in rows {
            assert_eq!(judge(&facts), Err(SandboxError::NotConfined(why)));
            refused += 1;
        }
        assert_eq!(refused, 8, "the instrument ran every row");
    }

    /// The reader reads real values: this test process has no sandbox, and the kernel allows it every operation the
    /// profile denies. `judge` refuses it even with a handshake that claims confinement.
    #[test]
    fn read_facts_reads_a_process_that_is_not_confined() {
        // SAFETY: takes no pointer.
        let me = unsafe { sys::getpid() };
        let facts = read_facts(me, confined().status);
        assert!(!facts.sandboxed);
        assert_eq!(facts.denied, 0, "nothing is denied to an unconfined process");
        assert_eq!(facts.allowed, MUST_BE_ALLOWED.len());
        assert_eq!(
            judge(&facts),
            Err(SandboxError::NotConfined("the kernel reports no sandbox"))
        );
        assert!(footprint(me).unwrap() > 0, "a running test has a footprint");
        assert!(address_space().unwrap() > 0);
    }

    /// A process identifier that names nothing is not taken for a confined helper.
    #[test]
    fn a_process_that_does_not_exist_is_not_confined() {
        // Above the system's highest process identifier (99998).
        let facts = read_facts(1_000_000, confined().status);
        assert!(judge(&facts).is_err(), "{facts:?}");
    }

    #[test]
    fn the_handshake_and_the_grant_message_keep_their_layout() {
        let status = Status {
            state: STATE_CONFINED,
            profile_version: 9,
        };
        let bytes = status.encode();
        assert_eq!(&bytes[..8], b"BBSBCNF1");
        assert_eq!(
            (
                u32_at(&bytes, 8),
                u32_at(&bytes, 12),
                u32_at(&bytes, 16),
                u32_at(&bytes, 20)
            ),
            (1, 0, 0, 9)
        );
        assert_eq!(Status::decode(&bytes), Some(status));
        assert_eq!(Status::decode(&bytes[..23]), None);
        let mut other = bytes;
        other[0] = b'X';
        assert_eq!(Status::decode(&other), None);

        let grant = grant_message(7, 4096, Access::ReadWrite);
        assert_eq!(&grant[..8], b"BBSBGRT1");
        assert_eq!(
            (u64_at(&grant, 8), u64_at(&grant, 16), u32_at(&grant, 24)),
            (7, 4096, 1)
        );
        assert_eq!(u32_at(&grant_message(7, 4096, Access::Read), 24), 0);
        assert_eq!(u64_at(&hello(1 << 30), 8), 1 << 30);
    }

    /// A helper path that would be searched, or is no file, is refused before anything is made.
    #[test]
    fn a_helper_path_that_is_relative_or_not_a_file_is_refused() {
        let refused = |path: &str| checked_path(Path::new(path)).unwrap_err();
        assert_eq!(refused(""), SandboxError::HelperPath("empty"));
        assert_eq!(refused("helper"), SandboxError::HelperPath("not an absolute path"));
        assert_eq!(refused("./helper"), SandboxError::HelperPath("not an absolute path"));
        assert_eq!(refused("/bin"), SandboxError::HelperPath("not a file"));
        assert_eq!(refused("/no/such/helper"), SandboxError::HelperPath("not a file"));
        assert_eq!(refused("/bin/sh\0x"), SandboxError::HelperPath("holds a NUL"));
        assert!(checked_path(Path::new("/bin/sh")).is_ok());
    }

    /// The exit code of a wait status: the helper's own, the app's kill, another signal.
    #[test]
    fn a_wait_status_becomes_an_exit_code() {
        let process = Process {
            pid: 0,
            ended: Mutex::new(None),
            killed: AtomicBool::new(false),
        };
        assert_eq!(process.code(0), 0);
        assert_eq!(process.code(3 << 8), 3);
        assert_eq!(process.code(10), SIGNALLED + 10, "a bus error");
        assert_eq!(process.code(9), SIGNALLED + 9, "a kill that was not the app's");
        process.killed.store(true, Ordering::SeqCst);
        assert_eq!(process.code(9), KILLED_EXIT_CODE);
        assert_eq!(process.code(2 << 8), 2, "an exit of its own stays its own");
    }
}
