//! The single write lock for `desktop.toml` and the error a failed write reports.
//!
//! `CONFIG_WRITE_LOCK` is process-wide and not re-entrant: `config_write_guard`
//! is the only way in, so a settings write can never interleave with another.
//! `HeldConfig` is the token a caller holds while writing: the config read under
//! the lock plus the queued follow-up writes.
//! Tests: `crate::config::tests`.

use super::*;

/// Task 1882 r5, task 1889: the one process-wide lock around writing `desktop.toml`.
///
/// Every save of the config goes through the same `desktop.toml.tmp`, so two saves at once could
/// tear each other's temp file, and two load-modify-save passes at once lose the earlier one's
/// change. There is no public `save`: the file is written in one place, `try_update_at_then`, which
/// takes this lock across the whole load, change and write. The ways in are
/// [`DesktopConfig::update_at`] (and [`DesktopConfig::update`], [`HeldConfig::update_held`] and
/// [`DesktopConfig::ensure_account_id`], which are `update_at`), [`DesktopConfig::try_update_at`],
/// [`DesktopConfig::update_at_then`], and [`DesktopConfig::update_at_durable_with`], the one save
/// that flushes its write (task 1887). The lock is not re-entrant: nothing inside an update closure
/// may call another update.
pub(super) static CONFIG_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Takes [`CONFIG_WRITE_LOCK`]. A holder that panicked leaves the lock usable: the file itself is
/// only ever replaced by a rename, so it is never half-written.
pub(super) fn config_write_guard() -> std::sync::MutexGuard<'static, ()> {
    CONFIG_WRITE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What stopped a [`DesktopConfig::try_update_at`]: the file could not be read (or parsed), or the
/// changed config could not be written. Nothing was written in the first case. A durable save's
/// `Write` can also come after its rename, when the directory flush failed: the change is then in
/// the file, but not flushed ([`DesktopConfig::update_at_durable_with`]).
#[derive(Debug)]
pub(crate) enum UpdateError {
    Read(ConfigError),
    Write(ConfigError),
}

impl UpdateError {
    /// The failure, as [`DesktopConfig::update_at`] hands it back.
    pub(crate) fn into_error(self) -> ConfigError {
        match self {
            Self::Read(error) | Self::Write(error) => error,
        }
    }
}

/// A record that its own write failed and that rides the flow's next write instead (fix round 1, M4): the change to
/// apply to the file, and its argument. It returns whether it changed the config.
pub(super) type QueuedRecord = (fn(&mut DesktopConfig, &str) -> bool, String);

/// A copy of the config that a flow keeps to READ across slow work (task 1889, fix round 1 M2). Add to Finder waits
/// many seconds on the File Provider and the config-write lock must not be held that long, so the Windows and Linux
/// install, Repair and sync-root flows load a copy at the start and read it as they go. The copy can be read (it
/// derefs to [`DesktopConfig`]) and it can only be changed through [`Self::update_held`], which applies the change to the
/// file as it is NOW and then makes the copy equal to what a load of the file returns. There is no `DerefMut`: an edit
/// to the copy outside the closure does not compile, so it cannot be silently dropped by the refresh, and the copy can
/// never be what is written.
#[derive(Debug, Clone)]
pub(crate) struct HeldConfig {
    pub(super) copy: DesktopConfig,
    pub(super) queued: Vec<QueuedRecord>,
}

impl std::ops::Deref for HeldConfig {
    type Target = DesktopConfig;

    fn deref(&self) -> &DesktopConfig {
        &self.copy
    }
}

impl From<DesktopConfig> for HeldConfig {
    fn from(copy: DesktopConfig) -> Self {
        Self {
            copy,
            queued: Vec::new(),
        }
    }
}

impl HeldConfig {
    /// [`DesktopConfig::load`], as a copy to hold.
    pub(crate) fn load() -> Result<Self, String> {
        DesktopConfig::load().map(Self::from)
    }

    /// Writes through the file, never through the copy. `change` is applied to the file as it is now
    /// ([`DesktopConfig::update_at`]), so what the copy loaded at the start cannot replace what another writer saved
    /// since (task 1889: the install and Repair commands used to save their copy and could drop the kept-folder record
    /// a sweep saved meanwhile). Afterwards the copy equals what a load of the file returns: the file as it was read
    /// plus `change`'s edits if it said to save, and the file as it was read, unedited, if it said not to (an edit that
    /// is not saved is not kept; M3), so the flow reads its own write and everyone else's.
    ///
    /// Records queued by [`Self::queue_for_next_write`] are applied first, in the same write; they make a write that
    /// says "nothing changed" a write when they changed something.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) fn update_held<R>(
        &mut self,
        change: impl FnOnce(&mut DesktopConfig) -> (bool, R),
    ) -> Result<R, String> {
        self.update_held_at(&DesktopConfig::path()?, change)
    }

    /// [`Self::update_held`] on the config file at `path` (a seam for the tests).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) fn update_held_at<R>(
        &mut self,
        path: &Path,
        change: impl FnOnce(&mut DesktopConfig) -> (bool, R),
    ) -> Result<R, String> {
        let queued = self.queued.clone();
        let (fresh, value) = DesktopConfig::update_at(path, |cfg| {
            let mut queued_changed = false;
            for (apply, argument) in &queued {
                queued_changed |= apply(cfg, argument);
            }
            let with_queued = cfg.clone();
            let (save, value) = change(cfg);
            if save {
                (true, (cfg.clone(), value))
            } else {
                // An edit that is not saved is not kept (M3): the file as it was read, plus what was queued.
                *cfg = with_queued.clone();
                (queued_changed, (with_queued, value))
            }
        })?;
        self.copy = fresh;
        self.queued.clear();
        Ok(value)
    }

    /// A record whose own write failed: it is applied to the file by this copy's next [`Self::update_held`], so it lands
    /// with the flow's own result or the flow gets that write's error (M4). Queuing the same record twice is one.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) fn queue_for_next_write(&mut self, apply: fn(&mut DesktopConfig, &str) -> bool, argument: &str) {
        if !self
            .queued
            .iter()
            .any(|(queued, arg)| std::ptr::fn_addr_eq(*queued, apply) && arg == argument)
        {
            self.queued.push((apply, argument.to_string()));
        }
    }
}
