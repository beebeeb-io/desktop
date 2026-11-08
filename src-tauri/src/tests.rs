// The install-era Finder helpers exist on Windows and Linux only (spec 2026-10-06 §6.1); the
// tests that call them run there, as the Windows/Linux proof of their unchanged behaviour.
use super::{
    AppState, DesktopMenuAction, DesktopMenuView, MENU_CHECK_UPDATES_ID, MENU_HELP_DOCS_ID, MENU_HELP_REPORT_ID,
    MENU_HELP_SHORTCUTS_ID, MENU_HELP_STATUS_ID, MENU_NEW_FOLDER_ID, MENU_OPEN_FOLDER_ID, MENU_OPEN_WEB_APP_ID,
    MENU_PREFERENCES_ID, MENU_QUIT_ID, MENU_SETTINGS_ID, MENU_SIGN_OUT_ID, MENU_TOGGLE_SYNC_ID, MENU_UPLOAD_FILES_ID,
    MENU_VIEW_ACTIVITY_ID, MENU_VIEW_FILES_ID, MENU_VIEW_SHARED_ID, MENU_VIEW_TRASH_ID, MENU_ZOOM_IN_ID,
    MENU_ZOOM_OUT_ID, MENU_ZOOM_RESET_ID, ManualUpdateCheckResult, MenuPlatform, MenuZoomAction,
    UpdateAvailablePayload, VaultEntryRow, VaultItem, WINDOWS_MAIN_APP_VIEW_ROUTES, build_vault_tree,
    build_vault_tree_of_rows, clear_cached_profile, compact_menu_page_for_view, desktop_manifest_path_for_channel,
    desktop_menu_spec_required, desktop_menu_specs, disposable_cache_roots, file_versions_payload_for_frontend,
    folder_leaf_name, free_up_space_allowed_roots, installed_release_channel_from_config, is_disposable_cache_path,
    manual_update_available_result, manual_update_result_for_remote, manual_update_up_to_date_result, menu_accelerator,
    menu_view_nav_target, newly_excluded_ids, next_menu_zoom_scale, normalize_recovery_phrase_input, now_unix_seconds,
    purge_local_state_files, queued_restore_version_response, real_app_version, release_channel_from_version,
    release_notes_url_for_version, resolve_purge_placeholder_paths, shared_roots_from_db, should_offer_channel_update,
    should_show_conflict_notification, should_show_quota_warning_notification, should_show_sync_complete_notification,
    state_db_from_state_dir, subtree_file_ids, unused_child_path, vault_entry_row,
};
#[cfg(not(target_os = "macos"))]
use super::{
    FINDER_IPC_NOT_READY, FINDER_USER_DISABLED_MESSAGE, classify_finder_install_error, clear_finder_install_failure,
    finder_install_failure_state, finder_install_state_from_config, record_finder_install_result,
};
use crate::account_dto::AccountProfile;
use crate::config::DesktopConfig;
use crate::config::ReleaseChannel;
use std::collections::HashSet;

/// A server-confirmed folder at its own path (`/<id>`), so no two helpers' rows share one.
pub(super) fn folder(id: &str, parent: Option<&str>, name: &str) -> VaultEntryRow {
    VaultEntryRow {
        id: id.to_string(),
        parent_id: parent.map(str::to_string),
        is_folder: true,
        name: name.to_string(),
        size_bytes: 0,
        on_disk: false,
        path: format!("/{id}"),
        server_known: true,
        local_status: false,
    }
}

/// A server-confirmed file at its own path (`/<id>`).
pub(super) fn file(id: &str, parent: Option<&str>, size: i64, on_disk: bool) -> VaultEntryRow {
    VaultEntryRow {
        id: id.to_string(),
        parent_id: parent.map(str::to_string),
        is_folder: false,
        name: String::new(),
        size_bytes: size,
        on_disk,
        path: format!("/{id}"),
        server_known: true,
        local_status: on_disk,
    }
}

/// `row` at `path`, as a row the server knows (status not `local` unless the file is on disk) or as a row this PC
/// minted and never sent (status `local`).
pub(super) fn at(mut row: VaultEntryRow, path: &str, server_known: bool) -> VaultEntryRow {
    row.path = path.to_string();
    row.server_known = server_known;
    row.local_status = !server_known || row.on_disk;
    row
}

#[test]
fn clear_cached_profile_drops_stale_identity() {
    let state = AppState::default();
    // The cached profile is per-account (decision 0800) — synthesize the
    // single account so `clear_cached_profile`/`active_account` resolve.
    crate::account::synthesize_single_account(&state, crate::account::AccountId("test-acct".to_string()));
    let acct = state.active_account().expect("synthesized account resolves");
    // Seed a cached profile, as login would.
    {
        let mut guard = acct.cached_profile.lock().unwrap();
        *guard = Some(AccountProfile {
            user_id: "u1".into(),
            email: "old@example.com".into(),
            email_verified: true,
            created_at: "2026-01-01T00:00:00Z".into(),
            frozen_at: None,
            role: Some("user".into()),
            totp_enabled: Some(false),
            is_impersonation: Some(false),
            admin_user_id: None,
        });
    }
    assert!(acct.cached_profile.lock().unwrap().is_some());

    // Logout/lock path clears it so account_profile can't serve a stale hit.
    clear_cached_profile(&state);
    assert!(
        acct.cached_profile.lock().unwrap().is_none(),
        "cached profile must be cleared on lock/logout"
    );
}

