//! A platform for tests: remembers what it was asked, touches nothing.

use std::sync::Mutex;

use super::{Platform, PlatformError, TrayHandler, TrayIcon};
use crate::shell::tray::TrayModel;
use crate::shell::tray_icon::TaskbarTheme;

#[derive(Default)]
pub struct FakePlatform {
    unbuilt: bool,
    autostart: Mutex<(bool, Vec<bool>)>,
    pub applied: std::sync::Arc<Mutex<Vec<TrayModel>>>,
    pub refusals: Mutex<Vec<String>>,
}

impl FakePlatform {
    /// A platform whose halves are all "not built yet", like the macOS and Linux stubs.
    pub fn unbuilt() -> FakePlatform {
        FakePlatform {
            unbuilt: true,
            ..FakePlatform::default()
        }
    }

    pub fn autostart_writes(&self) -> Vec<bool> {
        self.autostart.lock().unwrap().1.clone()
    }
}

struct FakeTray(std::sync::Arc<Mutex<Vec<TrayModel>>>);

impl TrayIcon for FakeTray {
    fn apply(&self, model: &TrayModel) {
        self.0.lock().unwrap().push(model.clone());
    }
}

impl Platform for FakePlatform {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn create_tray(&self, model: &TrayModel, _on_event: TrayHandler) -> Result<Box<dyn TrayIcon>, PlatformError> {
        if self.unbuilt {
            return Err(PlatformError::NotBuilt("the b"));
        }
        self.applied.lock().unwrap().push(model.clone());
        Ok(Box::new(FakeTray(self.applied.clone())))
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
    fn show_start_refusal(&self, message: &str) {
        self.refusals.lock().unwrap().push(message.to_owned());
    }
}
