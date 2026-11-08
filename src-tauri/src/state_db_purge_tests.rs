//! State-db resolution, purge-path and file-versions tests, split out of `tests.rs` (file-size ratchet; moved unchanged).

use super::*;
use crate::tests::{at, file, folder};
use std::collections::HashSet;

// ── Task 1538 Codex P1 (PR #49, lib.rs:1085 thread): DB resolution
//    independent of DesktopConfig ─────────────────────────────────────

/// `state_db_from_state_dir` opens `state.db` from a directory it is
/// handed directly — there is no `DesktopConfig` parameter for it to
/// depend on, so a missing/corrupt/unreadable `desktop.toml` (or one
/// whose `sync_root` was rejected) structurally cannot make this skip
/// the sign-out purge the way the old `DesktopConfig::load().ok()...`
/// gate could.
#[test]
fn state_db_from_state_dir_opens_an_existing_db_with_no_config_involved() {
    let dir = tempfile::tempdir().unwrap();
    // Create the DB first (as a prior engine run would have).
    let _ = crate::state_db::StateDb::open(dir.path().join(crate::state_paths::STATE_DB_FILENAME)).unwrap();

    let result = state_db_from_state_dir(dir.path());

    assert!(
        matches!(result, Ok(Some(_))),
        "an existing state.db must open even though no DesktopConfig was ever read"
    );
}

/// A fresh install that has never synced anything has no `state.db` yet
/// — `Ok(None)`, not an error, and not a reason to fail sign-out.
#[test]
fn state_db_from_state_dir_returns_none_when_the_db_does_not_exist_yet() {
    let dir = tempfile::tempdir().unwrap();

    let result = state_db_from_state_dir(dir.path());

    assert!(matches!(result, Ok(None)));
}

/// Lead ruling 13 (Task 12): a stat that fails is not "no database yet": that is an error the caller must fail
/// closed on. Two stats that fail: on every platform, a path with a NUL byte, which no file system can look up;
/// on macOS and Linux, a state dir that is a FILE, so the stat of `<file>/state.db` fails with "not a directory".
/// Windows reports that second path as not found (no database can exist under a file), so it is not a failed stat
/// there.
#[test]
fn state_db_from_state_dir_reads_a_failed_stat_as_unreadable_never_as_absent() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg_attr(target_os = "windows", allow(unused_mut))]
    let mut state_dirs = vec![dir.path().join("state\0dir")];
    #[cfg(not(target_os = "windows"))]
    {
        let not_a_directory = dir.path().join("state-dir-is-a-file");
        std::fs::write(&not_a_directory, b"not a directory").unwrap();
        state_dirs.push(not_a_directory);
    }
    for state_dir in state_dirs {
        let stat = state_dir.join(crate::state_paths::STATE_DB_FILENAME).try_exists();
        assert!(stat.is_err(), "the premise: the stat itself fails ({stat:?})");

        let result = state_db_from_state_dir(&state_dir);

        let error = result.err().expect("a failed stat is an error, never Ok(None)");
        assert!(error.starts_with("read state.db: "), "{error}");
    }
}

// ── Task 1538 Codex P1 (PR #49, state_db.rs:1785 thread): Windows
//    placeholder path resolution — cross-platform decision logic ──────

/// The "which files, which safe paths" decision `resolve_purge_placeholder_paths`
/// makes is plain path-joining + canonicalize + an allowed-roots check —
/// none of it is Windows-specific, so it must run (and be provable
/// correct) on every host, even though only a Windows build ever ACTS on
/// the result via `purge_windows_placeholders`.
#[test]
fn resolve_purge_placeholder_paths_resolves_candidates_under_sync_root() {
    let sync_root = tempfile::tempdir().unwrap();
    let nested_dir = sync_root.path().join("Docs");
    std::fs::create_dir_all(&nested_dir).unwrap();
    let file_path = nested_dir.join("report.docx");
    std::fs::write(&file_path, b"placeholder content").unwrap();

    let candidates = vec![("win-local".to_string(), "/Docs/report.docx".to_string())];
    let resolved = resolve_purge_placeholder_paths(&candidates, sync_root.path());

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].0, "win-local");
    assert_eq!(resolved[0].1, file_path.canonicalize().unwrap());
}

/// A candidate whose resolved path escapes the sync root (or the engine
/// cache dir) — e.g. a corrupt `path` column — is dropped, never handed
/// to a caller that might delete it. Same fail-closed invariant
/// `purge_local_state_files_refuses_paths_outside_known_roots` above
/// already proves for the `cache_path` list.
#[test]
fn resolve_purge_placeholder_paths_drops_candidates_outside_allowed_roots() {
    let sync_root = tempfile::tempdir().unwrap();
    let outside_dir = tempfile::tempdir().unwrap();
    let outside_file = outside_dir.path().join("not-in-sync-root.bin");
    std::fs::write(&outside_file, b"must survive").unwrap();

    // An absolute-looking `path` value that, joined naively onto
    // sync_root, would still land inside sync_root — so instead prove
    // the guard by resolving a path that canonicalizes OUTSIDE the
    // allowed roots entirely: a `..`-relative escape.
    let candidates = vec![("escaping".to_string(), "../../../../../../etc/hosts".to_string())];
    let resolved = resolve_purge_placeholder_paths(&candidates, sync_root.path());

    assert!(
        resolved.is_empty(),
        "a candidate resolving outside the sync root / engine cache dir must never be returned"
    );
}

/// A candidate whose file no longer exists on disk (already cleaned up,
/// or a stale/corrupt row) fails to canonicalize and is dropped, not
/// treated as an error — sign-out must still succeed.
#[test]
fn resolve_purge_placeholder_paths_drops_missing_files() {
    let sync_root = tempfile::tempdir().unwrap();
    let candidates = vec![("gone".to_string(), "/never/existed.txt".to_string())];

    let resolved = resolve_purge_placeholder_paths(&candidates, sync_root.path());

    assert!(resolved.is_empty());
}

#[test]
fn file_versions_payload_for_frontend_accepts_fixture_shape() {
    let payload = serde_json::json!({
        "file_id": "file-123",
        "current_version": 3,
        "versions": [
            {
                "id": "object-version-3",
                "object_version_id": "object-version-3",
                "file_version_id": null,
                "source": "object_version",
                "version_number": 3,
                "size_bytes": 9000,
                "chunk_count": 2,
                "chunk_size_bytes": 5000,
                "storage_pool_id": "pool-1",
                "created_by": "user-1",
                "uploaded_by": "user-1",
                "restorable": true,
                "created_at": "2026-07-25T12:03:00Z"
            },
            {
                "id": "file-version-2",
                "object_version_id": null,
                "file_version_id": "file-version-2",
                "source": "file_version",
                "version_number": 2,
                "size_bytes": 7000,
                "chunk_count": 2,
                "chunk_size_bytes": null,
                "storage_pool_id": "pool-1",
                "created_by": "user-1",
                "uploaded_by": "user-1",
                "restorable": true,
                "created_at": "2026-07-25T12:02:00Z"
            }
        ]
    });

    let got = file_versions_payload_for_frontend(payload).expect("fixture payload is accepted");

    assert_eq!(got["file_id"], "file-123");
    assert_eq!(got["current_version"], 3);
    assert_eq!(got["versions"][0]["id"], "object-version-3");
    assert_eq!(got["versions"][0]["object_version_id"], "object-version-3");
    assert_eq!(got["versions"][1]["id"], "file-version-2");
    assert_eq!(got["versions"][1]["file_version_id"], "file-version-2");
}
