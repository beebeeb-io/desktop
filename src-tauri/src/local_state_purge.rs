//! `local_state_purge` items split out of `lib.rs`.

use super::*;

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct FinderInstallState {
    pub(super) installed: bool,
    pub(super) path: Option<String>,
    pub(super) status: String,
    pub(super) last_error: Option<String>,
    pub(super) last_attempt_at: Option<i64>,
    pub(super) reason_category: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub(super) struct MacosIntegrationResetResult {
    pub(super) removed_file_provider_domain: bool,
    /// Task 1882: the folder where macOS kept un-synced Finder files when the
    /// repair removed the Finder location; `None` = nothing kept.
    pub(super) preserved_location: Option<String>,
    pub(super) disabled_autostart: bool,
    pub(super) removed_socket: bool,
    pub(super) removed_cache_files: usize,
    pub(super) skipped_cache_files: usize,
    pub(super) pending_operations_preserved: i64,
    pub(super) sync_root_preserved: Option<String>,
    /// An engine stop on this account is unconfirmed after Repair's own stop (this one, or an earlier one): its task
    /// may still run with the keys, so sync starts again only after Beebeeb is quit and reopened (spec §5.6). On macOS
    /// the surfaces render one fixed sentence for it, and `warnings` does not also carry the engine-stop text. On
    /// Windows and Linux `warnings` carries that text, as their surfaces show it.
    pub(super) engine_stop_unconfirmed: bool,
    pub(super) warnings: Vec<String>,
}

pub(super) fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

/// What `wait_for_file_provider_ipc_ready` fails with when the local sync socket does not come up in time. It is the
/// sentence a person reads, and it is also what a Linux or Windows install saves as `finder_install_last_error`, so the
/// classifier below recognises this exact text as a timeout: it holds neither "timed out" nor "timeout" (two-surface spec
/// §10.2: no "daemon", no "Timed out waiting for"), and a reworded sentence must not turn the saved category into "unknown"
/// (PR #140 review, P2-8).
pub(super) const FINDER_IPC_NOT_READY: &str = "Beebeeb wasn’t ready in time to add its Finder location.";

/// Whether a Finder-install error is a wait that ran out: the typed sentence above, or an OS error that says so.
#[cfg(any(test, not(target_os = "macos")))]
pub(super) fn finder_install_error_is_timeout(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("timed out") || lower.contains("timeout") || error.contains(FINDER_IPC_NOT_READY)
}

// The install-era Finder state (`finder_install_*` in desktop.toml and the helpers below) is for
// Windows and Linux only. On macOS the reconciler owns the truth (spec 2026-10-06 §6.1).
#[cfg(not(target_os = "macos"))]
pub(super) fn classify_finder_install_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("turned off in system settings") {
        // Issue 4 (task 1524): a user-disabled File Provider domain, detected BEFORE
        // any wait/timeout is attempted -- distinct from the generic "disabled" NSError
        // case below, which comes from the OS itself mid-operation.
        "user_disabled".to_string()
    } else if lower.contains("provision") || lower.contains("entitlement") || lower.contains("app-group") {
        "provisioning".to_string()
    } else if lower.contains("disabled") || lower.contains("-2011") || lower.contains("sync is not enabled") {
        "disabled".to_string()
    } else if finder_install_error_is_timeout(error) {
        "timeout".to_string()
    } else if lower.contains("not available") || lower.contains("only available") || lower.contains("unsupported") {
        "unsupported".to_string()
    } else {
        "unknown".to_string()
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn finder_install_state_from_config(
    cfg: &DesktopConfig,
    installed: bool,
    runtime_error: Option<String>,
) -> FinderInstallState {
    let status = if installed {
        "installed".to_string()
    } else if runtime_error.is_some() || cfg.finder_install_status.as_deref() == Some("error") {
        "error".to_string()
    } else {
        cfg.finder_install_status
            .clone()
            .unwrap_or_else(|| "missing".to_string())
    };
    let last_error = if installed {
        None
    } else {
        runtime_error.or_else(|| cfg.finder_install_last_error.clone())
    };
    let reason_category = if installed {
        None
    } else if let Some(error) = &last_error {
        Some(classify_finder_install_error(error))
    } else {
        cfg.finder_install_reason_category.clone()
    };

    FinderInstallState {
        installed,
        path: finder_state_path(cfg, installed),
        status,
        last_error,
        last_attempt_at: cfg.finder_install_last_attempt_at,
        reason_category,
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn finder_state_path(cfg: &DesktopConfig, installed: bool) -> Option<String> {
    let _ = installed;
    cfg.sync_root.as_ref().map(|p| p.to_string_lossy().into_owned())
}

/// The end of Repair, once the Finder location is gone (re-review P1, as review I1 is for
/// sign-out): saves the config, and if that save fails, shows the folder macOS kept BEFORE the
/// error goes back. The removal cannot be undone and a retry finds no domain, so without this the
/// folder would never be named: the error only says Repair failed, and Repair raises no alert of
/// its own. A save that works shows nothing here: the result carries the folder to the Sync tab.
/// `surface` is `surface_kept_folder` in the app; the tests pass a recorder.
///
/// Windows and Linux only in the app: on macOS Repair saves no config after the removal (spec 2026-10-06 §6.1),
/// so there is no save that can fail after it.
///
/// Rebase re-review Minor 4 (lead ruling, 2026-10-10): the error is the save's own. 1882 r4 marked it with a
/// `repair_failed_after_removal` code when the removal had worked, but that can happen on no platform now: a Mac
/// saves nothing after its removal, and Windows and Linux have no File Provider, so their removal never works. The
/// code and both surfaces' notes for it are gone.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(super) fn finish_repair_after_removal(
    cfg: &mut HeldConfig,
    preserved_location: Option<&str>,
    save: impl FnOnce(&mut HeldConfig) -> Result<(), String>,
    surface: impl FnOnce(Option<&str>),
) -> Result<(), String> {
    match save(cfg) {
        Ok(()) => Ok(()),
        Err(error) => {
            surface(preserved_location);
            Err(error)
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn persist_finder_install_result(cfg: &mut HeldConfig, installed: bool, error: Option<String>) -> Result<(), String> {
    // Task 1889: the result is written onto the file as it is now (`update_held`), not by saving the copy this
    // flow loaded before its slow work, which would put back what another writer saved since.
    cfg.update_held(|held| {
        record_finder_install_result(held, installed, error);
        (true, ())
    })
}

/// The in-memory half of [`persist_finder_install_result`]: stamp the attempt time and the
/// outcome fields on `cfg` without touching disk, so the state a failed install returns can be
/// built (and unit-tested) from exactly what gets saved.
#[cfg(not(target_os = "macos"))]
pub(super) fn record_finder_install_result(cfg: &mut DesktopConfig, installed: bool, error: Option<String>) {
    cfg.finder_install_last_attempt_at = Some(now_unix_seconds());
    if installed {
        cfg.finder_install_status = Some("installed".to_string());
        cfg.finder_install_last_error = None;
        cfg.finder_install_reason_category = None;
    } else if let Some(error) = error {
        cfg.finder_install_status = Some("error".to_string());
        cfg.finder_install_reason_category = Some(classify_finder_install_error(&error));
        cfg.finder_install_last_error = Some(error);
    } else {
        cfg.finder_install_status = Some("missing".to_string());
        cfg.finder_install_last_error = None;
        cfg.finder_install_reason_category = None;
    }
}

/// Task 1683 slice 5 / decision D1: a failed Finder install is SAVED or RETURNED AS AN ERROR,
/// not both. Before this, `install_finder_location` persisted the failure (read back by
/// `finder_location_state` and painted as a banner) and ALSO returned `Err`, which the pane
/// turned into a toast: the same string twice (screenshot 1). A failed install gates "Open in
/// Finder", so it is saved and returned as a state (`installed: false`, `status: "error"`,
/// `last_error` set); the UI renders that one inline and never toasts it. This builds that state
/// from the freshly recorded result, so what is returned is what is saved.
#[cfg(not(target_os = "macos"))]
pub(super) fn finder_install_failure_state(cfg: &mut DesktopConfig, error: String) -> FinderInstallState {
    record_finder_install_result(cfg, false, Some(error));
    finder_install_state_from_config(cfg, false, None)
}

/// Spec section 7: the saved install error "is cleared when a new attempt starts". An attempt can
/// wait many seconds on the File Provider domain; until it ends, the previous attempt's error is
/// no longer true of anything, yet the status page and the popover read it from this config.
/// Clears the failure fields (an `error` status becomes `missing`, like the frontend's
/// `finderInstallStateWhileAttempting`); an `installed` status and the last attempt time are
/// left alone. Returns whether anything changed, so an attempt with nothing to clear writes nothing.
#[cfg(not(target_os = "macos"))]
pub(super) fn clear_finder_install_failure(cfg: &mut DesktopConfig) -> bool {
    let had_failure = cfg.finder_install_status.as_deref() == Some("error")
        || cfg.finder_install_last_error.is_some()
        || cfg.finder_install_reason_category.is_some();
    if cfg.finder_install_status.as_deref() == Some("error") {
        cfg.finder_install_status = Some("missing".to_string());
    }
    cfg.finder_install_last_error = None;
    cfg.finder_install_reason_category = None;
    had_failure
}

/// Start of an install attempt: drop the previous attempt's failure from the saved state.
#[cfg(not(target_os = "macos"))]
pub(super) fn begin_finder_install_attempt(cfg: &mut HeldConfig) -> Result<(), String> {
    cfg.update_held(|held| (clear_finder_install_failure(held), ()))
}

/// Save a failed Finder install and return it as an `Ok` state (see
/// [`finder_install_failure_state`]). Only a failure to save the config itself is an `Err`.
#[cfg(not(target_os = "macos"))]
pub(super) fn finder_install_failed(cfg: &mut HeldConfig, error: String) -> Result<FinderInstallState, String> {
    // Task 1889: recorded and saved onto the file as it is now; the state is built from what was saved.
    cfg.update_held(|held| (true, finder_install_failure_state(held, error)))
}

pub(super) fn state_db_for_config(cfg: &DesktopConfig) -> Result<Option<state_db::StateDb>, String> {
    let Some(_) = &cfg.sync_root else {
        return Ok(None);
    };
    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Ok(None);
    }
    state_db::StateDb::open(&db_path)
        .map(Some)
        .map_err(|e| format!("open state.db: {e}"))
}

/// Resolve the local `state.db` from the app-local data dir alone — task
/// 1538 Codex P1 (PR #49, lib.rs:1085 thread).
///
/// `state.db` lives in `state_paths::beebeeb_state_dir()`, which resolves
/// purely from Tauri's `app_local_data_dir()`; it has NEVER depended on
/// `DesktopConfig`/`desktop.toml` (see that function's own doc comment).
/// `state_db_for_config` above nonetheless GATES on a successfully-loaded
/// `DesktopConfig` with a non-`None`, accepted `sync_root` — so a missing,
/// corrupt, unreadable `desktop.toml`, or one whose `sync_root` was rejected
/// on load, makes the caller's `.ok()` chain silently skip the ENTIRE
/// sign-out purge. That purge is a cross-account data-exfiltration control
/// (task 1538 findings 1+2); it must not be able to depend on config health
/// it needs no part of.
///
/// This resolves + opens `state.db` directly against the app-local state
/// dir, with no `DesktopConfig` involved at all. `Ok(None)` only when the
/// file genuinely doesn't exist yet (a fresh install that has never synced
/// anything — nothing to purge); any OTHER failure (state dir not
/// initialized, DB open/migration error) is returned as `Err` so the caller
/// logs it loudly instead of treating it the same as "nothing to do".
///
/// Thin wrapper around [`state_db_from_state_dir`] — the ONLY thing this
/// adds is resolving Tauri's global app-local-data path; that split exists
/// so the actual decision logic is unit-testable without a live Tauri
/// `AppHandle` having called `state_paths::init_from_app` first.
pub(super) fn state_db_from_app_local_state_dir() -> Result<Option<state_db::StateDb>, String> {
    state_db_from_state_dir(&state_paths::beebeeb_state_dir()?)
}

/// Pure core of [`state_db_from_app_local_state_dir`]: open `state.db`
/// directly under `state_dir`, given explicitly — no Tauri global, no
/// `DesktopConfig`. Task 1538 Codex P1 (PR #49, lib.rs:1085 thread): this
/// signature is the proof the sign-out purge's DB resolution cannot depend
/// on `desktop.toml` health, because there is no `DesktopConfig` parameter
/// for it to depend on.
pub(super) fn state_db_from_state_dir(state_dir: &std::path::Path) -> Result<Option<state_db::StateDb>, String> {
    let db_path = state_dir.join(state_paths::STATE_DB_FILENAME);
    // Only a database that is CERTAINLY not there is "none yet". A stat that fails for any other reason (a permission,
    // a path component that is not a directory) is unreadable, not absent (lead ruling 13, Task 12): every caller then
    // fails closed (a sign-in decides nothing, a sign-out stops, an engine start refuses).
    match db_path.try_exists() {
        Ok(true) => {}
        Ok(false) => return Ok(None),
        Err(error) => return Err(format!("read state.db: {error}")),
    }
    state_db::StateDb::open(&db_path)
        .map(Some)
        .map_err(|e| format!("open state.db: {e}"))
}

pub(super) fn disposable_cache_roots() -> Vec<PathBuf> {
    disposable_cache_roots_in(
        std::env::temp_dir(),
        dirs::cache_dir(),
        macos_hydrate_cache_root(),
        &engine_bridge::FinderStagingBases::current(),
    )
}

/// The hydrate cache's root on a Mac (task 1670); none elsewhere.
pub(super) fn macos_hydrate_cache_root() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(crate::ipc_socket::macos_hydrate_cache_dir())
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Spec 2026-10-10 §7.1 (U12): the roots `is_disposable_cache_path` accepts, from directories passed in, so a test can
/// check a release layout. The temp dir, the cache dir, the macOS hydrate cache, then every staging folder (spec
/// 2026-10-09 §8.4), never the whole data dir.
pub(super) fn disposable_cache_roots_in(
    temp_dir: PathBuf,
    cache_dir: Option<PathBuf>,
    hydrate_cache_dir: Option<PathBuf>,
    staging: &engine_bridge::FinderStagingBases,
) -> Vec<PathBuf> {
    let mut roots = vec![temp_dir];
    roots.extend(cache_dir);
    // Task 1670 round 2, Codex P1 (PR #75): `hydrate_file` records a macOS
    // hydrate-cache destination as `cache_path` via `mark_cached`, same as
    // any other cache entry. Without this, `is_disposable_cache_path` (the
    // gate `purge_local_state_files` — sign-out's purge — checks every
    // candidate path against) would refuse to remove it, since neither
    // `std::env::temp_dir()` nor `dirs::cache_dir()` contains it — a
    // sign-out purge would silently skip a staged plaintext file. In
    // practice `fetchContents` (`FileProviderExtension.swift`) already
    // deletes its own staged file unconditionally right after every
    // `completionHandler` call, and the dedicated
    // `macos_purge_hydrate_cache_dir` sweep (`purge_macos_hydrate_cache`)
    // independently wipes this whole directory on sign-out/lock/startup —
    // so this is defense in depth for the general-purpose per-path cleanup
    // machinery, not the primary mechanism, but it closes the gap Codex
    // named precisely (this general machinery must never treat a path
    // holding decrypted plaintext as "not ours to remove").
    roots.extend(hydrate_cache_dir);
    // Every folder a Finder write's plaintext copy is staged in, or was staged in by an earlier build (spec
    // 2026-10-09 §8.4): on macOS the data dir's `beebeeb/finder-writes`, which is under neither root above, so
    // without it a sign-out would skip those copies and leave plaintext on disk. The folders themselves, never the
    // whole data dir.
    roots.extend(staging.candidates());
    roots
}

/// `…/Logs/Beebeeb` anywhere in `path`: the lifecycle log's directory, in the home or in the app's
/// container (spec 2026-10-06 §8). It survives sign-out, Repair and the account reset (lead ruling
/// T5-⚠c), so no purge may treat anything under it as disposable, whatever root it sits in.
pub(super) fn is_lifecycle_log_path(path: &std::path::Path) -> bool {
    let names: Vec<&str> = path.components().filter_map(|part| part.as_os_str().to_str()).collect();
    names
        .windows(2)
        .any(|pair| pair[0].eq_ignore_ascii_case("Logs") && pair[1].eq_ignore_ascii_case("Beebeeb"))
}

pub(super) fn is_disposable_cache_path(path: &std::path::Path) -> bool {
    if is_lifecycle_log_path(path) {
        return false;
    }
    let canonical_path = match path.canonicalize() {
        Ok(path) => path,
        Err(_) => {
            let Some(parent) = path.parent() else {
                return false;
            };
            let Ok(parent) = parent.canonicalize() else {
                return false;
            };
            match path.file_name() {
                Some(name) => parent.join(name),
                None => parent,
            }
        }
    };
    // A symlink into the log directory is the same directory.
    if is_lifecycle_log_path(&canonical_path) {
        return false;
    }
    disposable_cache_roots()
        .into_iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| canonical_path.starts_with(root))
}

/// Counts from [`purge_local_state_files`] (task 1538), logged by the
/// sign-out caller.
pub(super) struct LocalStatePurgeSummary {
    pub(super) queued_ops_purged: usize,
    pub(super) files_removed: usize,
    pub(super) files_skipped: usize,
    /// Windows Cloud Files placeholders dehydrated (or, failing that,
    /// removed outright) — task 1538 Codex P1, PR #49 state_db.rs:1785
    /// thread. Always `0` on macOS/Linux.
    pub(super) windows_placeholders_dehydrated: usize,
}

/// Task 1538 findings 1+2: delete every queued operation's staged plaintext
/// payload and every cached plaintext file from disk, on top of
/// `state_db::StateDb::purge_all_local_state` clearing the DB rows that
/// pointed at them.
///
/// Pure w.r.t. Tauri/OS credential state (takes an already-opened `db`) so
/// it is directly unit-testable without a live `AppState`/Keychain — see
/// `clear_session_impl`, the only real caller, which resolves `db`
/// independently of `DesktopConfig` (task 1538 Codex P1) and never touches
/// this function's internals. `sync_root`, by contrast, is ONLY needed to
/// resolve Windows Cloud Files placeholder paths below — best-effort and
/// `None`-able; every other step here is sync-root-independent.
///
/// Each candidate path is gated by `is_disposable_cache_path` — the same
/// safety check `reset_macos_integration` already uses — before removal, so
/// a bug that fed this function an unexpected path can never turn a sign-out
/// into an arbitrary-file-delete. A gated-out or already-missing path is
/// logged, not fatal, per file. Since R10 (task 1834) a failure of the purge
/// ITSELF is fatal to the sign-out (macOS and Linux) and to an engine start
/// (the account reset), which both call this one function.
pub(super) fn purge_local_state_files(
    db: &state_db::StateDb,
    sync_root: Option<&std::path::Path>,
    kept: Option<&sign_out::kept_set::KeptSet>,
) -> Result<LocalStatePurgeSummary, String> {
    purge_local_state_files_in(db, sync_root, &production_staging_dirs(), kept)
}

/// The directories whose plaintext copies the purge sweeps as a whole. A unit test never sweeps the shared
/// per-process sandbox that the engine-bridge tests stage into (a sweep from one test would delete a copy another
/// is about to read); the sweep itself is tested with directories the test creates.
pub(super) fn production_staging_dirs() -> Vec<PathBuf> {
    #[cfg(test)]
    {
        STAGING_DIRS_OVERRIDE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_default()
    }
    #[cfg(not(test))]
    {
        release_staging_dirs()
    }
}

/// What a release build sweeps: every place the engine stages a plaintext copy. A function of its own, compiled in
/// every build, so a test can look at the list a release build would use (`production_staging_dirs` is the test
/// override in a test build).
pub(super) fn release_staging_dirs() -> Vec<PathBuf> {
    engine_bridge::finder_staging_candidates()
}

/// Unit tests only: empty (nothing is swept) unless a test sets it, so the production call sites can be driven with
/// directories the test created (`account_binding_tests::StagingOverride`).
#[cfg(test)]
pub(super) static STAGING_DIRS_OVERRIDE: std::sync::Mutex<Option<Vec<PathBuf>>> = std::sync::Mutex::new(None);

/// Every regular file under `dir`, not following symlinks (a link is never a plaintext copy of ours, and
/// following one could leave the staging directory). A directory that does not exist has none.
pub(super) fn files_under(dir: &std::path::Path, depth: u8, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() {
            out.push(entry.path());
        } else if kind.is_dir() && depth > 0 {
            files_under(&entry.path(), depth - 1, out);
        }
    }
}

/// The current user's id, where there is one to compare (Unix). A Windows temp dir is already per-user.
pub(super) fn current_uid() -> Option<u32> {
    #[cfg(unix)]
    {
        // SAFETY: `geteuid` has no preconditions and cannot fail.
        Some(unsafe { libc::geteuid() })
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// A staging directory the sweep may enter (R5, fix round 2): a real directory, not a link, that `uid` owns, under
/// a parent (`.../beebeeb`) that is a real directory `uid` owns too. On a shared `/tmp` another local user's
/// `beebeeb/finder-writes` is neither ours to read nor ours to delete from, and a link could lead out of the
/// staging area. Without a user id to compare (Windows) a real directory is enough.
pub(super) fn is_owned_staging_dir(dir: &std::path::Path, uid: Option<u32>) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let ours = |path: &std::path::Path| {
            std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir() && uid.is_none_or(|uid| meta.uid() == uid))
        };
        ours(dir) && dir.parent().is_none_or(ours)
    }
    #[cfg(not(unix))]
    {
        let _ = uid;
        std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir())
    }
}

/// Every orphan regular file in the staging directories this user owns; any other directory is left alone.
pub(super) fn orphan_staged_files(staging_dirs: &[PathBuf], uid: Option<u32>) -> Vec<PathBuf> {
    let mut orphans = Vec::new();
    for dir in staging_dirs {
        if is_owned_staging_dir(dir, uid) {
            files_under(dir, 8, &mut orphans);
        } else if dir.exists() {
            tracing::warn!(dir = %dir.display(), "purge did not sweep a staging directory that is not this user's");
        }
    }
    orphans
}

/// [`purge_local_state_files`] with the staging directories to sweep. In this order: delete the files the rows
/// point at, and every orphan in the staging directories this user owns, each behind `is_disposable_cache_path`;
/// if ANY removal fails (a missing file is fine) return an error and clear no row, so a retry finds everything and
/// nothing is forgotten while a plaintext copy is still on disk; only then clear the rows. Given a `KeptSet` (a
/// macOS sign-out), the guard of [`purge_local_state_files_with`] runs before all of it, and the waiting writes'
/// staged copies are deleted last.
pub(super) fn purge_local_state_files_in(
    db: &state_db::StateDb,
    sync_root: Option<&std::path::Path>,
    staging_dirs: &[PathBuf],
    kept: Option<&sign_out::kept_set::KeptSet>,
) -> Result<LocalStatePurgeSummary, String> {
    purge_local_state_files_as(db, sync_root, staging_dirs, current_uid(), kept)
}

/// [`purge_local_state_files_in`] as the user `uid` (a test pretends to be another user).
pub(super) fn purge_local_state_files_as(
    db: &state_db::StateDb,
    sync_root: Option<&std::path::Path>,
    staging_dirs: &[PathBuf],
    uid: Option<u32>,
    kept: Option<&sign_out::kept_set::KeptSet>,
) -> Result<LocalStatePurgeSummary, String> {
    purge_local_state_files_with(db, sync_root, staging_dirs, uid, kept, &|path| {
        std::fs::remove_file(path)
    })
}

/// [`purge_local_state_files_as`] with the removal passed in (a test makes one removal fail). Spec 2026-10-10 §5.2
/// step 8 and W13: given a `KeptSet`, the guard comes first and refuses with `unsent_copy_failed: …` before any path
/// is collected or deleted, unless every waiting change is kept or acknowledged. Then every other file is removed, and
/// only after all of them the waiting writes' staged copies.
pub(super) fn purge_local_state_files_with(
    db: &state_db::StateDb,
    sync_root: Option<&std::path::Path>,
    staging_dirs: &[PathBuf],
    uid: Option<u32>,
    kept: Option<&sign_out::kept_set::KeptSet>,
    remove: &dyn Fn(&std::path::Path) -> std::io::Result<()>,
) -> Result<LocalStatePurgeSummary, String> {
    // Spec 2026-10-10 §5.2 step 8: given a `KeptSet` (macOS), the guard is the FIRST statement: one read, and a
    // refusal before any path is collected or deleted. Without one there is no guard, and no copy is held back.
    let waiting_copies: std::collections::HashSet<PathBuf> = match kept {
        Some(kept) => {
            let guard = db
                .unkept_waiting_changes(kept)
                .map_err(|e| format!("check what the sign-out kept: {e}"))?;
            if !guard.unkept.is_empty() {
                tracing::error!(
                    unkept = guard.unkept.len(),
                    "sign-out stopped: a waiting change was not kept; nothing was deleted"
                );
                return Err(sign_out::copy::coded(
                    sign_out::copy::UNSENT_COPY_FAILED_CODE,
                    sign_out::copy::E2_UNSENT_COPY_FAILED,
                ));
            }
            guard.waiting_payloads.into_iter().map(PathBuf::from).collect()
        }
        None => std::collections::HashSet::new(),
    };
    let mut paths: Vec<PathBuf> = db
        .local_state_file_paths()
        .map_err(|e| format!("list the local files: {e}"))?
        .into_iter()
        .map(PathBuf::from)
        .collect();
    let owned: std::collections::HashSet<PathBuf> = paths.iter().cloned().collect();
    paths.extend(
        orphan_staged_files(staging_dirs, uid)
            .into_iter()
            .filter(|path| !owned.contains(path)),
    );
    // W13: every other file first, the waiting writes' staged copies last, so a failure on any other file leaves every
    // waiting write as it was.
    let (waiting_last, others_first): (Vec<PathBuf>, Vec<PathBuf>) =
        paths.drain(..).partition(|path| waiting_copies.contains(path));
    let mut files_removed = 0usize;
    let mut files_skipped = 0usize;
    let (removed, skipped) = remove_purge_paths(&others_first, remove)?;
    files_removed += removed;
    files_skipped += skipped;
    #[cfg(test)]
    sign_out::crash_point("AfterFirstGroup");
    // The waiting writes' staged copies one at a time: a removal that fails stops the purge before the next copy, and a
    // test (spec 2026-10-10 U42) can stop the process between two of them.
    for (index, path) in waiting_last.iter().enumerate() {
        if index == 1 {
            #[cfg(test)]
            sign_out::crash_point("BetweenWaitingCopies");
        }
        let (removed, skipped) = remove_purge_paths(std::slice::from_ref(path), remove)?;
        files_removed += removed;
        files_skipped += skipped;
    }
    #[cfg(test)]
    sign_out::crash_point("BeforeRowClear");

    let purge = db
        .purge_all_local_state()
        .map_err(|e| format!("purge operation queue and cache metadata: {e}"))?;

    // Task 1538 Codex P1 (state_db.rs:1785 thread): a Windows Cloud Files
    // placeholder's plaintext lives in the sync root, not at `cache_path` —
    // the loop above can never find it. `local_placeholder_paths` is the
    // cross-platform candidate list `purge_all_local_state` captured before
    // flipping those rows' status. `resolve_purge_placeholder_paths` (the
    // "which files, which safe paths" decision) runs on every platform so
    // it stays exercised and unit-testable without a Windows host; only the
    // actual `CfDehydratePlaceholder` FFI call is Windows-only. macOS/Linux
    // both store hydrated bytes as a separate cache copy already covered by
    // `cache_paths` above, and macOS's File Provider domain removal
    // (`clear_session_impl`, right after this function returns) handles the
    // analogous cleanup there — so this list is a deliberate no-op on those
    // platforms, not an oversight.
    let windows_placeholders_dehydrated: usize = if purge.local_placeholder_paths.is_empty() {
        0
    } else {
        match sync_root {
            Some(root) => {
                let resolved = resolve_purge_placeholder_paths(&purge.local_placeholder_paths, root);
                #[cfg(target_os = "windows")]
                {
                    // Windows production cleanup runs before metadata removal.
                    let _ = resolved;
                    0
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = resolved;
                    0
                }
            }
            None => {
                #[cfg(target_os = "windows")]
                tracing::warn!(
                    candidate_count = purge.local_placeholder_paths.len(),
                    "sign-out purge: no sync root known; Windows placeholders could not be dehydrated/removed"
                );
                0
            }
        }
    };

    Ok(LocalStatePurgeSummary {
        queued_ops_purged: purge.queued_ops_purged,
        files_removed,
        files_skipped,
        windows_placeholders_dehydrated,
    })
}

/// The purge's removal loop over one group: each path behind `is_disposable_cache_path`; a missing file is fine; any
/// other failure returns an error with no path in it. `(removed, skipped)`.
pub(super) fn remove_purge_paths(
    paths: &[PathBuf],
    remove: &dyn Fn(&std::path::Path) -> std::io::Result<()>,
) -> Result<(usize, usize), String> {
    let (mut removed, mut skipped, mut failed) = (0usize, 0usize, 0usize);
    let mut first_failure: Option<std::io::ErrorKind> = None;
    for path in paths {
        if !is_disposable_cache_path(path) {
            skipped += 1;
            tracing::warn!(path = %path.display(), "purge skipped a path outside the known cache/staging roots");
            continue;
        }
        match remove(path) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                failed += 1;
                first_failure.get_or_insert(error.kind());
                tracing::warn!(path = %path.display(), error = %error, "failed to remove a local file in the purge");
            }
        }
    }
    if failed > 0 {
        // No path in the message: it reaches the sign-out error and the engine-start error.
        return Err(format!(
            "{failed} local file(s) could not be removed ({})",
            first_failure.map_or_else(|| "unknown".to_string(), |kind| kind.to_string())
        ));
    }
    Ok((removed, skipped))
}

