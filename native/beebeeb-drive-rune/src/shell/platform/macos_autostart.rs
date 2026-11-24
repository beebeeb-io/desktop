//! macOS open at login (DM-1203…): `SMAppService.mainAppService`, which works inside the App
//! Sandbox and shows under System Settings › Login Items. The calls sit behind [`LoginItems`] so
//! tests never register anything. A start at login is told from a manual one by the launch Apple
//! event (`instance::launched_as_login_item`), not by an argument.

use std::path::{Path, PathBuf};

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::NSError;

use super::PlatformError;

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

/// `SMAppServiceStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    NotRegistered,
    Enabled,
    RequiresApproval,
    NotFound,
    Unknown(isize),
}

impl Status {
    fn from_raw(raw: isize) -> Self {
        match raw {
            0 => Self::NotRegistered,
            1 => Self::Enabled,
            2 => Self::RequiresApproval,
            3 => Self::NotFound,
            other => Self::Unknown(other),
        }
    }

    /// "On" is registered, whether or not the person has approved it yet: the toggle shows what
    /// Beebeeb asked for. Not registered, not found and anything new read as off.
    pub(crate) fn is_on(self) -> bool {
        matches!(self, Self::Enabled | Self::RequiresApproval)
    }
}

pub(crate) trait LoginItems {
    fn status(&self) -> Status;
    fn register(&self) -> Result<(), ()>;
    fn unregister(&self) -> Result<(), ()>;
}

const REGISTER_FAILED: &str = "macOS did not allow open at login (an unsigned build cannot register it)";
const UNREGISTER_FAILED: &str = "macOS did not remove open at login";

pub(crate) fn enabled_with(items: &dyn LoginItems) -> bool {
    items.status().is_on()
}

/// Idempotent: asking for what is already so does nothing.
pub(crate) fn set_with(items: &dyn LoginItems, enabled: bool) -> Result<(), PlatformError> {
    if items.status().is_on() == enabled {
        return Ok(());
    }
    if enabled {
        items.register().map_err(|()| PlatformError::Os(REGISTER_FAILED.into()))
    } else {
        items
            .unregister()
            .map_err(|()| PlatformError::Os(UNREGISTER_FAILED.into()))
    }
}

/// Where a 0.8.x LaunchAgent for this program would sit (tauri-plugin-autostart: `<name>.plist`).
pub(crate) fn legacy_agent_in(launch_agents: &Path) -> Option<PathBuf> {
    ["Beebeeb.plist", "io.beebeeb.app.plist"]
        .iter()
        .map(|name| launch_agents.join(name))
        .find(|path| path.is_file())
}

struct MainAppService;

impl MainAppService {
    fn object() -> Option<Retained<AnyObject>> {
        let class = AnyClass::get(c"SMAppService")?;
        unsafe { msg_send![class, mainAppService] }
    }
}

impl LoginItems for MainAppService {
    fn status(&self) -> Status {
        match Self::object() {
            Some(service) => {
                let raw: isize = unsafe { msg_send![&*service, status] };
                Status::from_raw(raw)
            }
            None => Status::NotFound,
        }
    }
    fn register(&self) -> Result<(), ()> {
        let service = Self::object().ok_or(())?;
        let result: Result<(), Retained<NSError>> = unsafe { msg_send![&*service, registerAndReturnError: _] };
        result.map_err(|_| ())
    }
    fn unregister(&self) -> Result<(), ()> {
        let service = Self::object().ok_or(())?;
        let result: Result<(), Retained<NSError>> = unsafe { msg_send![&*service, unregisterAndReturnError: _] };
        result.map_err(|_| ())
    }
}

pub fn autostart_enabled() -> Result<bool, PlatformError> {
    Ok(enabled_with(&MainAppService))
}

pub fn set_autostart(enabled: bool) -> Result<(), PlatformError> {
    set_with(&MainAppService, enabled)
}

/// A 0.8.x LaunchAgent is Guus's call: it is left alone and named in the log. Never rewrites, so
/// always false.
pub fn adopt_autostart() -> Result<bool, PlatformError> {
    if let Some(home) = std::env::var_os("HOME") {
        if legacy_agent_in(&Path::new(&home).join("Library/LaunchAgents")).is_some() {
            crate::shell::host::log("start at login: an earlier 0.8.x LaunchAgent is still there; left alone");
        }
    }
    Ok(false)
}

#[cfg(test)]
#[path = "macos_autostart_tests.rs"]
mod tests;
