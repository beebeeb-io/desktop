//! macOS: compiles, runs the window, and says "not built yet" for the rest. The Mac gets the
//! menu-bar b with its popover (DM-1201…) and a LaunchAgent for start at login, after Windows.

use std::time::Duration;

use super::{ControlHandler, ControlServer, Platform, PlatformError, ServeError, TrayHandler, TrayIcon};
use crate::shell::control::Request;
use crate::shell::identity::AppIdentity;
use crate::shell::instance::Delivery;
use crate::shell::tray::TrayModel;
use crate::shell::tray_icon::TaskbarTheme;

pub struct MacPlatform;

impl Platform for MacPlatform {
    fn name(&self) -> &'static str {
        "macos"
    }
    fn create_tray(&self, _model: &TrayModel, _on_event: TrayHandler) -> Result<Box<dyn TrayIcon>, PlatformError> {
        Err(PlatformError::NotBuilt("the menu-bar b on macOS"))
    }
    fn taskbar_theme(&self) -> TaskbarTheme {
        TaskbarTheme::Dark
    }
    fn autostart_enabled(&self) -> Result<bool, PlatformError> {
        Err(PlatformError::NotBuilt("open at login on macOS"))
    }
    fn set_autostart(&self, _enabled: bool) -> Result<(), PlatformError> {
        Err(PlatformError::NotBuilt("open at login on macOS"))
    }
    fn show_start_refusal(&self, title: &str, message: &str) {
        eprintln!("{title}: {message}");
    }
    fn serve_control(
        &self,
        _identity: AppIdentity,
        _handler: ControlHandler,
    ) -> Result<Box<dyn ControlServer>, ServeError> {
        Err(ServeError::NotBuilt(
            "the control channel (one instance) on this platform",
        ))
    }
    fn send_control(&self, _identity: AppIdentity, _request: &Request, _timeout: Duration) -> Delivery {
        Delivery::Unreachable
    }
}
