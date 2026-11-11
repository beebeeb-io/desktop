//! Reading and writing `desktop.toml`: loading, the update paths that save under
//! the write lock, and the `ConfigFs` seam the sandboxed tests use instead of
//! the real filesystem.
//!
//! Every mutating call must go through `try_update_at_then`, which takes the
//! lock in `lock`; no caller may hold it across a save. Under `#[cfg(test)]` the
//! config directory is a per-process sandbox, so tests never touch the real file.
//! Tests: `crate::config::tests`.

use super::lock::config_write_guard;
use super::paths::{APP_CONFIG_DIR, CONFIG_FILENAME};
use super::*;

/// Which OS a load is for, named so both branches run on any test host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigHost {
    MacOs,
    Other,
}

impl ConfigHost {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Other
        }
    }
}

impl DesktopConfig {
    /// Resolve the absolute path to `desktop.toml` for the current user,
    /// creating the parent directory if missing. Errors propagate as
    /// strings so they can flow through Tauri commands.
    pub fn path() -> Result<PathBuf, String> {
        let base = Self::config_base_dir().map_err(String::from)?;
        let dir = base.join(APP_CONFIG_DIR);
        fs::create_dir_all(&dir)
            .map_err(ConfigError::CreateConfigDirectory)
            .map_err(String::from)?;
        Ok(dir.join(CONFIG_FILENAME))
    }

    /// The directory that holds the `beebeeb` config folder: the user's config dir.
    #[cfg(not(any(test, feature = "test-support")))]
    pub(super) fn config_base_dir() -> Result<PathBuf, ConfigError> {
        dirs::config_dir().ok_or(ConfigError::ConfigDirectoryUnknown)
    }

