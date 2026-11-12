//! Which app this binary is: the real Beebeeb or the test build. The ONE place a per-app name
//! comes from in the native crate; every name the ENGINE owns (credentials, the settings folder,
//! the sync folder, the Cloud Files registration, the backup device, the socket) comes from the
//! engine's identity, reached through [`AppIdentity::engine`], so the two cannot disagree.
//!
//! The identifier is fixed at build time by `build.rs` from `BEEBEEB_APP_IDENTIFIER`:
//!
//! | `BEEBEEB_APP_IDENTIFIER` | `BEEBEEB_RELEASE_VERSION` | the build is |
//! |---|---|---|
//! | `io.beebeeb.app` | set or unset | the real app |
//! | `io.beebeeb.app.test` | unset | the test build ("Beebeeb Test") |
//! | `io.beebeeb.app.test` | set | refused: a release is the real app |
//! | unset | unset | the test build: `cargo build`, `cargo run` and `cargo test` |
//! | unset | set | refused: a release names its identity |
//! | anything else | any | refused |
//!
//! Nothing reads the profile (`debug_assertions`), an environment variable at run time or an
//! argument. A test build shares NOTHING with the real one: not the control channel (so neither
//! answers the other's second launch), not the state folder, not the start-at-login value, not
//! the window titles or classes, not the link scheme.
//! `every_derived_value_differs_between_the_two_identities` asserts each one. The tests name the
//! identity they mean, so they pass under either build.

/// The real app's bundle identifier (Tauri's `identifier`): the state and config folders of an
/// install in the field are named by it.
pub const REAL_BUNDLE_IDENTIFIER: &str = "io.beebeeb.app";
/// The test build's bundle identifier.
pub const TEST_BUNDLE_IDENTIFIER: &str = "io.beebeeb.app.test";
/// The identifier this binary was built with (`build.rs`: one of the two, or the build stops).
pub const BUILT_BUNDLE_IDENTIFIER: &str = env!("BEEBEEB_APP_IDENTIFIER");

