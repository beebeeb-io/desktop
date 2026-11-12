//! Local state the engine itself opens or empties: the state database for a configuration, and the macOS
//! hydrate cache; and the one gate every purge asks before it deletes a path (`is_disposable_cache_path`) with its
//! roots. Must not decide when a purge runs (sign-out and lock do, in the shell).

use std::path::PathBuf;

pub fn state_db_for_config(
    identity: &'static crate::identity::EngineIdentity,
    cfg: &crate::config::DesktopConfig,
) -> Result<Option<crate::state_db::StateDb>, String> {
    let Some(_) = &cfg.sync_root else {
        return Ok(None);
    };
    let db_path = crate::state_paths::beebeeb_state_dir_for(identity)?.join(crate::state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Ok(None);
    }
    crate::state_db::StateDb::open(&db_path)
        .map(Some)
        .map_err(|e| format!("open state.db: {e}"))
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
pub fn purge_macos_hydrate_cache(identity: &crate::identity::EngineIdentity, context: &str) {
    let dir = crate::ipc_socket::macos_hydrate_cache_dir(identity);
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
pub fn purge_macos_hydrate_cache(_identity: &crate::identity::EngineIdentity, _context: &str) {}

pub fn disposable_cache_roots(identity: &crate::identity::EngineIdentity) -> Vec<PathBuf> {
    disposable_cache_roots_in(
        identity.scratch_dir_in(&std::env::temp_dir()),
        dirs::cache_dir().map(|dir| identity.scratch_dir_in(&dir)),
        macos_hydrate_cache_root(identity),
        &crate::engine_bridge::FinderStagingBases::current(identity),
    )
}

/// The hydrate cache's root on a Mac (task 1670); none elsewhere.
pub fn macos_hydrate_cache_root(identity: &crate::identity::EngineIdentity) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(crate::ipc_socket::macos_hydrate_cache_dir(identity))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = identity;
        None
    }
}

/// Spec 2026-10-10 §7.1 (U12): the roots `is_disposable_cache_path` accepts, from directories passed in, so a test can
/// check a release layout. The temp dir, the cache dir, the macOS hydrate cache, then every staging folder (spec
/// 2026-10-09 §8.4), never the whole data dir.
pub fn disposable_cache_roots_in(
    temp_dir: PathBuf,
    cache_dir: Option<PathBuf>,
    hydrate_cache_dir: Option<PathBuf>,
    staging: &crate::engine_bridge::FinderStagingBases,
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

/// `…/Logs/<the identity's log folder>` (`Logs/Beebeeb` for the real app) anywhere in `path`: the lifecycle log's directory, in the home or in the app's
/// container (spec 2026-10-06 §8). It survives sign-out, Repair and the account reset (lead ruling
/// T5-⚠c), so no purge may treat anything under it as disposable, whatever root it sits in.
pub fn is_lifecycle_log_path(identity: &crate::identity::EngineIdentity, path: &std::path::Path) -> bool {
    let names: Vec<&str> = path.components().filter_map(|part| part.as_os_str().to_str()).collect();
    names
        .windows(2)
        .any(|pair| pair[0].eq_ignore_ascii_case("Logs") && pair[1].eq_ignore_ascii_case(identity.logs_dir_name()))
}

pub fn is_disposable_cache_path(identity: &crate::identity::EngineIdentity, path: &std::path::Path) -> bool {
    if is_lifecycle_log_path(identity, path) {
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
    if is_lifecycle_log_path(identity, &canonical_path) {
        return false;
    }
    disposable_cache_roots(identity)
        .into_iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| canonical_path.starts_with(root))
}
