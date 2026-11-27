//! Opens a web address (or a `mailto:` draft) in the default app on a Mac through `NSWorkspace openURL`. Sandbox-safe: no
//! child `open`, no `osascript`, no path registry (the Finder-location opener `macos_fp::open` is for files and
//! cannot open a web address, which is why Settings' web rows did nothing). The address is one `NSURL`, never a command
//! line. Only `https://` and `mailto:` open; `http://`, `file:` and the rest are refused. Logs a refusal in words,
//! never the address.

/// The seam: what the opener asks of the system. The real one is `NSWorkspace`; a test counts the asks.
pub(crate) trait Workspace {
    /// `true`: the system took the address to open it.
    fn open(&self, url: &str) -> bool;
}

/// `NSWorkspace.sharedWorkspace.openURL`.
#[allow(dead_code)] // no caller yet in every build; kept with its tests
pub(crate) struct NsWorkspace;

impl Workspace for NsWorkspace {
    fn open(&self, url: &str) -> bool {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::{NSString, NSURL};
        let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) else {
            return false;
        };
        #[allow(deprecated)]
        NSWorkspace::sharedWorkspace().openURL(&url)
    }
}

/// Whether the macOS opener takes this address: `https://` (with a host) or `mailto:`.
pub(crate) fn is_openable(url: &str) -> bool {
    match url.strip_prefix("https://") {
        Some(rest) => !rest.is_empty() && !rest.starts_with('/'),
        None => url.starts_with("mailto:"),
    }
}

/// Opens `url` through `workspace`. `Err(())`: refused or not opened (said in words in the log).
pub(crate) fn open_with(workspace: &dyn Workspace, url: &str) -> Result<(), ()> {
    if !is_openable(url) {
        crate::shell::host::log("web page: the address is not one the Mac opener takes");
        return Err(());
    }
    if workspace.open(url) {
        Ok(())
    } else {
        crate::shell::host::log("web page: macOS refused to open the address in the browser");
        Err(())
    }
}

/// Opens `url` in the default browser (or mail app).
#[allow(dead_code)] // no caller yet in every build; kept with its tests
pub(crate) fn open_url(url: &str) -> Result<(), ()> {
    open_with(&NsWorkspace, url)
}

#[cfg(test)]
#[path = "macos_open_url_tests.rs"]
mod tests;
