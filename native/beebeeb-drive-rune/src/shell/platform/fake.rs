//! A platform for tests: remembers what it was asked, touches nothing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{ControlHandler, ControlServer, Platform, PlatformError, ServeError, TrayHandler, TrayIcon};
use crate::shell::control::Request;
use crate::shell::identity::AppIdentity;
use crate::shell::instance::Delivery;
use crate::shell::tray::TrayModel;
use crate::shell::tray_icon::TaskbarTheme;

pub struct FakePlatform {
    unbuilt: bool,
    autostart: Mutex<(bool, Vec<bool>)>,
    pub applied: Arc<Mutex<Vec<TrayModel>>>,
    pub refusals: Mutex<Vec<(String, String)>>,
    /// Whether the fake icon is in the notification area (`NIM_ADD` worked). Tests flip it.
    pub icon_present: Arc<AtomicBool>,
    /// How often the shell asked the tray to take its icon away for a quit.
    pub removed_for_quit: Arc<Mutex<u32>>,
    /// The handler the shell gave the tray, so a test can send what the OS would.
    pub tray_handler: Mutex<Option<TrayHandler>>,
    /// Each `set_capture_exclusion` call records the `exclude` argument, in order.
    pub capture_exclusions: Mutex<Vec<bool>>,
    /// When true, `set_capture_exclusion` fails with `PlatformError::Os`.
    pub capture_exclusion_fails: AtomicBool,
}

impl Default for FakePlatform {
    fn default() -> FakePlatform {
        FakePlatform {
            unbuilt: false,
            autostart: Mutex::default(),
            applied: Arc::default(),
            refusals: Mutex::default(),
            // A fake icon is there unless a test says otherwise.
            icon_present: Arc::new(AtomicBool::new(true)),
            removed_for_quit: Arc::default(),
            tray_handler: Mutex::default(),
            capture_exclusions: Mutex::default(),
            capture_exclusion_fails: AtomicBool::new(false),
        }
    }
}

impl FakePlatform {
    /// A platform whose halves are all "not built yet", like the macOS and Linux stubs.
    pub fn unbuilt() -> FakePlatform {
        FakePlatform {
            unbuilt: true,
            ..FakePlatform::default()
        }
    }

    /// A platform whose tray exists but whose icon could not be added (no notification area).
    pub fn without_notification_area() -> FakePlatform {
        let platform = FakePlatform::default();
        platform.icon_present.store(false, Ordering::SeqCst);
        platform
    }

    pub fn autostart_writes(&self) -> Vec<bool> {
        self.autostart.lock().unwrap().1.clone()
    }
}

struct FakeTray {
    applied: Arc<Mutex<Vec<TrayModel>>>,
    present: Arc<AtomicBool>,
    removed_for_quit: Arc<Mutex<u32>>,
}

impl TrayIcon for FakeTray {
    fn apply(&self, model: &TrayModel) {
        self.applied.lock().unwrap().push(model.clone());
    }
    fn present(&self) -> bool {
        self.present.load(Ordering::SeqCst)
    }
    fn remove_for_quit(&self) {
        *self.removed_for_quit.lock().unwrap() += 1;
    }
}

struct FakeServer;
impl ControlServer for FakeServer {}

impl Platform for FakePlatform {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn create_tray(&self, model: &TrayModel, on_event: TrayHandler) -> Result<Box<dyn TrayIcon>, PlatformError> {
        if self.unbuilt {
            return Err(PlatformError::NotBuilt("the b"));
        }
        self.applied.lock().unwrap().push(model.clone());
        *self.tray_handler.lock().unwrap() = Some(on_event);
        Ok(Box::new(FakeTray {
            applied: self.applied.clone(),
            present: self.icon_present.clone(),
            removed_for_quit: self.removed_for_quit.clone(),
        }))
    }

    fn taskbar_theme(&self) -> TaskbarTheme {
        TaskbarTheme::Dark
    }

    fn autostart_enabled(&self) -> Result<bool, PlatformError> {
        if self.unbuilt {
            Err(PlatformError::NotBuilt("start at login"))
        } else {
            Ok(self.autostart.lock().unwrap().0)
        }
    }

    fn set_autostart(&self, enabled: bool) -> Result<(), PlatformError> {
        if self.unbuilt {
            return Err(PlatformError::NotBuilt("start at login"));
        }
        let mut state = self.autostart.lock().unwrap();
        state.0 = enabled;
        state.1.push(enabled);
        Ok(())
    }

    fn show_start_refusal(&self, title: &str, message: &str) {
        self.refusals
            .lock()
            .unwrap()
            .push((title.to_owned(), message.to_owned()));
    }

    fn serve_control(
        &self,
        _identity: AppIdentity,
        _handler: ControlHandler,
    ) -> Result<Box<dyn ControlServer>, ServeError> {
        if self.unbuilt {
            Err(ServeError::NotBuilt("the control channel"))
        } else {
            Ok(Box::new(FakeServer))
        }
    }

    fn send_control(&self, _identity: AppIdentity, _request: &Request, _timeout: Duration) -> Delivery {
        Delivery::Unreachable
    }

    fn set_capture_exclusion(&self, _window: &rune::WindowHandle, exclude: bool) -> Result<(), PlatformError> {
        if self.capture_exclusion_fails.load(Ordering::SeqCst) {
            return Err(PlatformError::Os("SetWindowDisplayAffinity failed (fake)".into()));
        }
        self.capture_exclusions.lock().unwrap().push(exclude);
        Ok(())
    }
}
