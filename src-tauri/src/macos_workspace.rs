//! Opening a URL that is not a Finder item, through `NSWorkspace` and not through a child `open` (task 1885).
//!
//! The one caller today is the link to System Settings' Login Items & Extensions pane
//! (`x-apple.systempreferences:`). It used to run `/usr/bin/open <url>` as a child and read the exit status.
//! That reported the real outcome, but it was a second place that spawned `open` on a Mac, and the pin
//! `test_1885_no_macos_code_path_spawns_open` keeps the sandbox bug from returning through any of them.
//!
//! This is not a File Provider call, so it does not take the bridge gate (`finder_setup::macos_ports`):
//! the link has to work exactly when the File Provider is stuck, which is when a person is sent to System
//! Settings. It blocks for LaunchServices' answer, at most 5 s, so callers run it on the blocking pool.

use std::ffi::{CString, c_void};
use std::os::raw::c_char;
use std::ptr::NonNull;

use crate::finder_setup::error::{BRIDGE_DOMAIN, FpError, app_code, bridge_code};
use crate::macos_file_provider::BeebeebFpErrorC;

unsafe extern "C" {
    fn beebeeb_open_url(url: *const c_char, out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_open_scoped_url(handle: *mut c_void, out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_reveal_scoped_url(handle: *mut c_void, out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_release_url_handle(handle: *mut c_void);
}

/// The security-scoped URL macOS returned for a Finder item, as the bridge's retained handle (task 1885, I1). It is
/// made by the GATED resolve (`macos_file_provider::resolve_location` / `resolve_item`), and the gate is released
/// before it is used: [`open_scoped`] and [`reveal_scoped`] are File Provider-free and take it by value. A handle
/// that is not used is released when it drops, so no path leaks it; `beebeeb_test_live_handles` counts them.
pub struct ScopedUrl(NonNull<c_void>);

impl ScopedUrl {
    /// Takes ownership of a handle from the bridge. `None` for NULL.
    pub(crate) fn from_raw(handle: *mut c_void) -> Option<Self> {
        NonNull::new(handle).map(Self)
    }

    /// Gives the handle to a bridge call that consumes (and always releases) it.
    fn into_raw(self) -> *mut c_void {
        let handle = self.0.as_ptr();
        std::mem::forget(self);
        handle
    }
}

impl Drop for ScopedUrl {
    fn drop(&mut self) {
        unsafe { beebeeb_release_url_handle(self.0.as_ptr()) }
    }
}

impl std::fmt::Debug for ScopedUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScopedUrl(..)")
    }
}

/// The bridge's `1` (done) or error answer to an open or a reveal. Any other return is a bridge that does not follow
/// its own contract: an app error, never a guess at success.
fn done_or_error(function: &str, code: i32, error: BeebeebFpErrorC) -> Result<(), FpError> {
    match code {
        1 => Ok(()),
        c if c < 0 => Err(error.into_fp_error()),
        other => Err(FpError::app(
            app_code::UNEXPECTED_BRIDGE_RETURN,
            format!("{function} returned {other}"),
        )),
    }
}

/// "Open in Finder" (task 1885): opens the scoped URL through `NSWorkspace` and returns when LaunchServices has
/// answered (5 s limit). It consumes the handle whatever happens. It holds no bridge gate.
pub fn open_scoped(url: ScopedUrl) -> Result<(), FpError> {
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe { beebeeb_open_scoped_url(url.into_raw(), &mut out) };
    done_or_error("beebeeb_open_scoped_url", code, out)
}

/// "Show in Finder" for one item (task 1885): checks that the URL is on disk and selects it in a Finder window. Finder's
/// own answer cannot be waited for (`activateFileViewerSelectingURLs:` is `void`), so `Ok` means the item resolved and
/// exists, not that a window was seen: the device check owns that. It consumes the handle whatever happens.
pub fn reveal_scoped(url: ScopedUrl) -> Result<(), FpError> {
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe { beebeeb_reveal_scoped_url(url.into_raw(), &mut out) };
    done_or_error("beebeeb_reveal_scoped_url", code, out)
}

/// Opens `url` and returns when LaunchServices has answered. `Err` carries the OS's refusal (domain and code
/// for a person, the message for the log) or the bridge's own `OpenTimeout`. A file URL is refused by the
/// bridge: those open only through a Finder item's scoped URL.
pub fn open_url(url: &str) -> Result<(), FpError> {
    let Ok(c_url) = CString::new(url) else {
        return Err(FpError::new(
            BRIDGE_DOMAIN,
            bridge_code::INVALID_URL,
            "the URL holds a NUL byte",
        ));
    };
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe { beebeeb_open_url(c_url.as_ptr(), &mut out) };
    if code < 0 { Err(out.into_fp_error()) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    /// A URL the bridge refuses before it asks the system anything: it does not parse, has no scheme, holds a
    /// NUL, or is a file URL. Each is the bridge's `InvalidURL`, never an `Ok`.
    #[test]
    fn test_1885_a_url_the_bridge_may_not_open_is_an_error_before_the_system_is_asked() {
        for url in [
            "",
            "not a url",
            "no-scheme",
            "file:///tmp/anything",
            "x-apple.systempreferences:\0",
        ] {
            let error = open_url(url).expect_err(url);
            assert_eq!(
                (error.domain.as_str(), error.code),
                (BRIDGE_DOMAIN, bridge_code::INVALID_URL),
                "{url:?}"
            );
        }
    }

    /// No false success, against the real `NSWorkspace`. A scheme nothing on this Mac handles: LaunchServices
    /// answers with an error (`kLSApplicationNotFoundErr`) without opening anything or showing a dialog
    /// (`promptsUserIfNeeded = NO`), and that error must reach the caller. A bridge that ignored the
    /// completion handler's error, or answered before the handler ran, would return `Ok` here.
    #[test]
    fn test_1885_an_open_launch_services_refuses_is_an_error_not_a_success() {
        let error = open_url("beebeeb-no-such-scheme-1885:nothing-opens-this")
            .expect_err("LaunchServices has no application for this scheme");
        assert_ne!(
            (error.domain.as_str(), error.code),
            (BRIDGE_DOMAIN, bridge_code::INVALID_URL),
            "the URL is valid; the refusal is LaunchServices': {error}"
        );
        assert_ne!(
            (error.domain.as_str(), error.code),
            (BRIDGE_DOMAIN, bridge_code::OPEN_TIMEOUT),
            "LaunchServices answered inside the limit: {error}"
        );
        assert!(
            !error.domain.is_empty() && error.code != 0,
            "the OS's error crosses as domain and code: {error}"
        );
    }

    unsafe extern "C" {
        fn beebeeb_test_url_handle(url: *const c_char) -> *mut c_void;
        fn beebeeb_test_live_handles() -> i64;
    }

    fn handle_for(url: &str) -> ScopedUrl {
        let url = CString::new(url).unwrap();
        ScopedUrl::from_raw(unsafe { beebeeb_test_url_handle(url.as_ptr()) }).expect("a URL that parses")
    }

    /// Task 1885 fix round 1 (I1): the handle is released on every way out. Dropping one that was never used
    /// releases it, and a reveal of an item that is not on disk fails in the bridge (before `NSWorkspace` is asked
    /// anything: it never opens or selects anything) with the bridge's `ItemMissing`, having consumed its handle.
    #[test]
    fn test_1885_a_handle_is_released_unused_and_by_a_reveal_that_fails() {
        let _serial = tests_support::serialize_handles();
        let before = unsafe { beebeeb_test_live_handles() };

        let unused = handle_for("https://example.invalid/");
        assert_eq!(
            unsafe { beebeeb_test_live_handles() },
            before + 1,
            "a handle is in flight"
        );
        drop(unused);
        assert_eq!(
            unsafe { beebeeb_test_live_handles() },
            before,
            "dropping it released it"
        );

        let missing = handle_for("file:///beebeeb-1885-this-folder-does-not-exist/nor-this-file");
        assert_eq!(unsafe { beebeeb_test_live_handles() }, before + 1);
        let error = reveal_scoped(missing).expect_err("a path that is not on disk is not revealed");
        assert_eq!(
            (error.domain.as_str(), error.code),
            (BRIDGE_DOMAIN, bridge_code::ITEM_MISSING),
            "{error}"
        );
        assert_eq!(
            unsafe { beebeeb_test_live_handles() },
            before,
            "the reveal consumed its handle on the error path"
        );
    }

    /// Task 1885 fix round 1 (I1), against the real `NSWorkspace` (like the test above for `open_url`): the ungated
    /// open takes the scoped-URL path, and its refusal reaches the caller with the handle released. A file that
    /// does not exist has no application: nothing opens and no dialog shows.
    #[test]
    #[ignore = "asks the real NSWorkspace, which answers only inside a logged-in GUI session; run with --ignored on a Mac you are sitting at"]
    fn test_1885_an_ungated_open_that_launch_services_refuses_is_an_error_and_releases_its_handle() {
        let _serial = tests_support::serialize_handles();
        let before = unsafe { beebeeb_test_live_handles() };
        let url = handle_for("beebeeb-no-such-scheme-1885:nothing-opens-this");
        let error = open_scoped(url).expect_err("LaunchServices has no application for this scheme");
        assert_ne!(
            (error.domain.as_str(), error.code),
            (BRIDGE_DOMAIN, bridge_code::OPEN_TIMEOUT),
            "LaunchServices answered inside the limit: {error}"
        );
        assert_eq!(
            unsafe { beebeeb_test_live_handles() },
            before,
            "the open consumed its handle"
        );
    }
}

/// Tests that count the bridge's live URL handles share one counter, so they take turns.
#[cfg(test)]
pub(crate) mod tests_support {
    use std::sync::{Mutex, MutexGuard};

    static HANDLES: Mutex<()> = Mutex::new(());

    pub(crate) fn serialize_handles() -> MutexGuard<'static, ()> {
        HANDLES.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
