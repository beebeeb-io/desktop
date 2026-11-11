//! Linux: compiles, runs the window, and says "not built yet" for the rest. The headless gates
//! and the dev loop run here; no Linux release of the native app is planned.

use std::time::Duration;

use super::{ControlHandler, ControlServer, Platform, PlatformError, ServeError, TrayHandler, TrayIcon};
use crate::shell::control::Request;
use crate::shell::identity::AppIdentity;
use crate::shell::instance::Delivery;
use crate::shell::tray::TrayModel;
use crate::shell::tray_icon::TaskbarTheme;

pub struct LinuxPlatform;

impl Platform for LinuxPlatform {
    fn name(&self) -> &'static str {
        "linux"
    }
    fn create_tray(&self, _model: &TrayModel, _on_event: TrayHandler) -> Result<Box<dyn TrayIcon>, PlatformError> {
        Err(PlatformError::NotBuilt("the notification-area b on Linux"))
    }
    fn taskbar_theme(&self) -> TaskbarTheme {
        TaskbarTheme::Dark
    }
    fn autostart_enabled(&self) -> Result<bool, PlatformError> {
        Err(PlatformError::NotBuilt("start at login on Linux"))
    }
    fn set_autostart(&self, _enabled: bool) -> Result<(), PlatformError> {
        Err(PlatformError::NotBuilt("start at login on Linux"))
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
