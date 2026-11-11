//! File-overview computation tests.
//!
//! `ovr` builds overview input rows; the tests cover status grouping, recent
//! ordering, the recent cap and trash activity.

use super::*;

// ── compute_file_overview ────────────────────────────────────────────────

fn ovr(path: &str, size: i64, status: &str, pinned: bool, modified_at: i64) -> FileOverviewInput {
    FileOverviewInput {
        path: path.into(),
        size_bytes: size,
        status: status.into(),
        pinned,
        modified_at,
    }
}

#[test]
fn file_overview_empty_is_zeroed() {
    let o = compute_file_overview(&[], &[], 5);
    assert_eq!(o.total_files, 0);
    assert_eq!(o.total_bytes, 0);
    assert_eq!(o.pinned_count, 0);
    assert_eq!(o.pinned_bytes, 0);
    // Unlike the by-category breakdown, status buckets are emitted only for
    // statuses that have files — an empty mirror has no buckets at all.
    assert!(o.by_status.is_empty());
    assert!(o.recent.is_empty());
}

#[test]
fn file_overview_groups_mixed_statuses_and_clamps_negative() {
    let files = vec![
        ovr("/a.txt", 100, "local", false, 10),
        ovr("/b.txt", 200, "local", true, 20),
        ovr("/c.jpg", 300, "cloud_only", false, 30),
        ovr("/d.pdf", 400, "conflict", false, 40),
        // Negative size is clamped to 0 in totals and the bucket.
        ovr("/e.bin", -50, "cloud_only", false, 50),
    ];
    let o = compute_file_overview(&files, &[], 10);

    assert_eq!(o.total_files, 5);
    assert_eq!(o.total_bytes, 100 + 200 + 300 + 400); // -50 clamped to 0

    // Pinned cuts across statuses: only /b.txt (local, pinned) here.
    assert_eq!(o.pinned_count, 1);
    assert_eq!(o.pinned_bytes, 200);

    let local = o.by_status.iter().find(|b| b.status == "local").unwrap();
    assert_eq!(local.count, 2);
    assert_eq!(local.bytes, 300);

    let cloud = o.by_status.iter().find(|b| b.status == "cloud_only").unwrap();
    assert_eq!(cloud.count, 2);
    assert_eq!(cloud.bytes, 300); // 300 + 0 (clamped)

    let conflict = o.by_status.iter().find(|b| b.status == "conflict").unwrap();
    assert_eq!(conflict.count, 1);
    assert_eq!(conflict.bytes, 400);

    // Buckets sorted by count desc: local & cloud_only (2 each, tie broken
    // by status name → cloud_only first), then conflict (1).
    assert_eq!(o.by_status[0].status, "cloud_only");
    assert_eq!(o.by_status[1].status, "local");
    assert_eq!(o.by_status[2].status, "conflict");
}

#[test]
fn file_overview_recent_is_ordered_by_modified_desc() {
    let files = vec![
        ovr("/old.txt", 1, "local", false, 100),
        ovr("/new.txt", 1, "local", true, 300),
        ovr("/mid.txt", 1, "cloud_only", false, 200),
    ];
    let o = compute_file_overview(&files, &[], 10);

    assert_eq!(o.recent.len(), 3);
    assert_eq!(o.recent[0].path, "/new.txt");
    assert_eq!(o.recent[0].modified_at, 300);
    assert!(o.recent[0].pinned);
    assert_eq!(o.recent[1].path, "/mid.txt");
    assert_eq!(o.recent[2].path, "/old.txt");
}

#[test]
fn file_overview_recent_limit_caps_list() {
    let files: Vec<FileOverviewInput> = (0..10)
        .map(|i| ovr(&format!("/f{i}.txt"), 1, "local", false, i as i64))
        .collect();
    let o = compute_file_overview(&files, &[], 3);

    // All ten counted in totals/buckets…
    assert_eq!(o.total_files, 10);
    // …but only the 3 most recent surfaced in `recent` (modified_at 9, 8, 7).
    assert_eq!(o.recent.len(), 3);
    assert_eq!(o.recent[0].modified_at, 9);
    assert_eq!(o.recent[1].modified_at, 8);
    assert_eq!(o.recent[2].modified_at, 7);
}

#[test]
fn file_overview_recent_includes_trash_activity_without_changing_totals() {
    let files = vec![ovr("/live.txt", 100, "local", false, 10)];
    let activity = vec![FileOverviewActivityInput {
        event_type: "moved_to_trash".into(),
        file_name: "deleted.txt".into(),
        rel_path: Some("deleted.txt".into()),
        occurred_at: 20,
    }];
    let o = compute_file_overview(&files, &activity, 10);

    assert_eq!(o.total_files, 1);
    assert_eq!(o.total_bytes, 100);
    assert_eq!(o.by_status.len(), 1);
    assert_eq!(o.by_status[0].status, "local");
    assert_eq!(o.recent.len(), 2);
    assert_eq!(o.recent[0].path, "deleted.txt");
    assert_eq!(o.recent[0].status, "moved_to_trash");
    assert_eq!(o.recent[0].activity_type.as_deref(), Some("moved_to_trash"));
    assert_eq!(o.recent[0].size_bytes, 0);
    assert_eq!(o.recent[1].path, "/live.txt");
}