/// Resolve every `(file_id, server_relative_path)` candidate from
/// [`state_db::LocalStatePurge::local_placeholder_paths`] to an absolute,
/// safety-checked on-disk path under `sync_root` — the same canonicalize +
/// allowed-roots invariant [`free_up_space_windows`] already enforces for
/// the analogous eviction sweep, reused here (task 1538 Codex P1, PR #49
/// state_db.rs:1785 thread).
///
/// Pure and platform-independent on purpose: the actual
/// `CfDehydratePlaceholder` FFI call that ACTS on this list is Windows-only
/// (`windows_cf::signout::purge`); this is only the "which files,
/// which paths" decision, so it is unit-testable on any host. A candidate is
/// silently dropped (not an error — sign-out must still succeed) when it
/// doesn't resolve to a real file under an allowed root: an
/// already-cleaned-up or corrupt row has nothing for the caller to act on.
pub(super) fn resolve_purge_placeholder_paths(
    candidates: &[(String, String)],
    sync_root: &std::path::Path,
) -> Vec<(String, PathBuf)> {
    let allowed_roots = free_up_space_allowed_roots(sync_root);
    candidates
        .iter()
        .filter_map(|(file_id, rel_path)| {
            let rel = rel_path.trim_matches('/');
            if rel.is_empty() {
                return None;
            }
            let local_path = sync_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            let canonical = local_path.canonicalize().ok()?;
            if !allowed_roots.iter().any(|root| canonical.starts_with(root)) {
                return None;
            }
            Some((file_id.clone(), canonical))
        })
        .collect()
}

