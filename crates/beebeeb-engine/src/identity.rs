//! Which app this process is: the real Beebeeb, or the test build that is installed beside it.
//!
//! Everything a second install on the same Windows or macOS user account must not share with the first is a
//! field of [`EngineIdentity`]: the config folder, the state folder, the credential service, the suggested sync
//! folder, the Cloud Files provider (name, sync-root id prefix, GUID, the two COM classes), the backup device,
//! the daemon socket, the macOS app group and the log folder. There are exactly two values, [`REAL`] and
//! [`TEST`]; the fields are private, so no third one can be made outside this file.
//!
//! There is no default and nothing to initialise. Every engine entry that reaches credentials, the config, local
//! state or Cloud Files takes a `&'static EngineIdentity`; code that has none does not compile. The host decides
//! which one it is, once, at build time (the Tauri shell is always [`REAL`]).
//!
//! This file names nothing else of the crate: `src-tauri/tests/keychain.rs` includes it by path, next to
//! `keychain.rs`.
//!
//! Must not: gain a public constructor, a `Default`, a process-wide "current" value, or a field that both
//! identities share when the thing it names is per-install.

/// One install's names. See the module doc; get one from [`REAL`] or [`TEST`].
#[derive(Debug, PartialEq, Eq)]
pub struct EngineIdentity {
    bundle_identifier: &'static str,
    config_dir_name: &'static str,
    state_dir_name: &'static str,
    credential_service: &'static str,
    sync_folder_name: &'static str,
    cloud_files_provider_name: &'static str,
    cloud_files_provider_guid: u128,
    status_ui_class_id: u128,
    thumbnail_provider_class_id: u128,
    backup_device_salt: &'static str,
    backup_device_name_suffix: &'static str,
    daemon_socket_name: &'static str,
    macos_app_group: &'static str,
    logs_dir_name: &'static str,
}

/// The released app. Every value is the literal the app has always used (`the_real_identity_is_todays_bytes`).
pub static REAL: EngineIdentity = EngineIdentity {
    bundle_identifier: "io.beebeeb.app",
    config_dir_name: "beebeeb",
    state_dir_name: "",
    credential_service: "io.beebeeb.app",
    sync_folder_name: "Beebeeb",
    cloud_files_provider_name: "Beebeeb",
    cloud_files_provider_guid: 0xb33b33b0_5217_4c0a_9e3f_1f2a7c4d8e90,
    status_ui_class_id: 0xc0b33b33_5217_4c0a_9e3f_2f3b8d5e9fa1,
    thumbnail_provider_class_id: 0xb2e0a7d4_9c61_4f3a_8d2e_7a1c5e9b0f44,
    backup_device_salt: "",
    backup_device_name_suffix: "",
    daemon_socket_name: "beebeeb-daemon.sock",
    macos_app_group: "R8352WDJJR.io.beebeeb.app.fileprovider",
    logs_dir_name: "Beebeeb",
};

/// The test build, installed beside the real app on the same user account. Shares nothing with [`REAL`]
/// (`the_two_identities_share_no_value`).
pub static TEST: EngineIdentity = EngineIdentity {
    bundle_identifier: "io.beebeeb.app.test",
    config_dir_name: "beebeeb-test",
    state_dir_name: "beebeeb-test",
    credential_service: "io.beebeeb.app.test",
    sync_folder_name: "Beebeeb Test",
    cloud_files_provider_name: "BeebeebTest",
    cloud_files_provider_guid: 0x1876bd5b_c73c_452a_a40c_e2501f3de243,
    status_ui_class_id: 0xecbe203e_9c20_4ba7_babf_bfc6dad89f0e,
    thumbnail_provider_class_id: 0x691049a8_86db_450a_885a_fde5b4dc1431,
    backup_device_salt: "io.beebeeb.app.test",
    backup_device_name_suffix: " (test)",
    daemon_socket_name: "beebeeb-test-daemon.sock",
    macos_app_group: "R8352WDJJR.io.beebeeb.app.test.fileprovider",
    logs_dir_name: "Beebeeb Test",
};

