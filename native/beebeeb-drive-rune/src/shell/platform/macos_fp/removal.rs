//! Domain removal that keeps unsynced files (task 1882, ported from `desktop-v0.8.25:src-tauri/src/finder_removal.rs`;
//! rules 1-7 of `docs/macos/file-provider-removal-and-kept-folders.md`). Every removal passes
//! `PreserveDirtyUserData` (in the bridge: one removal function, two entry points), the folder macOS reports is read
//! before the error, a folder that reads as missing is looked at again, the folder state is decoded, and the
//! `kept_unsynced_folder` record is written merge-only for Settings › Sync. There is ONE decode site
//! ([`decode_removal_with`]) and it settles first. Logs say THAT files were kept, never where.
//!
//! A removal blocks for about a second when nothing was kept: run it on a blocking pool, never the main thread or an
//! async worker. The sign-out keep step (rule 8), conflict copies (9) and the 0.8.x upgrade (10) are not here.

use std::ffi::c_char;
use std::time::Duration;

use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::finder_setup::error::FpError;

use super::bridge::{self, BeebeebFpErrorC, Gate, KeptState};

/// The title of the app's alert (rule 5).
pub(crate) const PRESERVED_FILES_TITLE: &str = "Files kept on this Mac";

/// The one constant sentence (rule 5). The folder's path follows it on its own line.
pub(crate) const PRESERVED_FILES_SENTENCE: &str =
    "Files that hadn’t reached your vault yet were kept on this Mac, in this folder:";

/// The folder states of `BeebeebKept*` in the bridge (a test pins the numbers).
pub(crate) const KEPT_NONE_REPORTED: i32 = 0;
/// A URL came back, but nothing exists there; looked at again ([`settle_kept_state`]).
pub(crate) const KEPT_MISSING: i32 = 1;
/// The folder exists and is empty: kept all the same, macOS may not have filled it yet.
pub(crate) const KEPT_EMPTY: i32 = 2;
/// The folder (or a single kept file) holds at least one entry.
pub(crate) const KEPT_HAS_ENTRIES: i32 = 3;
/// Something may be there, the sandbox refused the look.
pub(crate) const KEPT_UNCHECKED: i32 = 4;
/// A URL without a path: removed, and no folder can be named.
pub(crate) const KEPT_NO_PATH: i32 = 5;

/// How often a folder that reads as missing is looked at again before it counts as "nothing kept".
pub(crate) const KEPT_MISSING_RECHECKS: u32 = 4;
/// The wait before each of those looks (about one second in all, paid only by a removal that kept nothing).
pub(crate) const KEPT_MISSING_RECHECK_INTERVAL: Duration = Duration::from_millis(250);

/// A folder that reads as missing is looked at again, a few times over about a second, before the removal counts as
/// "nothing kept": nothing guarantees macOS made or filled the folder when the completion handler runs. `check` is the
/// bridge's folder check on a path, `wait` the sleep; both are injected so tests never sleep. Only a missing folder
/// with a non-blank path is looked at again; it stops at the first look that finds the folder and returns that look.
pub(crate) fn settle_kept_state(
    state: i32,
    location: Option<&str>,
    mut check: impl FnMut(&str) -> i32,
    mut wait: impl FnMut(Duration),
) -> i32 {
    if state != KEPT_MISSING {
        return state;
    }
    let Some(path) = location.filter(|path| !path.trim().is_empty()) else {
        return state;
    };
    for _ in 0..KEPT_MISSING_RECHECKS {
        wait(KEPT_MISSING_RECHECK_INTERVAL);
        let again = check(path);
        if again != KEPT_MISSING {
            return again;
        }
    }
    KEPT_MISSING
}

/// Why a removal kept nothing. Logged (debug), never shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NothingKept {
    NotReported,
    /// Still missing after the re-checks.
    Missing,
}

impl NothingKept {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotReported => "none",
            Self::Missing => "missing",
        }
    }
}

/// What one removal left on this Mac.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeptFolder {
    Nothing(NothingKept),
    /// Files were kept in this folder, exactly as the system reported it. `contents_checked` is `false` when the app
    /// could not look inside; `empty` is `true` when it looked and found nothing yet (still shown).
    Kept {
        path: String,
        contents_checked: bool,
        empty: bool,
    },
    /// macOS reported kept files without a folder: removed; folder unknown.
    Unknown,
}

impl Default for KeptFolder {
    fn default() -> Self {
        Self::Nothing(NothingKept::NotReported)
    }
}

