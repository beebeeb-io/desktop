//! macOS, the helper's side of being its app's XPC service (the app's side and the measurements: `macos_xpc.rs`).
//!
//! The system starts the service's process and gives it a sandbox from its one entitlement before its first
//! instruction. The helper's `main` gives its main thread to the system (`xpc_main`, with
//! [`beebeeb_office_sandbox_service_peer`] as the handler of new connections) and runs its work on a second thread,
//! which starts with [`crate::helper::confine`] as every helper does. In a service that call waits here until the
//! app's first message has been handled:
//!
//! 1. The first connection is the document's; every later one is cancelled unanswered (one document per process).
//! 2. Its first message carries the helper's ends of the two pipes and the memory limit. The pipes become standard
//!    input and output, so the helper's protocol runs on them as it does for a helper the app started itself.
//! 3. The resource limits (no core file, no file growth, no new process, address space), then the deny-default
//!    profile is tried. The system refuses a second sandbox (code 1, measured), so the App Sandbox stays the
//!    confinement; were it ever taken, the answer says so and the app holds the service to the profile's whole list.
//! 4. A thread that reads this process's own footprint every 100 ms and ends the process above its limit plus the
//!    sections it has mapped: the app is not allowed to read it (measured).
//! 5. The answer: which confinement this is. The app asks the kernel and the signature before it sends anything.
//!
//! Every later message is a grant: a shared-memory object, kept until [`map`] asks for its number. The connection's
//! end (the app closed the document, or is gone) ends the process with code 0.
#![allow(unsafe_code)]

use std::ffi::{c_int, c_void};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use super::xpc::{
    _xpc_type_dictionary, Block, KEY_GRANT, KEY_MEMORY, KEY_PROFILE, KEY_REGION, KEY_STATE, KEY_STDIN, KEY_STDOUT,
    KEY_WRITE, STATE_APP_SANDBOX, XpcObject, mprotect, own_service, xpc_connection_cancel, xpc_connection_resume,
    xpc_connection_send_message, xpc_connection_set_event_handler, xpc_dictionary_create_reply, xpc_dictionary_dup_fd,
    xpc_dictionary_get_remote_connection, xpc_dictionary_get_uint64, xpc_dictionary_get_value, xpc_get_type,
    xpc_release, xpc_retain, xpc_shmem_map,
};
use super::{
    HELLO_BOUND_MS, MappedInner, POLL, PROFILE_VERSION, STATE_CONFINED, STATE_REFUSED, UNCONFINED_EXIT_CODE,
    apply_limits, apply_profile, end, footprint, has_sandbox, lock, sys,
};
use crate::{Access, SandboxError, over_memory};

/// How long [`map`] waits for a grant the app has named on the pipe to arrive on the connection: the two travel
/// separately, the grant first.
const GRANT_WAIT: Duration = Duration::from_secs(5);

unsafe extern "C" {
    fn dup2(from: c_int, to: c_int) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn abort() -> !;
}

/// A grant that has arrived and is not mapped yet.
struct Arrived {
    wire: u64,
    writable: bool,
    memory: XpcObject,
}

// SAFETY: an XPC object may be used and released from any thread; this value holds one reference to it.
unsafe impl Send for Arrived {}

struct Service {
    started: bool,
    arrived: Vec<Arrived>,
}

static SERVICE: Mutex<Service> = Mutex::new(Service {
    started: false,
    arrived: Vec::new(),
});
static CHANGED: Condvar = Condvar::new();

/// Set by the first connection: the document's.
static TAKEN: AtomicBool = AtomicBool::new(false);

/// The bytes of the sections this process has mapped now: its footprint may be over its limit by that much.
static MAPPED_BYTES: AtomicU64 = AtomicU64::new(0);

/// Whether this process is an XPC service (its executable sits in a service's bundle).
pub(super) fn is_service() -> bool {
    own_service()
}

