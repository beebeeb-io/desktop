//! Per-process scratch directories for unit tests (compiled only into the lib test build).
//!
//! Some production code resolves a per-user location: the config dir (`desktop.toml`), the cache
//! dir (`finder-writes`), the macOS App Group `hydrate-cache`. A test that reaches that code,
//! directly or through a command it runs, would read, write or delete the person's REAL files,
//! and on a developer Mac the installed app uses the same ones. In a test build those resolvers
//! return a directory from here instead: `<test binary dir>/beebeeb-unit-<pid>/<name>`.
//!
//! The root is next to the test binary (under `target/`), which `cargo clean` removes. No test
//! depends on where that is (the `lib.rs` test that the real config is never under a disposable
//! root builds the real path by hand). The guard tests
//! `unit_tests_resolve_the_config_path_in_a_sandbox_never_the_real_one` (config),
//! `unit_tests_stage_finder_writes_in_a_sandbox_never_the_real_cache` (engine bridge) and
//! `unit_tests_resolve_the_hydrate_dir_in_a_sandbox_never_the_real_group_container` (IPC
//! socket) pin each resolver to it. Each compares against the SPECIFIC real directory, not its
//! parent, so a `CARGO_TARGET_DIR` inside the cache or config dir does not turn them red.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

/// Name prefix of the per-process root a unit-test run creates next to its binary.
pub(crate) const PREFIX: &str = "beebeeb-unit-";

/// How old a sandbox left by an earlier test process must be before the next run removes it.
const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// The directory `<test binary dir>/beebeeb-unit-<pid>/<name>`, created. Empty at the start of
/// every test process.
pub(crate) fn dir(name: &str) -> Result<PathBuf, String> {
    static ROOT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    let root = ROOT
        .get_or_init(|| {
            let exe = std::env::current_exe().map_err(|e| format!("test binary path: {e}"))?;
            let exe_dir = exe.parent().ok_or_else(|| "test binary has no directory".to_string())?;
            let root = exe_dir.join(format!("{PREFIX}{}", std::process::id()));
            // A directory left by an earlier process that had the same pid.
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).map_err(|e| format!("create test sandbox: {e}"))?;
            sweep_stale(exe_dir, &root, STALE_AFTER);
            Ok(root)
        })
        .clone()?;
    let dir = root.join(name);
    fs::create_dir_all(&dir).map_err(|e| format!("create test sandbox dir {name}: {e}"))?;
    Ok(dir)
}

/// Best-effort cleanup for the sandboxes earlier test runs left in `exe_dir` (one per process,
/// pid in the name): removes directories with the sandbox prefix, other than `keep`, that have
/// not been modified for `older_than`. Anything else in `exe_dir` is left alone, and a symlink
/// is never followed.
fn sweep_stale(exe_dir: &Path, keep: &Path, older_than: Duration) {
    let Ok(entries) = fs::read_dir(exe_dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let ours = entry.file_name().to_string_lossy().starts_with(PREFIX);
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > older_than);
        if ours && is_dir && stale && path != keep {
            let _ = fs::remove_dir_all(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_is_created_inside_this_process_root_next_to_the_test_binary() {
        let a = dir("alpha").expect("sandbox dir");
        let b = dir("beta").expect("sandbox dir");
        assert!(a.is_dir() && b.is_dir());
        assert_ne!(a, b);
        assert_eq!(a.parent(), b.parent(), "both live under the one per-process root");
        let root = a.parent().unwrap();
        assert!(root.file_name().unwrap().to_string_lossy().starts_with(PREFIX));
        let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
        assert_eq!(root.parent(), Some(exe_dir.as_path()));
        assert_eq!(dir("alpha").unwrap(), a, "asking twice gives the same dir");
    }

    #[cfg(unix)]
    #[test]
    fn the_sandbox_sweep_removes_only_old_sandboxes_and_never_the_current_one() {
        use std::time::SystemTime;

        let root = tempfile::tempdir().expect("temp dir");
        let make = |name: String, age: Duration| {
            let dir = root.path().join(name);
            fs::create_dir_all(dir.join("beebeeb")).unwrap();
            fs::write(dir.join("beebeeb").join("desktop.toml"), "x").unwrap();
            let when = SystemTime::now() - age;
            fs::File::open(&dir).unwrap().set_modified(when).unwrap();
            dir
        };
        let two_days = Duration::from_secs(2 * 24 * 60 * 60);
        let old = make(format!("{PREFIX}1"), two_days);
        let current = make(format!("{PREFIX}2"), two_days);
        let fresh = make(format!("{PREFIX}3"), Duration::from_secs(60));
        let unrelated = make("some-other-old-dir".to_string(), two_days);

        sweep_stale(root.path(), &current, STALE_AFTER);

        assert!(!old.exists(), "an old sandbox is removed");
        assert!(current.exists(), "the sandbox in use is never removed, however old");
        assert!(
            fresh.exists(),
            "a recent sandbox (another test process running now) is kept"
        );
        assert!(
            unrelated.exists(),
            "a directory without the sandbox prefix is never touched"
        );
    }
}
