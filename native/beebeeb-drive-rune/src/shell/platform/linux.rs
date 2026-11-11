//! Linux: compiles, runs the window, and says "not built yet" for the rest. The headless gates
//! and the dev loop run here; no Linux release of the native app is planned.

use super::{Platform, PlatformError, TrayHandler, TrayIcon};
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
    fn show_start_refusal(&self, message: &str) {
        eprintln!("Beebeeb: {message}");
    }
}