/// What a helper's `main` asks before anything else: whether this process is its app's XPC service, and so has to
/// give its main thread to the system. Exported by name because the helper's crate reaches this module through
/// the sandbox crate's two helper calls only.
#[unsafe(no_mangle)]
pub extern "C" fn beebeeb_office_sandbox_is_service() -> bool {
    is_service()
}

/// The handler a helper's `main` passes to `xpc_main`: called by the system for every new connection.
///
/// # Safety
///
/// `peer` is the new connection the system hands to an XPC service's connection handler.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn beebeeb_office_sandbox_service_peer(peer: *mut c_void) {
    let first = !TAKEN.swap(true, Ordering::SeqCst);
    // SAFETY: `peer` is a new connection: it is given a handler and resumed, as every connection must be. One that
    // is not the document's is cancelled unanswered.
    unsafe {
        xpc_connection_set_event_handler(peer, if first { &raw const DOCUMENT } else { &raw const OTHER });
        xpc_connection_resume(peer);
        if !first {
            xpc_connection_cancel(peer);
        }
    }
}

static DOCUMENT: Block = Block::new(document_event);
static OTHER: Block = Block::new(other_event);

unsafe extern "C" fn other_event(_block: *const Block, _event: XpcObject) {}

/// Every event of the document's connection: its first message starts the helper, later ones are grants, and
/// anything that is no message is the connection's end.
unsafe extern "C" fn document_event(_block: *const Block, event: XpcObject) {
    // SAFETY: `event` is the object the system hands the handler, alive for this call.
    if unsafe { xpc_get_type(event) } != &raw const _xpc_type_dictionary {
        // The app closed the document, or is gone.
        end(0);
    }
    // SAFETY: `event` is a dictionary; the key is a terminated constant.
    let region = unsafe { xpc_dictionary_get_value(event, KEY_REGION.as_ptr()) };
    if region.is_null() {
        start(event);
        return;
    }
    // SAFETY: as above; the object is kept past this call by its own reference, released when it is mapped.
    let arrived = unsafe {
        Arrived {
            wire: xpc_dictionary_get_uint64(event, KEY_GRANT.as_ptr()),
            writable: xpc_dictionary_get_uint64(event, KEY_WRITE.as_ptr()) == 1,
            memory: xpc_retain(region),
        }
    };
    lock(&SERVICE).arrived.push(arrived);
    CHANGED.notify_all();
}

/// The app's first message: steps 2 to 5 of the module documentation.
fn start(event: XpcObject) {
    if lock(&SERVICE).started {
        return;
    }
    // SAFETY: `event` is a dictionary, alive for this call; the keys are terminated constants. The two descriptors
    // are this process's own from here; each replaces a standard descriptor and is then closed.
    let (memory_bytes, piped) = unsafe {
        let memory_bytes = xpc_dictionary_get_uint64(event, KEY_MEMORY.as_ptr());
        let stdin = xpc_dictionary_dup_fd(event, KEY_STDIN.as_ptr());
        let stdout = xpc_dictionary_dup_fd(event, KEY_STDOUT.as_ptr());
        let piped = stdin >= 0 && stdout >= 0 && dup2(stdin, 0) == 0 && dup2(stdout, 1) == 1;
        for fd in [stdin, stdout] {
            if fd > 2 {
                close(fd);
            }
        }
        (memory_bytes, piped)
    };
    let mut state = STATE_REFUSED;
    if piped && memory_bytes > 0 {
        apply_limits(memory_bytes);
        // SAFETY: takes no pointer.
        let me = unsafe { sys::getpid() };
        if apply_profile() && has_sandbox(me) {
            state = STATE_CONFINED;
        } else if has_sandbox(me) {
            state = STATE_APP_SANDBOX;
        }
    }
    if state != STATE_REFUSED && !watch_own_memory(memory_bytes) {
        state = STATE_REFUSED;
    }
    // SAFETY: the answer to `event`, sent on the connection it came from and released once.
    unsafe {
        let reply = xpc_dictionary_create_reply(event);
        if !reply.is_null() {
            xpc_dictionary_set_u64(reply, KEY_STATE, u64::from(state));
            xpc_dictionary_set_u64(reply, KEY_PROFILE, u64::from(PROFILE_VERSION));
            xpc_connection_send_message(xpc_dictionary_get_remote_connection(event), reply);
            xpc_release(reply);
        }
    }
    if state == STATE_REFUSED {
        // The app ends a service that says this; it does not wait for that.
        std::thread::sleep(Duration::from_millis(200));
        end(c_int::from(UNCONFINED_EXIT_CODE));
    }
    lock(&SERVICE).started = true;
    CHANGED.notify_all();
}

