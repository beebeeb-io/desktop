//! macOS open at login (DM-1203…): a stub that says "not built yet" until the LaunchAgent lands.

use super::PlatformError;

pub fn autostart_enabled() -> Result<bool, PlatformError> {
    Err(PlatformError::NotBuilt("open at login on macOS"))
}

pub fn set_autostart(_enabled: bool) -> Result<(), PlatformError> {
    Err(PlatformError::NotBuilt("open at login on macOS"))
}

pub fn adopt_autostart() -> Result<bool, PlatformError> {
    Err(PlatformError::NotBuilt("open at login on macOS"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_says_not_built() {
        assert!(matches!(autostart_enabled(), Err(PlatformError::NotBuilt(_))));
        assert!(matches!(set_autostart(true), Err(PlatformError::NotBuilt(_))));
        assert!(matches!(adopt_autostart(), Err(PlatformError::NotBuilt(_))));
    }
}
