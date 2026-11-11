//! `sync_2` engine tests, split out of `engine_bridge.rs`.

use super::*;
/// Item 1: the pure containment predicate `hydrate_file` uses to reject an
/// out-of-root destination before writing plaintext.
#[test]
fn hydrate_dest_is_allowed_covers_the_containment_matrix() {
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let root_path = root.path();
    let other_path = other.path();

    // An absolute path outside all allowed roots (its parent exists but is
    // NOT under `root`) — the ~/.ssh/authorized_keys style attack — is
    // rejected. `other` exists, so this hits the parent-containment branch.
    let outside = other_path.join("evil.txt");
    assert!(
        !hydrate_dest_is_allowed(&outside, &[root_path]),
        "an existing-parent path outside every root must be rejected"
    );

    // A path with `..` that resolves outside the root is rejected: the
    // parent (`root/..`) is not contained even though the file name is a
    // single component.
    let traversal = root_path.join("../evil.txt");
    assert!(
        !hydrate_dest_is_allowed(&traversal, &[root_path]),
        "a `..` traversal escaping the root must be rejected"
    );

    // The normal hydrate case: a not-yet-existing file directly under the
    // allowed root is accepted (parent contained + single-component name).
    let legit = root_path.join("newfile.bin");
    assert!(
        !legit.exists(),
        "precondition: destination must not exist yet for the common case"
    );
    assert!(
        hydrate_dest_is_allowed(&legit, &[root_path]),
        "a not-yet-existing file under the allowed root must be accepted"
    );

    // Accepted when the destination is under a SECOND allowed root even
    // though it is outside the first (ANY root passing is enough).
    let legit_second = other_path.join("under-second.bin");
    assert!(
        hydrate_dest_is_allowed(&legit_second, &[root_path, other_path]),
        "a destination under any allowed root must be accepted"
    );

    // An empty allow-list fails closed — never vacuously true.
    assert!(
        !hydrate_dest_is_allowed(&legit, &[]),
        "an empty allowed_roots slice must reject everything"
    );

    // A destination whose parent directory does not exist is rejected too
    // (canonicalization of a missing parent fails) — this is exactly the
    // construction the malicious IPC test below relies on.
    let missing_parent = root_path.join("does-not-exist-dir").join("x.txt");
    assert!(
        !hydrate_dest_is_allowed(&missing_parent, &[root_path]),
        "a path under a non-existent subdirectory must be rejected"
    );
}

/// Task 1670 round 4 (Codex P2 on PR #75): the pure predicate
/// `hydrate_file` uses to decide whether a just-written destination is a
/// macOS Finder handoff staging file (never registered as durable cache)
/// versus a real cache copy (registered, as always). Uses a throwaway
/// tempdir as the "hydrate dir" — never the real, singleton App-Group
/// container a live Beebeeb.app/Finder extension share on this same
/// machine.
#[test]
#[cfg(target_os = "macos")]
fn is_macos_finder_handoff_staging_path_detects_dest_under_hydrate_dir() {
    let hydrate_dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();

    let staged = hydrate_dir.path().join("file-id.abcd1234");
    std::fs::write(&staged, b"decrypted plaintext").unwrap();
    assert!(
        is_macos_finder_handoff_staging_path(&staged, hydrate_dir.path()),
        "a destination inside the hydrate dir must be recognized as Finder handoff staging"
    );

    let cached = elsewhere.path().join("file-id");
    std::fs::write(&cached, b"a real cached copy").unwrap();
    assert!(
        !is_macos_finder_handoff_staging_path(&cached, hydrate_dir.path()),
        "a destination outside the hydrate dir must NOT be treated as Finder handoff staging"
    );
}

/// Task 1670 round 4: exercises the REAL [`record_hydration_cache_state`]
/// — the exact function [`EngineBridge::hydrate_file`] calls — end to
/// end against a real `StateDb`: a macOS-hydrate-dir-shaped destination
/// gets NO `cache_path`/`cache_bytes` after "hydration" (a plain on-disk
/// write here, no network involved — the decrypt/download path is
/// already covered by the pre-existing hydration tests, this one is
/// purely about the cache-registration DECISION), so the storage summary
/// (`cache_bytes_by_effective_pin`, `state_db.rs`) does not count it.
/// `macos_hydrate_dir` is a throwaway tempdir passed in directly (the
/// same injection seam production uses to pass the REAL
/// `macos_hydrate_cache_dir()` — see that function's call site in
/// `hydrate_file`) — this test never touches the real, singleton
/// App-Group container.
#[test]
#[cfg(target_os = "macos")]
fn record_hydration_cache_state_skips_mark_cached_under_the_hydrate_dir() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_bridge_row(
        &bridge,
        TEST_FILE_ID,
        "/finder-peek.bin",
        None,
        FileStatus::CloudOnly,
        0,
    );

    let hydrate_dir = tempfile::tempdir().unwrap();
    let finder_dest = hydrate_dir.path().join("finder-peek.abcd1234");
    std::fs::write(&finder_dest, b"decrypted plaintext for a Finder peek").unwrap();

    record_hydration_cache_state(&bridge.db, TEST_FILE_ID, &finder_dest, hydrate_dir.path()).unwrap();

    let row = bridge
        .db
        .get_file_contract_state(TEST_FILE_ID)
        .unwrap()
        .expect("row must still exist");
    assert_eq!(
        row.cache_path, None,
        "a macOS Finder handoff staging destination must NOT be registered as cache_path"
    );
    assert_eq!(
        bridge.db.cache_bytes_by_effective_pin(false).unwrap(),
        0,
        "the storage summary must not count a Finder handoff staging file as cache usage"
    );
}

/// Task 1670 round 4: the same function, but `dest_path` is OUTSIDE the
/// hydrate dir — `mark_cached` must still run exactly as before this
/// round, and the file must still be counted in the storage summary.
/// Regression guard for the ordinary (non-macOS-Finder) hydration path,
/// which this round must not have touched.
#[test]
#[cfg(target_os = "macos")]
fn record_hydration_cache_state_marks_cached_outside_the_hydrate_dir() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_bridge_row(&bridge, TEST_FILE_ID, "/ordinary.bin", None, FileStatus::CloudOnly, 0);

    let hydrate_dir = tempfile::tempdir().unwrap();
    let ordinary_root = tempfile::tempdir().unwrap();
    let ordinary_dest = ordinary_root.path().join("ordinary.bin");
    let plaintext = b"an ordinary cached copy, not a Finder handoff";
    std::fs::write(&ordinary_dest, plaintext).unwrap();

    record_hydration_cache_state(&bridge.db, TEST_FILE_ID, &ordinary_dest, hydrate_dir.path()).unwrap();

    let row = bridge
        .db
        .get_file_contract_state(TEST_FILE_ID)
        .unwrap()
        .expect("row must still exist");
    assert_eq!(
        row.cache_path.as_deref(),
        Some(ordinary_dest.to_string_lossy().as_ref()),
        "an ordinary (non-hydrate-dir) destination must still be registered as cache_path"
    );
    assert_eq!(
        bridge.db.cache_bytes_by_effective_pin(false).unwrap(),
        plaintext.len() as i64,
        "an ordinary cached file must still be counted in the storage summary"
    );
}