#[cfg(unix)]
pub(super) fn remove_stale_ipc_socket() -> Result<bool, String> {
    let path = ipc_socket::ipc_socket_path();
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("remove IPC socket {}: {error}", path.display())),
    }
}

// Windows has no Unix-socket daemon endpoint — the Cloud Files provider runs
// in-process — so there is never a stale socket to remove.
#[cfg(not(unix))]
pub(super) fn remove_stale_ipc_socket() -> Result<bool, String> {
    Ok(false)
}

// Windows and Linux only: on macOS the reconciler's `ensure_sync_root_and_engine` replaces it.
#[cfg(not(target_os = "macos"))]
pub(super) async fn persist_sync_root_and_start_engine(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    cfg: &mut HeldConfig,
    root: PathBuf,
    #[cfg(target_os = "windows")] _transition: &tokio::sync::MutexGuard<'_, ()>,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let generation = {
        windows_cf::ensure_reactivation_allowed()?;
        SESSION_COMMANDS.generation()?
    };
    // Task 1889: the sync root is written onto the file as it is now, not by saving the copy loaded earlier; `cfg`
    // afterwards holds what was saved, so the pause state below is the persisted one.
    cfg.update_held(|held| {
        held.sync_root = Some(root.clone());
        (true, ())
    })?;

    let acct = state.active_account()?;
    // Rehydrate the persisted pause state before spawning.
    rehydrate_sync_paused(&acct.sync_paused, cfg.pause_sync);
    {
        let mut engine_slot = acct.engine.lock().await;
        engine_start_refusal(&acct)?;
        if let Some(prev) = engine_slot.take() {
            // Task 1538 Codex P1: this is a re-login/sync-root-change
            // respawn, not sign-out — there's no purge to gate here, but an
            // unconfirmed stop is still worth knowing about (a not-really-
            // gone previous task could still be touching the same state.db
            // the freshly spawned runner is about to open).
            if !stop_engine_in_slot(&acct, prev).await.is_stopped() {
                #[cfg(target_os = "windows")]
                return Err("Could not stop the previous sync engine; retry locking before restarting sync.".into());
                tracing::warn!("previous engine did not confirm termination before respawning a new one");
            }
        }
        // Fix round 1: an unconfirmed stop of the old engine has set the flag; refuse the new one.
        engine_start_refusal(&acct)?;
        #[cfg(target_os = "windows")]
        {
            windows_cf::ensure_reactivation_allowed()?;
            SESSION_COMMANDS.validate_start(generation)?;
        }
        // No session means nothing to start (the root is saved either way); the keys are read under the slot.
        spawn_bound_engine(app, state, &acct, &mut engine_slot, root)?;
    }

    Ok(())
}

