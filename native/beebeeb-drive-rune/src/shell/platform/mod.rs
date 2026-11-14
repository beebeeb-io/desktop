//! The platform seam: everything the shell asks of the OS goes through [`Platform`].
//!
//! `windows` is the real one. `macos` and `linux` compile and say "not built yet" (macOS gets
//! the menu-bar b and its popover later; Linux has no notification-area work planned). Nothing
//! outside this directory names an OS API.

use std::sync::Arc;
use std::time::Duration;

use super::control::{Reply, Request};
use super::flyout::Anchor;
use super::identity::AppIdentity;
use super::instance::Delivery;
use super::tray::{MenuAction, TrayModel};
use super::tray_icon::TaskbarTheme;

#[cfg(test)]
pub mod fake;
#[cfg(all(unix, not(target_os = "macos")))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_control;
// The lock screen's prompt (`engine_host::hello`). Compiled on every platform (a stub off Windows).
#[allow(dead_code)]
pub mod windows_hello;
#[cfg(windows)]
mod windows_tray;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformError {
    /// This platform's half does not exist yet. The text says what.
    NotBuilt(&'static str),
    /// The thing belongs to another install of this app and was left alone. The text says what.
    NotOurs(&'static str),
    /// The OS refused; the text is for the log (no paths of the person's files, no secrets).
    Os(String),
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlatformError::NotBuilt(what) => write!(f, "not built yet: {what}"),
            PlatformError::NotOurs(what) => write!(f, "left alone: {what}"),
            PlatformError::Os(message) => f.write_str(message),
        }
    }
}

/// What the b reports. Sent on the tray's own thread: hand it to the app and return.
#[derive(Clone, Debug, PartialEq)]
pub enum TrayEvent {
    /// A left click, Enter or Space on the b. `anchor`: the display the b is on, for the flyout.
    Activate { anchor: Option<Anchor> },
    /// An item of the b's menu was chosen.
    Menu(MenuAction),
    /// The icon appeared in, or went from, the notification area (Explorer stopped or came
    /// back, a failed add that a retry made good).
    Presence(bool),
    /// Windows asked to shut down, the app took its icon away, and the shutdown was then
    /// cancelled: the icon is back, and whatever the quit stopped must start again.
    ShutdownCancelled,
    /// Something outside asked the app to close (`taskkill` without /F, Restart Manager).
    CloseRequested,
}

pub type TrayHandler = Arc<dyn Fn(TrayEvent) + Send + Sync>;

/// The icon in the notification area. Dropping it removes the icon.
pub trait TrayIcon: Send {
    /// Shows this model: icon, tooltip and menu. Cheap when nothing changed.
    fn apply(&self, model: &TrayModel);
    /// Whether the icon is in the notification area right now. A tray whose add failed is
    /// still a tray (it keeps trying), and is not present.
    fn present(&self) -> bool;
    /// The app's quit hook ran: take the icon away now, without ending the tray. Rune runs the
    /// quit hook when Windows ASKS to shut down, and another app may refuse; the tray then
    /// puts the icon back and reports [`TrayEvent::ShutdownCancelled`].
    fn remove_for_quit(&self);
}

/// Takes one request from the control channel, on the channel's own thread, and answers it.
pub type ControlHandler = Arc<dyn Fn(Request) -> Reply + Send + Sync>;

/// The served control channel. Dropping it stops serving and frees the name.
pub trait ControlServer: Send {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServeError {
    /// Another process serves this identity's channel: it is the running instance.
    AlreadyRunning,
    /// This platform has no channel yet.
    NotBuilt(&'static str),
    /// The channel could not be created.
    Os(String),
}

pub trait Platform: Send + Sync {
    fn name(&self) -> &'static str;
    /// Adds the b to the notification area.
    fn create_tray(&self, model: &TrayModel, on_event: TrayHandler) -> Result<Box<dyn TrayIcon>, PlatformError>;
    /// Whether the taskbar is light or dark, for the b's ink.
    fn taskbar_theme(&self) -> TaskbarTheme;
    fn autostart_enabled(&self) -> Result<bool, PlatformError>;
    fn set_autostart(&self, enabled: bool) -> Result<(), PlatformError>;
    /// Brings a start-at-login entry the 0.8.x app left FOR THIS PROGRAM (the bare path) into
    /// this app's shape. An entry for another program, and "off", are left alone. True when
    /// it rewrote one.
    fn adopt_autostart(&self) -> Result<bool, PlatformError> {
        Ok(false)
    }
    /// Called once the flyout's native window exists, still hidden: keeps it out of the
    /// taskbar and Alt+Tab where the OS needs telling.
    fn adopt_flyout_window(&self, _window: &rune::WindowHandle) {}
    /// Tells the person why the app will not start, when there is no window to say it in.
    fn show_start_refusal(&self, title: &str, message: &str);
    /// Serves this identity's control channel for the current user only. Succeeding IS being
    /// the one running instance.
    fn serve_control(
        &self,
        identity: AppIdentity,
        handler: ControlHandler,
    ) -> Result<Box<dyn ControlServer>, ServeError>;
    /// Hands one request to the running instance and waits at most `timeout` for its answer.
    fn send_control(&self, identity: AppIdentity, request: &Request, timeout: Duration) -> Delivery;
    /// Sets or clears the window's capture-exclusion. When set, the window region appears black
    /// (or absent) in screenshots, screen recordings and screen-sharing; clearing restores it.
    /// The default is a no-op (Linux, macOS without the NSWindow bridge). Failure is logged
    /// without content; the content stays drawn.
    /// `window` is `None` when the handle does not exist yet; the Shell retries on the next
    /// poll. An implementation that does not need the handle (the fake, the no-ops) ignores it.
    fn set_capture_exclusion(&self, _window: Option<&rune::WindowHandle>, _exclude: bool) -> Result<(), PlatformError> {
        Ok(())
    }
}

/// The platform this build runs on.
pub fn current() -> Arc<dyn Platform> {
    #[cfg(windows)]
    {
        Arc::new(windows::WindowsPlatform)
    }
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::MacPlatform)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Arc::new(linux::LinuxPlatform)
    }
}

/// On the platforms that are stubs, every call says so instead of pretending.
#[cfg(all(test, not(windows)))]
mod tests {
    use super::super::tray::{TrayInputs, tray_model};
    use super::*;

    #[test]
    fn the_stub_platform_says_not_built_for_everything() {
        let platform = current();
        let model = tray_model(&TrayInputs::default());
        assert!(matches!(
            platform.create_tray(&model, Arc::new(|_| {})),
            Err(PlatformError::NotBuilt(_))
        ));
        assert!(matches!(platform.autostart_enabled(), Err(PlatformError::NotBuilt(_))));
        assert!(matches!(platform.set_autostart(true), Err(PlatformError::NotBuilt(_))));
        assert!(matches!(
            platform.serve_control(AppIdentity::Test, Arc::new(|_| Reply::Ok)),
            Err(ServeError::NotBuilt(_))
        ));
        assert_eq!(
            platform.send_control(AppIdentity::Test, &Request::Show, Duration::from_millis(1)),
            Delivery::Unreachable
        );
        assert!(!platform.name().is_empty());
        let shown = PlatformError::NotBuilt("the b").to_string();
        assert_eq!(shown, "not built yet: the b");
    }
}
