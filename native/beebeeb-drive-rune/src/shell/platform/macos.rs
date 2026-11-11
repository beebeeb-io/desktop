//! macOS: compiles, runs the window, and says "not built yet" for the rest. The Mac gets the
//! menu-bar b with its popover (DM-1201…) and a LaunchAgent for start at login, after Windows.

use super::{Platform, PlatformError, TrayHandler, TrayIcon};
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
    fn show_start_refusal(&self, message: &str) {
        eprintln!("Beebeeb: {message}");
    }
}