/// Sets one number of a dictionary.
///
/// # Safety
///
/// `dictionary` is a live XPC dictionary.
unsafe fn xpc_dictionary_set_u64(dictionary: XpcObject, key: &std::ffi::CStr, value: u64) {
    // SAFETY: the caller's dictionary; a terminated key.
    unsafe { super::xpc::xpc_dictionary_set_uint64(dictionary, key.as_ptr(), value) };
}

/// The helper's first step in a service: returns once the app's first message has been handled and this process
/// has told the app how it is confined. Ends the process when that does not happen in time (a service nobody
/// started, or a `main` that did not hand its main thread to the system).
pub(super) fn wait_until_started() {
    let deadline = Instant::now() + Duration::from_millis(HELLO_BOUND_MS as u64);
    let mut service = lock(&SERVICE);
    while !service.started {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            end(c_int::from(UNCONFINED_EXIT_CODE));
        }
        service = CHANGED
            .wait_timeout(service, left)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0;
    }
}

/// Whether this process, with `footprint` bytes (or none readable), is over its limit: a footprint that cannot be
/// read is not left unwatched.
fn over(footprint: Option<u64>, limit: u64, mapped: u64) -> bool {
    footprint.is_none_or(|bytes| over_memory(bytes, 0, limit, mapped))
}

/// Starts the thread that ends this process once its footprint passes `limit` plus the sections it has mapped.
/// False when the thread did not start or the footprint cannot be read at all.
fn watch_own_memory(limit: u64) -> bool {
    // SAFETY: takes no pointer.
    let me = unsafe { sys::getpid() };
    if footprint(me).is_none() {
        return false;
    }
    std::thread::Builder::new()
        .name("own-memory-watch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(POLL);
                if over(footprint(me), limit, MAPPED_BYTES.load(Ordering::SeqCst)) {
                    // SAFETY: ends the calling process; takes no pointer.
                    unsafe { abort() };
                }
            }
        })
        .is_ok()
}

/// The helper's side of a grant in a service: waits (a moment, never for long) for the grant numbered `wire`, and
/// maps its memory. The object is released as soon as it is mapped or refused, so a grant maps once. A read-write
/// mapping of a grant the app sent read-only is refused with the system's code 1, as for a descriptor.
pub(super) fn map(wire: u64, len: usize, access: Access) -> Result<MappedInner, SandboxError> {
    map_within(wire, len, access, GRANT_WAIT)
}