    /// Unit tests never see the person's real config. In a test build this is a per-process
    /// sandbox next to the test binary (`crate::test_sandbox`), so a test that calls `load()`,
    /// `update()` or `ensure_account_id()` (directly, or through a command it runs) reads and
    /// writes the sandbox, not `~/Library/Application Support/beebeeb/desktop.toml`. The guard
    /// test `unit_tests_resolve_the_config_path_in_a_sandbox_never_the_real_one` pins this.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn config_base_dir() -> Result<PathBuf, ConfigError> {
        if let Ok(dir) = CONFIG_DIR_OVERRIDE.try_with(PathBuf::clone) {
            return Ok(dir);
        }
        crate::test_sandbox::dir("config").map_err(ConfigError::Sandbox)
    }

    /// Load the on-disk config. A missing file is normal on first
    /// launch and yields `DesktopConfig::default()` rather than an
    /// error. A corrupt or unparseable file IS an error — surfacing
    /// it loud avoids silently overwriting a user's settings on save.
    pub fn load() -> Result<Self, String> {
        Self::load_from(&Self::path()?).map_err(String::from)
    }

    /// [`Self::load`] from `path`. A seam so a test can run the real load against its own file.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        // toml 0.8 removed from_slice; read as UTF-8 string and parse.
        let s = fs::read_to_string(path).map_err(|source| ConfigError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
        let mut cfg: Self = toml::from_str(&s).map_err(|source| ConfigError::ParseFile {
            path: path.to_path_buf(),
            source,
        })?;

        // Reject relative paths defensively. Hand-editing the TOML to a
        // relative path would otherwise let the engine sync against
        // wherever the binary's cwd happens to be.
        if let Some(p) = &cfg.sync_root
            && !p.is_absolute()
        {
            tracing::warn!("ignoring non-absolute sync_root in desktop.toml: {}", p.display());
            cfg.sync_root = None;
        }
        if let Some(raw) = cfg.sync_mode.as_deref() {
            ConfigMode::parse(raw)?;
        }
        cfg.scrub_legacy_finder_error(ConfigHost::current());
        Ok(cfg)
    }

    /// Forget the old `finder_install_last_error` on macOS (lead ruling T4-3). 0.8.11 saved the
    /// OS's own message there, and that prose can name a file or a folder. macOS no longer
    /// writes the `finder_install_*` keys (it reads Finder's state from the OS, spec §9), so
    /// nothing there needs the text and the next update drops it from disk. The other old
    /// keys carry no path: they still load and are kept. Windows and Linux keep reading and
    /// writing the error unchanged. The host is a parameter so tests exercise both branches
    /// on any host; `load_from` passes the build's own target.
    pub(super) fn scrub_legacy_finder_error(&mut self, host: ConfigHost) {
        if host == ConfigHost::MacOs {
            self.finder_install_last_error = None;
        }
    }

    /// Return this install's account id, minting + persisting one on first use.
    ///
    /// If `account_id` is already set, returns it unchanged WITHOUT re-saving
    /// (idempotent: a relaunch must not rewrite the file or change the id). If
    /// it is `None` (fresh install, or a pre-Phase-1 config that predates the
    /// key), mints a fresh UUID v4, sets it, and persists immediately via the
    /// atomic save BEFORE returning.
    ///
    /// ★ INVARIANT ★ The id is on disk before this returns, so the caller can
    /// safely segment keychain secrets under it (see the load-bearing note on
    /// the `account_id` field). Persisting first closes the Phase-0
    /// orphan-on-relaunch hole: a crash after this returns still finds the same
    /// id on the next launch, so the secrets written under it are recoverable.
    ///
    /// Task 1889: the read, the mint and the write are one [`Self::update_at`], so two callers at once
    /// cannot mint two ids, and the mint cannot replace a setting another writer saved meanwhile.
    pub fn ensure_account_id() -> Result<String, String> {
        Self::ensure_account_id_at(&Self::path()?).map_err(String::from)
    }

    /// [`Self::ensure_account_id`] against an explicit file, so a test can run the real read, mint
    /// and write in a temp dir instead of the person's `desktop.toml`.
    pub fn ensure_account_id_at(path: &Path) -> Result<String, ConfigError> {
        Self::try_update_at(path, |cfg| match &cfg.account_id {
            Some(id) => (false, id.clone()),
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                cfg.account_id = Some(id.clone());
                (true, id)
            }
        })
        .map_err(UpdateError::into_error)
    }

    /// Load the config, let `change` modify it, and save it if `change` says so, all under the
    /// config-write lock ([`Self::update_at`] on [`Self::path`]). Every writer of `desktop.toml`
    /// outside this file goes through here (task 1889). `change` returns `(save, value)`.
    pub fn update<R>(change: impl FnOnce(&mut Self) -> (bool, R)) -> Result<R, String> {
        Self::update_at(&Self::path()?, change)
    }

    /// Load the config at `path`, let `change` modify it, and save it if `change` says so, all
    /// under the config-write lock, so no other update lands between the load and the save
    /// (task 1882 r5, task 1889). `change` returns `(save, value)`: whether the config changed, and
    /// what `update_at` hands back. A change that is not saved leaves the file untouched. Callers
    /// pass [`Self::path`]; a test passes its own file, so it runs the real lock, load and save.
    ///
    /// For a short, synchronous change: the lock is held while `change` runs, and it is not
    /// re-entrant, so `change` calls no other update.
    pub fn update_at<R>(path: &Path, change: impl FnOnce(&mut Self) -> (bool, R)) -> Result<R, String> {
        Self::try_update_at(path, change).map_err(|error| error.into_error().to_string())
    }

    /// [`Self::update_at`] that says whether the read or the write failed, for a caller whose log
    /// line names which. The messages are the same strings `update_at` returns.
    pub fn try_update_at<R>(path: &Path, change: impl FnOnce(&mut Self) -> (bool, R)) -> Result<R, UpdateError> {
        Self::try_update_at_then(path, change, |_| {}, None)
    }

    /// [`Self::update_at`] with a step that runs while the lock is STILL held, once the file is final: after a successful
    /// write, or at once when `change` said not to save (then `saved` sees the file as it was read, not `change`'s
    /// unsaved edit). For state that mirrors the file and must change in the same order as the file does, such as the
    /// in-memory pause flag (task 1889 fix 1, M6): the mirror is never ahead of a write that failed and never behind
    /// one that completed. `saved` does no I/O and takes no lock.
    pub fn update_at_then<R>(
        path: &Path,
        change: impl FnOnce(&mut Self) -> (bool, R),
        saved: impl FnOnce(&Self),
    ) -> Result<R, String> {
        Self::try_update_at_then(path, change, saved, None).map_err(|error| error.into_error().to_string())
    }

    /// Spec 2026-10-10 §7.3 (task 1887): `update_at`, whose write survives a power loss once it returns: the temp file
    /// is flushed before the rename, and the directory after it. Only the keep step's save uses it, with
    /// `RealConfigFs`; every other save stays as it is. The file-system steps are passed in so a test can record their
    /// order and fail one (U43). It is the one write place with a durable write step, so the lock and the read are
    /// still taken in one place. An `Err` can come after the rename, when the directory flush failed: the change is
    /// then in the file, so a caller that undoes its work on an error must take the change back (spec 2026-10-10 W8).
    pub fn update_at_durable_with<R>(
        path: &Path,
        change: impl FnOnce(&mut Self) -> (bool, R),
        fs_steps: &dyn ConfigFs,
    ) -> Result<R, String> {
        Self::try_update_at_then(path, change, |_| {}, Some(fs_steps)).map_err(|error| error.into_error().to_string())
    }

    /// The one place the file is written: the lock is taken before the read and held through the write and `saved`.
    /// `durable`: `None` is the plain write every save uses; `Some` is the keep step's durable write (task 1887, spec
    /// 2026-10-10 §7.3), with its file-system steps.
    pub(super) fn try_update_at_then<R>(
        path: &Path,
        change: impl FnOnce(&mut Self) -> (bool, R),
        saved: impl FnOnce(&Self),
        durable: Option<&dyn ConfigFs>,
    ) -> Result<R, UpdateError> {
        let _writing = config_write_guard();
        let mut cfg = Self::load_from(path).map_err(UpdateError::Read)?;
        let read = cfg.clone();
        let (save, value) = change(&mut cfg);
        if save {
            match durable {
                None => cfg.save_to(path),
                Some(fs_steps) => cfg.save_to_durable(path, fs_steps),
            }
            .map_err(UpdateError::Write)?;
            saved(&cfg);
        } else {
            saved(&read);
        }
        Ok(value)
    }

    /// The write itself. The caller holds the config-write lock.
    pub(super) fn save_to(&self, path: &Path) -> Result<(), ConfigError> {
        let toml_str = toml::to_string_pretty(self).map_err(ConfigError::Serialize)?;

        // Atomic write: temp + rename. Avoids leaving a half-written
        // file if the process is killed mid-save.
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, toml_str.as_bytes()).map_err(|source| ConfigError::WriteFile {
            path: tmp.clone(),
            source,
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&tmp, perms).map_err(|source| ConfigError::SetPermissions {
                path: tmp.clone(),
                source,
            })?;
        }

        fs::rename(&tmp, path).map_err(|source| ConfigError::Rename {
            from: tmp,
            to: path.to_path_buf(),
            source,
        })?;
        Ok(())
    }

    /// The durable write. The caller holds the config-write lock.
    pub(super) fn save_to_durable(&self, path: &Path, fs_steps: &dyn ConfigFs) -> Result<(), ConfigError> {
        let toml_str = toml::to_string_pretty(self).map_err(ConfigError::Serialize)?;
        let tmp = path.with_extension("toml.tmp");
        fs_steps
            .write(&tmp, toml_str.as_bytes())
            .map_err(|source| ConfigError::WriteFile {
                path: tmp.clone(),
                source,
            })?;
        fs_steps.sync_file(&tmp).map_err(|source| ConfigError::Flush {
            path: tmp.clone(),
            source,
        })?;
        fs_steps.rename(&tmp, path).map_err(|source| ConfigError::Rename {
            from: tmp,
            to: path.to_path_buf(),
            source,
        })?;
        let dir = path.parent().ok_or(ConfigError::MissingParentDirectory)?;
        fs_steps.sync_dir(dir).map_err(|source| ConfigError::Flush {
            path: dir.to_path_buf(),
            source,
        })
    }
}

/// The file-system steps of a durable save, so a test can record their order and fail one (spec 2026-10-10 U43).
pub trait ConfigFs {
    /// Write `bytes` to `path`, owner-only.
    fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()>;
    fn sync_file(&self, path: &Path) -> std::io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    fn sync_dir(&self, dir: &Path) -> std::io::Result<()>;
}

// Outside tests, only the macOS sign-out builds one (spec 2026-10-10 W5).
#[cfg_attr(not(any(target_os = "macos", any(test, feature = "test-support"))), allow(dead_code))]
pub struct RealConfigFs;

impl ConfigFs for RealConfigFs {
    fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        fs::write(path, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
    fn sync_file(&self, path: &Path) -> std::io::Result<()> {
        fs::File::open(path)?.sync_all()
    }
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        fs::rename(from, to)
    }
    fn sync_dir(&self, dir: &Path) -> std::io::Result<()> {
        fs::File::open(dir)?.sync_all()
    }
}
