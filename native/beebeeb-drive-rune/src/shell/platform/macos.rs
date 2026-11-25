//! macOS: compiles, runs the window, and says "not built yet" for the rest. The Mac gets the
//! menu-bar b with its popover (DM-1201…) and a LaunchAgent for start at login, after Windows.

use std::time::Duration;

use super::{ControlHandler, ControlServer, Platform, PlatformError, ServeError, TrayHandler, TrayIcon};
use crate::shell::control::Request;
use crate::shell::identity::AppIdentity;
use crate::shell::instance::Delivery;
use crate::shell::tray::TrayModel;
use crate::shell::tray_icon::TaskbarTheme;

#[path = "macos_autostart.rs"]
mod autostart;
#[path = "macos_control.rs"]
mod control_socket;
#[path = "macos_fp/mod.rs"]
pub(crate) mod macos_fp;
#[path = "macos_tray.rs"]
mod tray;

pub struct MacPlatform;

impl Platform for MacPlatform {
    fn name(&self) -> &'static str {
        "macos"
    }
    fn create_tray(&self, model: &TrayModel, on_event: TrayHandler) -> Result<Box<dyn TrayIcon>, PlatformError> {
        tray::create_tray(model, on_event)
    }
    fn adopt_flyout_window(&self, window: &rune::WindowHandle) {
        let view = window.with_raw_handle(|handle| match handle.as_raw() {
            raw_window_handle::RawWindowHandle::AppKit(appkit) => Some(appkit.ns_view.as_ptr() as usize),
            _ => None,
        });
        if let Ok(Some(view)) = view {
            tray::adopt_popover(view as *mut std::ffi::c_void);
        }
    }
    fn taskbar_theme(&self) -> TaskbarTheme {
        tray::taskbar_theme()
    }
    fn autostart_enabled(&self) -> Result<bool, PlatformError> {
        autostart::autostart_enabled()
    }
    fn set_autostart(&self, enabled: bool) -> Result<(), PlatformError> {
        autostart::set_autostart(enabled)
    }
    fn adopt_autostart(&self) -> Result<bool, PlatformError> {
        autostart::adopt_autostart()
    }
    fn file_provider(&self) -> Option<std::sync::Arc<dyn beebeeb_engine::file_provider_port::FileProviderPort>> {
        Some(std::sync::Arc::new(macos_fp::FileProviderBridgePort))
    }
    fn show_start_refusal(&self, title: &str, message: &str) {
        eprintln!("{title}: {message}");
    }
    fn serve_control(
        &self,
        identity: AppIdentity,
        handler: ControlHandler,
    ) -> Result<Box<dyn ControlServer>, ServeError> {
        let dir = control_socket::state_dir(identity).map_err(ServeError::Os)?;
        control_socket::serve_in(&dir, identity, handler)
    }
    fn send_control(&self, identity: AppIdentity, request: &Request, timeout: Duration) -> Delivery {
        match control_socket::state_dir(identity) {
            Ok(dir) => control_socket::send_in(&dir, request, timeout),
            Err(_) => Delivery::Unreachable,
        }
    }
}
