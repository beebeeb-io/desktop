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
pub mod packet_text;
#[cfg(all(unix, not(target_os = "macos")))]
mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(target_os = "macos")]
pub(crate) mod macos_open_url;
#[cfg(target_os = "macos")]
pub(crate) mod macos_panels;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_control;
#[cfg(windows)]
mod windows_contrast;
/// The Windows contrast theme's colours, when one is on (read now; `theme::contrast` keeps the answer).
#[cfg(windows)]
#[cfg_attr(test, allow(dead_code))] // tests never read the desktop's theme
pub(crate) fn contrast_colors() -> Option<crate::theme::contrast::SystemColors> {
    windows_contrast::read()
}
#[cfg(windows)]
mod windows_dpi;
#[cfg(windows)]
mod windows_folder;
#[cfg(windows)]
mod windows_packet;
#[cfg(windows)]
mod windows_photo;
#[cfg(windows)]
mod windows_save;
#[cfg(windows)]
mod windows_share;
// The lock screen's prompt (`engine_host::hello`). Compiled on every platform (a stub off Windows).
#[allow(dead_code)]
pub mod windows_hello;
// plan40 batch 1: toasts (slice B5) and File Explorer's menu items (slice B6).
#[cfg(windows)]
pub(crate) mod windows_toast;
#[cfg(windows)]
mod windows_tray;
#[cfg(windows)]
pub(crate) mod windows_verbs;

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
    /// The operating system's session was locked (Win+L, or Windows' own lock): the away lock's event.
    SessionLocked,
    /// The PC woke from sleep or hibernation: how long it was away, its time without input before it slept and the
    /// sleep itself (the away lock's event; a wake's own key press is input, so the idle count alone would miss it).
    Resumed { away: std::time::Duration },
}

/// What Windows' save panel answered (`Platform::pick_save_path`). The path is the person's own: never logged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveAnswer {
    /// The person chose where the file goes (the panel already asked about replacing an existing file).
    Chosen(std::path::PathBuf),
    /// The panel was closed without a choice.
    Cancelled,
    /// The panel could not open (the cause is logged in fixed words by the platform, never with a path).
    Failed,
}

impl SaveAnswer {
    /// The answer for the session of the press: `pressed` is the account that pressed Save, `now` the account the
    /// window may act on right now (`None`: signed out, locked or switching). An answer that lands under any other
    /// session is dropped, never handed on: a path chosen for one account must not start a save for another.
    pub fn for_session(self, pressed: &str, now: Option<&str>) -> Option<SaveAnswer> {
        (now == Some(pressed)).then_some(self)
    }
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
    /// macOS: the File Provider bridge the engine signals Finder through; `None` on a platform without one.
    fn file_provider(&self) -> Option<Arc<dyn beebeeb_engine::file_provider_port::FileProviderPort>> {
        None
    }
    /// Opens the OS's own folder picker (DW-0738: Windows draws it) with `title`, showing where `start` is.
    /// BLOCKS until the person closed it: a worker's call, never the window's thread. `Ok(None)`: closed
    /// without a folder. The folder is the person's own: shown to them, never logged.
    fn pick_folder(&self, _title: &str, _start: &std::path::Path) -> Result<Option<std::path::PathBuf>, PlatformError> {
        #[cfg(target_os = "macos")]
        {
            return macos_panels::pick_folder(_title, _start, "pick_folder").map_err(PlatformError::Os);
        }
        #[cfg(not(target_os = "macos"))]
        Err(PlatformError::NotBuilt("the folder picker"))
    }
    /// Opens the OS's own file picker for one picture this PC decodes, in `start` (photo previews' "Choose
    /// photo"). BLOCKS: a worker's call. `Ok(None)`: closed without a file. Never logged.
    fn pick_photo(&self, _title: &str, _start: &std::path::Path) -> Result<Option<std::path::PathBuf>, PlatformError> {
        Err(PlatformError::NotBuilt("the photo picker"))
    }
    /// Opens the OS's own save panel (Windows: `IFileSaveDialog`) with `suggested_name` filled in, in `start` when
    /// that folder exists. BLOCKS until the person closed it: a worker's call, never the window's thread; asked
    /// modeless like `pick_folder`, and its answer is handed over only for the session of the press
    /// ([`SaveAnswer::for_session`]). The path is shown to the person, never logged.
    fn pick_save_path(&self, _suggested_name: &str, _start: std::path::PathBuf) -> SaveAnswer {
        #[cfg(target_os = "macos")]
        {
            return macos_panels::pick_save_path(_suggested_name, _start, "save_folder");
        }
        #[cfg(not(target_os = "macos"))]
        SaveAnswer::Failed
    }
    /// Opens the OS's Share sheet (Windows: `IDataTransferManagerInterop::ShowShareUIForWindow`) for the file at
    /// `path`, over `window` (`None`: the window has no handle yet). Returns once the sheet is shown, not when the
    /// person is done. The file is the person's own: never logged.
    fn share_file(&self, _window: Option<&rune::WindowHandle>, _path: &std::path::Path) -> Result<(), PlatformError> {
        Err(PlatformError::NotBuilt("the share sheet"))
    }
    /// How long this PC has had no input from the person (keys, pointer, touch), in any app: the away lock's
    /// clock. `None`: this platform cannot say; the shell then counts from its own windows' last input.
    fn idle_for(&self) -> Option<Duration> {
        None
    }
    /// Sets or clears the window's capture-exclusion. When set, the window region appears black
    /// (or absent) in screenshots, screen recordings and screen-sharing; clearing restores it.
    /// The default is a no-op (Linux, macOS without the NSWindow bridge). Failure is logged
    /// without content; the content stays drawn.
    /// `window` is `None` when the handle does not exist yet; the Shell retries on the next
    /// poll. An implementation that does not need the handle (the fake, the no-ops) ignores it.
    fn set_capture_exclusion(&self, _window: Option<&rune::WindowHandle>, _exclude: bool) -> Result<(), PlatformError> {
        Ok(())
    }
    /// Starts taking the text that reaches the main window as Unicode packets (`packet_text`): what a remote
    /// desktop, an on-screen keyboard and `SendInput`'s Unicode mode type with, and what the key after a dead key
    /// types. Called on the window's thread, from the poll, until it answers true. `wake` is called (on that thread) when there is text to take, so the
    /// next poll runs. The default has nothing to watch: only Windows sends text this way.
    fn watch_packet_text(&self, _wake: PacketWake) -> bool {
        true
    }
    /// The packets' text since the last call, in the order it was typed. `shell::host::poll` hands it to the
    /// field that has the keyboard.
    fn take_packet_text(&self) -> Vec<String> {
        Vec::new()
    }
}

