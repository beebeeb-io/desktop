//! The Tauri shell is the real app. Every engine entry that reaches credentials, the settings file, local state
//! or an OS registration takes an [`EngineIdentity`]; the shell names the real one once, here ([`APP`]), and the
//! three engine surfaces it calls most (settings, credential stores, the state folder) keep their old names
//! through the wrappers below. No value is chosen at run time and there is no other identity in this crate.

use std::path::{Path, PathBuf};

use crate::config::{DesktopConfig, HeldConfig};
use crate::identity::EngineIdentity;

/// This app's identity: the real one.
pub(crate) static APP: &EngineIdentity = &crate::identity::REAL;

/// The settings file of this app (`DesktopConfig::*_for(APP, …)`).
pub(crate) trait AppConfig: Sized {
    fn path() -> Result<PathBuf, String>;
    fn load() -> Result<Self, String>;
    fn ensure_account_id() -> Result<String, String>;
    fn update<R>(change: impl FnOnce(&mut DesktopConfig) -> (bool, R)) -> Result<R, String>;
}

impl AppConfig for DesktopConfig {
    fn path() -> Result<PathBuf, String> {
        DesktopConfig::path_for(APP)
    }
    fn load() -> Result<Self, String> {
        DesktopConfig::load_for(APP)
    }
    fn ensure_account_id() -> Result<String, String> {
        DesktopConfig::ensure_account_id_for(APP)
    }
    fn update<R>(change: impl FnOnce(&mut DesktopConfig) -> (bool, R)) -> Result<R, String> {
        DesktopConfig::update_for(APP, change)
    }
}

/// The settings file of this app, read once and written back under the same hold.
pub(crate) trait AppHeldConfig: Sized {
    fn load() -> Result<Self, String>;
    fn update_held<R>(&mut self, change: impl FnOnce(&mut DesktopConfig) -> (bool, R)) -> Result<R, String>;
}

impl AppHeldConfig for HeldConfig {
    fn load() -> Result<Self, String> {
        HeldConfig::load_for(APP)
    }
    fn update_held<R>(&mut self, change: impl FnOnce(&mut DesktopConfig) -> (bool, R)) -> Result<R, String> {
        self.update_held_for(APP, change)
    }
}

/// The engine's credential stores, with this app's service name bound.
pub(crate) mod keychain {
    pub(crate) use beebeeb_engine::keychain::*;

    use super::APP;

    pub(crate) fn platform_keychain_store_for(account_id: &str) -> PlatformKeychainStore {
        beebeeb_engine::keychain::platform_keychain_store_for(APP, account_id)
    }

    pub(crate) fn legacy_platform_keychain_store() -> PlatformKeychainStore {
        beebeeb_engine::keychain::legacy_platform_keychain_store(APP)
    }

    pub(crate) fn migrate_legacy_keychain_to_account(account_id: &str) -> AuthResult<()> {
        beebeeb_engine::keychain::migrate_legacy_keychain_to_account(APP, account_id)
    }
}

/// The engine's state folder, resolved for this app.
pub(crate) mod state_paths {
    pub(crate) use beebeeb_engine::state_paths::*;

    use super::{APP, Path, PathBuf};

    pub(crate) fn init_from_app_local_data_dir(base_dir: &Path) -> Result<(), String> {
        beebeeb_engine::state_paths::init_from_app_local_data_dir(APP, base_dir)
    }

    pub(crate) fn beebeeb_state_dir() -> Result<PathBuf, String> {
        beebeeb_engine::state_paths::beebeeb_state_dir_for(APP)
    }

    pub(crate) fn prepare_beebeeb_state_dir_for_sync_root(sync_root: &Path) -> Result<StateDirMigration, String> {
        beebeeb_engine::state_paths::prepare_beebeeb_state_dir_for_sync_root(APP, sync_root)
    }
}