/// Decodes the bridge's folder state (rule 4). An unknown state with a path never hides that path.
pub(crate) fn kept_folder_from_bridge(state: i32, location: Option<String>) -> KeptFolder {
    let path = location.filter(|path| !path.trim().is_empty());
    match (state, path) {
        (KEPT_NONE_REPORTED, _) => KeptFolder::Nothing(NothingKept::NotReported),
        (KEPT_MISSING, _) => KeptFolder::Nothing(NothingKept::Missing),
        (KEPT_EMPTY, Some(path)) => KeptFolder::Kept {
            path,
            contents_checked: true,
            empty: true,
        },
        (KEPT_HAS_ENTRIES, Some(path)) => KeptFolder::Kept {
            path,
            contents_checked: true,
            empty: false,
        },
        (KEPT_UNCHECKED, Some(path)) => KeptFolder::Kept {
            path,
            contents_checked: false,
            empty: false,
        },
        (KEPT_NO_PATH, _) | (_, None) => KeptFolder::Unknown,
        (_, Some(path)) => KeptFolder::Kept {
            path,
            contents_checked: false,
            empty: false,
        },
    }
}

/// What one domain removal left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DomainRemoval {
    pub kept: KeptFolder,
}

impl DomainRemoval {
    /// The kept folder to show the person, if any. Logs THAT files were kept (or why nothing was), never where.
    pub fn kept_location(self, context: &'static str) -> Option<String> {
        match self.kept {
            KeptFolder::Kept {
                path,
                contents_checked,
                empty,
            } => {
                tracing::info!(
                    context,
                    preserved = true,
                    contents_checked,
                    empty,
                    "Finder location removed; macOS kept the files that had not reached the server"
                );
                Some(path)
            }
            KeptFolder::Nothing(reason) => {
                tracing::debug!(
                    context,
                    reported = reason.as_str(),
                    "Finder location removed; nothing kept"
                );
                None
            }
            KeptFolder::Unknown => {
                tracing::warn!(
                    context,
                    "Finder location removed; macOS reported kept files without a folder"
                );
                None
            }
        }
    }
}

/// A removal that failed. macOS may report a kept folder together with an error; the folder rides with the error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemovalFailure {
    pub message: String,
    pub kept: KeptFolder,
}

impl RemovalFailure {
    /// Like [`DomainRemoval::kept_location`]: the folder to show, if any; logs never carry it.
    pub fn kept_location(self, context: &'static str) -> Option<String> {
        DomainRemoval { kept: self.kept }.kept_location(context)
    }
}

impl From<String> for RemovalFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            kept: KeptFolder::default(),
        }
    }
}

