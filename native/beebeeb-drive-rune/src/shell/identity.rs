//! Which app this binary is: the real Beebeeb or a test build. The ONE place a per-app name
//! comes from in the native crate.
//!
//! The same shape, names and values as `src-tauri/src/app_identity.rs` (branch
//! `fix/2445-test-build-isolation`), so the two can become one module: `AppIdentity::{Real,
//! Test}`, `REAL_BUNDLE_IDENTIFIER`, `TEST_BUNDLE_IDENTIFIER`, `from_bundle_identifier` (only
//! the exact real identifier is the real app), `bundle_identifier()`, `product_name()`. The
//! values below those are the native shell's own derivations.
//!
//! The identifier is fixed at build time: `BEEBEEB_APP_IDENTIFIER=io.beebeeb.app.test cargo
//! build` makes a "Beebeeb Test". Unset, the build is the real app. A test build shares NOTHING
//! with the real one: not the control channel (so neither answers the other's second launch),
//! not the state folder, not the start-at-login value, not the window titles or classes, not
//! the link scheme. `every_derived_value_differs_between_the_two_identities` asserts each one.

/// The real app's bundle identifier (Tauri's `identifier`): the state and config folders of an
/// install in the field are named by it.
pub const REAL_BUNDLE_IDENTIFIER: &str = "io.beebeeb.app";
/// The test build's bundle identifier.
pub const TEST_BUNDLE_IDENTIFIER: &str = "io.beebeeb.app.test";
/// The identifier this binary was built with.
pub const BUILT_BUNDLE_IDENTIFIER: &str = match option_env!("BEEBEEB_APP_IDENTIFIER") {
    Some(identifier) => identifier,
    None => REAL_BUNDLE_IDENTIFIER,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppIdentity {
    /// The released app. Its values never change: an install in the field finds its data by them.
    Real,
    /// Any other build. It shares no name with `Real`.
    Test,
}

const fn bytes_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

impl AppIdentity {
    /// This binary's identity, fixed at compile time.
    pub const CURRENT: Self = Self::from_bundle_identifier(BUILT_BUNDLE_IDENTIFIER);
    #[cfg_attr(not(test), allow(dead_code))]
    pub const BOTH: [Self; 2] = [Self::Real, Self::Test];

    /// Only the exact real identifier is the real app. Anything else (a typo, a fork's id, an
    /// empty value) is a test build: an unknown build must never touch what the real app owns.
    pub const fn from_bundle_identifier(identifier: &str) -> Self {
        if bytes_equal(identifier.as_bytes(), REAL_BUNDLE_IDENTIFIER.as_bytes()) {
            Self::Real
        } else {
            Self::Test
        }
    }

    pub const fn bundle_identifier(self) -> &'static str {
        match self {
            Self::Real => REAL_BUNDLE_IDENTIFIER,
            Self::Test => TEST_BUNDLE_IDENTIFIER,
        }
    }

    /// The app's name: the tooltip's first word, the title of a native message box, and the
    /// name of the start-at-login value (what `tauri-plugin-autostart` took from the bundle).
    pub const fn product_name(self) -> &'static str {
        match self {
            Self::Real => "Beebeeb",
            Self::Test => "Beebeeb Test",
        }
    }

    // --- the native shell's own derivations ---

    /// The folder under the OS state, config and data dirs (`rune::dirs::AppDirs::new`).
    pub const fn state_dir_id(self) -> &'static str {
        self.bundle_identifier()
    }

    /// The folder (under the local data dir) that holds the live feedback mode's inbox. The
    /// feature exists only in test builds, so this is the TEST identity's folder for both: a
    /// build that has the feature and the real identifier still never writes a note into the
    /// real app's data folder.
    #[cfg(feature = "feedback")]
    pub const fn feedback_dir_id(self) -> &'static str {
        match self {
            Self::Real | Self::Test => TEST_BUNDLE_IDENTIFIER,
        }
    }

    /// The control channel's name (`control.rs`); the platform adds the user and the Windows
    /// session to it. It is also the single-instance lock: one server per identity, user AND
    /// session; exclusivity across sessions comes from the engine/profile lock (not linked yet).
    pub const fn control_channel(self) -> &'static str {
        match self {
            Self::Real => "io.beebeeb.app.control",
            Self::Test => "io.beebeeb.app.test.control",
        }
    }

    /// The value under `HKCU\…\Run` (and `StartupApproved\Run`).
    pub const fn run_value_name(self) -> &'static str {
        self.product_name()
    }

    pub const fn main_window_title(self) -> &'static str {
        match self {
            Self::Real => "Beebeeb Drive",
            Self::Test => "Beebeeb Test Drive",
        }
    }

    /// The flyout's (never shown) title.
    pub const fn flyout_window_title(self) -> &'static str {
        self.product_name()
    }

    /// The class of the hidden window that owns the b.
    pub const fn tray_window_class(self) -> &'static str {
        match self {
            Self::Real => "BeebeebTray",
            Self::Test => "BeebeebTestTray",
        }
    }

    /// The link scheme (`beebeeb://…`). Not registered with the OS in 0.9.0.
    pub const fn url_scheme(self) -> &'static str {
        match self {
            Self::Real => "beebeeb",
            Self::Test => "beebeeb-test",
        }
    }

    /// The scheme's friendly name in the registry (`URL:Beebeeb`).
    pub const fn url_scheme_label(self) -> &'static str {
        match self {
            Self::Real => "URL:Beebeeb",
            Self::Test => "URL:Beebeeb Test",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real app's values, byte for byte: an install in the field depends on each.
    #[test]
    fn the_real_identity_is_pinned() {
        let real = AppIdentity::Real;
        assert_eq!(real.bundle_identifier(), "io.beebeeb.app");
        assert_eq!(real.product_name(), "Beebeeb");
        assert_eq!(real.state_dir_id(), "io.beebeeb.app");
        assert_eq!(real.control_channel(), "io.beebeeb.app.control");
        assert_eq!(real.run_value_name(), "Beebeeb");
        assert_eq!(real.main_window_title(), "Beebeeb Drive");
        assert_eq!(real.flyout_window_title(), "Beebeeb");
        assert_eq!(real.tray_window_class(), "BeebeebTray");
        assert_eq!(real.url_scheme(), "beebeeb");
        assert_eq!(real.url_scheme_label(), "URL:Beebeeb");
    }

    #[test]
    fn the_test_identity_is_pinned() {
        let test = AppIdentity::Test;
        assert_eq!(test.bundle_identifier(), "io.beebeeb.app.test");
        assert_eq!(test.product_name(), "Beebeeb Test");
    }

    #[test]
    fn every_derived_value_differs_between_the_two_identities() {
        let (real, test) = (AppIdentity::Real, AppIdentity::Test);
        let pairs = [
            ("bundle_identifier", real.bundle_identifier(), test.bundle_identifier()),
            ("product_name", real.product_name(), test.product_name()),
            ("state_dir_id", real.state_dir_id(), test.state_dir_id()),
            ("control_channel", real.control_channel(), test.control_channel()),
            ("run_value_name", real.run_value_name(), test.run_value_name()),
            ("main_window_title", real.main_window_title(), test.main_window_title()),
            (
                "flyout_window_title",
                real.flyout_window_title(),
                test.flyout_window_title(),
            ),
            ("tray_window_class", real.tray_window_class(), test.tray_window_class()),
            ("url_scheme", real.url_scheme(), test.url_scheme()),
            ("url_scheme_label", real.url_scheme_label(), test.url_scheme_label()),
        ];
        assert_eq!(pairs.len(), 10, "one line per derived value");
        for (name, real, test) in pairs {
            assert_ne!(real, test, "{name}");
            assert!(!real.is_empty() && !test.is_empty(), "{name}");
        }
        // Windows names compare without case, and a prefix must not make one a sub-path of the
        // other's pipe or registry name.
        assert!(!real.control_channel().eq_ignore_ascii_case(test.control_channel()));
        assert!(!real.url_scheme().eq_ignore_ascii_case(test.url_scheme()));
    }

    #[test]
    fn only_the_exact_real_identifier_is_the_real_app() {
        assert_eq!(AppIdentity::from_bundle_identifier("io.beebeeb.app"), AppIdentity::Real);
        for other in [
            "io.beebeeb.app.test",
            "",
            "io.beebeeb.App",
            "io.beebeeb.app ",
            "com.example",
        ] {
            assert_eq!(
                AppIdentity::from_bundle_identifier(other),
                AppIdentity::Test,
                "{other:?}"
            );
        }
        // This build: the default is the real app.
        if option_env!("BEEBEEB_APP_IDENTIFIER").is_none() {
            assert_eq!(AppIdentity::CURRENT, AppIdentity::Real);
        }
        assert_eq!(AppIdentity::BOTH.len(), 2);
    }
}
