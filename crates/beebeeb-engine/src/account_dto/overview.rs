//! Per-device file overview (local sync-state lens).
//!
//! `compute_file_overview` groups mirrored files by sync status and surfaces
//! the most recently changed ones; folders are excluded by the caller.

use super::*;

// ── 9. File overview — the per-PC sync-state lens (Files tab) ──────────────────
//
// Where `compute_storage_breakdown` (above) is the by-TYPE / billing lens,
// this is the by-SYNC-STATE / per-device lens that powers the in-app "Files"
// tab. It groups every mirrored file by its `state_db::FileStatus` (how many
// are local vs cloud-only vs conflicting…), surfaces the most-recently-changed
// files, and rolls up totals — all computed LOCALLY from the SQLite mirror, no
// server endpoint. Folders are excluded by the caller; `pinned` is the
// effective pin state computed in the state-db pass (see
// `state_db::file_overview_rows`).

/// One sync-status bucket (e.g. `local`, `cloud_only`, `conflict`). The
/// `status` string matches `state_db::FileStatus::as_str` exactly so the
/// frontend can map it to a label/pill without a separate enum.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatusBucket {
    pub status: String,
    pub count: u64,
    pub bytes: i64,
}

/// One recently-changed file row for the Files tab. `modified_at` is the
/// row's `modified_at` (seconds since the Unix epoch — see
/// `state_db::FileEntry::modified_at`); the frontend renders it relative.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RecentFile {
    pub path: String,
    pub size_bytes: i64,
    pub status: String,
    pub pinned: bool,
    pub modified_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_type: Option<String>,
}

/// Result of [`compute_file_overview`]: the device sync-state dashboard.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileOverview {
    /// One entry per status that actually has files, descending by count then
    /// status name (stable). Statuses with zero files are omitted — the
    /// frontend renders a fixed legend and treats a missing status as zero.
    pub by_status: Vec<StatusBucket>,
    /// Most-recently-changed files, descending by `modified_at` (ties broken by
    /// path), capped at `recent_limit`.
    pub recent: Vec<RecentFile>,
    /// Total file count (folders excluded by the caller).
    pub total_files: u64,
    /// Sum of all file sizes (folders excluded). Negative sizes clamped to 0.
    pub total_bytes: i64,
    /// Count of effectively-pinned files (cuts across status buckets — a pinned
    /// file may be `local`, `downloading`, etc.). Surfaced as its own metric so
    /// the UI doesn't have to undercount from the capped `recent` list.
    pub pinned_count: u64,
    /// Sum of effectively-pinned file sizes (negative sizes clamped to 0).
    pub pinned_bytes: i64,
}

/// A minimal view of a mirrored file for the overview computation, paired with
/// its effective-pinned flag. Decoupled from `state_db::FileEntry` so the pure
/// computation is unit-testable without a SQLite handle — the caller maps
/// `(FileEntry, bool)` → this, skipping folders.
#[derive(Debug, Clone)]
pub struct FileOverviewInput {
    pub path: String,
    pub size_bytes: i64,
    /// The file's sync status string (`state_db::FileStatus::as_str`).
    pub status: String,
    pub pinned: bool,
    pub modified_at: i64,
}

/// Local-only activity entry to merge into the overview's recent list. These
/// rows are not live files and must not affect totals or status buckets.
#[derive(Debug, Clone)]
pub struct FileOverviewActivityInput {
    pub event_type: String,
    pub file_name: String,
    pub rel_path: Option<String>,
    pub occurred_at: i64,
}

/// One server-side Trash row for the desktop Trash view. Names are decrypted in
/// Rust before crossing the Tauri bridge.
#[derive(Clone, Deserialize, Serialize)]
pub struct DesktopTrashItem {
    pub id: String,
    pub name: String,
    pub is_folder: bool,
    pub size_bytes: i64,
    pub updated_at: String,
    pub parent_id: Option<String>,
}

/// Never the name or an id: the name is decrypted, and a `{:?}` must not print it.
impl std::fmt::Debug for DesktopTrashItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DesktopTrashItem")
            .field("is_folder", &self.is_folder)
            .field("size_bytes", &self.size_bytes)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConfirmActionResult {
    pub confirmation_token: String,
    pub expires_at: String,
}

/// Pure file-overview computation. `files` should already exclude folders.
/// `recent_limit` caps the `recent` list. Negative sizes are clamped to 0.
pub fn compute_file_overview(
    files: &[FileOverviewInput],
    activity: &[FileOverviewActivityInput],
    recent_limit: usize,
) -> FileOverview {
    use std::collections::BTreeMap;

    // Group by status string. BTreeMap keeps a deterministic base order before
    // we re-sort by count, so ties are stable across runs.
    let mut buckets: BTreeMap<String, StatusBucket> = BTreeMap::new();
    let mut total_bytes: i64 = 0;
    let mut total_files: u64 = 0;
    let mut pinned_count: u64 = 0;
    let mut pinned_bytes: i64 = 0;
    let mut ranked: Vec<RecentFile> = Vec::with_capacity(files.len());

    for f in files {
        let size = f.size_bytes.max(0);
        let bucket = buckets.entry(f.status.clone()).or_insert_with(|| StatusBucket {
            status: f.status.clone(),
            count: 0,
            bytes: 0,
        });
        bucket.count += 1;
        bucket.bytes = bucket.bytes.saturating_add(size);

        total_bytes = total_bytes.saturating_add(size);
        total_files += 1;
        if f.pinned {
            pinned_count += 1;
            pinned_bytes = pinned_bytes.saturating_add(size);
        }

        ranked.push(RecentFile {
            path: f.path.clone(),
            size_bytes: size,
            status: f.status.clone(),
            pinned: f.pinned,
            modified_at: f.modified_at,
            activity_type: None,
        });
    }

    for event in activity {
        if event.event_type != "moved_to_trash" {
            continue;
        }
        let path = event
            .rel_path
            .as_deref()
            .filter(|p| !p.is_empty())
            .unwrap_or(&event.file_name);
        ranked.push(RecentFile {
            path: path.to_string(),
            size_bytes: 0,
            status: "moved_to_trash".to_string(),
            pinned: false,
            modified_at: event.occurred_at,
            activity_type: Some("moved_to_trash".to_string()),
        });
    }

    // Most recent first; ties (same modified_at) broken by path for stability.
    ranked.sort_by(|a, b| b.modified_at.cmp(&a.modified_at).then_with(|| a.path.cmp(&b.path)));
    ranked.truncate(recent_limit);

    // Buckets descending by count, ties by status name (stable, deterministic).
    let mut by_status: Vec<StatusBucket> = buckets.into_values().collect();
    by_status.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.status.cmp(&b.status)));

    FileOverview {
        by_status,
        recent: ranked,
        total_files,
        total_bytes,
        pinned_count,
        pinned_bytes,
    }
}
