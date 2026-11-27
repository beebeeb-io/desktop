//! The C side of the File Provider bridge: the declarations of `packaging/macos/FileProviderBridge.m`, the zeroed
//! error struct, and the decoding of what it answers (error struct to `FpError`, a count, a URL handle, the kept
//! state of a removal), plus the one-per-process gate every File Provider call goes through. Must not decide what
//! Finder shows, and never logs: an `FpError` message is the OS's own prose and may name a file.

use std::ffi::{c_void, CString};
use std::os::raw::c_char;
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use beebeeb_engine::file_provider_port::WorkingSetSignalOutcome;
use beebeeb_engine::finder_setup::core::DomainState;
use beebeeb_engine::finder_setup::error::{app_code, FpError, FpErrorCode};

/// Mirror of `BeebeebFpError` in the bridge. The C side `_Static_assert`s the same size and offsets, and the tests
/// compare `sizeof`, `alignof` and every `offsetof` exported from Objective-C against this struct.
#[repr(C)]
pub(crate) struct BeebeebFpErrorC {
    pub code: i64,
    pub underlying_code: i64,
    pub has_underlying: i32,
    pub domain: [c_char; 128],
    pub underlying_domain: [c_char; 128],
    pub message: [c_char; 1024],
}

const _: () = assert!(std::mem::size_of::<BeebeebFpErrorC>() == 1304);

impl BeebeebFpErrorC {
    /// Every call starts from an all-zero struct, so a bridge path that fails without filling the error is seen as
    /// "no domain" and becomes an app error, never as stale or garbage data.
    pub fn zeroed() -> Self {
        Self {
            code: 0,
            underlying_code: 0,
            has_underlying: 0,
            domain: [0; 128],
            underlying_domain: [0; 128],
            message: [0; 1024],
        }
    }

    pub fn into_fp_error(self) -> FpError {
        let domain = c_array_to_string(&self.domain);
        if domain.is_empty() {
            return FpError::app(
                app_code::UNEXPECTED_BRIDGE_RETURN,
                "the File Provider bridge failed without describing the error",
            );
        }
        FpError {
            domain,
            code: self.code,
            message: c_array_to_string(&self.message),
            underlying: (self.has_underlying != 0).then(|| FpErrorCode {
                domain: c_array_to_string(&self.underlying_domain),
                code: self.underlying_code,
            }),
        }
    }
}