// Windows and Linux only: on macOS the reconciler's `start_check_engine` replaces it, with its limit
// on the engine-slot wait alone.
#[cfg(not(target_os = "macos"))]
pub(super) async fn start_engine_for_pending_finder_install(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    root: PathBuf,
    #[cfg(target_os = "windows")] _transition: &tokio::sync::MutexGuard<'_, ()>,
) -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    let generation = {
        windows_cf::ensure_reactivation_allowed()?;
        SESSION_COMMANDS.generation()?
    };
    let acct = state.active_account()?;

    // Rehydrate the persisted pause state before spawning. Best-effort —
    // a missing/unreadable config defaults to not-paused.
    rehydrate_sync_paused(&acct.sync_paused, false);

    #[cfg_attr(not(unix), allow(unused_variables))]
    let (started, ipc_bind_error) = {
        let mut engine_slot = acct.engine.lock().await;
        engine_start_refusal(&acct)?;
        if let Some(existing) = engine_slot.as_ref() {
            // Already running (e.g. a retry after a transient failure) —
            // reuse its bind-status handle so we still see a REAL bind
            // error instead of just re-timing-out silently.
            (false, existing.ipc_bind_error_handle())
        } else {
            #[cfg(target_os = "windows")]
            {
                windows_cf::ensure_reactivation_allowed()?;
                SESSION_COMMANDS.validate_start(generation)?;
            }
            // The keys are read under the slot: a vault that is locked by now is the same refusal as before.
            if spawn_bound_engine(app, state, &acct, &mut engine_slot, root)? == EngineStart::NoSession {
                return Err("Unlock the vault before installing the Finder location.".to_string());
            }
            let bind_error = engine_slot
                .as_ref()
                .map(|runner| runner.ipc_bind_error_handle())
                .ok_or_else(|| "the sync engine is not in its slot after it started".to_string())?;
            (true, bind_error)
        }
    };

    // The macOS File Provider extension talks to the daemon over a Unix
    // socket, so we wait for that endpoint before claiming the install is
    // ready. Windows has no such socket (the Cloud Files provider is
    // in-process), so the readiness probe is unix-only.
    #[cfg(unix)]
    if let Err(error) = wait_for_file_provider_ipc_ready(ipc_bind_error).await {
        stop_pending_finder_install_engine(state, started).await;
        return Err(error);
    }

    Ok(started)
}