impl std::fmt::Display for RemovalFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Decodes `beebeeb_fp_remove` / `beebeeb_fp_remove_domain_by_id` (rule 2): `0` = removed (`kept_state` says what the
/// reported folder holds), `-1` = error (the folder state still rides along), any other code is an error too.
pub(crate) fn removal_from_bridge(
    code: i32,
    kept_state: i32,
    location: Option<String>,
    error: Option<String>,
) -> Result<DomainRemoval, RemovalFailure> {
    match code {
        0 => Ok(DomainRemoval {
            kept: kept_folder_from_bridge(kept_state, location),
        }),
        code if code < 0 => Err(RemovalFailure {
            message: error
                .filter(|message| !message.trim().is_empty())
                .unwrap_or_else(|| "File Provider domain removal failed".to_string()),
            kept: kept_folder_from_bridge(kept_state, location),
        }),
        other => Err(RemovalFailure {
            message: format!("File Provider domain removal returned unexpected code {other}"),
            kept: kept_folder_from_bridge(kept_state, location),
        }),
    }
}

// ── the bridge calls ─────────────────────────────────────────────────────────

/// Bytes the bridge may write into the location buffer (path, NUL included).
const LOCATION_BUFFER_LEN: usize = 4096;

// The bridge's own declarations (`bridge.rs`) are private to that file; these are the same signatures.
unsafe extern "C" {
    fn beebeeb_fp_remove(
        location_buffer: *mut c_char,
        location_buffer_len: usize,
        kept_state: *mut i32,
        out_error: *mut BeebeebFpErrorC,
    ) -> i32;
    fn beebeeb_fp_remove_domain_by_id(
        identifier: *const c_char,
        location_buffer: *mut c_char,
        location_buffer_len: usize,
        kept_state: *mut i32,
        out_error: *mut BeebeebFpErrorC,
    ) -> i32;
}

/// The one decode site of a removal: settles a missing folder first (the bridge's own check, a real sleep), then
/// decides. Both removal paths go through it.
fn decode_removal(
    code: i32,
    kept_state: i32,
    location_buffer: &[c_char],
    error: BeebeebFpErrorC,
) -> Result<DomainRemoval, RemovalFailure> {
    decode_removal_with(
        code,
        kept_state,
        location_buffer,
        error_message(code, error),
        |path| kept_state_number(bridge::kept_state_for_path(path)),
        std::thread::sleep,
    )
}

/// The bridge's state as the number the decode works on.
fn kept_state_number(state: KeptState) -> i32 {
    match state {
        KeptState::NoneReported => KEPT_NONE_REPORTED,
        KeptState::Missing => KEPT_MISSING,
        KeptState::Empty => KEPT_EMPTY,
        KeptState::HasEntries => KEPT_HAS_ENTRIES,
        KeptState::Unchecked => KEPT_UNCHECKED,
        KeptState::NoPath => KEPT_NO_PATH,
    }
}

/// The error's message, only for an error reply that filled the error (a stale error on a success is never read).
fn error_message(code: i32, error: BeebeebFpErrorC) -> Option<String> {
    let filled = error.domain.iter().any(|c| *c != 0);
    (code != 0 && filled).then(|| error.into_fp_error().message)
}

/// [`decode_removal`] with its check and sleep injected (a seam for the tests).
pub(crate) fn decode_removal_with(
    code: i32,
    kept_state: i32,
    location_buffer: &[c_char],
    error: Option<String>,
    check: impl FnMut(&str) -> i32,
    wait: impl FnMut(Duration),
) -> Result<DomainRemoval, RemovalFailure> {
    // A reply that reports no folder leaves the buffer stale: the state, not the buffer, says there is one.
    let location = (kept_state != KEPT_NONE_REPORTED).then(|| buffer_to_string(location_buffer));
    let location = location.filter(|path| !path.is_empty());
    let kept_state = settle_kept_state(kept_state, location.as_deref(), check, wait);
    removal_from_bridge(code, kept_state, location, error)
}

/// Bounded read: never past the buffer, even if the C side forgot the NUL.
fn buffer_to_string(buffer: &[c_char]) -> String {
    let bytes: Vec<u8> = buffer.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn busy(error: FpError) -> RemovalFailure {
    RemovalFailure::from(error.message)
}

/// Removes the Finder location, keeping what never reached the server (sign-out, Repair, rollback). One call through
/// the bridge gate; blocks about a second when nothing was kept.
#[allow(dead_code)] // the reconciler's and sign-out's call (dm-23, dm-30)
pub(crate) fn remove_keeping_unsynced() -> Result<DomainRemoval, RemovalFailure> {
    let _gate = Gate::try_enter().map_err(busy)?;
    let mut buffer = vec![0 as c_char; LOCATION_BUFFER_LEN];
    let mut kept_state = KEPT_NONE_REPORTED;
    let mut error = BeebeebFpErrorC::zeroed();
    let code = unsafe { beebeeb_fp_remove(buffer.as_mut_ptr(), buffer.len(), &mut kept_state, &mut error) };
    decode_removal(code, kept_state, &buffer, error)
}

/// The sweep's removal of one stale domain by identifier: the same bridge function, the same decode.
#[allow(dead_code)] // the sweep's call (dm-23)
pub(crate) fn remove_domain(identifier: &str) -> Result<DomainRemoval, RemovalFailure> {
    let identifier = std::ffi::CString::new(identifier)
        .map_err(|_| RemovalFailure::from("the domain identifier holds a NUL byte".to_string()))?;
    let _gate = Gate::try_enter().map_err(busy)?;
    let mut buffer = vec![0 as c_char; LOCATION_BUFFER_LEN];
    let mut kept_state = KEPT_NONE_REPORTED;
    let mut error = BeebeebFpErrorC::zeroed();
    let code = unsafe {
        beebeeb_fp_remove_domain_by_id(
            identifier.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut kept_state,
            &mut error,
        )
    };
    decode_removal(code, kept_state, &buffer, error)
}

// ── what the person sees ─────────────────────────────────────────────────────

/// The alert after a removal that kept files: `(title, body)`, or nothing when nothing was kept (rule 5).
#[allow(dead_code)] // the alert's caller (dm-30)
pub(crate) fn kept_folder_alert(preserved_location: Option<&str>) -> Option<(&'static str, String)> {
    preserved_location.map(|location| (PRESERVED_FILES_TITLE, preserved_files_message(location)))
}

/// The alert's body: the sentence, a blank line, then the folder.
pub(crate) fn preserved_files_message(location: &str) -> String {
    format!("{PRESERVED_FILES_SENTENCE}\n\n{location}")
}

/// Sign-out's removal (rule 6): a failure is logged and never stops the sign-out; a folder kept with the error still
/// reaches the alert.
#[allow(dead_code)] // sign-out (dm-30)
pub(crate) fn sign_out_kept_location(removal: Result<DomainRemoval, RemovalFailure>) -> Option<String> {
    match removal {
        Ok(removal) => removal.kept_location("sign-out"),
        Err(failure) => {
            tracing::warn!("Finder File Provider domain removal on sign-out failed (best-effort)");
            failure.kept_location("sign-out")
        }
    }
}

/// Repair's removal (rule 6): `(removed, kept folder)`. A failure becomes one warning.
#[allow(dead_code)] // Repair (dm-23)
pub(crate) fn repair_removal(
    removal: Result<DomainRemoval, RemovalFailure>,
    warnings: &mut Vec<String>,
) -> (bool, Option<String>) {
    match removal {
        Ok(removal) => (true, removal.kept_location("repair")),
        Err(failure) => {
            warnings.push(format!(
                "Could not remove Finder File Provider domain: {}",
                failure.message
            ));
            (false, failure.kept_location("repair"))
        }
    }
}

// ── the Settings › Sync row's record (`kept_unsynced_folder`, rule 7) ────────

/// Saves `location` as the latest kept folder; a newer one replaces an older one. Returns whether the config changed.
pub(crate) fn record_kept_folder(cfg: &mut DesktopConfig, location: &str) -> bool {
    if cfg.kept_unsynced_folder.as_deref() == Some(location) {
        return false;
    }
    cfg.kept_unsynced_folder = Some(location.to_string());
    true
}

/// Dismisses the row: clears the saved folder only if it is still the one the row showed.
pub(crate) fn dismiss_kept_folder(cfg: &mut DesktopConfig, shown: &str) -> bool {
    if cfg.kept_unsynced_folder.as_deref() != Some(shown) {
        return false;
    }
    cfg.kept_unsynced_folder = None;
    true
}

/// Saves `location` as the latest kept folder in the settings file, as one load-change-save under the config-write
/// lock, so a settings save running at the same moment lands too. Merge-only: nothing else in the file changes.
#[allow(dead_code)] // the surfacing step (dm-30)
pub(crate) fn remember_kept_folder(
    identity: &'static beebeeb_engine::identity::EngineIdentity,
    location: &str,
) -> Result<bool, String> {
    remember_kept_folder_at(&DesktopConfig::path_for(identity)?, location)
}

/// [`remember_kept_folder`] on the config file at `path` (a seam for the tests).
pub(crate) fn remember_kept_folder_at(path: &std::path::Path, location: &str) -> Result<bool, String> {
    DesktopConfig::update_at(path, |cfg| {
        let changed = record_kept_folder(cfg, location);
        (changed, changed)
    })
}

/// What the row's "Dismiss" did: `cleared` says whether the saved folder was the one shown and is now gone; `current`
/// is the folder saved after the command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DismissOutcome {
    pub cleared: bool,
    pub current: Option<String>,
}

/// The row's "Dismiss" as one load-change-save under the config-write lock.
#[allow(dead_code)] // dm-18's row
pub(crate) fn dismiss_saved_kept_folder(
    identity: &'static beebeeb_engine::identity::EngineIdentity,
    shown: &str,
) -> Result<DismissOutcome, String> {
    dismiss_saved_kept_folder_at(&DesktopConfig::path_for(identity)?, shown)
}

/// [`dismiss_saved_kept_folder`] on the config file at `path` (a seam for the tests).
pub(crate) fn dismiss_saved_kept_folder_at(path: &std::path::Path, shown: &str) -> Result<DismissOutcome, String> {
    DesktopConfig::update_at(path, |cfg| {
        let cleared = dismiss_kept_folder(cfg, shown);
        (
            cleared,
            DismissOutcome {
                cleared,
                current: cfg.kept_unsynced_folder.clone(),
            },
        )
    })
}

#[cfg(test)]
#[path = "removal_tests/mod.rs"]
mod tests;
