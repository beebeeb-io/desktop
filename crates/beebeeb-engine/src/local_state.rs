//! Local state the engine itself opens or empties: the state database for a configuration, and the macOS
//! hydrate cache; and the one gate every purge asks before it deletes a path (`is_disposable_cache_path`) with its
//! roots. Must not decide when a purge runs (sign-out and lock do, in the shell).

use std::path::PathBuf;

/// Task 2488: `pub` only with the `direct-state-db` feature (the Tauri shell). The native shell reaches the local
/// state through `app::Engine::local_state`.
#[cfg(any(test, feature = "direct-state-db"))]
pub fn state_db_for_config(
    identity: &'static crate::identity::EngineIdentity,
    cfg: &crate::config::DesktopConfig,
) -> Result<Option<crate::state_db::StateDb>, String> {
    state_db_for_config_unbound(identity, cfg)
}

/// The state database for a configuration, opened WITHOUT asking whose it is: for the engine's own device-level
/// reads (`backup::stop`) and the Tauri shell's door above.
#[cfg(any(test, feature = "direct-state-db"))]
pub(crate) fn state_db_for_config_unbound(
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

/// What became of the state database at a sign-out or sign-in (backlog 1857).
#[derive(Debug, PartialEq, Eq)]
pub enum StateDbCheck {
    /// No `state.db`, or one that opens: nothing was changed.
    Sound,
    /// `state.db` is corrupt (SQLite says not-a-database or malformed). It was moved aside to this path, with its
    /// `-wal` and `-shm` files, so the next sign-in starts a fresh one. The caller says so to the person.
    MovedAside(PathBuf),
}

/// Is this open error SQLite's "the file is corrupt"? By result code only (`error_category`), never by text.
pub fn is_corrupt_state_db_error(error: &(dyn std::error::Error + 'static)) -> bool {
    crate::engine_bridge::error_category(error) == "database_corrupt"
}

/// Opens `state.db` in `state_dir`. A corrupt one is moved aside (never deleted: the person may still want it
/// looked at) and reported as [`StateDbCheck::MovedAside`]. Any other failure to open (busy, read-only, a
/// permission error) is an `Err` and the file stays where it is: it fails closed, it never starts over on a
/// database that may be healthy.
pub fn move_corrupt_state_db_aside(state_dir: &std::path::Path) -> Result<StateDbCheck, String> {
    let path = crate::state_paths::state_db_path_from_state_dir(state_dir);
    match std::fs::metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(StateDbCheck::Sound),
        Err(error) => return Err(format!("check state.db: {error}")),
        Ok(meta) if !meta.is_file() => return Err("state.db is not a file".to_string()),
        Ok(_) => {}
    }
    let opened = crate::state_db::StateDb::open(&path).and_then(|db| db.has_account_data().map(|_| ()));
    let error = match opened {
        Ok(()) => return Ok(StateDbCheck::Sound),
        Err(error) => error,
    };
    if !is_corrupt_state_db_error(&error) {
        return Err(format!("open state.db: {error}"));
    }
    let aside = unused_aside_path(state_dir)?;
    std::fs::rename(&path, &aside).map_err(|e| format!("move the corrupt state.db aside: {e}"))?;
    for suffix in ["-wal", "-shm"] {
        let mut from = path.clone().into_os_string();
        from.push(suffix);
        let mut to = aside.clone().into_os_string();
        to.push(suffix);
        let _ = std::fs::rename(from, to);
    }
    // RC2-4: the fresh `state.db` lists the files again with no content agreed with the server. The marker asks the
    // engine to check the files on disk against the server's content once the first pull is in
    // (`EngineBridge::check_after_rebuild`). A marker that cannot be written leaves them held, as before: never lost.
    if let Err(error) = RebuiltCheck::owe(state_dir) {
        tracing::warn!(%error, "state.db rebuilt: the check of the files on disk could not be asked for; they stay held");
    }
    tracing::warn!("state.db was corrupt; moved aside, the next sign-in starts fresh");
    Ok(StateDbCheck::MovedAside(aside))
}

/// RC2-4: the file next to `state.db` that says a rebuilt `state.db` still owes the check of the files on disk.
pub const REBUILT_CHECK_FILENAME: &str = "state.db.rebuilt-check";

/// RC2-4: the check a rebuilt `state.db` owes. Its file lists the ids of files already checked whose bytes on disk
/// differ from the server's content (held as real edits; never downloaded again by this check). Ids only, no names.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RebuiltCheck {
    /// When the engine first saw the rebuild's first pull in (seconds since the epoch), if it has.
    pub pulled_at: Option<i64>,
    pub differing: std::collections::BTreeSet<String>,
}

/// RC2-4: the check ends only once no file is left to check this long after the first pull, so rows a platform
/// marks synced a little after that pull (Windows' placeholders) are checked too.
pub const REBUILT_CHECK_SETTLE_SECS: i64 = 600;

impl RebuiltCheck {
    fn path(state_dir: &std::path::Path) -> PathBuf {
        state_dir.join(REBUILT_CHECK_FILENAME)
    }

    /// A rebuild owes the check: a fresh marker (a second rebuild starts it again, empty).
    pub fn owe(state_dir: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(Self::path(state_dir), b"")
    }

    /// The check owed in `state_dir`, or `None` when none is.
    pub fn read(state_dir: &std::path::Path) -> std::io::Result<Option<Self>> {
        match std::fs::read_to_string(Self::path(state_dir)) {
            Ok(text) => {
                let mut owed = Self::default();
                for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                    match line.strip_prefix("pulled_at=") {
                        Some(at) => owed.pulled_at = at.parse().ok(),
                        None => {
                            owed.differing.insert(line.to_string());
                        }
                    }
                }
                Ok(Some(owed))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Records the files found differing so far.
    pub fn save(&self, state_dir: &std::path::Path) -> std::io::Result<()> {
        let mut text = String::new();
        if let Some(at) = self.pulled_at {
            text.push_str(&format!("pulled_at={at}\n"));
        }
        for id in &self.differing {
            text.push_str(id);
            text.push('\n');
        }
        let path = Self::path(state_dir);
        let tmp = state_dir.join(format!("{REBUILT_CHECK_FILENAME}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }

    /// Every file is checked: no check is owed any more.
    pub fn finish(state_dir: &std::path::Path) -> std::io::Result<()> {
        match std::fs::remove_file(Self::path(state_dir)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

fn unused_aside_path(state_dir: &std::path::Path) -> Result<PathBuf, String> {
    for n in 0..1000 {
        let name = if n == 0 {
            "state.db.corrupt".to_string()
        } else {
            format!("state.db.corrupt-{n}")
        };
        let candidate = state_dir.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("no free name to move the corrupt state.db to".to_string())
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
/// container (spec 2026-10-06 §8). It survives sign-out, Repair and the account reset, so no purge may treat anything under it as disposable, whatever root it sits in.
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

/// A development build's read-only look at one state database file the person names (`--db` of the native app).
/// It asks nobody whose the file is: it is the developer's own file, on their own command line. Compiled into debug
/// builds and builds with the `dev-viewer` feature only; a shipped build has no such door (task 2488).
#[cfg(any(debug_assertions, test, feature = "dev-viewer"))]
pub fn open_for_dev_viewer(
    path: &std::path::Path,
) -> Result<crate::state_db::StateDb, crate::state_db::ReadOnlyOpenError> {
    crate::state_db::StateDb::open_read_only(path)
}

#[cfg(test)]
#[path = "local_state_tests.rs"]
mod local_state_tests;