// Windows and Linux only: macOS has `stop_check_engine`.
#[cfg(not(target_os = "macos"))]
pub(super) async fn stop_pending_finder_install_engine(state: &State<'_, AppState>, started: bool) {
    if !started {
        return;
    }
    let Ok(acct) = state.active_account() else { return };
    let mut engine_slot = acct.engine.lock().await;
    if let Some(prev) = engine_slot.take() {
        // Task 1538 Codex P1 — see `start_engine_if_possible`'s identical
        // respawn guard.
        if !stop_engine_in_slot(&acct, prev).await.is_stopped() {
            tracing::warn!("pending-finder-install engine did not confirm termination on stop");
        }
    }
}

/// Poll for the daemon's IPC socket to come up, OR return immediately with
/// the real bind error if the accept loop already failed to bind (task
/// 1524) — e.g. the macOS sandbox refusing a socket path outside the app's
/// container. Before this, a bind failure panicked the fire-and-forget IPC
/// task silently and this function always burned the full 3-second timeout
/// before returning a generic "Timed out…" message with no indication of
/// the real cause.
#[cfg(unix)]
pub(super) async fn wait_for_file_provider_ipc_ready(
    ipc_bind_error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
) -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;
    use tokio::time::{Duration, Instant, sleep, timeout};

    let path = ipc_socket::ipc_socket_path();
    let deadline = Instant::now() + Duration::from_secs(3);
    // One `\n`-terminated frame (task 1670 issue 3; see `docs/IPC_PROTOCOL.md`).
    let mut request = serde_json::to_vec(&ipc_socket::IpcRequest::GetSyncSummary)
        .map_err(|e| format!("encode IPC readiness probe: {e}"))?;
    request.push(b'\n');

    loop {
        if let Ok(guard) = ipc_bind_error.lock()
            && let Some(bind_error) = guard.as_ref()
        {
            return Err(format!("Could not start the local Beebeeb sync socket: {bind_error}"));
        }

        if let Ok(Ok(mut stream)) = timeout(Duration::from_millis(300), UnixStream::connect(&path)).await {
            let mut response = vec![0u8; 4096];
            if stream.write_all(&request).await.is_ok()
                && let Ok(Ok(bytes_read)) = timeout(Duration::from_millis(300), stream.read(&mut response)).await
                && bytes_read > 0
                && serde_json::from_slice::<ipc_socket::IpcResponse>(&response[..bytes_read]).is_ok()
            {
                return Ok(());
            }
        }

        if Instant::now() >= deadline {
            return Err(FINDER_IPC_NOT_READY.to_string());
        }
        sleep(Duration::from_millis(100)).await;
    }
}

