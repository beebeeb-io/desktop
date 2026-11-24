//! Finder-setup ports, the part that needs no gate, no bridge and no engine slot: the session facts and the
//! merge-only settings writes. The gated bridge calls, the engine slot and `impl Ports` are not here yet (see the
//! dm-23 report: they need `LifecycleEvent` re-exported by the engine driver, the bridge's add/read/wait/remove
//! wrappers in `bridge.rs`, and tokio's `sync` feature in this crate). Reference: `desktop-v0.8.25`
//! `finder_setup/macos_ports.rs`. Nothing here reads an OS error message; `FailureRecord` is reason, domain, code,
//! time only.

use std::path::Path;

use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::finder_setup::core::SessionFacts;
use beebeeb_engine::finder_setup::error::FailureRecord;

/// §5.1's facts. Fail safe: an unreadable config never counts as "signed out by choice", so it never removes the
/// domain.
#[allow(dead_code)]
pub(super) fn facts_from(
    vault_unlocked: bool,
    auth_present: bool,
    removal_owed: bool,
    config: Result<DesktopConfig, String>,
) -> SessionFacts {
    let signed_out_by_choice = config.map(|cfg| cfg.finder_signed_out_by_choice).unwrap_or(false);
    SessionFacts {
        vault_unlocked,
        auth_present,
        signed_out_by_choice,
        removal_owed,
    }
}

/// Each setter returns whether `cfg` changed, so a repeat writes nothing.
#[allow(dead_code)]
pub(super) fn set_failure(cfg: &mut DesktopConfig, record: Option<FailureRecord>) -> bool {
    if cfg.finder_last_failure == record {
        return false;
    }
    cfg.finder_last_failure = record;
    true
}

#[allow(dead_code)]
pub(super) fn set_signed_out_by_choice(cfg: &mut DesktopConfig, value: bool) -> bool {
    if cfg.finder_signed_out_by_choice == value {
        return false;
    }
    cfg.finder_signed_out_by_choice = value;
    true
}

/// One load-change-save on the settings file at `path`, under the config-write lock (`DesktopConfig::try_update_at`,
/// which is `try_update_at_then`). Merge-only: no copy loaded earlier is ever saved, so a kept folder another writer
/// saved meanwhile survives. A change that is not saved leaves the file untouched; a failed write is logged (no path,
/// no OS text) and ignored.
#[allow(dead_code)]
pub(super) fn update_config_at(path: &Path, change: impl FnOnce(&mut DesktopConfig) -> bool) {
    if DesktopConfig::try_update_at(path, |cfg| (change(cfg), ())).is_err() {
        tracing::warn!("finder setup: could not update the settings file");
    }
}

#[cfg(test)]
#[path = "ports_tests/mod.rs"]
mod tests;