/// What [`Platform::watch_packet_text`] calls when text arrived.
pub type PacketWake = Arc<dyn Fn() + Send + Sync>;

/// The platform this build runs on.
pub fn current() -> Arc<dyn Platform> {
    #[cfg(windows)]
    {
        Arc::new(windows::WindowsPlatform)
    }
    #[cfg(target_os = "macos")]
    {
        // Once per process, before the engine reads a backup folder: reopen the picked folders' sandbox scopes.
        static SCOPES: std::sync::Once = std::sync::Once::new();
        SCOPES.call_once(macos_panels::resolve_backup_folders);
        Arc::new(macos::MacPlatform)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Arc::new(linux::LinuxPlatform)
    }
}

#[cfg(test)]
mod save_share_tests {
    use super::fake::FakePlatform;
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn a_save_path_that_lands_after_a_sign_out_is_dropped() {
        let chosen = SaveAnswer::Chosen(PathBuf::from("C:/Users/x/a.jpg"));
        assert_eq!(chosen.clone().for_session("acct-1", Some("acct-1")), Some(chosen.clone()));
        assert_eq!(chosen.clone().for_session("acct-1", None), None);
        assert_eq!(chosen.for_session("acct-1", Some("acct-2")), None);
        assert_eq!(SaveAnswer::Cancelled.for_session("acct-1", None), None);
    }

    #[test]
    fn a_cancelled_panel_answers_cancelled_and_remembers_the_ask() {
        let fake = FakePlatform::default();
        assert_eq!(fake.pick_save_path("a.jpg", PathBuf::from("C:/x")), SaveAnswer::Cancelled);
        fake.saves.lock().unwrap().push_back(SaveAnswer::Chosen(PathBuf::from("C:/x/b.jpg")));
        fake.saves.lock().unwrap().push_back(SaveAnswer::Failed);
        assert_eq!(
            fake.pick_save_path("b.jpg", PathBuf::from("C:/x")),
            SaveAnswer::Chosen(PathBuf::from("C:/x/b.jpg"))
        );
        assert_eq!(fake.pick_save_path("c.jpg", PathBuf::from("C:/y")), SaveAnswer::Failed);
        let asks = fake.save_asks.lock().unwrap();
        assert_eq!(asks.len(), 3);
        assert_eq!(asks[0], ("a.jpg".to_string(), PathBuf::from("C:/x")));
    }

    #[test]
    fn the_fake_shares_a_file_and_the_unbuilt_one_says_so() {
        let fake = FakePlatform::default();
        assert_eq!(fake.share_file(None, Path::new("C:/x/a.jpg")), Ok(()));
        assert_eq!(fake.shared.lock().unwrap().as_slice(), [PathBuf::from("C:/x/a.jpg")]);
        let unbuilt = FakePlatform::unbuilt();
        assert!(matches!(unbuilt.share_file(None, Path::new("a")), Err(PlatformError::NotBuilt(_))));
        assert_eq!(unbuilt.pick_save_path("a", PathBuf::new()), SaveAnswer::Failed);
    }
}

/// On the platforms that are stubs, every call says so instead of pretending.
#[cfg(all(test, target_os = "linux"))]
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
        // macOS has a control channel (`macos_control.rs`, tested there in a temp folder).
        #[cfg(not(target_os = "macos"))]
        {
            assert!(matches!(
                platform.serve_control(AppIdentity::Test, Arc::new(|_| Reply::Ok)),
                Err(ServeError::NotBuilt(_))
            ));
            assert_eq!(
                platform.send_control(AppIdentity::Test, &Request::Show, Duration::from_millis(1)),
                Delivery::Unreachable
            );
        }
        assert!(!platform.name().is_empty());
        let shown = PlatformError::NotBuilt("the b").to_string();
        assert_eq!(shown, "not built yet: the b");
    }
}
