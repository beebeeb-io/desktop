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

use std::ffi::CString;
use std::os::raw::c_char;

use crate::finder_setup::error::{BRIDGE_DOMAIN, FpError, bridge_code};
use crate::macos_file_provider::BeebeebFpErrorC;

unsafe extern "C" {
    fn beebeeb_open_url(url: *const c_char, out_error: *mut BeebeebFpErrorC) -> i32;
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
}