/// Non-macOS mirror of what `macos_file_provider` reported before the reconciler (spec
/// 2026-10-06 §6.1 retired both on macOS). `finder_location_state` matches on this directly.
/// Windows and Linux only ever get the error arm from `file_provider_installed`.
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FileProviderStatusOutcome {
    Installed,
    NotInstalled,
    UserDisabled,
}

#[cfg(not(target_os = "macos"))]
pub(super) fn file_provider_installed() -> Result<FileProviderStatusOutcome, String> {
    Err("File Provider is only available on macOS.".to_string())
}

/// Non-macOS mirror of what `macos_file_provider::install` reported (retired on macOS).
/// `install_finder_location` matches on this directly.
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FileProviderInstallOutcome {
    Installed,
    UserDisabled,
}

/// Shown to the user (and persisted as `finder_install_last_error`) when Issue 4's
/// condition is detected: the Beebeeb File Provider domain exists but the user (or
/// macOS) has turned it off in System Settings, so waiting for it to come up would
/// never succeed. `classify_finder_install_error` recognizes this exact copy and
/// maps it to the `"user_disabled"` reason category the frontend switches on.
#[cfg(not(target_os = "macos"))]
pub(super) const FINDER_USER_DISABLED_MESSAGE: &str = "Beebeeb is turned off in System Settings. Open Login \
    Items & Extensions, turn on Beebeeb under File Providers, then try again.";

