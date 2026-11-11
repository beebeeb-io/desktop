//! Which app this binary is: the real Beebeeb or a test build. The ONE place a per-app name comes from.
//!
//! A test build is the same code built with another bundle identifier (`io.beebeeb.app.test`). Everything one
//! install owns on a machine is named here, so a test build can never read, write or remove what the real app
//! owns: the config dir, the suggested sync folder, the credential-store names, the Cloud Files registration, the
//! data/cache dirs, the daemon socket, the log dir and the updater.
//!
//! Plain Rust, no Tauri types: the native app links the same module and sets `BEEBEEB_APP_IDENTIFIER` in its own
//! build. Every REAL value is byte-identical to what shipped before this module existed (an existing install must
//! keep finding its files and credentials); the tests below pin each one, and `source_census` fails when a per-app
//! literal appears anywhere else in the crate.

/// The real app's bundle identifier (`tauri.conf.json`'s `identifier`).
pub const REAL_BUNDLE_IDENTIFIER: &str = "io.beebeeb.app";
/// The test build's bundle identifier (`tools/agent-team/testbuild.ps1`).
pub const TEST_BUNDLE_IDENTIFIER: &str = "io.beebeeb.app.test";

/// The identifier this binary was built with (`build.rs`: the Tauri config's `identifier`, after `--config`).
pub const BUILT_BUNDLE_IDENTIFIER: &str = env!("BEEBEEB_APP_IDENTIFIER");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppIdentity {
    /// The released app. Its values never change: an install in the field finds its data by them.
    Real,
    /// Any other build. It shares no name with `Real`.
    Test,
}

impl AppIdentity {
    /// This binary's identity, fixed at compile time.
    pub const CURRENT: Self = Self::from_bundle_identifier(BUILT_BUNDLE_IDENTIFIER);

    pub const BOTH: [Self; 2] = [Self::Real, Self::Test];

    /// Only the exact real identifier is the real app. Anything else (a typo, a fork's id, an empty value) is a
    /// test build: an unknown build must never be handed the real app's credentials or folder.
    pub const fn from_bundle_identifier(identifier: &str) -> Self {
        if bytes_equal(identifier.as_bytes(), REAL_BUNDLE_IDENTIFIER.as_bytes()) {
            Self::Real
        } else {
            Self::Test
        }
    }

    pub const fn is_real(self) -> bool {
        matches!(self, Self::Real)
    }

