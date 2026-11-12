//! Which app this binary is: the real Beebeeb or a test build. The ONE place the credential
//! service name and the config folder name are defined.
//!
//! The host initialises the identity once before opening an engine or reading the config. The
//! Tauri shell defaults to `REAL` (backward compat); the native shell sets it from its own
//! `AppIdentity`, which enforces release = Real, debug = Test.
//!
//! A test build that forgets to initialise gets `REAL` as the fallback so existing Tauri-shell
//! tests (which never call `init`) keep working. The native shell calls `init` before any engine
//! work, and its own tests call `init_for_test`.

use std::sync::OnceLock;

/// What the engine needs from the host to name its credential entries and its config folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineIdentity {
    /// The service name in the platform credential store (Windows Credential Manager target prefix,
    /// macOS Keychain service name): `io.beebeeb.app` for the real app.
    pub credential_service: &'static str,
    /// The folder inside the user's config dir (`~/.config/<this>/desktop.toml`): `beebeeb` for
    /// the real app.
    pub config_dir_name: &'static str,
}

/// The released app. Its values never change: an install in the field finds its data by them.
pub const REAL: EngineIdentity = EngineIdentity {
    credential_service: "io.beebeeb.app",
    config_dir_name: "beebeeb",
};

/// A test build. It shares no name with [`REAL`].
pub const TEST: EngineIdentity = EngineIdentity {
    credential_service: "io.beebeeb.app.test",
    config_dir_name: "beebeeb-test",
};

static CREDENTIAL_SERVICE: OnceLock<&'static str> = OnceLock::new();
static CONFIG_DIR_NAME: OnceLock<&'static str> = OnceLock::new();

/// Set the process-wide identity. First writer wins (same semantics as `state_paths::init_*`).
pub fn init(identity: &EngineIdentity) {
    let _ = CREDENTIAL_SERVICE.set(identity.credential_service);
    let _ = CONFIG_DIR_NAME.set(identity.config_dir_name);
}

/// The credential service name for the platform store. Falls back to `REAL` when not initialised
/// (Tauri-shell backward compat).
pub fn credential_service() -> &'static str {
    CREDENTIAL_SERVICE.get().copied().unwrap_or(REAL.credential_service)
}

/// The config folder name. Falls back to `REAL` when not initialised.
pub fn config_dir_name() -> &'static str {
    CONFIG_DIR_NAME.get().copied().unwrap_or(REAL.config_dir_name)
}

/// Test helper: initialise with `TEST`.
#[cfg(any(test, feature = "test-support"))]
pub fn init_for_test() {
    init(&TEST);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real identity's values are the constants the engine has always used: an install in the
    /// field depends on each byte.
    #[test]
    fn the_real_identity_is_pinned() {
        assert_eq!(REAL.credential_service, "io.beebeeb.app");
        assert_eq!(REAL.config_dir_name, "beebeeb");
    }

    #[test]
    fn the_test_identity_is_pinned() {
        assert_eq!(TEST.credential_service, "io.beebeeb.app.test");
        assert_eq!(TEST.config_dir_name, "beebeeb-test");
    }

    #[test]
    fn every_field_differs_between_the_two_identities() {
        let pairs = [
            ("credential_service", REAL.credential_service, TEST.credential_service),
            ("config_dir_name", REAL.config_dir_name, TEST.config_dir_name),
        ];
        assert_eq!(pairs.len(), 2, "one line per field");
        for (name, real, test) in pairs {
            assert_ne!(real, test, "{name}");
            assert!(!real.is_empty() && !test.is_empty(), "{name}");
        }
    }

    #[test]
    fn only_the_exact_real_values_are_the_real_identity() {
        assert_eq!(
            REAL,
            EngineIdentity {
                credential_service: "io.beebeeb.app",
                config_dir_name: "beebeeb"
            }
        );
        // Anything else is not REAL.
        assert_ne!(TEST, REAL);
        let forged = EngineIdentity {
            credential_service: "io.beebeeb.App",
            config_dir_name: "beebeeb",
        };
        assert_ne!(forged, REAL);
    }

    #[test]
    fn credential_service_defaults_to_the_real_value() {
        // OnceLock is per-process; if another test set it first, this test sees that value.
        // The point is: an un-initialised engine reads the real name (Tauri compat).
        let value = credential_service();
        assert!(
            value == REAL.credential_service || value == TEST.credential_service,
            "unexpected: {value}"
        );
    }

    #[test]
    fn config_dir_name_defaults_to_the_real_value() {
        let value = config_dir_name();
        assert!(
            value == REAL.config_dir_name || value == TEST.config_dir_name,
            "unexpected: {value}"
        );
    }
}
