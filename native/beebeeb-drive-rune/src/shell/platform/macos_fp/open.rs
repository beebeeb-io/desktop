//! Open in Finder (the Beebeeb location) and Show in Finder (one item), ported from `desktop-v0.8.25`
//! `finder_open.rs` and `macos_workspace.rs` (`docs/macos/file-provider-bridge.md`). The resolve is a File Provider
//! call and runs under the bridge gate; the gate is let go BEFORE the open, which goes through `NSWorkspace` with the
//! scoped URL the bridge handed out (never a path, never a child `open`) and can stall for seconds. A shared-with-me
//! row is never asked of the File Provider (ruling 1701). Must not decide what Finder shows. Never logs a path, a name
//! or the OS's error text.

// Open in Finder and Show in Finder are not wired to a press in every build yet; kept with their tests.
#![allow(dead_code)]

use std::ffi::{c_void, CString};
use std::ptr::NonNull;

use beebeeb_engine::finder_setup::error::{app_code, bridge_code, FpError, BRIDGE_DOMAIN};
use beebeeb_engine::state_db::Namespace;

use super::bridge::{BeebeebFpErrorC, Gate};
use super::ports::BRIDGE_GATE_WAIT;

unsafe extern "C" {
    fn beebeeb_fp_resolve_location(out_handle: *mut *mut c_void, out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_fp_resolve_item(
        identifier: *const std::os::raw::c_char,
        out_handle: *mut *mut c_void,
        out_error: *mut BeebeebFpErrorC,
    ) -> i32;
    fn beebeeb_open_scoped_url(handle: *mut c_void, out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_reveal_scoped_url(handle: *mut c_void, out_error: *mut BeebeebFpErrorC) -> i32;
    fn beebeeb_release_url_handle(handle: *mut c_void);
    /// A NON-file URL (System Settings' pane) through `NSWorkspace`; 0 = opened. No File Provider call, no gate.
    fn beebeeb_open_url(url_string: *const std::os::raw::c_char, out_error: *mut BeebeebFpErrorC) -> i32;
}

/// System Settings › General › Login Items & Extensions.
pub(crate) const LOGIN_ITEMS_URL: &str = "x-apple.systempreferences:com.apple.LoginItems-Settings.extension";

/// Opens a System Settings address through the bridge's `NSWorkspace`.
pub(crate) fn open_system_url(url: &str) -> Result<(), FpError> {
    let Ok(text) = CString::new(url) else {
        return Err(FpError::new(BRIDGE_DOMAIN, bridge_code::NO_IDENTIFIER, "the address holds a NUL byte"));
    };
    let mut out = BeebeebFpErrorC::zeroed();
    let code = unsafe { beebeeb_open_url(text.as_ptr(), &mut out) };
    match code {
        0 => Ok(()),
        c if c < 0 => Err(out.into_fp_error()),
        other => Err(FpError::app(
            app_code::UNEXPECTED_BRIDGE_RETURN,
            format!("beebeeb_open_url returned {other}"),
        )),
    }
}

/// The sentences a person is told (verbatim from the tag's `finder_open.rs` and the engine's `finder_show.rs`).
pub(crate) const NOTHING_TO_OPEN: &str = "Beebeeb isn’t in Finder right now, so there is nothing to open there.";
pub(crate) const NOTHING_TO_SHOW: &str = "Beebeeb isn’t in Finder right now, so there is nothing to show there.";
pub(crate) use beebeeb_engine::finder_show::{ITEM_NOT_KNOWN, SHARED_NOT_IN_FINDER};

/// The security-scoped URL macOS returned for a Finder item, as the bridge's retained handle. A handle that is not
/// used is released when it drops, so no path leaks it.
pub(crate) struct ScopedUrl(NonNull<c_void>);

// SAFETY: a retained NSURL is immutable and thread-safe to retain and release.
unsafe impl Send for ScopedUrl {}

impl ScopedUrl {
    fn from_raw(raw: *mut c_void) -> Option<Self> {
        NonNull::new(raw).map(Self)
    }
    /// Gives the handle to a bridge call that consumes (and always releases) it.
    fn into_raw(self) -> *mut c_void {
        let raw = self.0.as_ptr();
        std::mem::forget(self);
        raw
    }
}

impl Drop for ScopedUrl {
    fn drop(&mut self) {
        unsafe { beebeeb_release_url_handle(self.0.as_ptr()) }
    }
}

/// Why an open or a show did not happen.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OpenFailure {
    /// A shared-with-me row: it opens in the web app, and the File Provider was not asked.
    Shared,
    /// No row for the id on this Mac.
    NotKnown,
    /// macOS reports no location (the domain is not added).
    NoLocation,
    /// The bridge or the system said no (domain and code only, never the OS's text).
    Bridge(FpError),
}

impl OpenFailure {
    /// The one sentence for a person; a bridge failure is its redacted domain and code.
    pub(crate) fn sentence(&self, opening: bool) -> String {
        match self {
            Self::Shared => SHARED_NOT_IN_FINDER.to_string(),
            Self::NotKnown => ITEM_NOT_KNOWN.to_string(),
            Self::NoLocation if opening => NOTHING_TO_OPEN.to_string(),
            Self::NoLocation => NOTHING_TO_SHOW.to_string(),
            Self::Bridge(error) => error.redacted(),
        }
    }
}

/// The calls the open makes, so a test puts a fake here and no test asks the real `NSWorkspace`. `resolve_*` are File
/// Provider calls (the caller holds the gate); `open` and `reveal` are not (the caller does not).
pub(crate) trait Workspace {
    type Url;
    fn resolve_location(&self) -> Result<Option<Self::Url>, FpError>;
    fn resolve_item(&self, item_id: &str) -> Result<Option<Self::Url>, FpError>;
    fn open(&self, url: Self::Url) -> Result<(), FpError>;
    fn reveal(&self, url: Self::Url) -> Result<(), FpError>;
}

/// The real workspace: the bridge.
pub(crate) struct SystemWorkspace;

/// The bridge's `1` with a handle, `0` (no location) or its filled error. Any other return is a bridge that does not
/// follow its own contract: an app error, never a guess.
fn decode_resolve(
    function: &str,
    code: i32,
    handle: Option<ScopedUrl>,
    error: BeebeebFpErrorC,
) -> Result<Option<ScopedUrl>, FpError> {
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

impl Workspace for SystemWorkspace {
    type Url = ScopedUrl;

    fn resolve_location(&self) -> Result<Option<ScopedUrl>, FpError> {
        let mut handle: *mut c_void = std::ptr::null_mut();
        let mut out = BeebeebFpErrorC::zeroed();
        let code = unsafe { beebeeb_fp_resolve_location(&mut handle, &mut out) };
        decode_resolve("beebeeb_fp_resolve_location", code, ScopedUrl::from_raw(handle), out)
    }

    fn resolve_item(&self, item_id: &str) -> Result<Option<ScopedUrl>, FpError> {
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
        decode_resolve("beebeeb_fp_resolve_item", code, ScopedUrl::from_raw(handle), out)
    }

    /// Opens in this process through `NSWorkspace` and waits for LaunchServices (5 s). Takes the handle whatever happens.
    fn open(&self, url: ScopedUrl) -> Result<(), FpError> {
        let mut out = BeebeebFpErrorC::zeroed();
        let code = unsafe { beebeeb_open_scoped_url(url.into_raw(), &mut out) };
        done_or_error("beebeeb_open_scoped_url", code, out)
    }

    /// `activateFileViewerSelectingURLs:` after an `lstat`; `Ok` means "it exists and was handed to Finder".
    fn reveal(&self, url: ScopedUrl) -> Result<(), FpError> {
        let mut out = BeebeebFpErrorC::zeroed();
        let code = unsafe { beebeeb_reveal_scoped_url(url.into_raw(), &mut out) };
        done_or_error("beebeeb_reveal_scoped_url", code, out)
    }
}

/// The resolve under the gate, which is let go when this returns, before anything is opened.
fn resolved<W: Workspace>(
    resolve: impl FnOnce(&W) -> Result<Option<W::Url>, FpError>,
    workspace: &W,
) -> Result<Option<W::Url>, FpError> {
    let _gate = Gate::enter_waiting(BRIDGE_GATE_WAIT)?;
    resolve(workspace)
}

/// "Open in Finder": the Beebeeb location. The gate is not held across the open.
pub(crate) fn open_location_with<W: Workspace>(workspace: &W) -> Result<(), OpenFailure> {
    let url = resolved(|w| w.resolve_location(), workspace)
        .map_err(OpenFailure::Bridge)?
        .ok_or(OpenFailure::NoLocation)?;
    workspace.open(url).map_err(OpenFailure::Bridge)
}

/// A plain file's open with the system's default app: the item's scoped URL through `NSWorkspace`, the gate let go
/// before the open (dm-28 regression: only the location and Show in Finder were wired).
pub(crate) fn open_item_with<W: Workspace>(workspace: &W, item_id: &str) -> Result<(), OpenFailure> {
    let url = resolved(|w| w.resolve_item(item_id), workspace)
        .map_err(OpenFailure::Bridge)?
        .ok_or(OpenFailure::NoLocation)?;
    workspace.open(url).map_err(OpenFailure::Bridge)
}

/// The namespace rule, before any bridge call: only one of the person's own rows may be asked of the File Provider.
/// `namespace_of` reads the row (`Ok(None)`: no such row); a failing lookup is refused too.
pub(crate) fn may_show(
    item_id: &str,
    namespace_of: impl FnOnce(&str) -> Result<Option<Namespace>, String>,
) -> Result<(), OpenFailure> {
    beebeeb_engine::finder_show::finder_may_show(item_id, namespace_of).map_err(|sentence| {
        if sentence == SHARED_NOT_IN_FINDER {
            OpenFailure::Shared
        } else {
            OpenFailure::NotKnown
        }
    })
}

/// "Show in Finder" for one item the caller already passed [`may_show`] for, and the gate is not held across the reveal.
pub(crate) fn reveal_item_with<W: Workspace>(workspace: &W, item_id: &str) -> Result<(), OpenFailure> {
    let url = resolved(|w| w.resolve_item(item_id), workspace)
        .map_err(OpenFailure::Bridge)?
        .ok_or(OpenFailure::NoLocation)?;
    workspace.reveal(url).map_err(OpenFailure::Bridge)
}

pub(crate) fn open_location() -> Result<(), OpenFailure> {
    open_location_with(&SystemWorkspace)
}

pub(crate) fn open_item(item_id: &str) -> Result<(), OpenFailure> {
    open_item_with(&SystemWorkspace, item_id)
}

pub(crate) fn reveal_item(item_id: &str) -> Result<(), OpenFailure> {
    reveal_item_with(&SystemWorkspace, item_id)
}

/// What a press registered for the path it handed the launch worker: the worker's starts take a path, and on a Mac
/// the File Provider, not the path, says where an item is.
enum Target {
    Location,
    /// Shown in Finder: a folder, or a file with no app of its kind.
    Item(String),
    /// A plain file, opened with its default app.
    OpenItem(String),
}

static TARGETS: std::sync::Mutex<Vec<(std::path::PathBuf, Target)>> = std::sync::Mutex::new(Vec::new());

fn register(path: &std::path::Path, target: Target) {
    let mut targets = TARGETS.lock().unwrap_or_else(|p| p.into_inner());
    targets.retain(|(known, _)| known != path);
    if targets.len() >= 256 {
        targets.remove(0);
    }
    targets.push((path.to_path_buf(), target));
}

pub(crate) fn register_location(path: &std::path::Path) {
    register(path, Target::Location);
}

pub(crate) fn register_open_item(path: &std::path::Path, item_id: &str) {
    register(path, Target::OpenItem(item_id.to_string()));
}

pub(crate) fn register_item(path: &std::path::Path, item_id: &str) {
    register(path, Target::Item(item_id.to_string()));
}

/// The launch worker's start on a Mac: opens the location, opens the plain file or reveals the item the press
/// registered for `path`. A path nobody registered is not a Finder location and is not opened.
pub(crate) fn start_registered(path: &std::path::Path) -> Result<(), OpenFailure> {
    start_registered_with(&SystemWorkspace, path)
}

pub(crate) fn start_registered_with<W: Workspace>(workspace: &W, path: &std::path::Path) -> Result<(), OpenFailure> {
    let target = {
        let targets = TARGETS.lock().unwrap_or_else(|p| p.into_inner());
        targets.iter().find(|(known, _)| known == path).map(|(_, target)| match target {
            Target::Location => Target::Location,
            Target::Item(id) => Target::Item(id.clone()),
            Target::OpenItem(id) => Target::OpenItem(id.clone()),
        })
    };
    match target {
        Some(Target::Item(id)) => reveal_item_with(workspace, &id),
        Some(Target::OpenItem(id)) => open_item_with(workspace, &id),
        Some(Target::Location) => open_location_with(workspace),
        None => Err(OpenFailure::NoLocation),
    }
}

/// A shared-with-me row (or an unknown one) is refused here, before any bridge call.
pub(crate) fn refuse_shared(db: &beebeeb_engine::state_db::StateDb, item_id: &str) -> Result<(), OpenFailure> {
    may_show(item_id, |id| beebeeb_engine::finder_show::item_namespace(db, id))
}

/// The line when Finder is not set up (the Files menu offers the action only on `Ready`; a press that raced it).
pub(crate) fn not_ready_line(view: &beebeeb_engine::finder_setup::driver::FinderSetupView) -> Option<&'static str> {
    use beebeeb_engine::surfaces::phase::FinderSetup;
    match view.setup {
        FinderSetup::Ready => None,
        FinderSetup::Missing | FinderSetup::Adding => Some("Beebeeb isn’t in Finder yet."),
        FinderSetup::UserDisabled => Some("Beebeeb is turned off in Finder."),
        FinderSetup::Failed => Some("Beebeeb couldn’t be added to Finder."),
    }
}

#[cfg(test)]
#[path = "open_tests.rs"]
mod open_tests;