#[cfg(not(target_os = "macos"))]
pub(super) fn install_file_provider_domain() -> Result<FileProviderInstallOutcome, finder_removal::InstallFailure> {
    Err("File Provider is only available on macOS.".to_string().into())
}

// On macOS the reconciler removes the domain (`finder_remove_for`); nothing calls the bridge's
// `remove` directly, because every bridge call goes through the one gate (lead ruling T8-gate-all).
#[cfg(not(target_os = "macos"))]
pub(super) fn remove_file_provider_domain() -> Result<finder_removal::DomainRemoval, finder_removal::RemovalFailure> {
    Err("File Provider is only available on macOS.".to_string().into())
}

/// Runs a blocking File Provider call on the blocking pool, so it never holds the main thread or
/// an async runtime worker (1882 round 4). The removal blocks on the system's completion handler,
/// and when a kept folder reads as missing it then sleeps about a second (`settle_kept_state`).
/// The failure is the pool's own (a panicked or cancelled job); it carries no path.
///
/// Windows and Linux only in the app: on macOS every File Provider call goes through the reconciler's bridge
/// gate, which runs it on the blocking pool itself (`finder_setup::macos_ports::BridgeGate`).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(super) async fn on_blocking_pool<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| format!("A File Provider call did not finish: {error}"))
}

/// The removal, off the runtime (round 4).
#[cfg(not(target_os = "macos"))]
pub(super) async fn remove_file_provider_domain_blocking() -> Result<finder_removal::DomainRemoval, finder_removal::RemovalFailure>
{
    match on_blocking_pool(remove_file_provider_domain).await {
        Ok(result) => result,
        Err(message) => Err(message.into()),
    }
}

/// The install, off the runtime (round 4): its cleanup is a removal.
#[cfg(not(target_os = "macos"))]
pub(super) async fn install_file_provider_domain_blocking() -> Result<FileProviderInstallOutcome, finder_removal::InstallFailure> {
    match on_blocking_pool(install_file_provider_domain).await {
        Ok(result) => result,
        Err(message) => Err(message.into()),
    }
}