    pub const fn bundle_identifier(self) -> &'static str {
        match self {
            Self::Real => REAL_BUNDLE_IDENTIFIER,
            Self::Test => TEST_BUNDLE_IDENTIFIER,
        }
    }

    /// The app's name as the bundle is built (`productName`); also the autostart entry's name, which the autostart
    /// plugin takes from the bundle.
    pub const fn product_name(self) -> &'static str {
        match self {
            Self::Real => "Beebeeb",
            Self::Test => "Beebeeb Test",
        }
    }

    /// The directory under the OS config, data, cache and temp dirs: `desktop.toml`, kept copies, staged writes.
    pub const fn data_dir_name(self) -> &'static str {
        match self {
            Self::Real => "beebeeb",
            Self::Test => "beebeeb-test",
        }
    }

    /// The folder name suggested for the sync folder (`~/Beebeeb`).
    pub const fn sync_folder_name(self) -> &'static str {
        match self {
            Self::Real => "Beebeeb",
            Self::Test => "Beebeeb Test",
        }
    }

    /// The service every credential-store item is named under: `<service>/<account id>/<leaf>`, and the legacy
    /// flat `<service>/<leaf>`. One namespace per identity covers both layouts, so a test build's legacy read,
    /// migration and sweep stay inside its own names. No real name starts with the test service plus `/`, and no
    /// test name starts with the real service plus `/`.
    pub const fn credential_service(self) -> &'static str {
        match self {
            Self::Real => "io.beebeeb.app",
            Self::Test => "io.beebeeb.app.test",
        }
    }

    /// Windows Cloud Files: the provider name (also the sync-root id's prefix, `<name>!<hash>`, and the name
    /// File Explorer shows).
    pub const fn cloud_files_provider_name(self) -> &'static str {
        match self {
            Self::Real => "Beebeeb",
            Self::Test => "BeebeebTest",
        }
    }

    /// Windows Cloud Files: the provider GUID a sync root is registered with.
    pub const fn cloud_files_provider_guid(self) -> u128 {
        match self {
            Self::Real => 0xb33b33b0_5217_4c0a_9e3f_1f2a7c4d8e90,
            Self::Test => 0x1876bd5b_c73c_452a_a40c_e2501f3de243,
        }
    }

    /// Windows: the COM class of the Explorer status column. It is registered per user with this build's exe path,
    /// so a shared class id would point the real app's sync root at the test build's exe.
    pub const fn status_ui_class_id(self) -> u128 {
        match self {
            Self::Real => 0xc0b33b33_5217_4c0a_9e3f_2f3b8d5e9fa1,
            Self::Test => 0xecbe203e_9c20_4ba7_babf_bfc6dad89f0e,
        }
    }

    /// Windows: the COM class of the thumbnail provider (registered like the status column's).
    pub const fn thumbnail_provider_class_id(self) -> u128 {
        match self {
            Self::Real => 0xb2e0a7d4_9c61_4f3a_8d2e_7a1c5e9b0f44,
            Self::Test => 0x691049a8_86db_450a_885a_fde5b4dc1431,
        }
    }

    /// Linux: the daemon socket's file name in the runtime dir.
    pub const fn daemon_socket_name(self) -> &'static str {
        match self {
            Self::Real => "beebeeb-daemon.sock",
            Self::Test => "beebeeb-test-daemon.sock",
        }
    }

    /// The sign-in diagnostic log's file name, directly in the OS data dir.
    pub const fn signin_log_name(self) -> &'static str {
        match self {
            Self::Real => "beebeeb-signin.log",
            Self::Test => "beebeeb-test-signin.log",
        }
    }

    /// macOS: the directory under `~/Library/Logs`.
    pub const fn logs_dir_name(self) -> &'static str {
        match self {
            Self::Real => "Beebeeb",
            Self::Test => "Beebeeb Test",
        }
    }

    /// A test build never asks for, downloads or installs an update: the update would be the real app's installer.
    pub const fn checks_for_updates(self) -> bool {
        self.is_real()
    }

    /// The updater's answer for this identity, before any network call.
    pub fn updater_refusal(self) -> Result<(), String> {
        if self.checks_for_updates() {
            Ok(())
        } else {
            Err(UPDATES_OFF_IN_TEST_BUILD.to_string())
        }
    }

    /// The bundle this binary runs in must be the one it was compiled for. A mismatch means the per-app names
    /// compiled in belong to another app than the one the OS started (its app-data dir, its installer), so the
    /// app must not start. `bundle_identifier` and `product_name` are what the shell reports at run time.
    pub fn check_bundle(self, bundle_identifier: &str, product_name: &str) -> Result<(), String> {
        if Self::from_bundle_identifier(bundle_identifier) != self {
            return Err(format!(
                "this build was compiled as {} but is running as {bundle_identifier}",
                self.bundle_identifier()
            ));
        }
        // A test build that kept the real product name would share the real app's autostart entry and installer.
        if !self.is_real() && product_name == Self::Real.product_name() {
            return Err(format!(
                "a test build must not be named {product_name}; set productName to {}",
                self.product_name()
            ));
        }
        Ok(())
    }
}

/// What the person reads when a test build is asked to update.
pub const UPDATES_OFF_IN_TEST_BUILD: &str = "This is a test build. It does not update itself.";

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

#[cfg(test)]
#[path = "app_identity_tests.rs"]
mod tests;
