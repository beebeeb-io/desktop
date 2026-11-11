//! The platform seam: everything the shell asks of the OS goes through [`Platform`].
//!
//! `windows` is the real one. `macos` and `linux` compile and say "not built yet" (macOS gets
//! the menu-bar b and its popover later; Linux has no notification-area work planned). Nothing
//! outside this directory names an OS API.

use std::sync::Arc;

use super::flyout::Anchor;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformError {
    /// This platform's half does not exist yet. The text says what.
    NotBuilt(&'static str),
    /// The OS refused; the text is for the log (no paths of the person's files, no secrets).
    Os(String),
}

impl std::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlatformError::NotBuilt(what) => write!(f, "not built yet: {what}"),
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
}

pub type TrayHandler = Arc<dyn Fn(TrayEvent) + Send + Sync>;

/// The icon in the notification area. Dropping it removes the icon.
pub trait TrayIcon: Send {
    /// Shows this model: icon, tooltip and menu. Cheap when nothing changed.
    fn apply(&self, model: &TrayModel);
}

pub trait Platform: Send + Sync {
    fn name(&self) -> &'static str;
    /// Adds the b to the notification area.
    fn create_tray(&self, model: &TrayModel, on_event: TrayHandler) -> Result<Box<dyn TrayIcon>, PlatformError>;
    /// Whether the taskbar is light or dark, for the b's ink.
    fn taskbar_theme(&self) -> TaskbarTheme;
    fn autostart_enabled(&self) -> Result<bool, PlatformError>;
    fn set_autostart(&self, enabled: bool) -> Result<(), PlatformError>;
    /// Replaces a start-at-login entry the 0.8.x app left (a bare path, or another install's)
    /// with this program's, leaving "off" off. True when it rewrote one.
    fn adopt_autostart(&self) -> Result<bool, PlatformError> {
        Ok(false)
    }
    /// Called once the flyout's native window exists, still hidden: keeps it out of the
    /// taskbar and Alt+Tab where the OS needs telling.
    fn adopt_flyout_window(&self, _window: &rune::WindowHandle) {}
    /// Tells the person why the app will not start, when there is no window to say it in.
    fn show_start_refusal(&self, message: &str);
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
        assert!(!platform.name().is_empty());
        let shown = PlatformError::NotBuilt("the b").to_string();
        assert_eq!(shown, "not built yet: the b");
    }
}
