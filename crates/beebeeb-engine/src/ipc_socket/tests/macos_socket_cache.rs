//! macOS socket path, hydrate cache and IPC publication tests.

use super::*;

// ── Task 1524: macOS IPC socket path + non-panicking bind ──────────────

#[cfg(target_os = "macos")]
#[test]
fn test_macos_ipc_socket_path_is_inside_group_container_not_tmp() {
    // Regression pin for the actual bug: the daemon must resolve the
    // socket inside the sandbox-reachable shared App Group container,
    // never under /tmp or $XDG_RUNTIME_DIR (which the sandbox blocks
    // entirely, causing the original "Timed out waiting for the local
    // Beebeeb sync daemon" report).
    let home = std::path::Path::new("/Users/alice");
    let path = macos_ipc_socket_path_in(&crate::identity::REAL, home);

    assert!(
        path.starts_with(home.join("Library").join("Group Containers")),
        "must live inside the shared App Group container, got {path:?}"
    );
    assert!(
        path.to_str().unwrap().contains(crate::identity::REAL.macos_app_group()),
        "must be namespaced under the app's own App Group id, got {path:?}"
    );
    assert!(
        !path.starts_with("/tmp"),
        "must not fall back to /tmp under the sandbox, got {path:?}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_ipc_socket_path_fits_sun_path_length_budget() {
    // sockaddr_un.sun_path on macOS is 104 bytes INCLUDING the NUL
    // terminator (<sys/un.h>), so the usable path length is 103 bytes.
    // Exercise a realistically long macOS short username (20 chars,
    // longer than this dev machine's own "guuslangelaar" at 13, and
    // longer than macOS's own 20-char "short name" convention allows in
    // most default cases) to prove there is real headroom, not a value
    // that only happens to fit this one machine. Built by repetition
    // (not hand-counted) so the length is exact by construction.
    let username: String = "x".repeat(20);
    let home = std::path::PathBuf::from(format!("/Users/{username}"));
    let home = home.as_path();

    let path = macos_ipc_socket_path_in(&crate::identity::REAL, home);
    let byte_len = path.to_str().unwrap().len();
    assert!(
        byte_len < 104,
        "path must fit sockaddr_un.sun_path (104 bytes incl. NUL); \
             got a {byte_len}-byte path for a 20-char username: {path:?}"
    );
}

// ── Task 1670: macOS hydrate-cache dir (fetchContents helper error) ────

#[cfg(target_os = "macos")]
#[test]
fn test_macos_hydrate_cache_dir_is_inside_group_container_not_tmp() {
    // Regression pin for the actual bug: a hydration destination built
    // from THIS process's own `std::env::temp_dir()` can never match one
    // the File Provider extension builds from ITS OWN (different)
    // sandboxed temp dir — see `macos_hydrate_cache_dir`'s doc comment.
    // The shared App Group container is the one directory both sides can
    // actually agree on.
    let home = std::path::Path::new("/Users/alice");
    let path = macos_hydrate_cache_dir_in(&crate::identity::REAL, home);

    assert!(
        path.starts_with(home.join("Library").join("Group Containers")),
        "must live inside the shared App Group container, got {path:?}"
    );
    assert!(
        path.to_str().unwrap().contains(crate::identity::REAL.macos_app_group()),
        "must be namespaced under the app's own App Group id, got {path:?}"
    );
    assert!(
        !path.starts_with(std::env::temp_dir()),
        "must not be this process's own private sandboxed temp dir \
             (that is exactly the bug), got {path:?}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn unit_tests_resolve_the_hydrate_dir_in_a_sandbox_never_the_real_group_container() {
    // `macos_hydrate_cache_dir()` is what the hydrate handler and the thumbnail handler
    // (create the dir, then TTL-sweep it), the periodic sweep, and the sign-out / lock purge
    // (empties it) all resolve. In a test build it must be a per-process sandbox: the REAL one
    // is in the App Group container shared with the installed app, `getpwuid`-resolved so
    // `$HOME` cannot redirect it. Paths only; nothing on disk is read or touched. The real path is
    // built from the password database directly (not through `macos_real_home_dir`) and
    // compared as the specific `hydrate-cache` dir, so a `CARGO_TARGET_DIR` elsewhere in the
    // container cannot turn this red.
    let real_home = getpwuid_home_dir().expect("the password database knows this user");
    let real = macos_hydrate_cache_dir_in(&crate::identity::REAL, &real_home);
    let resolved = macos_hydrate_cache_dir(&crate::identity::REAL);

    assert_ne!(
        resolved, real,
        "unit tests resolved the REAL App Group hydrate-cache dir"
    );
    assert!(
        !resolved.starts_with(&real),
        "{resolved:?} is inside the real hydrate-cache dir {real:?}"
    );
    let exe_dir = std::env::current_exe()
        .expect("test binary path")
        .parent()
        .expect("exe dir")
        .to_path_buf();
    assert!(
        resolved.starts_with(&exe_dir),
        "{resolved:?} is not inside the test binary's directory {exe_dir:?}"
    );
    assert!(
        resolved.ends_with(
            std::path::Path::new("Group Containers")
                .join(crate::identity::REAL.macos_app_group())
                .join(MACOS_HYDRATE_CACHE_DIRNAME)
        ),
        "the sandbox keeps the production path shape, got {resolved:?}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_hydrate_cache_dir_does_not_collide_with_ipc_socket_path() {
    // Both live under the same App Group container by design (task
    // 1670's doc comment); they must still be distinct paths, or a
    // hydrated file could shadow (or be shadowed by) the IPC socket.
    let home = std::path::Path::new("/Users/alice");
    let socket = macos_ipc_socket_path_in(&crate::identity::REAL, home);
    let hydrate_dir = macos_hydrate_cache_dir_in(&crate::identity::REAL, home);

    assert_ne!(socket, hydrate_dir, "must not reuse the socket's own path");
    assert!(
        !hydrate_dir.starts_with(&socket) && !socket.starts_with(&hydrate_dir),
        "must not nest one inside the other: socket={socket:?} hydrate_dir={hydrate_dir:?}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_hydrate_cache_dir_is_a_real_allowed_root_for_hydration() {
    // End-to-end proof (within what a sandbox-free `cargo test` process
    // can exercise) that a destination built the way
    // `XPCBridge.hydrateDestinationURL(for:)` builds it — a file directly
    // inside the hydrate-cache dir — passes the SAME containment check
    // `hydrate_dest_is_allowed` (engine_bridge.rs) runs against the
    // `allowed_roots` this handler now includes it in.
    let dir = std::env::temp_dir().join(format!("bb-hydrate-cache-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("some-file-id");

    assert!(
        crate::engine_bridge::hydrate_dest_is_allowed(&dest, &[dir.as_path()]),
        "a destination directly inside the hydrate-cache dir must be an allowed hydration target"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── Task 1670 round 2: identifier validation, dir hardening, purge/TTL ──

#[cfg(target_os = "macos")]
#[test]
fn test_macos_validate_hydrate_item_identifier_accepts_a_real_uuid() {
    assert!(macos_validate_hydrate_item_identifier("3f9a2b7e-1234-4c1a-8f9a-abcdef012345").is_ok());
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_validate_hydrate_item_identifier_rejects_hostile_identifiers() {
    for hostile in [
        "",
        "..",
        "../../etc/passwd",
        "/etc/passwd",
        "a/b",
        "a\\b",
        "foo..bar",
        "trailing/",
    ] {
        assert!(
            macos_validate_hydrate_item_identifier(hostile).is_err(),
            "must reject hostile identifier {hostile:?}"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_ensure_hydrate_cache_dir_sets_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let parent = tempdir().unwrap();
    let dir = parent.path().join("hydrate-cache");
    // Precondition: the dir does not exist yet, AND (defense-in-depth
    // case) an existing dir with looser perms must still be forced down.
    macos_ensure_hydrate_cache_dir(&dir).expect("must create the dir");
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "hydrate-cache dir must be owner-only, got {mode:o}");

    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    macos_ensure_hydrate_cache_dir(&dir).expect("must be idempotent on an existing dir");
    let mode_after = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode_after, 0o700,
        "a pre-existing dir with looser perms must be forced back to 0o700, got {mode_after:o}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_exclude_from_backups_sets_the_time_machine_xattr() {
    use std::os::unix::ffi::OsStrExt;

    let dir = tempdir().unwrap();
    let staging = StagingDir::open(dir.path()).expect("a temp dir is a private directory");
    macos_exclude_from_backups(&staging).expect("setxattr must succeed on a writable dir");

    let attr_name = std::ffi::CString::new("com.apple.metadata:com_apple_backup_excludeItem").unwrap();
    let path_c = std::ffi::CString::new(dir.path().as_os_str().as_bytes()).unwrap();
    let mut buf = vec![0u8; 64];
    let n = unsafe {
        libc::getxattr(
            path_c.as_ptr(),
            attr_name.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
            0,
            0,
        )
    };
    assert!(
        n > 0,
        "the backup-exclude xattr must be readable back after setting it, got {n}"
    );
    assert_eq!(&buf[..n as usize], b"com.apple.backupd");
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_purge_hydrate_cache_dir_removes_all_entries_but_keeps_the_dir() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("staged-a.bin"), b"plaintext-a").unwrap();
    std::fs::write(dir.path().join("staged-b.bin"), b"plaintext-b").unwrap();
    std::fs::create_dir(dir.path().join("stray-subdir")).unwrap();
    std::fs::write(dir.path().join("stray-subdir").join("c.bin"), b"c").unwrap();

    let removed = macos_purge_hydrate_cache_dir(dir.path()).expect("purge must succeed");

    assert_eq!(
        removed, 3,
        "must report one removal per top-level entry (2 files + 1 dir)"
    );
    assert!(dir.path().exists(), "the hydrate-cache dir itself must remain");
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "every entry inside the dir must be gone"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_purge_hydrate_cache_dir_missing_dir_is_not_an_error() {
    let parent = tempdir().unwrap();
    let missing = parent.path().join("never-created");
    assert_eq!(macos_purge_hydrate_cache_dir(&missing).unwrap(), 0);
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_sweep_stale_hydrate_cache_entries_removes_old_keeps_fresh() {
    let dir = tempdir().unwrap();
    let stale = dir.path().join("stale.bin");
    let fresh = dir.path().join("fresh.bin");
    std::fs::write(&stale, b"old-plaintext").unwrap();
    std::fs::write(&fresh, b"new-plaintext").unwrap();

    let now = std::time::SystemTime::now();
    let ttl = std::time::Duration::from_secs(600);
    // Backdate only `stale`'s mtime past the TTL; `fresh` keeps its
    // just-written (now-ish) mtime.
    let old_mtime = now - std::time::Duration::from_secs(700);
    std::fs::File::open(&stale).unwrap().set_modified(old_mtime).unwrap();

    let removed = macos_sweep_stale_hydrate_cache_entries(dir.path(), ttl, now).expect("sweep must succeed");

    assert_eq!(removed, 1, "exactly the stale entry must be swept");
    assert!(!stale.exists(), "the stale entry must be removed");
    assert!(fresh.exists(), "the fresh entry must survive the sweep");
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_sweep_stale_hydrate_cache_entries_skips_in_progress_temp_files() {
    // Task 1670 round 3: `write_hydrated_plaintext` (`engine_bridge.rs`)
    // stages a hydration under a temp name shaped exactly like this
    // (`.{leaf}.{uuid}.part`) BEFORE it writes a single byte, then
    // publishes it with one atomic `renameat`. Give the temp-shaped entry
    // an mtime far past the TTL — if the sweep age-checked it like any
    // other file, this would (wrongly) remove it out from under a write
    // that could still be in flight. A real, already-published (non-temp)
    // stale entry alongside it proves the sweep is still doing real work,
    // not just skipping everything.
    let dir = tempdir().unwrap();
    let in_progress = dir.path().join(".real-file.9f8e7d6c-1234-4abc-9def-0123456789ab.part");
    let stale_final = dir.path().join("real-file");
    std::fs::write(&in_progress, b"partial-write-in-flight").unwrap();
    std::fs::write(&stale_final, b"already-published-plaintext").unwrap();

    let now = std::time::SystemTime::now();
    let ttl = std::time::Duration::from_secs(600);
    let old_mtime = now - std::time::Duration::from_secs(700);
    std::fs::File::open(&in_progress)
        .unwrap()
        .set_modified(old_mtime)
        .unwrap();
    std::fs::File::open(&stale_final)
        .unwrap()
        .set_modified(old_mtime)
        .unwrap();

    let removed = macos_sweep_stale_hydrate_cache_entries(dir.path(), ttl, now).expect("sweep must succeed");

    assert_eq!(removed, 1, "only the final (non-temp) stale entry must be swept");
    assert!(
        in_progress.exists(),
        "an in-progress temp-named entry must never be removed by the sweep, however old its mtime"
    );
    assert!(
        !stale_final.exists(),
        "the stale, already-published entry must still be removed"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_is_hydrate_cache_temp_name_matches_write_hydrated_plaintexts_shape() {
    assert!(macos_is_hydrate_cache_temp_name(std::ffi::OsStr::new(
        ".some-file-id.3f9a2b7e-1234-4c1a-8f9a-abcdef012345.part"
    )));
    // A final, already-published name must never be mistaken for a temp
    // name — this is exactly what `macos_sweep_stale_hydrate_cache_entries`
    // relies on to still age-check real entries.
    assert!(!macos_is_hydrate_cache_temp_name(std::ffi::OsStr::new(
        "some-file-id.3f9a2b7e"
    )));
    assert!(!macos_is_hydrate_cache_temp_name(std::ffi::OsStr::new(
        ".dotfile-without-the-part-suffix"
    )));
    assert!(!macos_is_hydrate_cache_temp_name(std::ffi::OsStr::new(
        "no-leading-dot.uuid.part"
    )));
}

/// Serializes every test that mutates the process-wide `$HOME` env var,
/// so `cargo test`'s default parallel test threads can't interleave two
/// mutations of the same global state.
#[cfg(target_os = "macos")]
static HOME_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(target_os = "macos")]
#[test]
fn test_macos_ipc_socket_path_ignores_sandboxed_home_env_var() {
    // Task 1524 follow-up regression pin: a sandboxed process has $HOME
    // rewritten to the app's container directory by macOS itself. This
    // proves `ipc_socket_path()` does NOT trust $HOME for that — it must
    // resolve the same real home regardless of what $HOME says, via the
    // password database (getpwuid_r), matching the Swift side's
    // `FileManager.homeDirectoryForCurrentUser`.
    let _guard = HOME_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let real_home = getpwuid_home_dir().expect("getpwuid_r must resolve a real home directory on this machine");
    let expected = macos_ipc_socket_path_in(&crate::identity::REAL, &real_home);

    let original_home = std::env::var_os("HOME");
    // A realistic sandboxed $HOME, per the task 1524 root-cause report:
    // "/Users/alice/Library/Containers/io.beebeeb.app/Data".
    let sandbox_home = "/Users/x/Library/Containers/io.beebeeb.app/Data";
    // SAFETY: mutation is serialized by HOME_ENV_LOCK above, and every
    // path out of this test (including panics, via catch_unwind below)
    // restores the original value before returning.
    unsafe { std::env::set_var("HOME", sandbox_home) };

    let result = std::panic::catch_unwind(|| ipc_socket_path(&crate::identity::REAL));

    // SAFETY: same guard as the set_var above.
    match &original_home {
        Some(v) => unsafe { std::env::set_var("HOME", v) },
        None => unsafe { std::env::remove_var("HOME") },
    }

    let path = result.unwrap_or_else(|e| std::panic::resume_unwind(e));

    assert_eq!(
        path, expected,
        "ipc_socket_path() must ignore a sandboxed $HOME and resolve the real home via \
             getpwuid_r instead, got {path:?}, expected {expected:?}"
    );
    assert!(
        !path.to_str().unwrap().contains("/Library/Containers/"),
        "must never resolve into the sandbox container path even when $HOME points there, \
             got {path:?}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_ipc_socket_path_fits_sun_path_on_this_machine() {
    // Complements test_macos_ipc_socket_path_fits_sun_path_budget's
    // synthetic 20-char username: this exercises the REAL
    // `ipc_socket_path()` (through getpwuid_home_dir()/the dirs
    // fallback) against whatever machine cargo test actually runs on,
    // so a regression that only shows up with this machine's real home
    // directory doesn't hide behind the synthetic case.
    let _guard = HOME_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let path = ipc_socket_path(&crate::identity::REAL);
    let byte_len = path.to_str().expect("path must be valid UTF-8").len();
    // `byte_len + 1 <= 104` (path bytes + NUL terminator, within the
    // 104-byte sun_path budget) rewritten as `byte_len < 104` for clippy
    // (int_plus_one) — same inequality, since both sides are integers.
    assert!(
        byte_len < 104,
        "ipc_socket_path() ({byte_len} bytes) + NUL terminator must fit \
             sockaddr_un.sun_path (104 bytes total) on this machine, got {path:?}"
    );
}

#[test]
fn test_bind_ipc_listener_returns_error_instead_of_panicking_on_bad_path() {
    // A path under a directory that does not exist: UnixListener::bind
    // must fail with a real OS error (ENOENT), not panic and not
    // silently succeed. This is the exact failure shape the sandbox
    // produced on macOS before this fix (bind refused, formerly hidden
    // behind `.expect()`).
    let bad_path = std::path::Path::new("/nonexistent-beebeeb-1524-dir/socket.sock");

    let result = std::panic::catch_unwind(|| bind_ipc_listener(bad_path));

    match result {
        Ok(Ok(_listener)) => panic!("binding under a nonexistent directory must not succeed"),
        Ok(Err(_io_error)) => {} // expected: a real error, not a panic
        Err(_) => panic!("bind_ipc_listener must return Err on a bind failure, not panic"),
    }
}

#[tokio::test]
async fn ipc_publication_failure_does_not_signal_readiness() {
    use std::sync::Arc;

    let dir = tempdir().unwrap();
    let db = Arc::new(crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap());
    let api = Arc::new(crate::api_client::ApiClient::new(
        "https://api.beebeeb.io".into(),
        "token".into(),
        [7u8; 32],
    ));
    let bridge = Arc::new(crate::engine_bridge::EngineBridge::new(
        &crate::identity::REAL,
        db.clone(),
        api,
    ));
    let (_cancel_tx, cancel_rx) = oneshot::channel();
    let (ready_tx, ready_rx) = oneshot::channel();
    let result = serve_ipc_at_with_ready(
        dir.path().join("missing/ipc.sock"),
        db,
        bridge,
        cancel_rx,
        Some(ready_tx),
        WriteContentsPolicy::AnyPath,
    )
    .await;
    assert!(result.is_err(), "startup must fail for a nonexistent parent");
    assert!(ready_rx.await.is_err(), "startup failure must not signal readiness");
}

// Exercise filesystem publication separately from the OS socket call. The
// real-socket test below also verifies that clients can reach the listener.
#[test]
fn ipc_publication_is_private_until_hardened() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = dir.path().join("ipc.sock");
    let mut staging_path = None;
    bind_ipc_listener_with(&path, |bound_path| {
        std::fs::write(bound_path, b"socket stand-in")?;
        // Force the permissive creation mode independently of the test umask.
        std::fs::set_permissions(bound_path, std::fs::Permissions::from_mode(0o755))?;
        assert!(!path.exists(), "endpoint must not be published before hardening");
        let parent = bound_path.parent().unwrap();
        assert_eq!(std::fs::metadata(parent)?.permissions().mode() & 0o777, 0o700);
        assert!(bound_path.as_os_str().len() <= path.as_os_str().len());
        staging_path = Some(parent.to_path_buf());
        Ok(())
    })
    .unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::read(&path).unwrap(), b"socket stand-in");
    assert!(!staging_path.unwrap().exists(), "staging directory must be cleaned");
}

#[test]
fn ipc_publication_fails_closed_when_hardening_fails() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ipc.sock");
    // A dangling symlink makes chmod fail while rename would succeed.
    // Ignoring the chmod error must therefore be caught by this test.
    let result = bind_ipc_listener_with(&path, |bound_path| {
        std::os::unix::fs::symlink(dir.path().join("missing"), bound_path)
    });
    assert!(result.is_err(), "chmod failure must prevent a ready listener");
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn ipc_publication_cleans_up_when_bind_or_rename_fails() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ipc.sock");
    let result = bind_ipc_listener_with::<()>(&path, |_| {
        Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
    });
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    std::fs::create_dir(&path).unwrap(); // a directory cannot be replaced by the socket
    let result = bind_ipc_listener_with(&path, |bound_path| std::fs::write(bound_path, b""));
    assert!(result.is_err());
    assert!(path.is_dir());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn test_bind_ipc_listener_succeeds_and_chmods_0600_on_a_valid_path() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("valid.sock");

    let listener = bind_ipc_listener(&sock_path).expect("bind must succeed on a valid, writable path");
    assert!(sock_path.exists(), "the socket file must exist after a successful bind");
    let mode = std::fs::metadata(&sock_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "the socket file must be chmod 0o600, got {mode:o}");

    // The synchronous return is the bind+harden completion barrier. Also
    // prove the published pathname reaches the listener after its rename.
    let client = UnixStream::connect(&sock_path).await.unwrap();
    let (_accepted, _) = tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
        .await
        .unwrap()
        .unwrap();
    drop(client);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    drop(listener);
}