impl EngineIdentity {
    /// The identity a bundle identifier names: exactly the two known ones, anything else is `None`.
    pub const fn from_bundle_identifier(identifier: &str) -> Option<&'static EngineIdentity> {
        if bytes_equal(identifier.as_bytes(), REAL.bundle_identifier.as_bytes()) {
            Some(&REAL)
        } else if bytes_equal(identifier.as_bytes(), TEST.bundle_identifier.as_bytes()) {
            Some(&TEST)
        } else {
            None
        }
    }

    /// The other build of this app (the test build for the real app, and the other way around): the one whose
    /// folders this one must never sync into.
    pub fn other_build(&self) -> &'static EngineIdentity {
        if self.is_real() { &TEST } else { &REAL }
    }

    /// Whether this is the released app.
    pub const fn is_real(&self) -> bool {
        bytes_equal(self.bundle_identifier.as_bytes(), REAL.bundle_identifier.as_bytes())
    }

    /// `io.beebeeb.app` or `io.beebeeb.app.test`.
    pub const fn bundle_identifier(&self) -> &'static str {
        self.bundle_identifier
    }

    /// The folder under the user's config dir that holds `desktop.toml`; also the folder under the data, cache
    /// and temp dirs for staged and kept copies, and the Linux FUSE filesystem name.
    pub const fn config_dir_name(&self) -> &'static str {
        self.config_dir_name
    }

    /// The local state folder (`state.db`, the sync lock) for a host that names a BASE dir: the base itself for
    /// the real app (its host has always passed the app's own data dir), a sub-folder of it for the test build.
    pub fn state_dir_in(&self, base: &std::path::Path) -> std::path::PathBuf {
        if self.state_dir_name.is_empty() {
            base.to_path_buf()
        } else {
            base.join(self.state_dir_name)
        }
    }

    /// Why this app must not run on `os` (`std::env::consts::OS`), or `None` when it may. The test build has no
    /// macOS App Group and no File Provider extension signed for it, so on a Mac it could only use the real
    /// app's group container: it does not start there. `Engine::open` refuses with this, and a host asks
    /// before it makes anything.
    pub fn refusal_on(&self, os: &str) -> Option<&'static str> {
        (!self.is_real() && os == "macos").then_some(
            "This is a test build of Beebeeb. Test builds do not run on macOS: they would share the real app's data there.",
        )
    }

    /// Where this app's throw-away files go under an OS temp or cache dir, and so the only part of that dir a
    /// purge may delete from: the dir itself for the real app (its cache paths have always been recorded
    /// anywhere under it, and a narrower root would leave an earlier build's decrypted copies behind at
    /// sign-out), a sub-folder for the test build, which therefore never deletes outside its own folder.
    pub fn scratch_dir_in(&self, base: &std::path::Path) -> std::path::PathBuf {
        self.state_dir_in(base)
    }

    /// The service every stored credential is named under (Windows: `<service>/<leaf>`; macOS: the Keychain
    /// service).
    pub const fn credential_service(&self) -> &'static str {
        self.credential_service
    }

    /// The folder suggested in the person's home dir for synced files.
    pub const fn sync_folder_name(&self) -> &'static str {
        self.sync_folder_name
    }

    /// The Cloud Files provider name: what Explorer shows, and the prefix of every sync-root id.
    pub const fn cloud_files_provider_name(&self) -> &'static str {
        self.cloud_files_provider_name
    }

    /// The Cloud Files provider GUID.
    pub const fn cloud_files_provider_guid(&self) -> u128 {
        self.cloud_files_provider_guid
    }

    /// The COM class of the Explorer status flyout.
    pub const fn status_ui_class_id(&self) -> u128 {
        self.status_ui_class_id
    }

    /// The COM class of the thumbnail provider.
    pub const fn thumbnail_provider_class_id(&self) -> u128 {
        self.thumbnail_provider_class_id
    }

    /// Mixed into the backup device id, so the test build is never the same backup device as the real app on
    /// the same machine. Empty for the real app: its device id is the machine's, as always.
    pub const fn backup_device_salt(&self) -> &'static str {
        self.backup_device_salt
    }

    /// Appended to the backup device's name (`" (test)"` for the test build).
    pub const fn backup_device_name_suffix(&self) -> &'static str {
        self.backup_device_name_suffix
    }

    /// The Linux daemon socket's file name in the runtime dir.
    pub const fn daemon_socket_name(&self) -> &'static str {
        self.daemon_socket_name
    }

    /// The macOS app group container the app shares with its File Provider extension.
    pub const fn macos_app_group(&self) -> &'static str {
        self.macos_app_group
    }

    /// The folder under `~/Library/Logs` (macOS).
    pub const fn logs_dir_name(&self) -> &'static str {
        self.logs_dir_name
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The released app's names, as literals: a change here moves every person's config, credentials and sync
    /// root registration.
    #[test]
    fn the_real_identity_is_todays_bytes() {
        assert_eq!(REAL.bundle_identifier(), "io.beebeeb.app");
        assert_eq!(REAL.config_dir_name(), "beebeeb");
        assert_eq!(REAL.credential_service(), "io.beebeeb.app");
        assert_eq!(REAL.sync_folder_name(), "Beebeeb");
        assert_eq!(REAL.cloud_files_provider_name(), "Beebeeb");
        assert_eq!(REAL.cloud_files_provider_guid(), 0xb33b33b0_5217_4c0a_9e3f_1f2a7c4d8e90);
        assert_eq!(REAL.status_ui_class_id(), 0xc0b3_3b33_5217_4c0a_9e3f_2f3b8d5e9fa1);
        assert_eq!(
            REAL.thumbnail_provider_class_id(),
            0xb2e0a7d4_9c61_4f3a_8d2e_7a1c5e9b0f44
        );
        assert_eq!(REAL.backup_device_salt(), "");
        assert_eq!(REAL.backup_device_name_suffix(), "");
        assert_eq!(REAL.daemon_socket_name(), "beebeeb-daemon.sock");
        assert_eq!(REAL.macos_app_group(), "R8352WDJJR.io.beebeeb.app.fileprovider");
        assert_eq!(REAL.logs_dir_name(), "Beebeeb");
        let base = std::path::Path::new("/data/io.beebeeb.app");
        assert_eq!(
            REAL.state_dir_in(base),
            base,
            "the real app's state folder is the dir its host names"
        );
        assert!(REAL.is_real() && !TEST.is_real());
    }

    #[test]
    fn the_test_identity_is_the_documented_one() {
        assert_eq!(TEST.bundle_identifier(), "io.beebeeb.app.test");
        assert_eq!(TEST.config_dir_name(), "beebeeb-test");
        assert_eq!(TEST.credential_service(), "io.beebeeb.app.test");
        assert_eq!(TEST.sync_folder_name(), "Beebeeb Test");
        assert_eq!(TEST.cloud_files_provider_name(), "BeebeebTest");
        assert_eq!(TEST.backup_device_name_suffix(), " (test)");
        assert_eq!(TEST.daemon_socket_name(), "beebeeb-test-daemon.sock");
        assert_eq!(TEST.cloud_files_provider_guid(), 0x1876bd5b_c73c_452a_a40c_e2501f3de243);
        assert_eq!(TEST.status_ui_class_id(), 0xecbe203e_9c20_4ba7_babf_bfc6dad89f0e);
        assert_eq!(
            TEST.thumbnail_provider_class_id(),
            0x691049a8_86db_450a_885a_fde5b4dc1431
        );
        assert_eq!(TEST.backup_device_salt(), "io.beebeeb.app.test");
        assert_eq!(TEST.macos_app_group(), "R8352WDJJR.io.beebeeb.app.test.fileprovider");
        assert_eq!(TEST.logs_dir_name(), "Beebeeb Test");
        let base = std::path::Path::new("/data/base");
        assert_eq!(TEST.state_dir_in(base), base.join("beebeeb-test"));
    }

    /// The real app's throw-away files are anywhere under the OS dir (as its recorded cache paths have always
    /// been); the test build's only under its own folder, so its purge cannot reach anything else.
    #[test]
    fn the_test_build_keeps_its_scratch_files_in_its_own_folder() {
        let temp = std::path::Path::new("/tmp");
        assert_eq!(REAL.scratch_dir_in(temp), temp);
        assert_eq!(TEST.scratch_dir_in(temp), temp.join("beebeeb-test"));
        assert!(!temp.join("bb_real-file").starts_with(TEST.scratch_dir_in(temp)));
    }

    /// The test build does not run on macOS; nothing else is refused.
    #[test]
    fn only_the_test_build_on_macos_is_refused() {
        assert!(TEST.refusal_on("macos").is_some_and(|words| words.contains("macOS")));
        for os in ["windows", "linux", "freebsd", ""] {
            assert_eq!(TEST.refusal_on(os), None, "{os:?}");
        }
        for os in ["macos", "windows", "linux"] {
            assert_eq!(REAL.refusal_on(os), None, "{os}");
        }
    }

    /// Field by field: nothing per-install is the same for both. A field added to the struct must be added here
    /// (the destructuring below does not compile otherwise).
    #[test]
    fn the_two_identities_share_no_value() {
        let EngineIdentity {
            bundle_identifier,
            config_dir_name,
            state_dir_name,
            credential_service,
            sync_folder_name,
            cloud_files_provider_name,
            cloud_files_provider_guid,
            status_ui_class_id,
            thumbnail_provider_class_id,
            backup_device_salt,
            backup_device_name_suffix,
            daemon_socket_name,
            macos_app_group,
            logs_dir_name,
        } = &REAL;
        assert_ne!(*bundle_identifier, TEST.bundle_identifier);
        assert_ne!(*config_dir_name, TEST.config_dir_name);
        assert_ne!(*state_dir_name, TEST.state_dir_name);
        assert_ne!(*credential_service, TEST.credential_service);
        assert_ne!(*sync_folder_name, TEST.sync_folder_name);
        assert_ne!(*cloud_files_provider_name, TEST.cloud_files_provider_name);
        assert_ne!(*cloud_files_provider_guid, TEST.cloud_files_provider_guid);
        assert_ne!(*status_ui_class_id, TEST.status_ui_class_id);
        assert_ne!(*thumbnail_provider_class_id, TEST.thumbnail_provider_class_id);
        assert_ne!(*backup_device_salt, TEST.backup_device_salt);
        assert_ne!(*backup_device_name_suffix, TEST.backup_device_name_suffix);
        assert_ne!(*daemon_socket_name, TEST.daemon_socket_name);
        assert_ne!(*macos_app_group, TEST.macos_app_group);
        assert_ne!(*logs_dir_name, TEST.logs_dir_name);
        // No test name is a prefix-free accident: none equals the real one case-insensitively either.
        assert!(!TEST.sync_folder_name.eq_ignore_ascii_case(REAL.sync_folder_name));
        assert!(
            !TEST
                .cloud_files_provider_name
                .eq_ignore_ascii_case(REAL.cloud_files_provider_name)
        );
    }

    #[test]
    fn only_the_two_bundle_identifiers_name_an_identity() {
        assert!(std::ptr::eq(
            EngineIdentity::from_bundle_identifier("io.beebeeb.app").unwrap(),
            &REAL
        ));
        assert!(std::ptr::eq(
            EngineIdentity::from_bundle_identifier("io.beebeeb.app.test").unwrap(),
            &TEST
        ));
        for other in [
            "",
            "io.beebeeb.app ",
            "IO.BEEBEEB.APP",
            "io.beebeeb.app.dev",
            "io.beebeeb",
        ] {
            assert!(EngineIdentity::from_bundle_identifier(other).is_none(), "{other:?}");
        }
    }
}
