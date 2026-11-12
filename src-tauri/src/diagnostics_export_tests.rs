use super::*;
use crate::state_db::{FileEntry, FileStatus, ItemKind, OperationKind, PendingOperation, StateDb};

fn seeded_db(dir: &Path, name_path: &str, error: &str) -> PathBuf {
    let db_path = dir.join("state.db");
    let db = StateDb::open(&db_path).unwrap();
    db.upsert_file(&FileEntry {
        file_id: "f1".into(),
        path: name_path.into(),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    db.enqueue_operation(&PendingOperation {
        op_id: "op-1".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some("f1".into()),
        parent_id: None,
        target_path: Some(name_path.into()),
        metadata_json: None,
        payload_path: None,
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
    db.record_operation_attempt("op-1", 1, 10, Some(error)).unwrap();
    db_path
}

/// Task 1685: the whole bundle that `report_problem` writes to disk and the
/// user emails to support, not just one struct.
#[test]
fn bundle_has_no_path_or_known_name_and_reports_the_redaction_count() {
    let dir = tempfile::tempdir().unwrap();
    let root = Path::new("/Users/alice/Library/CloudStorage/Beebeeb-Drive");
    let db_path = seeded_db(
        dir.path(),
        "Tax 2025/aangifte.pdf",
        "/Users/alice/Library/CloudStorage/Beebeeb-Drive/Tax 2025/aangifte.pdf failed Bearer abc.def.ghi",
    );
    let bundle = export_diagnostics_from(Some(root), &db_path).unwrap();
    let text = serde_json::to_string_pretty(&bundle).unwrap();
    for leaked in [
        "guus",
        "Users",
        "CloudStorage",
        "Beebeeb-Drive",
        "Tax 2025",
        "Tax",
        "aangifte",
        "abc.def.ghi",
    ] {
        assert!(!text.contains(leaked), "{leaked:?} leaked into the bundle:\n{text}");
    }
    assert_eq!(bundle["diagnostics_format"], 2);
    assert_eq!(bundle["queue"]["last_error_code"], "other");
    assert!(
        bundle["queue"]["last_error_redactions"].as_u64().unwrap() >= 2,
        "a path and a token were removed, the count must say so:\n{text}"
    );
    assert!(bundle["queue"]["last_error"].as_str().unwrap().contains("failed"));
}

#[test]
fn sync_root_components_are_scrubbed_even_when_the_error_names_only_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = Path::new("/Users/alice/Library/CloudStorage/Beebeeb-Drive");
    let db_path = seeded_db(dir.path(), "a.txt", "sync root Beebeeb-Drive guus is not writable");
    let bundle = export_diagnostics_from(Some(root), &db_path).unwrap();
    let text = serde_json::to_string(&bundle).unwrap();
    assert!(!text.contains("guus"), "{text}");
    assert!(!text.contains("Beebeeb-Drive"), "{text}");
}

#[test]
fn sync_root_components_reach_the_name_scan() {
    // `folder` is an ordinary error word, so only the name scan (fed the
    // sync root) can remove it: this proves the extra-path plumbing.
    let dir = tempfile::tempdir().unwrap();
    let db_path = seeded_db(dir.path(), "a.txt", "sync root folder is not allowed");
    let bundle = export_diagnostics_from(Some(Path::new("/srv/folder")), &db_path).unwrap();
    let text = bundle["queue"]["last_error"].as_str().unwrap().to_string();
    assert_eq!(text, "sync root [name] is not allowed", "{text}");
}

#[test]
fn multiword_name_without_a_slash_is_scrubbed_end_to_end() {
    // `2025` is a bare number, which the allow-list keeps, so only the
    // known-name scan can remove this one.
    let dir = tempfile::tempdir().unwrap();
    let db_path = seeded_db(dir.path(), "Tax 2025/aangifte.pdf", "upload of Tax 2025 failed");
    let bundle = export_diagnostics_from(Some(Path::new("/x")), &db_path).unwrap();
    let text = bundle["queue"]["last_error"].as_str().unwrap().to_string();
    assert!(!text.contains("2025"), "{text}");
    assert!(!text.contains("Tax"), "{text}");
    assert!(text.contains("failed"), "{text}");
}

#[test]
fn email_draft_names_the_bundle_file_but_not_its_folder() {
    let body = problem_report_email_body(Path::new(
        "/Users/alice/Library/Application Support/io.beebeeb.app/diagnostics/beebeeb-diagnostics-1.json",
    ));
    assert!(body.contains("beebeeb-diagnostics-1.json"), "{body}");
    for leaked in ["guus", "Users", "Application Support", "io.beebeeb.app"] {
        assert!(!body.contains(leaked), "{leaked:?} in {body}");
    }
}

/// Task 1685 review: the bundle is written before the mail draft is tried,
/// so a mail failure must still hand the saved path back to the UI.
#[test]
fn mail_failure_is_a_partial_success_that_keeps_the_saved_path() {
    let path = Path::new("/tmp/beebeeb-diagnostics-7.json");
    let failed = problem_report_outcome(path, Err::<(), _>("no mail handler registered"));
    assert_eq!(
        failed,
        ProblemReport {
            path: "/tmp/beebeeb-diagnostics-7.json".into(),
            email_opened: false
        }
    );
    let opened = problem_report_outcome(path, Ok::<(), String>(()));
    assert_eq!(
        opened,
        ProblemReport {
            path: "/tmp/beebeeb-diagnostics-7.json".into(),
            email_opened: true
        }
    );
    let json = serde_json::to_value(&failed).unwrap();
    assert_eq!(json["path"], "/tmp/beebeeb-diagnostics-7.json");
    assert_eq!(json["email_opened"], false);
}

/// Task 1685 review: both commands build the bundle by reading every known
/// name from the state DB. A plain `#[tauri::command]` fn runs on the main
/// thread in Tauri v2, so they must be `(async)`. Source-level guard,
/// because the thread a command runs on is not observable in a unit test.
#[test]
fn bundle_commands_run_off_the_main_thread() {
    let src = crate::root_source::lib_rs();
    for name in ["report_problem", "export_diagnostics"] {
        let needle = format!("fn {name}(");
        let at = src.find(&needle).unwrap_or_else(|| panic!("{name} not found"));
        let head = &src[..at];
        let attr = head
            .lines()
            .rev()
            .find(|l| l.trim_start().starts_with("#[tauri::command"))
            .unwrap();
        assert_eq!(
            attr.trim(),
            "#[tauri::command(async)]",
            "{name} must not run on the main thread"
        );
    }
    // The native menu path must not call the blocking impl inline either.
    assert!(
        !src.contains(concat!("log_menu_result(spec.id, ", "report_problem_impl(")),
        "menu ReportProblem must go through spawn_menu_task"
    );
}

#[test]
fn unconfigured_and_missing_db_bundles_carry_the_format_version_and_no_queue() {
    let none = export_diagnostics_from(None, Path::new("")).unwrap();
    assert_eq!(none["diagnostics_format"], 2);
    assert_eq!(none["sync_root_configured"], false);
    let dir = tempfile::tempdir().unwrap();
    let missing = export_diagnostics_from(Some(Path::new("/x")), &dir.path().join("absent.db")).unwrap();
    assert_eq!(missing["state_db_exists"], false);
    assert!(missing["queue"].is_null());
}