/// Task 1538 findings 1+2: `purge_local_state_files` is the sign-out-safe
/// glue `clear_session_impl` calls — it must delete every queued
/// operation's staged plaintext payload AND every cached plaintext file
/// from disk (not just clear the DB rows pointing at them), and it must
/// refuse anything outside the known cache/staging roots (the same
/// `is_disposable_cache_path` gate `reset_macos_integration` already
/// uses), so a bug in a future caller can never turn this into an
/// arbitrary-file-delete.
#[test]
fn purge_local_state_files_removes_every_queued_payload_and_cached_file() {
    let db_dir = tempfile::tempdir().unwrap();
    let db = crate::state_db::StateDb::open(db_dir.path().join("state.db")).unwrap();

    // Real files under the OS temp dir so `is_disposable_cache_path`
    // (the actual safety gate, not a stub) allows their removal.
    let staging_root = std::env::temp_dir().join(format!("bb-test-1538-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&staging_root).unwrap();
    let payload_path = staging_root.join("op-1-payload.bin");
    let cache_path = staging_root.join("file-a-cache.bin");
    std::fs::write(&payload_path, b"account A staged plaintext upload").unwrap();
    std::fs::write(&cache_path, b"account A decrypted cache content").unwrap();

    db.upsert_file(&crate::state_db::FileEntry {
        file_id: "file-a".into(),
        path: "/A.txt".into(),
        status: crate::state_db::FileStatus::Local,
        size_bytes: 34,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: crate::state_db::ItemKind::File,
    })
    .unwrap();
    db.mark_cached("file-a", cache_path.to_str().unwrap(), 34, 10).unwrap();
    db.enqueue_operation(&crate::state_db::PendingOperation {
        op_id: "op-1".into(),
        kind: crate::state_db::OperationKind::UploadVersion,
        file_id: Some("file-a".into()),
        parent_id: None,
        target_path: Some("/A.txt".into()),
        metadata_json: None,
        payload_path: Some(payload_path.to_str().unwrap().to_string()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 100,
        updated_at: 100,
    })
    .unwrap();

    let summary = purge_local_state_files(&db, None, None).expect("purge succeeds");

    assert_eq!(summary.queued_ops_purged, 1);
    assert_eq!(
        summary.files_removed, 2,
        "both the staged payload and the cache file are removed"
    );
    assert_eq!(summary.files_skipped, 0);
    assert!(
        !payload_path.exists(),
        "staged plaintext payload must be deleted from disk"
    );
    assert!(!cache_path.exists(), "decrypted cache file must be deleted from disk");
    assert!(db.list_due_operations(i64::MAX).unwrap().is_empty());

    std::fs::remove_dir_all(&staging_root).ok();
}

/// F4 (fix round 1 of Task 10): a file the purge cannot remove is a purge failure, not a warning. The purge
/// stops with an error, removes no row (so a retry finds everything still there), and the retry succeeds once
/// the file can go. A directory standing where a payload should be is a removal that fails on every platform
/// and for every user, root included.
#[test]
fn a_file_that_cannot_be_removed_fails_the_purge_and_keeps_every_row() {
    let db_dir = tempfile::tempdir().unwrap();
    let db = crate::state_db::StateDb::open(db_dir.path().join("state.db")).unwrap();
    let root = std::env::temp_dir().join(format!("bb-test-1834-f4-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let removable = root.join("removable-payload.bin");
    let stuck = root.join("stuck-payload.bin");
    std::fs::write(&removable, b"plaintext a").unwrap();
    std::fs::create_dir(&stuck).unwrap(); // `remove_file` on a directory fails
    for (n, path) in [&removable, &stuck].into_iter().enumerate() {
        db.enqueue_operation(&crate::state_db::PendingOperation {
            op_id: format!("op-{n}"),
            kind: crate::state_db::OperationKind::UploadVersion,
            file_id: None,
            parent_id: None,
            target_path: Some(format!("/{n}.txt")),
            metadata_json: None,
            payload_path: Some(path.to_str().unwrap().to_string()),
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 1,
            updated_at: 1,
        })
        .unwrap();
    }
    db.set_owner(&crate::account_binding::Identity::new(Some("u-a"), None))
        .unwrap();

    let Err(error) = super::purge_local_state_files_in(&db, None, &[], None) else {
        panic!("a file that cannot go fails the purge");
    };
    assert!(error.contains("could not be removed"), "{error}");
    assert!(
        !error.contains(root.to_str().unwrap()),
        "the message names no path: {error}"
    );
    assert_eq!(
        db.list_due_operations(i64::MAX).unwrap().len(),
        2,
        "no row was removed: a retry finds everything"
    );
    assert_eq!(
        db.owner().unwrap(),
        Some(crate::account_binding::Identity::new(Some("u-a"), None))
    );

    std::fs::remove_dir(&stuck).unwrap(); // the obstacle goes away
    let summary = super::purge_local_state_files_in(&db, None, &[], None).expect("the retry succeeds");
    assert_eq!(summary.queued_ops_purged, 2);
    assert!(!removable.exists());
    assert!(db.list_due_operations(i64::MAX).unwrap().is_empty());
    std::fs::remove_dir_all(&root).ok();
}

/// F4: the purge also removes orphan staged files, plaintext copies no row points at any more,
/// from the staging directory as a whole, still only behind `is_disposable_cache_path`. Nested files go too.
#[test]
fn the_purge_sweeps_orphan_staged_files_behind_the_gate() {
    let db_dir = tempfile::tempdir().unwrap();
    let db = crate::state_db::StateDb::open(db_dir.path().join("state.db")).unwrap();
    let root = std::env::temp_dir().join(format!("bb-test-1834-f4-sweep-{}", uuid::Uuid::new_v4()));
    let staging = root.join("beebeeb").join("finder-writes");
    let nested = staging.join("op-9");
    std::fs::create_dir_all(&nested).unwrap();
    let (orphan, deep) = (staging.join("orphan.bin"), nested.join("deep.bin"));
    std::fs::write(&orphan, b"plaintext with no row").unwrap();
    std::fs::write(&deep, b"nested plaintext").unwrap();
    let outside = root.join("not-staging.bin");
    std::fs::write(&outside, b"not in the staging dir").unwrap();

    let summary =
        super::purge_local_state_files_in(&db, None, std::slice::from_ref(&staging), None).expect("purge succeeds");

    assert_eq!(summary.files_removed, 2, "both orphans are removed");
    assert!(!orphan.exists() && !deep.exists(), "no orphan survives");
    assert!(outside.exists(), "nothing outside the staging directory is touched");
    // A staging directory that does not exist is nothing to sweep, not an error.
    super::purge_local_state_files_in(&db, None, &[root.join("missing")], None).expect("a missing directory is fine");
    std::fs::remove_dir_all(&root).ok();
}

/// R5 (fix round 2): the sweep enters only staging directories THIS user owns, so on a shared `/tmp` another
/// local user's `beebeeb/finder-writes` can neither be read nor block a sign-out or a reset. A directory that is a
/// link, or that sits under a link, is not entered either (following one could leave the staging directory).
#[cfg(unix)]
#[test]
fn the_sweep_enters_only_staging_directories_this_user_owns() {
    use std::os::unix::fs::{MetadataExt, symlink};
    let root = std::env::temp_dir().join(format!("bb-test-r5-{}", uuid::Uuid::new_v4()));
    let staging = root.join("beebeeb").join("finder-writes");
    std::fs::create_dir_all(&staging).unwrap();
    let orphan = staging.join("orphan.bin");
    std::fs::write(&orphan, b"plaintext").unwrap();
    let me = std::fs::metadata(&staging).unwrap().uid();

    assert_eq!(
        super::orphan_staged_files(std::slice::from_ref(&staging), Some(me)),
        vec![orphan.clone()],
        "ours: swept"
    );
    assert!(
        super::orphan_staged_files(std::slice::from_ref(&staging), Some(me + 1)).is_empty(),
        "another user's: not entered"
    );

    let link_to_dir = root.join("link-to-staging");
    symlink(&staging, &link_to_dir).unwrap();
    assert!(
        super::orphan_staged_files(&[link_to_dir], Some(me)).is_empty(),
        "a link to a directory is not followed"
    );
    let linked_parent = root.join("beebeeb-link");
    symlink(root.join("beebeeb"), &linked_parent).unwrap();
    assert!(
        super::orphan_staged_files(&[linked_parent.join("finder-writes")], Some(me)).is_empty(),
        "a staging directory under a linked parent is not entered"
    );
    assert!(orphan.exists(), "nothing was deleted by looking");
    std::fs::remove_dir_all(&root).ok();
}

/// R5, through the purge: a staging directory that is not ours is left alone and does not make the purge fail.
#[cfg(unix)]
#[test]
fn another_users_staging_directory_is_left_alone_and_never_fails_a_purge() {
    use std::os::unix::fs::MetadataExt;
    let db_dir = tempfile::tempdir().unwrap();
    let db = crate::state_db::StateDb::open(db_dir.path().join("state.db")).unwrap();
    let root = std::env::temp_dir().join(format!("bb-test-r5-purge-{}", uuid::Uuid::new_v4()));
    let staging = root.join("beebeeb").join("finder-writes");
    std::fs::create_dir_all(&staging).unwrap();
    let foreign_file = staging.join("someone-elses.bin");
    std::fs::write(&foreign_file, b"not ours").unwrap();
    let me = std::fs::metadata(&staging).unwrap().uid();

    let summary = super::purge_local_state_files_as(&db, None, std::slice::from_ref(&staging), Some(me + 1), None)
        .expect("the purge succeeds");
    assert_eq!((summary.files_removed, summary.files_skipped), (0, 0));
    assert!(foreign_file.exists(), "a directory that is not ours is not swept");
    let summary = super::purge_local_state_files_as(&db, None, std::slice::from_ref(&staging), Some(me), None)
        .expect("the purge succeeds");
    assert_eq!(summary.files_removed, 1, "ours is");
    assert!(!foreign_file.exists());
    std::fs::remove_dir_all(&root).ok();
}

/// A path outside the known cache/staging roots is refused, not deleted —
/// the same fail-closed behavior `reset_macos_integration` already relies
/// on via `is_disposable_cache_path`. Uses a REAL file that exists but
/// sits outside the OS temp/cache dirs (this crate's own source tree),
/// so the assertion proves the gate is consulted — not just that a
/// nonexistent path happens to be skipped.
#[test]
fn purge_local_state_files_refuses_paths_outside_known_roots() {
    struct CleanupDir(std::path::PathBuf);
    impl Drop for CleanupDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let db_dir = tempfile::tempdir().unwrap();
    let db = crate::state_db::StateDb::open(db_dir.path().join("state.db")).unwrap();

    let outside_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!(".test-scratch-1538-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&outside_root).unwrap();
    let _cleanup = CleanupDir(outside_root.clone());
    let outside_path = outside_root.join("not-a-cache-root.bin");
    std::fs::write(&outside_path, b"must survive").unwrap();

    db.upsert_file(&crate::state_db::FileEntry {
        file_id: "file-b".into(),
        path: "/B.txt".into(),
        status: crate::state_db::FileStatus::Local,
        size_bytes: 12,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: crate::state_db::ItemKind::File,
    })
    .unwrap();
    db.mark_cached("file-b", outside_path.to_str().unwrap(), 12, 10)
        .unwrap();

    let summary = purge_local_state_files(&db, None, None).expect("purge succeeds");

    assert_eq!(summary.files_removed, 0);
    assert_eq!(summary.files_skipped, 1);
    assert!(
        outside_path.exists(),
        "a path outside the known roots must never be deleted"
    );
}

#[test]
fn queued_restore_version_response_reports_durable_operation() {
    let op = crate::state_db::PendingOperation {
        op_id: "op-restore-1".to_string(),
        kind: crate::state_db::OperationKind::RestoreVersion,
        file_id: Some("file-123".to_string()),
        parent_id: None,
        target_path: None,
        metadata_json: Some(
            serde_json::json!({
                "operation": "restore_version",
                "version_id": "version-2"
            })
            .to_string(),
        ),
        payload_path: None,
        base_version: None,
        base_object_version_id: Some("version-2".to_string()),
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 123,
        last_error: None,
        backup_source_key: None,
        created_at: 123,
        updated_at: 123,
    };

    let got = queued_restore_version_response("file-123", "version-2", &op);

    assert_eq!(got["status"], "queued");
    assert_eq!(got["file_id"], "file-123");
    assert_eq!(got["version_id"], "version-2");
    assert_eq!(got["operation_id"], "op-restore-1");
    assert_eq!(got["operation"], "restore_version");
}

#[test]
fn normalizes_recovery_phrase_in_rust() {
    let input = "Alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima";
    let normalized = normalize_recovery_phrase_input(input).expect("phrase should normalize");

    assert_eq!(
        normalized,
        "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima"
    );
}

#[test]
fn rejects_non_twelve_word_recovery_phrase() {
    let error = normalize_recovery_phrase_input("alpha bravo charlie").expect_err("phrase should be incomplete");

    assert_eq!(error, "Recovery phrase must contain exactly 12 words.");
}

#[cfg(not(target_os = "macos"))]
#[test]
fn classifies_file_provider_setup_errors() {
    assert_eq!(
        classify_finder_install_error("Entitlement com.apple.security.application-groups is ignored"),
        "provisioning"
    );
    assert_eq!(
        classify_finder_install_error("NSFileProviderErrorDomain Code=-2011"),
        "disabled"
    );
    assert_eq!(
        classify_finder_install_error("Timed out waiting for the Beebeeb File Provider domain"),
        "timeout"
    );
    // The sentence the sync-socket wait fails with now reads without either word (PR #140 review, P2-8).
    assert_eq!(classify_finder_install_error(FINDER_IPC_NOT_READY), "timeout");
    assert_eq!(
        classify_finder_install_error(
            "Timed out waiting for the local Beebeeb sync daemon before installing Finder location."
        ),
        "timeout",
        "a failure saved before the rewording keeps its category"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn classifies_user_disabled_domain_distinctly_from_the_generic_disabled_case() {
    // Task 1524 Issue 4: the crafted "turned off in System Settings" copy must
    // classify as "user_disabled", never falling through to the older generic
    // "disabled" NSError-code branch (which stays keyed on "-2011"/"disabled").
    assert_eq!(
        classify_finder_install_error(FINDER_USER_DISABLED_MESSAGE),
        "user_disabled"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn maps_persisted_file_provider_error_into_state() {
    let cfg = DesktopConfig {
        finder_install_status: Some("error".to_string()),
        finder_install_last_error: Some("Timed out waiting for setup".to_string()),
        finder_install_last_attempt_at: Some(42),
        finder_install_reason_category: Some("timeout".to_string()),
        ..DesktopConfig::default()
    };

    let state = finder_install_state_from_config(&cfg, false, None);

    assert!(!state.installed);
    assert_eq!(state.status, "error");
    assert_eq!(state.last_error.as_deref(), Some("Timed out waiting for setup"));
    assert_eq!(state.last_attempt_at, Some(42));
    assert_eq!(state.reason_category.as_deref(), Some("timeout"));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn failed_finder_install_is_returned_as_the_saved_state_not_as_an_error() {
    // Task 1683 slice 5 / decision D1 (screenshot 1): the failure used to be saved AND
    // returned as `Err`, so the UI painted a banner from the saved copy and a toast from the
    // returned one. Now the failure is returned as a state, and that state IS the saved one.
    const TIMEOUT: &str = "Timed out waiting for the Beebeeb File Provider domain to become available";
    let mut cfg = DesktopConfig::default();

    let returned = finder_install_failure_state(&mut cfg, TIMEOUT.to_string());

    assert!(!returned.installed);
    assert_eq!(returned.status, "error");
    assert_eq!(returned.last_error.as_deref(), Some(TIMEOUT));
    assert_eq!(returned.reason_category.as_deref(), Some("timeout"));
    assert!(returned.last_attempt_at.is_some(), "the attempt time is stamped");
    // What was returned is exactly what `finder_location_state` reads back from the saved config.
    assert_eq!(cfg.finder_install_last_error.as_deref(), Some(TIMEOUT));
    let read_back = finder_install_state_from_config(&cfg, false, None);
    assert_eq!(read_back.last_error, returned.last_error);
    assert_eq!(read_back.status, returned.status);
    assert_eq!(read_back.reason_category, returned.reason_category);
    assert_eq!(read_back.last_attempt_at, returned.last_attempt_at);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn a_later_successful_finder_install_clears_the_saved_failure() {
    let mut cfg = DesktopConfig::default();
    let _ = finder_install_failure_state(&mut cfg, "boom".to_string());
    assert_eq!(cfg.finder_install_status.as_deref(), Some("error"));

    record_finder_install_result(&mut cfg, true, None);

    let state = finder_install_state_from_config(&cfg, true, None);
    assert!(state.installed);
    assert_eq!(state.last_error, None);
    assert_eq!(state.reason_category, None);
    assert_eq!(cfg.finder_install_last_error, None);
}

/// The text of `install_finder_location`, for the structural wiring tests below.
fn install_finder_location_source() -> String {
    // Windows checkouts can carry CRLF line endings (CI run 36772299380 failed on exactly
    // that): normalise, so the multi-line shape check below is about code, not about EOLs.
    let source = crate::root_source::lib_rs().replace("\r\n", "\n");
    let start = source
        .find("async fn install_finder_location(")
        .expect("install_finder_location exists");
    let end = start
        + source[start..]
            .find("async fn continue_without_finder_location(")
            .expect("next command exists");
    source[start..end].to_string()
}

#[test]
fn install_finder_location_never_saves_a_failure_and_also_returns_it_as_an_error() {
    // Structural wiring check on the command itself (the helper tests above cannot see which
    // arm calls what). The invariant is "a failure is saved OR returned as an error, never
    // both", so it is asserted on SHAPE, not on variable names: once the config has been
    // loaded (the first point where anything can be saved), the command has no way to
    // return `Err` by hand, and the only hand-written save of a failure is the user-disabled
    // state, which is returned as `Ok`.
    let body = install_finder_location_source();
    let after_load = &body[body.find("HeldConfig::load()").expect("the command loads the config")..];

    // 1. No hand-written Err after the config is loaded: neither `return Err(..)` nor an
    //    `=> Err(..)` match arm. (`?` before the load is fine: nothing was saved yet.)
    assert_eq!(
        after_load.matches("return Err(").count(),
        0,
        "a failure after the load must not be returned as Err"
    );
    assert_eq!(
        after_load.matches("=> Err(").count(),
        0,
        "a match arm must not evaluate to Err after the load"
    );

    // 2. Every failure goes through the D1 helper, and always as the returned value.
    let helper_calls = after_load.matches("finder_install_failed(").count();
    assert_eq!(helper_calls, 2, "both failure arms use the D1 helper");
    assert_eq!(
        after_load.matches("return finder_install_failed(").count(),
        helper_calls,
        "the helper's result is what the command returns"
    );

    // 3. The only hand-written failure save is the user-disabled state, and its result is an Ok.
    let hand_saves = after_load
        .matches("persist_finder_install_result(&mut cfg, false")
        .count();
    assert_eq!(
            after_load
                .matches("persist_finder_install_result(&mut cfg, false, Some(FINDER_USER_DISABLED_MESSAGE.to_string()))?;\n            return Ok(")
                .count(),
            hand_saves,
            "a hand-written failure save must be the user-disabled state followed by `return Ok(`"
        );

    // 4. The in-memory recorder is an implementation detail of the helpers, never called here.
    assert_eq!(after_load.matches("record_finder_install_result(").count(), 0);
}

#[test]
fn install_finder_location_clears_the_previous_failure_before_the_new_attempt_starts() {
    // Spec section 7: the saved error "is cleared when a new attempt starts", so the status
    // page and the popover never show the last attempt's error as the current one while the
    // new attempt waits on the File Provider domain. Order matters, so assert the ORDER.
    let body = install_finder_location_source();
    let clear = body
        .find("begin_finder_install_attempt(&mut cfg)?")
        .expect("the attempt clears the old failure");
    let load = body.find("HeldConfig::load()").expect("loads the config");
    let engine = body
        .find("start_engine_for_pending_finder_install(")
        .expect("starts the pending engine");
    let domain = body
        .find("install_file_provider_domain_blocking().await")
        .expect("installs the domain");
    assert!(
        load < clear && clear < engine && engine < domain,
        "load, then clear, then the slow work"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn starting_an_attempt_clears_the_saved_failure_but_not_an_installed_state() {
    let mut cfg = DesktopConfig::default();
    let _ = finder_install_failure_state(&mut cfg, "Timed out waiting for the domain".to_string());
    assert_eq!(cfg.finder_install_status.as_deref(), Some("error"));

    assert!(clear_finder_install_failure(&mut cfg), "a saved failure is a change");

    let state = finder_install_state_from_config(&cfg, false, None);
    assert_eq!(state.status, "missing");
    assert_eq!(state.last_error, None);
    assert_eq!(state.reason_category, None);
    assert!(
        state.last_attempt_at.is_some(),
        "the previous attempt time is history, not an error"
    );
    assert!(
        !clear_finder_install_failure(&mut cfg),
        "nothing left to clear: no change, no write"
    );

    let mut installed = DesktopConfig::default();
    record_finder_install_result(&mut installed, true, None);
    assert!(!clear_finder_install_failure(&mut installed));
    assert_eq!(installed.finder_install_status.as_deref(), Some("installed"));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn fresh_user_disabled_runtime_result_beats_a_stale_persisted_timeout() {
    // Task 1524 Issue 4, status-path leg (PR #63 Codex review, Onboarding.tsx:488):
    // before the status-path fix, a remounted card that persisted a "timeout"
    // category from an OLD attempt would keep showing that stale timeout even
    // once a FRESH check correctly detected the domain is user-disabled. A fresh
    // runtime_error must win.
    let cfg = DesktopConfig {
        finder_install_status: Some("error".to_string()),
        finder_install_last_error: Some(
            "Timed out waiting for the Beebeeb File Provider domain to become available".to_string(),
        ),
        finder_install_last_attempt_at: Some(1),
        finder_install_reason_category: Some("timeout".to_string()),
        ..DesktopConfig::default()
    };

    let state = finder_install_state_from_config(&cfg, false, Some(FINDER_USER_DISABLED_MESSAGE.to_string()));

    assert!(!state.installed);
    assert_eq!(state.reason_category.as_deref(), Some("user_disabled"));
    assert_eq!(state.last_error.as_deref(), Some(FINDER_USER_DISABLED_MESSAGE));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn fresh_genuine_timeout_beats_a_stale_persisted_user_disabled_state() {
    // Reverse of the above: once the user has re-enabled the domain in System
    // Settings and a later attempt hits a genuine, unrelated stabilization
    // timeout, that fresh timeout must win over the old persisted
    // "user_disabled" category -- it must NOT keep showing the stale
    // disabled-domain card / System Settings button for a condition that no
    // longer applies.
    let cfg = DesktopConfig {
        finder_install_status: Some("error".to_string()),
        finder_install_last_error: Some(FINDER_USER_DISABLED_MESSAGE.to_string()),
        finder_install_last_attempt_at: Some(1),
        finder_install_reason_category: Some("user_disabled".to_string()),
        ..DesktopConfig::default()
    };

    let state = finder_install_state_from_config(
        &cfg,
        false,
        Some("Timed out waiting for the Beebeeb File Provider domain to become available".to_string()),
    );

    assert!(!state.installed);
    assert_eq!(state.reason_category.as_deref(), Some("timeout"));
    assert_eq!(
        state.last_error.as_deref(),
        Some("Timed out waiting for the Beebeeb File Provider domain to become available")
    );
}

#[test]
fn session_zeroizes_master_key_on_drop() {
    use zeroize::Zeroize;

    // Compile-time guarantee: `Session` derives `ZeroizeOnDrop` (which
    // requires `Drop`). If the derive is ever removed this stops compiling.
    fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}
    assert_zeroize_on_drop::<super::Session>();

    // Sanity-check the field types the derive relies on are themselves
    // `Zeroize` — the property the `ZeroizeOnDrop` derive expands to per
    // field. `[u8; 32]`, `String`, and `Option<String>` all qualify.
    let mut key = [7u8; 32];
    key.zeroize();
    assert_eq!(key, [0u8; 32]);
    let mut token = String::from("super-secret-session-token");
    token.zeroize();
    assert!(token.is_empty());

    // Exercise the real drop path (no panic / double-free) on a populated
    // session; the wipe itself runs inside `Drop` where the bytes are no
    // longer observably aliased.
    let session = super::Session {
        token: "tok".to_string(),
        master_key: [9u8; 32],
        email: Some("user@example.com".to_string()),
    };
    drop(session);
}

#[test]
fn tray_recent_activity_from_db_returns_more_than_eight_rows() {
    use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};

    let dir = tempfile::tempdir().expect("tempdir");
    let db = StateDb::open(dir.path().join("state.db")).expect("state db");

    for i in 0..12 {
        db.upsert_file(&FileEntry {
            file_id: format!("local-{i}"),
            path: format!("Batch/file-{i}.txt"),
            status: FileStatus::Local,
            size_bytes: 100 + i,
            modified_at: 1_700_000_000 + i,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .expect("seed local file");
    }

    let items = super::tray_recent_activity_from_db(&db);

    assert_eq!(items.len(), 12);
    assert_eq!(items[0].name, "file-11.txt");
    assert_eq!(items[11].name, "file-0.txt");
}

#[test]
fn list_shared_roots_reads_local_shared_with_me_root_rows() {
    use crate::state_db::{FileEntry, FileStatus, ItemKind, Namespace, PERMISSION_READ, PERMISSION_WRITE, StateDb};

    let dir = tempfile::tempdir().expect("tempdir");
    let db = StateDb::open(dir.path().join("state.db")).expect("state db");

    db.upsert_file(&FileEntry {
        file_id: "root-folder".into(),
        path: "Shared with me/Quarterly planning".into(),
        status: FileStatus::CloudOnly,
        size_bytes: 0,
        modified_at: 1_700_000_000,
        content_hash: None,
        remote_updated_at: 1_700_000_000,
        parent_id: None,
        item_kind: ItemKind::Folder,
    })
    .expect("seed shared root");
    let mut root_contract = db
        .get_file_contract_state("root-folder")
        .expect("contract")
        .expect("root contract");
    root_contract.namespace = Namespace::SharedWithMe;
    root_contract.shared_root_id = Some("root-folder".into());
    root_contract.share_id = Some("invite-folder".into());
    root_contract.permission_bits = PERMISSION_READ | PERMISSION_WRITE;
    root_contract.item_kind = ItemKind::Folder;
    root_contract.owner_email = Some("owner@example.com".into());
    db.set_file_contract_state(&root_contract).expect("update contract");

    db.upsert_file(&FileEntry {
        file_id: "child-file".into(),
        path: "Shared with me/Quarterly planning/notes.txt".into(),
        status: FileStatus::CloudOnly,
        size_bytes: 12,
        modified_at: 1_700_000_001,
        content_hash: None,
        remote_updated_at: 1_700_000_001,
        parent_id: Some("root-folder".into()),
        item_kind: ItemKind::File,
    })
    .expect("seed child");
    let mut child_contract = db
        .get_file_contract_state("child-file")
        .expect("contract")
        .expect("child contract");
    child_contract.namespace = Namespace::SharedWithMe;
    child_contract.parent_id = Some("root-folder".into());
    child_contract.shared_root_id = Some("root-folder".into());
    child_contract.share_id = Some("invite-folder".into());
    child_contract.permission_bits = PERMISSION_READ;
    child_contract.item_kind = ItemKind::File;
    child_contract.owner_email = Some("owner@example.com".into());
    db.set_file_contract_state(&child_contract)
        .expect("update child contract");

    let roots = shared_roots_from_db(&db, None).expect("list shared roots");

    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].id, "root-folder");
    assert_eq!(roots[0].name, "Quarterly planning");
    assert_eq!(roots[0].owner_email.as_deref(), Some("owner@example.com"));
    assert_eq!(roots[0].permission, "write");
    assert_eq!(roots[0].kind, "folder");
}

#[test]
fn macos_tray_flyout_geometry_matches_configured_window() {
    assert_eq!(super::MACOS_SETTINGS_WINDOW_WIDTH, 680.0);
    assert_eq!(super::MACOS_SETTINGS_WINDOW_HEIGHT, 540.0);
}

// The anchored Settings window's placement math and its five tests retired with that window (task 1683 slice
// 1, two-surface spec §2: "The anchored Settings placement (slice 6) retires"). The Beebeeb window's placement is
// `surfaces::window_state` and its tests. The tray flyout's anchor math (`macos_tray_flyout_anchor` and its
// position, monitor and scale helpers) and its nine tests retired with the menu-bar popover (review P3-7 of #160):
// the tray click opens the popover, placed by `surfaces::anchor`.

#[test]
fn macos_window_mode_is_popover_when_anchor_present() {
    // An anchor handed off through `MACOS_TRAY_FLYOUT_ANCHOR` (the tray
    // click set one before the menu-bar popover replaced it; nothing sets
    // it now) — the window opens as a chrome-less popover.
    let anchor = Some(tauri::PhysicalPosition::new(1_704, 34));
    assert_eq!(
        super::macos_window_mode_for_anchor(anchor),
        super::MacosWindowMode::Popover
    );
}

#[test]
fn macos_window_mode_is_regular_when_anchor_absent() {
    // Every non-tray caller ("Open Beebeeb", Preferences/Settings menu
    // items, a notification, a deep link) never sets an anchor — the
    // window must open as a regular decorated, centered window.
    assert_eq!(
        super::macos_window_mode_for_anchor(None),
        super::MacosWindowMode::Regular
    );
}

#[test]
fn disposable_cache_path_check_accepts_temp_files_only() {
    let temp_file = std::env::temp_dir().join("beebeeb-disposable-cache-test");
    std::fs::write(&temp_file, b"cache").unwrap();
    assert!(is_disposable_cache_path(&temp_file));
    let _ = std::fs::remove_file(&temp_file);
}

use std::path::{Path, PathBuf};

/// The disposable roots that would hold `config`, ignoring any root that is an ancestor of (or
/// equal to) `home`: such a root contains the whole home directory, so every home-relative
/// path is "under" it by construction and it says nothing about the config. That never
/// happens for a real user, but it does for a test run whose `HOME` is a scratch directory
/// under the OS temp dir. Plain `Path::starts_with`, no filesystem access.
fn disposable_roots_holding(config: &Path, home: &Path, roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .filter(|root| !home.starts_with(root) && config.starts_with(root))
        .cloned()
        .collect()
}

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
    // A root elsewhere that holds the config (config dir outside HOME, for example by
    // `XDG_CONFIG_HOME`) is still a finding.
    let config = Path::new("/c/beebeeb/desktop.toml");
    assert_eq!(
        disposable_roots_holding(config, home, &[PathBuf::from("/c")]),
        vec![PathBuf::from("/c")]
    );
    // A root that does not hold the config is not.
    assert!(disposable_roots_holding(config, home, &[PathBuf::from("/other")]).is_empty());
}

/// The sign-out purge deletes anything under a disposable root, so the person's REAL
/// `desktop.toml` must never sit under one. Built by hand and compared with plain
/// `Path::starts_with`, with no filesystem access, because neither helper can carry this:
/// `DesktopConfig::path()` resolves a test sandbox in a test build (so it is independent of
/// where `CARGO_TARGET_DIR` points), and `is_disposable_cache_path` canonicalizes and answers
/// `false` whenever the parent directory is missing, which would pass this vacuously. A root
/// that contains the whole home directory is skipped (see `disposable_roots_holding`), so a
/// scratch `HOME` under the temp dir does not turn it red.
#[test]
fn the_real_config_path_is_never_under_a_disposable_cache_root() {
    let real_config = dirs::config_dir()
        .expect("a user config dir exists on a dev machine or CI runner")
        .join("beebeeb/desktop.toml");
    let home = dirs::home_dir().expect("a home dir exists on a dev machine or CI runner");
    let roots = disposable_cache_roots();
    assert!(!roots.is_empty(), "the roots under test must exist");
    let holding = disposable_roots_holding(&real_config, &home, &roots);
    assert!(
        holding.is_empty(),
        "the real config {real_config:?} is under the disposable root(s) {holding:?}: a sign-out purge would delete it"
    );
}

/// Task 1670 round 2, Codex P1 (PR #75): a macOS hydrate-cache
/// `cache_path` — what `hydrate_file`'s `mark_cached` records for a
/// Finder-triggered `fetchContents` — must be treated as disposable by
/// the SAME general-purpose gate `purge_local_state_files` (sign-out)
/// checks every candidate path against. Before this fix, staged
/// plaintext left on disk would have been silently skipped by sign-out's
/// purge (it lives under neither `temp_dir()` nor `dirs::cache_dir()`).
///
/// Deliberately read-only / no filesystem mutation: `macos_hydrate_cache_dir()`
/// resolves the REAL, shared App Group directory (the same one a real
/// running Beebeeb.app would use — round 1/2's restrictions rule out
/// writing test fixtures there), so this pins the fix at the pure
/// `disposable_cache_roots()` list level instead of exercising
/// `is_disposable_cache_path` against a real file on disk.
#[cfg(target_os = "macos")]
#[test]
fn disposable_cache_roots_includes_the_macos_hydrate_cache_dir() {
    let roots = disposable_cache_roots();
    let hydrate_dir = crate::ipc_socket::macos_hydrate_cache_dir();
    assert!(
        roots.contains(&hydrate_dir),
        "disposable_cache_roots must include the macOS hydrate-cache dir so sign-out's \
             purge gate doesn't skip staged plaintext there, got {roots:?}"
    );
}

/// Same reasoning as the test above, for the smart-cache "free up space"
/// eviction path's own allowed-roots list. Read-only for the same
/// real-shared-directory reason.
#[cfg(target_os = "macos")]
#[test]
fn free_up_space_allowed_roots_includes_the_macos_hydrate_cache_dir() {
    let sync_root = std::env::temp_dir();
    let roots = free_up_space_allowed_roots(&sync_root);
    let hydrate_dir = crate::ipc_socket::macos_hydrate_cache_dir();
    let expected = hydrate_dir.canonicalize().unwrap_or_else(|_| hydrate_dir.clone());
    assert!(
        roots.contains(&expected),
        "free_up_space_allowed_roots must include the macOS hydrate-cache dir, got {roots:?}"
    );
}

#[test]
fn conflict_notification_gate_honors_disabled_preference() {
    let cfg = DesktopConfig {
        notify_conflicts: false,
        ..DesktopConfig::default()
    };

    assert!(!should_show_conflict_notification(Ok(cfg)));
}

#[test]
fn conflict_notification_gate_fails_open_when_config_unreadable() {
    assert!(should_show_conflict_notification(Err("config unreadable".to_string())));
}

#[test]
fn sync_complete_notification_gate_honors_preference_and_default() {
    let enabled = DesktopConfig {
        notify_sync_complete: true,
        ..DesktopConfig::default()
    };
    let disabled = DesktopConfig {
        notify_sync_complete: false,
        ..DesktopConfig::default()
    };

    assert!(should_show_sync_complete_notification(Ok(enabled)));
    assert!(!should_show_sync_complete_notification(Ok(disabled)));
    assert!(!should_show_sync_complete_notification(Err(
        "config unreadable".to_string()
    )));
}

#[test]
fn quota_warning_notification_gate_honors_disabled_preference() {
    let disabled = DesktopConfig {
        notify_quota_warnings: false,
        ..DesktopConfig::default()
    };

    assert!(!should_show_quota_warning_notification(Ok(disabled)));
    assert!(should_show_quota_warning_notification(Ok(DesktopConfig::default())));
    assert!(should_show_quota_warning_notification(Err(
        "config unreadable".to_string()
    )));
}

#[test]
fn release_channel_is_derived_from_semver_prerelease_suffix() {
    assert_eq!(release_channel_from_version("0.2.0"), ReleaseChannel::Stable);
    assert_eq!(release_channel_from_version("0.2.0-beta.1"), ReleaseChannel::Beta);
    assert_eq!(release_channel_from_version("0.2.0-alpha.1"), ReleaseChannel::Alpha);
}

#[test]
fn release_channel_manifest_paths_keep_stable_latest_unchanged() {
    assert_eq!(
        desktop_manifest_path_for_channel(ReleaseChannel::Stable),
        "desktop/latest.json"
    );
    assert_eq!(
        desktop_manifest_path_for_channel(ReleaseChannel::Beta),
        "desktop/beta.json"
    );
    assert_eq!(
        desktop_manifest_path_for_channel(ReleaseChannel::Alpha),
        "desktop/alpha.json"
    );
}

#[test]
fn channel_update_guard_holds_prerelease_until_stable_catches_up() {
    assert!(
        !should_offer_channel_update("0.2.0-beta.1", "0.1.9"),
        "switching back to Stable must not offer a downgrade"
    );
    assert!(
        should_offer_channel_update("0.2.0-beta.1", "0.2.0"),
        "stable 0.2.0 supersedes its 0.2.0 prereleases"
    );
    assert!(
        should_offer_channel_update("0.2.0-beta.1", "0.2.0-beta.2"),
        "newer prerelease builds on the selected channel should still be offered"
    );
}

#[test]
fn manual_update_check_result_reports_current_version_and_selected_channel() {
    assert_eq!(
        manual_update_up_to_date_result(ReleaseChannel::Beta),
        ManualUpdateCheckResult::UpToDate {
            current_version: real_app_version().to_string(),
            channel: "beta".to_string(),
        }
    );
}

#[test]
fn manual_update_available_result_reuses_banner_payload_shape() {
    let (result, payload) = manual_update_available_result(
        ReleaseChannel::Alpha,
        real_app_version(),
        "0.2.0-alpha.3".to_string(),
        Some("Alpha notes"),
    );

    assert_eq!(
        payload,
        UpdateAvailablePayload {
            version: "0.2.0-alpha.3".to_string(),
            channel: "alpha".to_string(),
            body: "Alpha notes".to_string(),
            release_notes_url: release_notes_url_for_version("0.2.0-alpha.3"),
        }
    );
    assert_eq!(
        result,
        ManualUpdateCheckResult::UpdateAvailable {
            current_version: real_app_version().to_string(),
            channel: "alpha".to_string(),
            version: "0.2.0-alpha.3".to_string(),
            body: "Alpha notes".to_string(),
            release_notes_url: release_notes_url_for_version("0.2.0-alpha.3"),
        }
    );
}

#[test]
fn manual_update_result_reports_downgrade_without_banner_payload() {
    let (result, payload) = manual_update_result_for_remote(
        ReleaseChannel::Stable,
        ReleaseChannel::Beta,
        "0.2.0",
        "0.1.9".to_string(),
        Some("Stable notes"),
    );

    assert_eq!(payload, None);
    assert_eq!(
        result,
        ManualUpdateCheckResult::DowngradeAvailable {
            current_version: "0.2.0".to_string(),
            current_channel: "beta".to_string(),
            channel: "stable".to_string(),
            version: "0.1.9".to_string(),
            body: "Stable notes".to_string(),
            release_notes_url: release_notes_url_for_version("0.1.9"),
        }
    );
}

#[test]
fn installed_release_channel_prefers_persisted_update_provenance_over_version_suffix() {
    let cfg = DesktopConfig {
        installed_release_channel: Some(ReleaseChannel::Beta),
        release_channel: ReleaseChannel::Stable,
        ..DesktopConfig::default()
    };

    assert_eq!(
        installed_release_channel_from_config(Ok(cfg), "0.2.0"),
        ReleaseChannel::Beta
    );
}

#[test]
fn installed_release_channel_falls_back_to_legacy_version_suffix_when_metadata_is_absent() {
    assert_eq!(
        installed_release_channel_from_config(Ok(DesktopConfig::default()), "0.2.0-alpha.1"),
        ReleaseChannel::Alpha
    );
    assert_eq!(
        installed_release_channel_from_config(Err("config unreadable".to_string()), "0.2.0-beta.1",),
        ReleaseChannel::Beta
    );
}

#[test]
fn manual_update_result_keeps_configured_channel_and_installed_channel_separate() {
    let (result, payload) = manual_update_result_for_remote(
        ReleaseChannel::Alpha,
        ReleaseChannel::Beta,
        "0.2.0-beta.1",
        "0.1.9".to_string(),
        Some("Alpha notes"),
    );

    assert_eq!(payload, None);
    assert_eq!(
        result,
        ManualUpdateCheckResult::DowngradeAvailable {
            current_version: "0.2.0-beta.1".to_string(),
            current_channel: "beta".to_string(),
            channel: "alpha".to_string(),
            version: "0.1.9".to_string(),
            body: "Alpha notes".to_string(),
            release_notes_url: release_notes_url_for_version("0.1.9"),
        }
    );
}