/// `beebeeb-app-identity=<identifier>;`, written by `build.rs` and kept in the binary by `main`. The packager
/// (`packaging/windows/build-installer.ps1`) and the release workflow read it out of the built file and refuse a
/// binary whose mark is not the one they package: what is checked is the binary, not the environment it was
/// meant to be built in. Exactly one mark is in a binary; the bare identifiers of both apps are in every binary.
pub static BUILD_MARK: &str = env!("BEEBEEB_APP_IDENTITY_MARK");

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
    /// This binary's identity, fixed at compile time. An identifier that is neither app does not compile
    /// (`build.rs` refuses it first; this is the second lock, for a build that bypasses the build script).
    pub const CURRENT: Self = match Self::from_bundle_identifier(BUILT_BUNDLE_IDENTIFIER) {
        Some(identity) => identity,
        None => panic!("BEEBEEB_APP_IDENTIFIER is neither io.beebeeb.app nor io.beebeeb.app.test"),
    };
    #[cfg_attr(not(test), allow(dead_code))]
    pub const BOTH: [Self; 2] = [Self::Real, Self::Test];

    /// Only the two exact identifiers name an app. Anything else (a typo, a fork's id, an empty
    /// value, another case) names none: an unknown build is not built.
    pub const fn from_bundle_identifier(identifier: &str) -> Option<Self> {
        if bytes_equal(identifier.as_bytes(), REAL_BUNDLE_IDENTIFIER.as_bytes()) {
            Some(Self::Real)
        } else if bytes_equal(identifier.as_bytes(), TEST_BUNDLE_IDENTIFIER.as_bytes()) {
            Some(Self::Test)
        } else {
            None
        }
    }

    /// The engine's identity for this app: what `Engine::open` and every engine entry take. The one bridge
    /// between the two types (a test pins that they name the same bundle identifier).
    #[cfg_attr(not(test), allow(dead_code))]
    pub const fn engine(self) -> &'static beebeeb_engine::identity::EngineIdentity {
        match self {
            Self::Real => &beebeeb_engine::identity::REAL,
            Self::Test => &beebeeb_engine::identity::TEST,
        }
    }

    /// Why this app must not start on `os` (`std::env::consts::OS`), or `None` when it may. The engine's
    /// rule: the test build has no macOS App Group and no File Provider extension of its own, so there it
    /// would have to use the real app's. `shell::run` asks before anything else exists.
    pub fn start_refusal(self, os: &str) -> Option<&'static str> {
        self.engine().refusal_on(os)
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
        assert_eq!(
            AppIdentity::from_bundle_identifier("io.beebeeb.app"),
            Some(AppIdentity::Real)
        );
        assert_eq!(
            AppIdentity::from_bundle_identifier("io.beebeeb.app.test"),
            Some(AppIdentity::Test)
        );
        for other in [
            "",
            "io.beebeeb.App",
            "io.beebeeb.app ",
            "io.beebeeb.app.test2",
            "io.beebeeb.app.tes",
            "com.example",
        ] {
            assert_eq!(AppIdentity::from_bundle_identifier(other), None, "{other:?}");
        }
        assert_eq!(AppIdentity::BOTH.len(), 2);
    }

    /// `main` asks, and leaves, before the shell runs: the refusal has a caller.
    #[test]
    fn main_asks_before_it_runs_the_shell() {
        let main = include_str!("../main.rs").replace("\r\n", "\n");
        let body = &main[main.find("fn main() {").expect("main")..];
        let ask = body
            .find("AppIdentity::CURRENT.start_refusal(std::env::consts::OS)")
            .expect("main asks whether this identity may start on this OS");
        let run = body.find("shell::run();").expect("main runs the shell");
        assert!(ask < run, "the question comes before the shell");
        assert!(
            body[ask..run].contains("std::process::exit(2);"),
            "a refusal ends the process"
        );
        assert_eq!(body.matches("shell::run();").count(), 1);
    }

    /// What a packager searches the built file for, put together at run time: written out here it would be a
    /// second mark in the test binary.
    fn mark_prefix() -> String {
        ["beebeeb-app", "-identity", "="].concat()
    }

    /// The mark names this build's identifier, in the exact form the packager and the workflow search for.
    #[test]
    fn the_build_mark_names_this_build() {
        assert_eq!(
            BUILD_MARK,
            [mark_prefix().as_str(), AppIdentity::CURRENT.bundle_identifier(), ";"].concat()
        );
        let main = include_str!("../main.rs");
        assert!(
            main.contains("std::hint::black_box(shell::identity::BUILD_MARK);"),
            "main keeps the mark in the binary"
        );
    }

    /// This built file holds exactly ONE mark, and it is this build's. Both apps' bare identifiers are in the
    /// file (the engine names both), so "the file holds the identifier" proves nothing; the mark does. Run as
    /// each identity (`BEEBEEB_APP_IDENTIFIER=io.beebeeb.app cargo test` for the real one);
    /// `scripts/native-identity-mark.sh` does the same on the app binary itself.
    #[test]
    fn this_built_file_holds_exactly_one_identity_mark() {
        let file = std::fs::read(std::env::current_exe().expect("this test binary")).expect("read it");
        let prefix = mark_prefix().into_bytes();
        let mut marks = Vec::new();
        let mut at = 0;
        while let Some(found) = file[at..]
            .windows(prefix.len())
            .position(|bytes| bytes == prefix.as_slice())
        {
            let start = at + found + prefix.len();
            let id: Vec<u8> = file[start..]
                .iter()
                .copied()
                .take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'.')
                .collect();
            if file.get(start + id.len()) == Some(&b';') {
                marks.push(String::from_utf8(id).expect("ascii"));
            }
            at = start;
        }
        assert_eq!(marks, [AppIdentity::CURRENT.bundle_identifier()]);
        // And the other app's identifier IS in this file: why the mark exists.
        let other = match AppIdentity::CURRENT {
            AppIdentity::Real => AppIdentity::Test,
            AppIdentity::Test => AppIdentity::Real,
        };
        let other_id = std::hint::black_box(other).engine().bundle_identifier().as_bytes();
        assert!(file.windows(other_id.len()).any(|bytes| bytes == other_id));
    }

    /// A build nobody named is the test build (plain `cargo test`, as CI runs it, is one): the default never
    /// reaches the real app's data.
    #[test]
    fn an_unnamed_build_is_the_test_build() {
        match env!("BEEBEEB_APP_IDENTIFIER_SOURCE") {
            "default" => assert_eq!(AppIdentity::CURRENT, AppIdentity::Test),
            "named" => {}
            other => panic!("the build script says {other:?}"),
        }
    }

    /// The build script's answer is one of the two, and this binary is that one.
    #[test]
    fn this_build_is_the_identity_it_was_built_as() {
        assert_eq!(
            AppIdentity::from_bundle_identifier(BUILT_BUNDLE_IDENTIFIER),
            Some(AppIdentity::CURRENT)
        );
        assert_eq!(AppIdentity::CURRENT.bundle_identifier(), BUILT_BUNDLE_IDENTIFIER);
    }

    /// The native identity and the engine's are the same app: one bundle identifier each way.
    #[test]
    fn the_engine_identity_is_the_same_app() {
        for identity in AppIdentity::BOTH {
            assert_eq!(identity.engine().bundle_identifier(), identity.bundle_identifier());
            assert_eq!(
                beebeeb_engine::identity::EngineIdentity::from_bundle_identifier(identity.bundle_identifier())
                    .map(std::ptr::from_ref),
                Some(std::ptr::from_ref(identity.engine()))
            );
        }
        assert!(AppIdentity::Real.engine().is_real());
        assert!(!AppIdentity::Test.engine().is_real());
    }

    /// The test build does not start on macOS; the real app starts everywhere, the test build elsewhere.
    #[test]
    fn the_test_build_is_refused_on_macos_only() {
        assert!(AppIdentity::Test.start_refusal("macos").is_some());
        for os in ["windows", "linux"] {
            assert_eq!(AppIdentity::Test.start_refusal(os), None, "{os}");
        }
        for os in ["macos", "windows", "linux"] {
            assert_eq!(AppIdentity::Real.start_refusal(os), None, "{os}");
        }
    }
}