/// Bounded read: never past the array, even if the C side forgot the NUL.
fn c_array_to_string(array: &[c_char]) -> String {
    let bytes: Vec<u8> = array.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

// The one set of bridge declarations. `removal.rs` calls the two remove functions through here (dm-24 once declared
// its own copy).
#[allow(dead_code)]
unsafe extern "C" {
    fn beebeeb_fp_resolve_location(out_handle: *mut *mut c_void, out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_resolve_item(
        identifier: *const c_char,
        out_handle: *mut *mut c_void,
        out_error: *mut BeebeebFpErrorC,
    ) -> i32;
    fn beebeeb_release_url_handle(handle: *mut c_void);
    fn beebeeb_fp_begin_item_access(handle: *mut c_void, path_buffer: *mut c_char, path_buffer_len: usize) -> i32;
    fn beebeeb_fp_end_item_access(handle: *mut c_void);
    fn beebeeb_fp_domain_user_enabled(out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_add_domain(out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_wait_for_domain_ready(timeout_seconds: f64, out_error: *mut BeebeebFpErrorC) -> i32;
    pub(super) fn beebeeb_fp_remove(
        location_buffer: *mut c_char,
        location_buffer_len: usize,
        kept_state: *mut i32,
        out_error: *mut BeebeebFpErrorC,
    ) -> i32;
    pub(super) fn beebeeb_fp_remove_domain_by_id(
        identifier: *const c_char,
        location_buffer: *mut c_char,
        location_buffer_len: usize,
        kept_state: *mut i32,
        out_error: *mut BeebeebFpErrorC,
    ) -> i32;
    fn beebeeb_fp_kept_folder_state_for_path(path: *const c_char) -> i32;
    fn beebeeb_fp_signal_working_set(out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_reimport(out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_list_domains(ids: *mut c_char, len: usize, out_error: *mut BeebeebFpErrorC) -> i32;
}

/// A retained, opaque `NSURL` the bridge handed out. It never becomes a path (a path carries no security scope).
/// Dropping it releases it; whoever opens it hands it back to the bridge instead (dm-24).
#[derive(Debug)]
pub(crate) struct UrlHandle(std::ptr::NonNull<c_void>);

// SAFETY: the handle is a retained NSURL, which is immutable and thread-safe to retain and release.
unsafe impl Send for UrlHandle {}

impl UrlHandle {
    /// Takes ownership of a handle the bridge returned; `None` for NULL.
    pub fn from_raw(raw: *mut c_void) -> Option<Self> {
        std::ptr::NonNull::new(raw).map(Self)
    }
}

impl Drop for UrlHandle {
    fn drop(&mut self) {
        unsafe { beebeeb_release_url_handle(self.0.as_ptr()) }
    }
}

/// What the folder macOS reported after a removal holds (`BeebeebKept*` in the bridge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeptState {
    NoneReported,
    Missing,
    Empty,
    HasEntries,
    Unchecked,
    NoPath,
}

impl KeptState {
    /// A number the bridge does not document reads as "unchecked", never as "nothing kept".
    pub fn from_bridge(number: i32) -> Self {
        match number {
            0 => Self::NoneReported,
            1 => Self::Missing,
            2 => Self::Empty,
            3 => Self::HasEntries,
            5 => Self::NoPath,
            _ => Self::Unchecked,
        }
    }
}

/// The bridge's answer to a resolve: `1` with a handle, `0` (no location), or an error. Every path that does not
/// return the handle drops (releases) it. A `1` without a handle, or any other non-negative return, is a bridge that
/// does not follow its own contract: an app error, never a guess.
#[allow(dead_code)] // dm-24's resolve calls
pub(crate) fn decode_resolve(
    function: &str,
    code: i32,
    handle: Option<UrlHandle>,
    error: BeebeebFpErrorC,
) -> Result<Option<UrlHandle>, FpError> {
    match (code, handle) {
        (1, Some(url)) => Ok(Some(url)),
        (0, _) => Ok(None),
        (code, _) if code < 0 => Err(error.into_fp_error()),
        (other, _) => Err(FpError::app(
            app_code::UNEXPECTED_BRIDGE_RETURN,
            format!("{function} returned {other}"),
        )),
    }
}

/// A count (`beebeeb_fp_list_domains`): negative is the filled error.
#[allow(dead_code)] // dm-23's domain sweep
pub(crate) fn decode_count(function: &str, count: i32, error: BeebeebFpErrorC) -> Result<usize, FpError> {
    match usize::try_from(count) {
        Ok(count) => Ok(count),
        Err(_) if count < 0 && !error.domain.iter().all(|c| *c == 0) => Err(error.into_fp_error()),
        Err(_) => Err(FpError::app(
            app_code::UNEXPECTED_BRIDGE_RETURN,
            format!("{function} returned {count}"),
        )),
    }
}

/// The bridge's own folder check on a path (what the removal runs on the URL macOS returned). A path that cannot
/// be handed over (an interior NUL) reads as unchecked, never as empty.
#[allow(dead_code)] // dm-23's removal decode
pub(crate) fn kept_state_for_path(path: &str) -> KeptState {
    match CString::new(path) {
        Ok(path) => KeptState::from_bridge(unsafe { beebeeb_fp_kept_folder_state_for_path(path.as_ptr()) }),
        Err(_) => KeptState::Unchecked,
    }
}

/// The resolve of one item's Finder location, as a retained handle. An identifier that cannot cross (empty, or
/// holding a NUL) is the bridge's `NoIdentifier` error, never `Ok(None)`. The caller holds the gate.
#[allow(dead_code)] // dm-24's reveal
pub(crate) fn resolve_item(item_id: &str) -> Result<Option<UrlHandle>, FpError> {
    use beebeeb_engine::finder_setup::error::{bridge_code, BRIDGE_DOMAIN};
    let Ok(identifier) = CString::new(item_id) else {
        return Err(FpError::new(
            BRIDGE_DOMAIN,
            bridge_code::NO_IDENTIFIER,
            "the item identifier holds a NUL byte",
        ));
    };
    let mut handle: *mut c_void = std::ptr::null_mut();
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe { beebeeb_fp_resolve_item(identifier.as_ptr(), &mut handle, &mut out) };
    decode_resolve("beebeeb_fp_resolve_item", code, UrlHandle::from_raw(handle), out)
}

/// Dm-72: Finder's own copy of the item `item_id`, for the app's own read of it. The system is asked for the item's
/// place (through the gate, waited for briefly: a burst of reads takes turns); `read` is then called once with its
/// path, outside the gate, while the security scope of the URL the system returned is held. `false`: no place (the
/// item is not in Finder, the gate stayed busy, the system failed) and `read` was not called. Never logs the path.
pub(crate) fn with_finder_copy(item_id: &str, read: &mut dyn FnMut(&std::path::Path)) -> bool {
    let resolved = match Gate::enter_waiting(FINDER_COPY_GATE_WAIT) {
        Ok(_gate) => resolve_item(item_id),
        Err(_) => return false,
    };
    match resolved {
        Ok(Some(url)) => read_at_url(&url, read),
        Ok(None) | Err(_) => false,
    }
}

/// How long a read of Finder's copy waits for the gate. Short: the caller falls back to Beebeeb's version.
const FINDER_COPY_GATE_WAIT: Duration = Duration::from_millis(250);

/// The second half of [`with_finder_copy`]: the handle's path, its scope held around `read`. No File Provider call.
fn read_at_url(url: &UrlHandle, read: &mut dyn FnMut(&std::path::Path)) -> bool {
    use std::os::unix::ffi::OsStrExt;
    /// Ends the scope when the read is over, a panic in it included.
    struct Scope(*mut c_void);
    impl Drop for Scope {
        fn drop(&mut self) {
            unsafe { beebeeb_fp_end_item_access(self.0) }
        }
    }
    let mut buffer = vec![0 as c_char; 4096];
    // SAFETY: the handle is a live retained NSURL, and the buffer's length is the one passed.
    let started = unsafe { beebeeb_fp_begin_item_access(url.0.as_ptr(), buffer.as_mut_ptr(), buffer.len()) };
    if started < 0 {
        return false;
    }
    let _scope = (started == 1).then(|| Scope(url.0.as_ptr()));
    let bytes: Vec<u8> = buffer.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    read(std::path::Path::new(std::ffi::OsStr::from_bytes(&bytes)));
    true
}

static GATE: Mutex<()> = Mutex::new(());

/// Tests that take the process-wide gate run one at a time (the gate is one per process; tests run in parallel).
#[cfg(test)]
pub(crate) static GATE_TESTS: Mutex<()> = Mutex::new(());

/// The one File Provider call in flight per process. A call that finds it taken is told so at once
/// ([`beebeeb_engine::file_provider_port::is_gate_busy`]) and owes its work to the next tick; it never queues behind
/// a call that waits on the system.
pub(crate) struct Gate(#[allow(dead_code)] MutexGuard<'static, ()>); // the guard is held for its drop

impl Gate {
    pub fn try_enter() -> Result<Self, FpError> {
        match GATE.try_lock() {
            Ok(guard) => Ok(Self(guard)),
            Err(TryLockError::Poisoned(poisoned)) => Ok(Self(poisoned.into_inner())),
            Err(TryLockError::WouldBlock) => Err(FpError::app(
                app_code::OP_TIMEOUT,
                "an earlier File Provider call has not returned",
            )),
        }
    }
}

impl Gate {
    /// Like [`Gate::try_enter`], but a busy gate is waited for, at most `wait` (polling on this thread: call it on a
    /// blocking thread, never on the async executor). For the calls that must not be lost to one merely in flight.
    pub fn enter_waiting(wait: Duration) -> Result<Self, FpError> {
        let deadline = Instant::now() + wait;
        loop {
            match Self::try_enter() {
                Ok(gate) => return Ok(gate),
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
}

/// Run one bridge primitive; a negative return carries the filled error.
fn call(function: unsafe extern "C" fn(*mut BeebeebFpErrorC) -> i32) -> Result<i32, FpError> {
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe { function(&mut out) };
    if code >= 0 {
        Ok(code)
    } else {
        Err(out.into_fp_error())
    }
}

/// The domain's `userEnabled`, read from the OS's live registry. `NotRegistered` is a fresh install or the state after
/// a clean removal. The caller holds the gate.
pub(crate) fn domain_state() -> Result<DomainState, FpError> {
    decode_domain_state(call(beebeeb_fp_domain_user_enabled))
}

/// Dm-87: macOS gives the domain's root a place, and that folder is on disk: the domain exists and the system serves
/// it (RC13: a shell read of a dataless file worked while setup waited for stabilization). `getUserVisibleURL` on the
/// root, then a `stat` of its path while the URL's scope is held; it lists nothing and opens nothing. The caller
/// holds the gate. Never logs the path.
pub(crate) fn root_in_place() -> Result<bool, FpError> {
    let mut handle: *mut c_void = std::ptr::null_mut();
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe { beebeeb_fp_resolve_location(&mut handle, &mut out) };
    let Some(url) = decode_resolve("beebeeb_fp_resolve_location", code, UrlHandle::from_raw(handle), out)? else {
        return Ok(false);
    };
    let mut there = false;
    read_at_url(&url, &mut |path| {
        there = std::fs::metadata(path).is_ok_and(|meta| meta.is_dir());
    });
    Ok(there)
}

/// The bridge's `1` / `0` / `2` as a state; any other answer is an app error, never a guess.
fn decode_domain_state(answer: Result<i32, FpError>) -> Result<DomainState, FpError> {
    match answer? {
        1 => Ok(DomainState::Enabled),
        0 => Ok(DomainState::Disabled),
        2 => Ok(DomainState::NotRegistered),
        other => Err(FpError::app(
            app_code::UNEXPECTED_BRIDGE_RETURN,
            format!("beebeeb_fp_domain_user_enabled returned {other}"),
        )),
    }
}

/// `addDomain` only; it does not wait for the domain to stabilise. The caller holds the gate.
pub(crate) fn add_domain() -> Result<(), FpError> {
    call(beebeeb_fp_add_domain).map(|_| ())
}

/// Waits for the domain to be usable, at most `timeout` on the bridge's own clock. NOT under the gate: it is a test helper that waits for the system to catch
/// up with every change, so holding the gate would starve the working-set signals and the lock.
pub(crate) fn wait_for_domain_ready(timeout: Duration) -> Result<(), FpError> {
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe { beebeeb_fp_wait_for_domain_ready(timeout.as_secs_f64(), &mut out) };
    if code < 0 {
        Err(out.into_fp_error())
    } else {
        Ok(())
    }
}

/// Whether the File Provider extension has connected to the IPC socket (the engine sets it on the first accepted peer).
pub(crate) fn extension_connected() -> bool {
    beebeeb_engine::ipc_socket::peer_connected()
}

/// One best-effort working-set signal, through the gate. `0` is signaled, or the domain is not registered (nothing to
/// signal is not a failure). A failure is returned for the caller to count; it never fails the sync tick.
pub(crate) fn signal_working_set() -> Result<WorkingSetSignalOutcome, FpError> {
    let _gate = Gate::try_enter()?;
    match call(beebeeb_fp_signal_working_set)? {
        0 => Ok(WorkingSetSignalOutcome::Delivered),
        other => Err(FpError::app(
            app_code::UNEXPECTED_BRIDGE_RETURN,
            format!("beebeeb_fp_signal_working_set returned {other}"),
        )),
    }
}

/// dm-41: ask the system to list the whole tree again (`reimportItemsBelowItemWithIdentifier:` root), through the gate.
/// Called once when the stored db identity or app generation changes. Never fails the caller's start.
pub(crate) fn reimport() -> Result<(), FpError> {
    let _gate = Gate::try_enter()?;
    match call(beebeeb_fp_reimport)? {
        0 => Ok(()),
        other => Err(FpError::app(
            app_code::UNEXPECTED_BRIDGE_RETURN,
            format!("beebeeb_fp_reimport returned {other}"),
        )),
    }
}

#[cfg(test)]
mod bridge_error_tests {
    use super::*;
    use beebeeb_engine::finder_setup::error::{app_code, bridge_code, FpErrorCode, APP_DOMAIN, BRIDGE_DOMAIN};
    use std::ffi::CString;
    use std::mem::{align_of, offset_of, size_of};

    unsafe extern "C" {
        fn beebeeb_fp_test_fill_error(
            domain: *const c_char,
            code: i64,
            message: *const c_char,
            underlying_domain: *const c_char,
            underlying_code: i64,
            out_error: *mut BeebeebFpErrorC,
        );
        fn beebeeb_fp_test_fill_bridge_error(code: i64, message: *const c_char, out_error: *mut BeebeebFpErrorC);
        fn beebeeb_fp_test_error_size() -> usize;
        fn beebeeb_fp_test_error_align() -> usize;
        /// code, underlying_code, has_underlying, domain, underlying_domain, message.
        fn beebeeb_fp_test_error_offsets(out: *mut usize);
        /// ManagerUnavailable, StabilizationTimeout, ResolveUrlTimeout, SignalTimeout, NoIdentifier,
        /// OpenTimeout, InvalidURL, ItemMissing.
        fn beebeeb_fp_test_bridge_codes(out: *mut i64);
        // The one real bridge entry point that fails before it touches the system (task 1882: its
        // removal reply carries the kept folder too).
        fn beebeeb_fp_remove_domain_by_id(
            identifier: *const c_char,
            location_buffer: *mut c_char,
            location_buffer_len: usize,
            kept_state: *mut i32,
            out_error: *mut BeebeebFpErrorC,
        ) -> i32;
    }

    fn through_bridge(domain: &str, code: i64, message: &str, underlying: Option<(&str, i64)>) -> FpError {
        let domain = CString::new(domain).unwrap();
        let message = CString::new(message).unwrap();
        let underlying_domain = underlying.map(|(d, _)| CString::new(d).unwrap());
        let mut out = BeebeebFpErrorC::zeroed();
        unsafe {
            beebeeb_fp_test_fill_error(
                domain.as_ptr(),
                code,
                message.as_ptr(),
                underlying_domain.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
                underlying.map_or(0, |(_, c)| c),
                &mut out,
            );
        }
        out.into_fp_error()
    }

    #[test]
    fn the_error_struct_has_the_same_size_on_both_sides() {
        assert_eq!(std::mem::size_of::<BeebeebFpErrorC>(), 1304);
        assert_eq!(
            unsafe { beebeeb_fp_test_error_size() },
            std::mem::size_of::<BeebeebFpErrorC>()
        );
    }

    #[test]
    fn every_field_sits_at_the_same_offset_on_both_sides() {
        let mut c_offsets = [usize::MAX; 6];
        unsafe { beebeeb_fp_test_error_offsets(c_offsets.as_mut_ptr()) };
        let rust_offsets = [
            offset_of!(BeebeebFpErrorC, code),
            offset_of!(BeebeebFpErrorC, underlying_code),
            offset_of!(BeebeebFpErrorC, has_underlying),
            offset_of!(BeebeebFpErrorC, domain),
            offset_of!(BeebeebFpErrorC, underlying_domain),
            offset_of!(BeebeebFpErrorC, message),
        ];
        assert_eq!(c_offsets, rust_offsets, "ObjC offsetof vs Rust offset_of!");
        assert_eq!(rust_offsets, [0, 8, 16, 20, 148, 276], "the layout the spec describes");
        assert_eq!(unsafe { beebeeb_fp_test_error_align() }, align_of::<BeebeebFpErrorC>());
        assert_eq!(size_of::<BeebeebFpErrorC>() % align_of::<BeebeebFpErrorC>(), 0);
    }

    #[test]
    fn the_bridge_error_codes_are_the_rust_constants() {
        let mut c_codes = [i64::MIN; 8];
        unsafe { beebeeb_fp_test_bridge_codes(c_codes.as_mut_ptr()) };
        assert_eq!(
            c_codes,
            [
                bridge_code::MANAGER_UNAVAILABLE,
                bridge_code::STABILIZATION_TIMEOUT,
                bridge_code::RESOLVE_URL_TIMEOUT,
                bridge_code::SIGNAL_TIMEOUT,
                bridge_code::NO_IDENTIFIER,
                bridge_code::OPEN_TIMEOUT,
                bridge_code::INVALID_URL,
                bridge_code::ITEM_MISSING,
            ],
            "BeebeebBridge* enum in FileProviderBridge.m vs finder_setup::error::bridge_code"
        );
        assert_eq!(c_codes, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    /// Task 1885: a resolve with no usable identifier fails in the bridge before it touches the system, with the
    /// bridge's own `NoIdentifier` error. It is an `Err`, never `Ok(None)` ("macOS has no location"), which would
    /// read as a quiet nothing.
    #[test]
    fn test_1885_a_resolve_without_an_identifier_is_a_bridge_error_not_a_quiet_nothing() {
        for identifier in ["", "has\0a-nul"] {
            let error = resolve_item(identifier).expect_err("no identifier is an error");
            assert_eq!(
                (error.domain.as_str(), error.code),
                (BRIDGE_DOMAIN, bridge_code::NO_IDENTIFIER),
                "{identifier:?}"
            );
        }
    }

    /// The tests that make URL handles run one at a time (the live-handle count is process-wide).
    static HANDLE_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Dm-72: the app's read of Finder's copy. A file URL's path reaches the read, which reads the file there; a URL
    /// that is no file reads nothing; the handle stays the caller's and is released once.
    #[test]
    fn a_file_urls_path_reaches_the_read_and_a_url_that_is_no_file_reads_nothing() {
        unsafe extern "C" {
            fn beebeeb_test_url_handle(url: *const c_char) -> *mut c_void;
            fn beebeeb_test_live_handles() -> i64;
        }
        let _serial = HANDLE_TESTS.lock().unwrap_or_else(|p| p.into_inner());
        let before = unsafe { beebeeb_test_live_handles() };
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("photo.jpg");
        std::fs::write(&file, b"finder's copy").unwrap();
        let handle = |text: String| {
            let url = CString::new(text).unwrap();
            UrlHandle::from_raw(unsafe { beebeeb_test_url_handle(url.as_ptr()) }).expect("a handle")
        };

        let mut read = None;
        let found = read_at_url(&handle(format!("file://{}", file.display())), &mut |path| {
            read = std::fs::read(path).ok()
        });
        assert!(found);
        assert_eq!(read.as_deref(), Some(&b"finder's copy"[..]));

        let mut called = false;
        let found = read_at_url(&handle("https://example.invalid/".into()), &mut |_| called = true);
        assert!(!found && !called, "a URL that is no file is never read");
        assert_eq!(unsafe { beebeeb_test_live_handles() }, before, "no handle leaks");
    }

    /// The bridge's resolve returns only `1` with a handle, `0`, or an error. A `1` without a handle and any other
    /// return are app errors; a handle on any path that does not return it is released (the count of live handles is
    /// back where it started).
    #[test]
    fn only_a_one_with_a_handle_or_a_zero_is_an_answer_and_no_handle_leaks() {
        unsafe extern "C" {
            fn beebeeb_test_url_handle(url: *const c_char) -> *mut c_void;
            fn beebeeb_test_live_handles() -> i64;
        }
        // The live-handle count is process-wide: serialize with the other test that makes handles.
        let _serial = HANDLE_TESTS.lock().unwrap_or_else(|p| p.into_inner());
        let handle = || {
            let url = CString::new("https://example.invalid/").unwrap();
            UrlHandle::from_raw(unsafe { beebeeb_test_url_handle(url.as_ptr()) })
        };
        let live = || unsafe { beebeeb_test_live_handles() };
        let before = live();

        assert!(matches!(
            decode_resolve("f", 1, handle(), BeebeebFpErrorC::zeroed()),
            Ok(Some(_))
        ));
        assert_eq!(live(), before, "a returned handle was dropped by the assert above");
        assert!(matches!(
            decode_resolve("f", 0, None, BeebeebFpErrorC::zeroed()),
            Ok(None)
        ));
        for (code, with_handle) in [(1, false), (2, true), (i32::MAX, true), (0, true)] {
            let answer = decode_resolve("f", code, with_handle.then(handle).flatten(), BeebeebFpErrorC::zeroed());
            if code == 0 {
                assert!(
                    matches!(answer, Ok(None)),
                    "a zero is no location, whatever it came with"
                );
            } else {
                let error = answer.expect_err("an undocumented answer is an error");
                assert_eq!(
                    (error.domain.as_str(), error.code),
                    (APP_DOMAIN, app_code::UNEXPECTED_BRIDGE_RETURN),
                    "code {code}"
                );
            }
            assert_eq!(
                live(),
                before,
                "code {code}: the handle that did not come back was released"
            );
        }
    }

    #[test]
    fn a_removal_reply_decodes_its_kept_state_and_a_list_reply_its_count() {
        for (number, state) in [
            (0, KeptState::NoneReported),
            (1, KeptState::Missing),
            (2, KeptState::Empty),
            (3, KeptState::HasEntries),
            (4, KeptState::Unchecked),
            (5, KeptState::NoPath),
        ] {
            assert_eq!(KeptState::from_bridge(number), state);
        }
        // A number the bridge does not document is never read as "nothing kept".
        assert_eq!(KeptState::from_bridge(99), KeptState::Unchecked);
        assert_eq!(KeptState::from_bridge(-1), KeptState::Unchecked);
        assert_eq!(decode_count("f", 3, BeebeebFpErrorC::zeroed()), Ok(3));
        assert_eq!(decode_count("f", 0, BeebeebFpErrorC::zeroed()), Ok(0));
        let error = decode_count("f", -1, BeebeebFpErrorC::zeroed()).expect_err("a negative count is an error");
        assert_eq!(
            (error.domain.as_str(), error.code),
            (APP_DOMAIN, app_code::UNEXPECTED_BRIDGE_RETURN)
        );
    }

    #[test]
    fn the_domain_state_reads_one_zero_two_and_nothing_else() {
        assert_eq!(decode_domain_state(Ok(1)), Ok(DomainState::Enabled));
        assert_eq!(decode_domain_state(Ok(0)), Ok(DomainState::Disabled));
        assert_eq!(decode_domain_state(Ok(2)), Ok(DomainState::NotRegistered));
        let error = decode_domain_state(Ok(7)).expect_err("an undocumented answer is an error");
        assert_eq!(
            (error.domain.as_str(), error.code),
            (APP_DOMAIN, app_code::UNEXPECTED_BRIDGE_RETURN)
        );
    }

    #[test]
    fn a_waiting_entry_gets_the_gate_when_it_frees_and_gives_up_when_it_does_not() {
        let _serial = GATE_TESTS.lock().unwrap_or_else(|p| p.into_inner());
        let held = Gate::try_enter().expect("idle");
        let started = Instant::now();
        let error = Gate::enter_waiting(Duration::from_millis(60))
            .err()
            .expect("still held");
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert!(beebeeb_engine::file_provider_port::is_gate_busy(&error));
        drop(held);
        assert!(Gate::enter_waiting(Duration::from_millis(60)).is_ok());
    }

    #[test]
    fn the_gate_answers_busy_at_once_while_a_call_is_in_flight() {
        let _serial = GATE_TESTS.lock().unwrap_or_else(|p| p.into_inner());
        let first = Gate::try_enter().expect("an idle gate lets one in");
        let busy = Gate::try_enter().err().expect("a second call meets a busy gate");
        assert!(beebeeb_engine::file_provider_port::is_gate_busy(&busy));
        drop(first);
        assert!(
            Gate::try_enter().is_ok(),
            "the slot is free again once the call returned"
        );
    }

    #[test]
    fn every_classified_domain_and_code_crosses_the_bridge_unchanged() {
        let cases: &[(&str, i64)] = &[
            ("NSFileProviderErrorDomain", -2001),
            ("NSFileProviderErrorDomain", -2002),
            ("NSFileProviderErrorDomain", -2003),
            ("NSFileProviderErrorDomain", -2004),
            ("NSFileProviderErrorDomain", -2005),
            ("NSFileProviderErrorDomain", -2011),
            ("NSFileProviderErrorDomain", -2012),
            ("NSFileProviderErrorDomain", -2013),
            ("NSFileProviderErrorDomain", -2014),
            ("NSCocoaErrorDomain", 516),
            ("NSPOSIXErrorDomain", 17),
        ];
        for (domain, code) in cases {
            let error = through_bridge(domain, *code, "fixture", None);
            assert_eq!((error.domain.as_str(), error.code), (*domain, *code));
            assert_eq!(error.message, "fixture");
            assert_eq!(error.underlying, None, "{domain} {code}");
        }
    }

    #[test]
    fn the_underlying_error_crosses_with_its_own_domain_and_code() {
        let error = through_bridge("NSCocoaErrorDomain", 516, "exists", Some(("NSPOSIXErrorDomain", 17)));
        assert_eq!(
            error.underlying,
            Some(FpErrorCode {
                domain: "NSPOSIXErrorDomain".into(),
                code: 17
            })
        );
    }

    #[test]
    fn bridge_synthesized_errors_carry_the_bridge_domain() {
        let message =
            CString::new("Timed out waiting for the Beebeeb File Provider domain to become available").unwrap();
        let mut out = BeebeebFpErrorC::zeroed();
        unsafe { beebeeb_fp_test_fill_bridge_error(bridge_code::STABILIZATION_TIMEOUT, message.as_ptr(), &mut out) };
        let error = out.into_fp_error();
        assert_eq!(
            (error.domain.as_str(), error.code),
            (BRIDGE_DOMAIN, bridge_code::STABILIZATION_TIMEOUT)
        );
        assert_eq!(error.underlying, None);
    }

    #[test]
    fn a_real_bridge_entry_point_reports_its_synthesized_code() {
        // No system call happens before the NULL check, so this runs the real call site of
        // BeebeebBridgeNoIdentifier on any machine.
        let mut out = BeebeebFpErrorC::zeroed();
        let code = unsafe {
            beebeeb_fp_remove_domain_by_id(
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut out,
            )
        };
        assert_eq!(code, -1);
        let error = out.into_fp_error();
        assert_eq!(
            (error.domain.as_str(), error.code),
            (BRIDGE_DOMAIN, bridge_code::NO_IDENTIFIER)
        );
        assert!(!error.message.is_empty());
    }

    #[test]
    fn a_long_message_is_cut_not_overflowed() {
        // 6000 UTF-8 bytes into a 1024-byte field: strlcpy may cut inside a code point, which
        // from_utf8_lossy turns into U+FFFD, so count chars, not bytes.
        let error = through_bridge("NSCocoaErrorDomain", 1, &"é".repeat(3000), None);
        assert!(
            error.message.chars().count() <= 1023,
            "{}",
            error.message.chars().count()
        );
        assert_eq!(error.domain, "NSCocoaErrorDomain");
    }

    #[test]
    fn an_overlong_ascii_message_fills_exactly_1023_bytes_and_the_last_byte_is_a_nul() {
        let domain = CString::new("NSCocoaErrorDomain").unwrap();
        let message = CString::new("m".repeat(5000)).unwrap();
        let mut out = BeebeebFpErrorC::zeroed();
        unsafe { beebeeb_fp_test_fill_error(domain.as_ptr(), 1, message.as_ptr(), std::ptr::null(), 0, &mut out) };
        assert_eq!(out.message[1022], b'm' as c_char);
        assert_eq!(out.message[1023], 0, "strlcpy must terminate inside the field");
        assert_eq!(out.into_fp_error().message.len(), 1023);
    }

    #[test]
    fn an_overlong_domain_is_cut_and_terminated_without_touching_its_neighbours() {
        let domain = CString::new("d".repeat(300)).unwrap();
        let message = CString::new("kept").unwrap();
        let mut out = BeebeebFpErrorC::zeroed();
        unsafe { beebeeb_fp_test_fill_error(domain.as_ptr(), 1, message.as_ptr(), std::ptr::null(), 0, &mut out) };
        assert_eq!(out.domain[126], b'd' as c_char);
        assert_eq!(out.domain[127], 0, "strlcpy must terminate inside the field");
        assert!(
            out.underlying_domain.iter().all(|c| *c == 0),
            "the domain overran into underlying_domain"
        );
        let error = out.into_fp_error();
        assert_eq!(error.domain.len(), 127);
        assert_eq!(error.message, "kept");
    }

    #[test]
    fn filling_clears_whatever_the_struct_held_before() {
        let mut out = BeebeebFpErrorC {
            code: -1,
            underlying_code: -1,
            has_underlying: 1,
            domain: [0x7f; 128],
            underlying_domain: [0x7f; 128],
            message: [0x7f; 1024],
        };
        let domain = CString::new("NSCocoaErrorDomain").unwrap();
        let message = CString::new("short").unwrap();
        unsafe { beebeeb_fp_test_fill_error(domain.as_ptr(), 4, message.as_ptr(), std::ptr::null(), 0, &mut out) };
        assert_eq!((out.has_underlying, out.underlying_code), (0, 0));
        assert!(
            out.underlying_domain.iter().all(|c| *c == 0),
            "stale underlying_domain survived the fill"
        );
        assert!(
            out.message[6..].iter().all(|c| *c == 0),
            "stale message bytes survived the fill"
        );
        let error = out.into_fp_error();
        assert_eq!(
            (error.domain.as_str(), error.code, error.message.as_str()),
            ("NSCocoaErrorDomain", 4, "short")
        );
        assert_eq!(error.underlying, None);
    }

    #[test]
    fn a_missing_nul_is_read_only_up_to_the_end_of_its_own_array() {
        let mut out = BeebeebFpErrorC::zeroed();
        out.code = 9;
        out.domain = [b'a' as c_char; 128];
        out.underlying_domain = [b'b' as c_char; 128];
        out.message = [b'm' as c_char; 1024];
        let error = out.into_fp_error();
        assert_eq!(
            error.domain,
            "a".repeat(128),
            "a read past the array would append the b bytes"
        );
        assert_eq!(error.message, "m".repeat(1024));
    }

    #[test]
    fn an_unfilled_error_becomes_an_app_error_not_an_empty_one() {
        let error = BeebeebFpErrorC::zeroed().into_fp_error();
        assert_eq!(
            (error.domain.as_str(), error.code),
            (APP_DOMAIN, app_code::UNEXPECTED_BRIDGE_RETURN)
        );
    }
}
