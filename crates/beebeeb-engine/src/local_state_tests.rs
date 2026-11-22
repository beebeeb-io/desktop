//! The deletion safety gate every purge asks (`is_disposable_cache_path`) and its roots: a path is disposable only
//! under one of the app's throw-away roots, never the lifecycle log, and the person's real settings file is under no
//! root. Ported from the old shell's `tests.rs`.

use super::*;
use crate::identity::REAL;
use std::path::{Path, PathBuf};

/// Port of `disposable_cache_path_check_accepts_temp_files_only`. A cache file in the temp dir is disposable for the
/// real app (its cache paths have always been recorded anywhere under it). Inside that same root the gate still
/// refuses the lifecycle log, a path that reaches the log through a link, and a path whose folder does not exist.
/// A unique file name: the temp dir is shared with every other test and process. The test build's narrower root is
/// pinned by `tests/test_identity.rs` (`the_purge_deletes_only_inside_the_identitys_folders`).
/// Mutants: drop the log check on the canonical path (the link case goes red, on unix); drop the first log check
/// (the plain log case goes red).
#[test]
fn disposable_cache_path_check_accepts_temp_files_only() {
    let temp = std::env::temp_dir();
    let cache_file = tempfile::Builder::new().prefix("bb_").tempfile_in(&temp).unwrap();
    assert!(
        is_disposable_cache_path(&REAL, cache_file.path()),
        "a cache file in the temp dir"
    );

    let dir = tempfile::tempdir_in(&temp).unwrap();
    let sibling = dir.path().join("bb_cached");
    std::fs::write(&sibling, b"cache").unwrap();
    assert!(
        is_disposable_cache_path(&REAL, &sibling),
        "control: a file in the same folder"
    );

    let logs = dir.path().join("Logs").join(REAL.logs_dir_name());
    std::fs::create_dir_all(&logs).unwrap();
    let log = logs.join("lifecycle.log");
    std::fs::write(&log, b"log").unwrap();
    assert!(
        !is_disposable_cache_path(&REAL, &log),
        "the lifecycle log, even under the temp dir"
    );

    #[cfg(unix)]
    {
        let link = dir.path().join("to-logs");
        std::os::unix::fs::symlink(&logs, &link).unwrap();
        assert!(
            !is_disposable_cache_path(&REAL, &link.join("lifecycle.log")),
            "the lifecycle log through a link"
        );
    }

    assert!(
        !is_disposable_cache_path(&REAL, &dir.path().join("missing").join("bb_cached")),
        "a path whose folder does not exist"
    );
}

/// A lifecycle log that is itself a link to a file inside a disposable root is still the lifecycle log: the plain path
/// names `Logs/<dir>/lifecycle.log`, but its canonical path lands in the root with no `Logs` part, so only the first
/// (plain-path) log check refuses it. Mutant: drop that first check (this goes red, on unix).
#[cfg(unix)]
#[test]
fn a_lifecycle_log_linked_into_a_disposable_root_is_never_disposable() {
    let temp = std::env::temp_dir();
    let root_dir = tempfile::tempdir_in(&temp).unwrap();
    let target = root_dir.path().join("bb_cached_log");
    std::fs::write(&target, b"log").unwrap();
    assert!(
        is_disposable_cache_path(&REAL, &target),
        "control: the link target is disposable on its own"
    );

    // The link lives outside the temp root, so the plain path is the only place the log folder shows.
    let home = tempfile::tempdir().unwrap();
    let logs = home.path().join("Logs").join(REAL.logs_dir_name());
    std::fs::create_dir_all(&logs).unwrap();
    let link = logs.join("lifecycle.log");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(
        !is_lifecycle_log_path(&REAL, &target.canonicalize().unwrap()),
        "control: the canonical path has no log folder in it"
    );
    assert!(
        !is_disposable_cache_path(&REAL, &link),
        "the lifecycle log linked into a disposable root"
    );
}

/// The disposable roots that would hold `config`, ignoring any root that is an ancestor of (or equal to) `home`: such
/// a root contains the whole home directory, so every home-relative path is "under" it by construction and it says
/// nothing about the config. That never happens for a real user, but it does for a test run whose `HOME` is a scratch
/// directory under the OS temp dir. Plain `Path::starts_with`, no filesystem access.
fn disposable_roots_holding(config: &Path, home: &Path, roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .filter(|root| !home.starts_with(root) && config.starts_with(root))
        .cloned()
        .collect()
}