/// [`map`], waiting at most `wait` for the grant.
fn map_within(wire: u64, len: usize, access: Access, wait: Duration) -> Result<MappedInner, SandboxError> {
    let failed = |code: i32| SandboxError::Os {
        step: "map",
        code: code as u32,
    };
    let arrived = {
        let deadline = Instant::now() + wait;
        let mut service = lock(&SERVICE);
        loop {
            if let Some(at) = service.arrived.iter().position(|grant| grant.wire == wire) {
                break service.arrived.swap_remove(at);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(failed(sys::ENOENT));
            }
            service = CHANGED
                .wait_timeout(service, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    };
    let mut view: *mut c_void = null_mut();
    // SAFETY: `arrived.memory` is the object kept in `document_event`; a shared-memory object maps at an address
    // the system chooses, anything else maps nothing (length 0). The reference is released once, here.
    let span = unsafe {
        let span = xpc_shmem_map(arrived.memory, &raw mut view);
        xpc_release(arrived.memory);
        span
    };
    if span == 0 || view.is_null() {
        return Err(failed(sys::EBADMSG));
    }
    let wants_write = access == Access::ReadWrite;
    let refusal = if span < len {
        // A length past the section's.
        Some(22)
    } else if wants_write && !arrived.writable {
        Some(1)
    } else {
        None
    };
    if let Some(code) = refusal {
        // SAFETY: the view just mapped, unmapped once.
        unsafe { sys::munmap(view, span) };
        return Err(failed(code));
    }
    if !wants_write && arrived.writable {
        // SAFETY: the view just mapped; only lowers its protection.
        unsafe { mprotect(view, span, sys::PROT_READ) };
    }
    MAPPED_BYTES.fetch_add(span as u64, Ordering::SeqCst);
    Ok(MappedInner {
        view: view.cast(),
        len,
        writable: wants_write,
        span,
        counted: true,
    })
}

/// A service's mapping is gone: its bytes no longer count towards what the footprint may be.
pub(super) fn unmapped(span: usize) {
    MAPPED_BYTES.fetch_sub(span as u64, Ordering::SeqCst);
}

/// Puts a grant where the connection's handler puts one: the tests' stand-in for the app's message.
#[cfg(test)]
pub(super) fn arrive(wire: u64, writable: bool, memory: XpcObject) {
    lock(&SERVICE).arrived.push(Arrived { wire, writable, memory });
    CHANGED.notify_all();
}

#[cfg(test)]
pub(super) fn map_for_test(wire: u64, len: usize, access: Access, wait: Duration) -> Result<MappedInner, SandboxError> {
    map_within(wire, len, access, wait)
}

#[cfg(test)]
pub(super) fn mapped_bytes() -> u64 {
    MAPPED_BYTES.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The service's own watch: over its limit plus what it has mapped, or a footprint that cannot be read.
    #[test]
    fn the_services_own_watch_ends_it_above_its_limit_and_when_it_cannot_read_itself() {
        let limit = 1 << 30;
        assert!(!over(Some(limit), limit, 0), "at the limit is not over it");
        assert!(over(Some(limit + 1), limit, 0));
        assert!(
            !over(Some(limit + 4096), limit, 4096),
            "a mapped section is not the helper's own memory"
        );
        assert!(over(Some(limit + 4097), limit, 4096));
        assert!(
            over(None, limit, 0),
            "a footprint that cannot be read is not left unwatched"
        );
        assert!(!over(Some(0), limit, 0), "the instrument can fail");
        // The instrument reads real values: this process has a footprint.
        // SAFETY: takes no pointer.
        let me = unsafe { sys::getpid() };
        let now = footprint(me).expect("a running test can read its own footprint");
        assert!(now > 0);
        assert!(!over(Some(now), now, 0));
        assert!(over(Some(now), now - 1, 0));
    }

    /// A test binary is no XPC service, so its helper calls take the path of a helper the app started itself.
    #[test]
    fn a_process_outside_a_service_bundle_is_no_service() {
        assert!(!is_service());
        assert!(!beebeeb_office_sandbox_is_service());
    }

    /// A grant that never arrives is refused after the wait, not waited for without end.
    #[test]
    fn a_grant_number_that_was_never_sent_is_not_mapped() {
        let unknown = 0xFFFF_0000_0000_0001u64;
        let wait = Duration::from_millis(60);
        let started = Instant::now();
        let refused = map_within(unknown, 4096, Access::Read, wait).err();
        assert_eq!(
            refused,
            Some(SandboxError::Os {
                step: "map",
                code: sys::ENOENT as u32
            })
        );
        assert!(started.elapsed() >= wait, "it waited for the grant first");
    }
}