/// Task 1670 round 2: wipe the macOS hydrate-cache staging directory at every
/// account-security boundary (sign-out, lock, daemon startup) so a decrypted
/// plaintext copy staged for a Finder open can never outlive the boundary
/// that's supposed to end its access — independent of, and in addition to,
/// the per-request cleanup `fetchContents` itself does. Distinct from the
/// general local-file/smart-cache purge `purge_local_state_files` runs on
/// sign-out: THAT purge intentionally does NOT run on lock (lock keeps the
/// regular local file cache so re-unlocking stays fast), but the hydrate
/// cache holds nothing but ephemeral per-open staging copies, so it is always
/// safe — and always right — to empty here too. Best-effort and non-fatal:
/// logout/lock must always appear to succeed, and daemon startup must never
/// block on this.
#[cfg(target_os = "macos")]
pub(crate) fn purge_macos_hydrate_cache(context: &str) {
    let dir = crate::ipc_socket::macos_hydrate_cache_dir();
    match crate::ipc_socket::macos_purge_hydrate_cache_dir(&dir) {
        Ok(removed) if removed > 0 => {
            tracing::info!(removed, context, "purged macOS hydrate-cache staged plaintext");
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(error = %error, context, "failed to purge macOS hydrate-cache dir (best-effort)");
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn purge_macos_hydrate_cache(_context: &str) {}

/// "Open in Finder" on a Mac (task 1885): the bridge opens the Beebeeb folder through `NSWorkspace` and answers
/// when LaunchServices has. `Ok` only if it opened; an `Err` is what the frontend turns into the toast.
#[cfg(target_os = "macos")]
pub(super) fn file_provider_open_location() -> Result<(), String> {
    finder_open::finder_open_outcome(finder_setup::macos_ports::open_location(), finder_open::NOTHING_TO_OPEN)
}

/// "Show in Finder" for one item on a Mac (task 1885): the item's File Provider identifier is its file id.
#[cfg(target_os = "macos")]
pub(super) fn file_provider_reveal_item(item_id: &str) -> Result<(), String> {
    finder_open::finder_open_outcome(
        finder_setup::macos_ports::reveal_item(item_id),
        finder_open::NOTHING_TO_SHOW,
    )
}

/// The one conversion from a bridge error to text that can leave the bridge's callers: it
/// reaches Repair's `warnings` and `finder_domain_user_enabled`'s returned error, both shown to a
/// person. Domain and code only, never the OS's message (lead ruling T1-4). `tracing` sites log
/// the `FpError` itself, which keeps the message for stdout. (`finder_open::finder_open_outcome`
/// is the same conversion for the open and the reveal.)
#[cfg(target_os = "macos")]
pub(super) fn user_facing_fp<T>(result: Result<T, crate::finder_setup::error::FpError>) -> Result<T, String> {
    result.map_err(|e| e.redacted())
}

/// `Some(true)` = the Beebeeb domain is user-enabled, `Some(false)` = the user (or
/// macOS) has disabled it in System Settings, `None` = the domain isn't registered
/// yet. Backs the `finder_domain_user_enabled` poll the "turned off in System
/// Settings" onboarding card uses to notice when the user flips it back on.
#[cfg(target_os = "macos")]
pub(super) fn file_provider_domain_user_enabled() -> Result<Option<bool>, String> {
    // Through `user_facing_fp`, never an `FpError`'s `Display`: the error text reaches the frontend,
    // and `Display` carries the OS's message (lead ruling T1-4).
    match user_facing_fp(finder_setup::macos_ports::domain_state())? {
        finder_setup::core::DomainState::Enabled => Ok(Some(true)),
        finder_setup::core::DomainState::Disabled => Ok(Some(false)),
        finder_setup::core::DomainState::NotRegistered => Ok(None),
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn file_provider_domain_user_enabled() -> Result<Option<bool>, String> {
    Err("File Provider is only available on macOS.".to_string())
}

/// Spec 2026-10-06 §5.3: tell the macOS Finder reconciler something happened. A no-op where no
/// reconciler runs (Windows, Linux, and tests that install none). A reconciler that has stopped
/// (it panicked; it starts again at the next launch) cannot be told, and that is logged, never
/// silent (lead ruling 3).
pub(super) fn notify_finder(state: &AppState, trigger: finder_setup::core::Trigger) {
    if let Some(handle) = state.finder_setup.get()
        && let Err(error) = handle.trigger(trigger)
    {
        tracing::warn!(%error, trigger = trigger.as_str(), "finder setup: could not deliver a trigger");
    }
}

/// Where keys arrived from, for the lifecycle log (§8): a sign-in that completed is logged, an unlock after a Lock is
/// not (a Lock is not logged either).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeysFrom {
    /// A sign-in, a re-sign-in or the recovery-phrase step that completes one.
    SignIn,
    /// The Keychain unlock after a Lock.
    Unlock,
}

/// Keys arrived on this Mac (spec 2026-10-06 §5.3 (2)). The functions that put a `Session` in
/// memory call this right after the engine start, whatever its outcome.
/// `finder_setup_wiring_tests` keeps that list complete.
pub(super) fn keys_arrived(state: &AppState, from: KeysFrom) {
    // Keys are here: no recovery phrase is owed any more (review P2-3).
    forget_pending_setup(&state.pending_setup);
    if from == KeysFrom::SignIn {
        lifecycle_log::event(lifecycle_log::LifecycleEvent::SignedIn);
    }
    notify_finder(state, finder_setup::core::Trigger::KeysArrived);
}

// ── R8: signing in on a Mac that already holds an account (spec 2026-10-06 §3) ─────────────────────────────────────

/// Where a sign-in looks for what an earlier account left on this computer: the state directory that holds `state.db`.
/// Production resolves the app's own; a test hands in a throwaway one, so it never reads the real one. A state
/// directory that cannot be resolved is carried as the error it is: it counts as a trace.
#[cfg(not(target_os = "windows"))]
pub(super) struct LocalSources {
    pub(super) state_dir: Result<PathBuf, String>,
}

#[cfg(not(target_os = "windows"))]
impl LocalSources {
    pub(super) fn production() -> Self {
        Self {
            state_dir: state_paths::beebeeb_state_dir(),
        }
    }

    #[cfg(test)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(super) fn for_test(state_dir: &Path) -> Self {
        Self {
            state_dir: Ok(state_dir.to_path_buf()),
        }
    }

    /// `state.db` when the directory has one: `Ok(None)` for none yet, `Err` for a directory or a database that
    /// cannot be read.
    pub(super) fn state_db(&self) -> Result<Option<state_db::StateDb>, String> {
        match &self.state_dir {
            Ok(dir) => state_db_from_state_dir(dir),
            Err(error) => Err(error.clone()),
        }
    }
}

/// R8 + R10: the recorded owner of this computer's local data: the only account R8 compares a sign-in with.
#[cfg(not(target_os = "windows"))]
pub(super) fn local_data_owner(sources: &LocalSources) -> Result<Option<account_binding::Identity>, String> {
    match sources.state_db()? {
        Some(db) => db.owner().map_err(|e| format!("read the local data owner: {e}")),
        None => Ok(None),
    }
}

/// Does the Keychain still hold a vault key for this account slot, in the id-keyed or the legacy store? It never
/// unlocks and never reads the key (the real Keychain answers from the item's attributes). Fails closed.
#[cfg(not(target_os = "windows"))]
pub(super) fn keychain_vault_key_present(account_id: &str) -> bool {
    vault_key_in_either_layout(
        &platform_keychain_store_for(account_id),
        &keychain::legacy_platform_keychain_store(),
    )
}

/// The question over any two stores (the id-keyed layout, then the legacy one), so a test can hold a fake in each. A store that
/// cannot answer counts as holding a key (`keychain::holds_vault_key`'s fail-closed rule): never a reason to say "no key".
#[cfg(not(target_os = "windows"))]
pub(super) fn vault_key_in_either_layout<A: keychain::AuthSecretStore, B: keychain::AuthSecretStore>(
    id_keyed: &A,
    legacy: &B,
) -> bool {
    keychain::holds_vault_key(id_keyed) || keychain::holds_vault_key(legacy)
}
