//! macOS menu-bar b (DM-1201…): a stub that says "not built yet" until the status item lands.

use super::{PlatformError, TrayHandler, TrayIcon};
use crate::shell::tray::TrayModel;
use crate::shell::tray_icon::TaskbarTheme;

pub fn create_tray(_model: &TrayModel, _on_event: TrayHandler) -> Result<Box<dyn TrayIcon>, PlatformError> {
    Err(PlatformError::NotBuilt("the menu-bar b on macOS"))
}

pub fn taskbar_theme() -> TaskbarTheme {
    TaskbarTheme::Dark
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::tray::{TrayInputs, tray_model};
    use std::sync::Arc;

    #[test]
    fn tray_says_not_built() {
        let model = tray_model(&TrayInputs::default());
        assert!(matches!(
            create_tray(&model, Arc::new(|_| {})),
            Err(PlatformError::NotBuilt(_))
        ));
        assert_eq!(taskbar_theme(), TaskbarTheme::Dark);
    }
}