/// Port of `a_disposable_root_above_home_is_ignored_but_one_inside_home_is_not`: the helper of the test below.
#[test]
fn a_disposable_root_above_home_is_ignored_but_one_inside_home_is_not() {
    let config = Path::new("/t/h/Library/Application Support/beebeeb/desktop.toml");
    let home = Path::new("/t/h");
    // HOME is a scratch dir under the temp dir: the temp dir itself says nothing.
    assert!(disposable_roots_holding(config, home, &[PathBuf::from("/t")]).is_empty());
    // ... and neither does a root that IS the home dir.
    assert!(disposable_roots_holding(config, home, &[PathBuf::from("/t/h")]).is_empty());
    // A root inside the home that holds the config is the real finding.
    let config_dir = PathBuf::from("/t/h/Library/Application Support");
    assert_eq!(
        disposable_roots_holding(config, home, std::slice::from_ref(&config_dir)),
        vec![config_dir]
    );
    // A root elsewhere that holds the config (a config dir outside HOME, for example by `XDG_CONFIG_HOME`) is still a
    // finding.
    let config = Path::new("/c/beebeeb/desktop.toml");
    assert_eq!(
        disposable_roots_holding(config, home, &[PathBuf::from("/c")]),
        vec![PathBuf::from("/c")]
    );
    // A root that does not hold the config is not.
    assert!(disposable_roots_holding(config, home, &[PathBuf::from("/other")]).is_empty());
}

/// Port of `the_real_config_path_is_never_under_a_disposable_cache_root` (the consumer of the helper above; the row
/// named only the helper's test). A sign-out purge deletes anything under a disposable root, so the person's REAL
/// `desktop.toml` must never sit under one. Built by hand, as `DesktopConfig::path_for` builds it in a release build,
/// because a test build resolves the config into a sandbox; compared with plain `Path::starts_with`, because
/// `is_disposable_cache_path` answers `false` whenever the parent folder is missing, which would pass this vacuously.
/// Mutant: add the config dir to `disposable_cache_roots_in`'s roots; this goes red.
#[test]
fn the_real_config_path_is_never_under_a_disposable_cache_root() {
    let real_config = dirs::config_dir()
        .expect("a user config dir exists on a dev machine or CI runner")
        .join(REAL.config_dir_name())
        .join("desktop.toml");
    let home = dirs::home_dir().expect("a home dir exists on a dev machine or CI runner");
    let roots = disposable_cache_roots(&REAL);
    assert!(!roots.is_empty(), "the roots under test must exist");
    let holding = disposable_roots_holding(&real_config, &home, &roots);
    assert!(
        holding.is_empty(),
        "the real config {real_config:?} is under the disposable root(s) {holding:?}: a sign-out purge would delete it"
    );
}

#[test]
fn a_corrupt_state_db_is_moved_aside_and_the_next_open_is_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("state.db");
    std::fs::write(&db, vec![0xABu8; 8192]).unwrap();
    let StateDbCheck::MovedAside(aside) = move_corrupt_state_db_aside(dir.path()).unwrap() else {
        panic!("a corrupt state.db is moved aside");
    };
    assert!(!db.exists() && aside.exists());
    assert_eq!(move_corrupt_state_db_aside(dir.path()).unwrap(), StateDbCheck::Sound);
    assert!(crate::state_db::StateDb::open(&db).is_ok(), "a fresh database opens");
}

#[test]
fn a_sound_or_missing_state_db_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(move_corrupt_state_db_aside(dir.path()).unwrap(), StateDbCheck::Sound);
    crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
    assert_eq!(move_corrupt_state_db_aside(dir.path()).unwrap(), StateDbCheck::Sound);
    assert!(dir.path().join("state.db").exists());
}

#[test]
fn a_state_db_that_is_a_directory_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("state.db")).unwrap();
    assert!(move_corrupt_state_db_aside(dir.path()).is_err());
    assert!(dir.path().join("state.db").is_dir());
}
