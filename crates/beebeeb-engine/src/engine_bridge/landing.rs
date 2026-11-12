//! `landing` items split out of `engine_bridge.rs`.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinderWriteItemKind {
    File,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinderWriteTarget {
    pub file_id: Option<String>,
    pub parent_id: Option<String>,
    pub filename: String,
    /// Full server-relative, '/'-joined path of this item under the sync root
    /// (e.g. `docs/a.txt` for a nested file, `a.txt` at the root). When `None`,
    /// the path defaults to the leaf `filename` — the legacy top-level behaviour.
    ///
    /// **Why this exists (task 0780 nested correctness):** the local-create path
    /// stores this as the row's `path` and threads it through as the upload's
    /// `target_path`, so (a) `classify_local_path`'s "already a known server
    /// file?" filter (which queries the FULL relative key) matches a nested
    /// file's row immediately, (b) `defer_local_upload_finalization` joins
    /// `sync_root + rel_path` to find the file on disk and re-stamp it in-sync,
    /// and (c) a local delete looks up the full key and trashes it on the server.
    /// Without it, nested rows were keyed by the leaf only and all three broke
    /// until the next `sync_tick` re-keyed them. Top-level is unaffected (leaf ==
    /// full key). The OS-extension IPC paths that don't supply it keep the leaf
    /// fallback.
    #[serde(default)]
    pub rel_path: Option<String>,
    pub kind: FinderWriteItemKind,
    pub contents_path: Option<String>,
    pub content_type: Option<String>,
    pub base_version_identifier: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FinderWriteOutcome {
    Queued {
        op_id: String,
        file_id: Option<String>,
        kind: OperationKind,
        ignored: bool,
        message: String,
    },
    Ignored {
        message: String,
    },
}

/// A delete under a provisional id that does not name the server file's held token (m-6).
pub(super) fn log_alias_delete_kept(provisional_id: &str, file_id: &str) {
    let (provisional_id, id_kind) = wire_id_fields(provisional_id);
    tracing::warn!(
        provisional_id,
        id_kind,
        file_id = %file_id,
        "delete of a provisional id not applied to the server file"
    );
}

/// What every Finder create checks first, and the ids it gets (`start_finder_create`).
pub(super) struct FinderCreateStart {
    pub(super) parent_contract: Option<FileContractState>,
    pub(super) file_id: String,
    pub(super) rel_path: String,
}

/// A Finder file create, staged and described, before it is queued.
pub(super) struct PreparedFinderCreate {
    pub(super) file_id: String,
    pub(super) rel_path: String,
    pub(super) staged: crate::staged_payload::StagedPayload,
    pub(super) staged_path: String,
    pub(super) size_bytes: i64,
    pub(super) row: FileEntry,
    pub(super) payload: serde_json::Value,
}

/// A Finder content modify, staged and described, before it is queued.
pub(super) struct PreparedFinderModify {
    pub(super) staged: crate::staged_payload::StagedPayload,
    pub(super) staged_path: String,
    pub(super) size_bytes: i64,
    pub(super) modified_at: i64,
    pub(super) payload: serde_json::Value,
}

/// The reply to a write of a name Finder writes for itself.
pub(super) fn ignored_finder_item(filename: &str) -> FinderWriteOutcome {
    FinderWriteOutcome::Ignored {
        message: format!("ignored temporary Finder item {filename}"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionConflictEntry {
    pub id: String,
    pub file_id: String,
    pub file_name: String,
    pub kind: String,
    pub status: String,
    pub updated_at: Option<i64>,
    pub detail: String,
    pub action: String,
    pub op_id: Option<String>,
    pub version_id: Option<String>,
    pub base_version: Option<i64>,
    pub last_error: Option<String>,
}

/// Task 2190 (plan §3.3, D4): the name of the "Keep both" copy, `{stem} ({device}, HH.MM){ext}`.
/// While that is taken, it becomes `{stem} ({device}, HH.MM) 2{ext}`, then ` 3`, and so on. It is
/// never a name in `taken` (the parent's names, lowercased). The time uses a dot, not D4's colon:
/// a `:` in a Mac file name shows as `/` in Finder, which is why 1887's kept folders use dots too. A
/// leading dot (`.env`) is not an extension.
///
/// Review-169 F3: the whole name fits the file-name limit, as a kept file's does
/// ([`crate::sign_out::kept_names`]'s `with_suffix`, 255 bytes): the stem is cut at a character boundary,
/// never the suffix, the counter or the extension. The device is cut first, to
/// [`KEEP_BOTH_DEVICE_MAX_BYTES`], so the suffix always leaves the stem room.
pub fn keep_both_name(original: &str, device: &str, hhmm: &str, taken: &std::collections::HashSet<String>) -> String {
    let device = crate::sign_out::kept_names::prefix_within(device, KEEP_BOTH_DEVICE_MAX_BYTES);
    let mut counter = 1u32;
    loop {
        let suffix = if counter == 1 {
            format!(" ({device}, {hhmm})")
        } else {
            format!(" ({device}, {hhmm}) {counter}")
        };
        let candidate = crate::sign_out::kept_names::with_suffix(original, &suffix);
        if !taken.contains(&candidate.to_lowercase()) {
            return candidate;
        }
        counter += 1;
    }
}

pub fn is_ignored_finder_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    matches!(
        trimmed,
        ".DS_Store" | ".DocumentRevisions-V100" | ".Spotlight-V100" | ".TemporaryItems" | ".Trashes" | "TemporaryItems"
    ) || trimmed.starts_with("._")
        || trimmed.starts_with("~$")
        || trimmed.ends_with('~')
        || lower.ends_with(".tmp")
        || lower.ends_with(".temp")
        || lower.ends_with(".swp")
        || lower.ends_with(".swo")
        || lower.ends_with(".part")
        || lower.ends_with(".crdownload")
}

pub fn version_conflict_feed_from_db(db: &StateDb) -> anyhow::Result<Vec<VersionConflictEntry>> {
    let mut entries = Vec::new();

    // Task 2190 (the lead's condition on E3): a sync-folder edit the scan holds is shown, so it
    // never looks synced in the app. It is not sent, never marked in sync and never written over
    // (task 1640), and both versions are kept.
    // - `base_unknown`: a known edit whose base is not known: one row each, needing a decision.
    // - `content_unknown`: a file hydrated before task 1640. The scan cannot tell an edited one from
    //   an untouched one (task 2191 S1 marks them all not in sync), so they are ONE row that says
    //   they are not checked yet, never "needs a decision" per file.
    let mut not_checked: Vec<(String, String, i64)> = Vec::new();
    let held = db.list_held_edits().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "review feed: the held edits could not be read; they are left out");
        Vec::new()
    });
    for (file_id, path, reason, detected_at) in held {
        if reason == HELD_CONTENT_UNKNOWN {
            not_checked.push((file_id, path, detected_at));
            continue;
        }
        entries.push(VersionConflictEntry {
            id: format!("held:{file_id}"),
            file_id,
            file_name: display_name_for_path(&path),
            kind: "held_edit".to_string(),
            status: "needs a decision".to_string(),
            updated_at: Some(detected_at),
            detail: "Changed on this PC, but Beebeeb doesn't know which version it was based on, so it \
                     isn't sent. Both versions are kept: yours on this PC, the other in the file's history."
                .to_string(),
            action: "review_upload".to_string(),
            op_id: None,
            version_id: None,
            base_version: None,
            last_error: None,
        });
    }
    if let Some((first_id, _, first_at)) = not_checked.first().cloned() {
        let names: Vec<String> = not_checked
            .iter()
            .take(5)
            .map(|(_, path, _)| display_name_for_path(path))
            .collect();
        let more = not_checked.len().saturating_sub(names.len());
        let files = if not_checked.len() == 1 {
            "1 file".to_string()
        } else {
            format!("{} files", not_checked.len())
        };
        entries.push(VersionConflictEntry {
            id: "held:content_unknown".to_string(),
            file_id: first_id,
            file_name: format!("{files} from before version 0.8.13"),
            kind: "held_unknown".to_string(),
            status: "not checked yet".to_string(),
            updated_at: Some(first_at),
            detail: format!(
                "A change to one of them stays on this PC and isn't sent until a later update checks it. \
                 Nothing is lost. {}{}",
                names.join(", "),
                if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                }
            ),
            action: "review_upload".to_string(),
            op_id: None,
            version_id: None,
            base_version: None,
            last_error: None,
        });
    }

    for file in db.list_by_status(FileStatus::Conflict)? {
        entries.push(VersionConflictEntry {
            id: format!("conflict:{}", file.file_id),
            file_id: file.file_id.clone(),
            file_name: display_name_for_path(&file.path),
            kind: "conflict".to_string(),
            status: "needs review".to_string(),
            updated_at: Some(file.modified_at),
            detail: "Local and remote both changed from the last synced base.".to_string(),
            action: "open_conflict".to_string(),
            op_id: None,
            version_id: None,
            base_version: None,
            last_error: None,
        });
    }

    // Review-169 #5: one entry that cannot be read is logged and left out; the rest of the feed shows.
    for op in db.list_review_operations()? {
        match review_entry_for_operation(&op, db) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                tracing::warn!(op_id = %op.op_id, error = %e, "review feed: an entry could not be read; it is left out")
            }
        }
    }

    // Task 1640 (review N9): a file deleted elsewhere while it held changes this device had not
    // sent; they were kept as a copy, which uploads as a new file. Informational: no decision is
    // needed, so the person can dismiss it.
    for kept in db.list_kept_copies()? {
        // Review F5: the text says why the file was kept.
        let original_name = display_name_for_path(&kept.original_path);
        let (file_name, status, detail) = match kept.reason {
            KeptReason::Copy => {
                let copy_name = display_name_for_path(&kept.copy_path);
                let place = match kept.copy_path.rsplit_once('/') {
                    Some((folder, _)) => format!("in the folder {folder}"),
                    None => "at the top of your Beebeeb folder".to_string(),
                };
                let original = if kept.original_removed {
                    ""
                } else {
                    " The original stays on this PC until it can be removed."
                };
                (
                    copy_name.clone(),
                    "kept as a copy",
                    format!(
                        "Deleted elsewhere while it held changes not sent from this PC. Those changes are kept as {copy_name}, {place}.{original}"
                    ),
                )
            }
            KeptReason::InUse => (
                original_name,
                "waiting to close",
                "Deleted elsewhere while it was open on this PC. It is removed from this PC once it is closed.".to_string(),
            ),
            KeptReason::PartlyLocal => (
                original_name,
                "kept in place",
                "Deleted elsewhere while only part of it was on this PC, so it could not be checked for changes. It stays where it is, as it is. Dismiss this once you have dealt with it.".to_string(),
            ),
            KeptReason::CopyFailed => (
                original_name,
                "kept in place",
                "Deleted elsewhere while it held changes not sent from this PC, and they could not be copied. It stays where it is; the copy is tried again.".to_string(),
            ),
        };
        entries.push(VersionConflictEntry {
            id: format!("kept:{}", kept.id),
            file_id: kept.original_file_id.clone(),
            file_name,
            kind: "kept_copy".to_string(),
            status: status.to_string(),
            updated_at: Some(kept.kept_at),
            detail,
            // Dismissable once nothing waits, or when no tick can resolve it (review F7).
            action: if kept.dismissable() { "dismiss" } else { "none" }.to_string(),
            op_id: None,
            version_id: None,
            base_version: None,
            last_error: None,
        });
    }

    entries.sort_by(|a, b| {
        b.updated_at
            .unwrap_or_default()
            .cmp(&a.updated_at.unwrap_or_default())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(entries)
}

/// Task 1670 round 4 (Codex P2 on PR #75, review thread on `runner.rs:1044`):
/// `true` when `dest_path` is a macOS Finder `fetchContents` HANDOFF staging
/// file — i.e. it lives inside `hydrate_dir`, never a durable local cache
/// copy. `hydrate_dir` is a parameter (never
/// `crate::ipc_socket`'s `macos_hydrate_cache_dir()` read inline) so this is
/// unit-testable with a throwaway directory, never the real, singleton
/// App-Group container a live Beebeeb.app/Finder extension share on this
/// same machine (`macos_hydrate_cache_dir`'s own doc comment covers why that
/// directory must never be written to by a test process here — round 2's
/// `disposable_cache_roots_includes_the_macos_hydrate_cache_dir` test follows
/// the identical "real path, read-only, no write into it" precedent).
///
/// See [`record_hydration_cache_state`] for the full "why this matters"
/// writeup: registering this file's path via `mark_cached` would point
/// `files.cache_path`/`cache_bytes` at a file the SYSTEM (via
/// `FileProviderExtension.copyToSystemTemporaryDirectory(stagedAt:)`), not
/// this daemon, now owns and that our own copy stops existing moments after
/// this check runs — `desktop_storage_summary` and smart-cache eviction both
/// sum/act on any row with a non-null `cache_path` and no existence check.
#[cfg(target_os = "macos")]
pub(super) fn is_macos_finder_handoff_staging_path(dest_path: &Path, hydrate_dir: &Path) -> bool {
    crate::is_contained(hydrate_dir, dest_path)
}

/// The daemon's own copy of a Finder write's contents, in `root`: from the
/// opened file when the caller holds one (only `source` is journaled then, as
/// a label), else from `source` by path.
pub(super) fn stage_finder_contents(
    db: &Arc<StateDb>,
    root: PathBuf,
    source: &Path,
    opened: Option<&std::fs::File>,
) -> Result<crate::staged_payload::StagedPayload, EngineError> {
    let staged = match opened {
        Some(file) => crate::staged_payload::StagedPayload::copy_from_file(db.clone(), file, source, root),
        None => crate::staged_payload::StagedPayload::copy(db.clone(), source, root),
    };
    staged.map_err(EngineError::from)
}

/// The folders a Finder write's plaintext copy can be staged in, from the bases this process
/// resolves: the app's data dir, its cache dir and the temp dir. Paths only: nothing is created
/// or touched until a root is chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinderStagingBases {
    pub data: Option<PathBuf>,
    pub cache: Option<PathBuf>,
    pub temp: PathBuf,
}

impl FinderStagingBases {
    /// This process's bases: the OS's data, cache and temp dirs.
    #[cfg(not(any(test, feature = "test-support")))]
    pub fn current() -> Self {
        Self {
            data: dirs::data_dir(),
            cache: dirs::cache_dir(),
            temp: std::env::temp_dir(),
        }
    }

    /// A test build stages under a per-process sandbox, never the person's real data or cache
    /// dir: the installed app stages there too, and every test that queues a Finder write
    /// (engine bridge, IPC socket, watcher) would otherwise add plaintext files to it. A test
    /// points one bridge at bases of its own with `Seams::stage_under`. Pinned by
    /// `unit_tests_stage_finder_writes_in_a_sandbox_never_the_real_cache`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn current() -> Self {
        Self {
            data: Some(crate::test_sandbox::dir("data").expect("create the unit-test sandbox")),
            cache: Some(crate::test_sandbox::dir("cache").expect("create the unit-test sandbox")),
            temp: std::env::temp_dir(),
        }
    }

    /// `<data dir>/beebeeb/finder-writes`, where macOS stages: a folder the system does not
    /// purge. `None` when the OS gives no data dir.
    #[cfg_attr(not(any(target_os = "macos", test, feature = "test-support")), allow(dead_code))]
    pub fn durable_root(&self) -> Option<PathBuf> {
        self.data
            .as_ref()
            .map(|data| data.join(crate::identity::config_dir_name()).join("finder-writes"))
    }

    /// `<cache dir>/beebeeb/finder-writes` (the temp dir when the OS gives no cache dir): where
    /// every platform staged before, and where Windows and Linux still stage.
    pub fn cache_root(&self) -> PathBuf {
        self.cache
            .as_ref()
            .unwrap_or(&self.temp)
            .join(crate::identity::config_dir_name())
            .join("finder-writes")
    }

    /// `<temp dir>/beebeeb/finder-writes`: the fallback when the cache root is not writable
    /// (Windows and Linux; macOS used it before as well).
    pub fn temp_root(&self) -> PathBuf {
        self.temp.join(crate::identity::config_dir_name()).join("finder-writes")
    }

    /// Every folder a staged copy can be in. On macOS: the data root, then the cache root and the
    /// temp root, which builds before the move staged into, so their copies are still found. Elsewhere:
    /// the cache root and the temp root.
    pub fn candidates(&self) -> Vec<PathBuf> {
        let mut candidates = Vec::with_capacity(3);
        #[cfg(target_os = "macos")]
        candidates.extend(self.durable_root());
        candidates.extend([self.cache_root(), self.temp_root()]);
        candidates
    }
}

/// Where this process stages a Finder write's plaintext copy now.
pub(super) fn default_finder_staging_root() -> Result<PathBuf, FinderStagingUnavailable> {
    finder_staging_root_from(&FinderStagingBases::current())
}

/// The staging root `bases` give on this platform.
pub(super) fn finder_staging_root_from(bases: &FinderStagingBases) -> Result<PathBuf, FinderStagingUnavailable> {
    #[cfg(target_os = "macos")]
    {
        durable_finder_staging_root(bases)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(cache_finder_staging_root(bases))
    }
}

/// macOS: the staged copy can be the only copy of a save until it uploads (once the extension
/// has handed the write over, the system counts the item as synced and may evict its bytes), so
/// it lives in the app's data dir (`Library/Application Support`), never in the cache or temp
/// dir, which the system may purge. The folder is private (`0700`, excluded from backups: it
/// holds plaintext) and must take a file. When it cannot be created or written to, the accept
/// fails with [`FinderStagingUnavailable`], and nothing falls back.
#[cfg(any(target_os = "macos", any(test, feature = "test-support")))]
pub(super) fn durable_finder_staging_root(bases: &FinderStagingBases) -> Result<PathBuf, FinderStagingUnavailable> {
    let root = bases.durable_root().ok_or(FinderStagingUnavailable(None))?;
    let unavailable = |e: std::io::Error| FinderStagingUnavailable(Some(e.kind()));
    prepare_private_staging_root(&root).map_err(unavailable)?;
    verify_staging_root_writable(&root).map_err(unavailable)?;
    Ok(root)
}

/// Windows and Linux: the cache root, or the temp root when the cache root is not writable.
#[cfg(not(target_os = "macos"))]
pub(super) fn cache_finder_staging_root(bases: &FinderStagingBases) -> PathBuf {
    let primary = bases.cache_root();
    match verify_staging_root_writable(&primary) {
        Ok(()) => primary,
        Err(e) => {
            let fallback = bases.temp_root();
            tracing::warn!(
                path = %primary.display(),
                fallback = %fallback.display(),
                error = %e,
                "Finder staging cache root is unavailable; falling back to temp directory"
            );
            fallback
        }
    }
}

/// Where Finder-write plaintext copies are staged, and were staged by earlier builds
/// ([`FinderStagingBases::candidates`]). The account reset and the sign-out purge sweep every one, so a copy no
/// row points at any more does not outlive the account, and the sign-out purge's allow-list accepts every one.
/// Paths only: nothing is created or touched here.
pub fn finder_staging_candidates() -> Vec<PathBuf> {
    FinderStagingBases::current().candidates()
}

/// Task 2190 (review-169 #5): whether the review row may open the decision. A read that fails offers
/// no decision (the row stays, with its old action), and is logged.
pub(super) fn decidable_or_logged(db: &StateDb, op_id: &str) -> bool {
    decidable_check(db, op_id).unwrap_or_else(|e| {
        tracing::warn!(op_id = %op_id, error = %e, "review feed: the decision check failed");
        false
    })
}

pub(super) fn decidable_check(db: &StateDb, op_id: &str) -> Result<bool, EngineError> {
    #[cfg(any(test, feature = "test-support"))]
    if FAIL_DECIDABLE_READ.with(|fail| fail.borrow().as_deref() == Some(op_id)) {
        return Err(anyhow::anyhow!("the decision check failed (test)").into());
    }
    Ok(db.is_decidable_parked(op_id)?)
}

pub(super) fn review_entry_for_operation(
    op: &PendingOperation,
    db: &StateDb,
) -> Result<VersionConflictEntry, EngineError> {
    let file_id = op.file_id.clone().unwrap_or_else(|| op.op_id.clone());
    let file_name = op
        .target_path
        .as_deref()
        .map(display_name_for_path)
        .or_else(|| {
            op.file_id
                .as_deref()
                .and_then(|id| db.get_file(id).ok().flatten())
                .map(|entry| display_name_for_path(&entry.path))
        })
        .unwrap_or_else(|| file_id.clone());
    let (kind, status, detail, action) = classify_review_operation(op);
    // Task 2190 (plan §3.4): a parked stale-base Finder write the person may decide opens the
    // conflict; every other row keeps its action (D3).
    let action = if kind == "stale_base" && decidable_or_logged(db, &op.op_id) {
        "open_conflict"
    } else {
        action
    };

    Ok(VersionConflictEntry {
        id: format!("op:{}", op.op_id),
        file_id,
        file_name,
        kind: kind.to_string(),
        status: status.to_string(),
        updated_at: Some(op.updated_at),
        detail,
        action: action.to_string(),
        op_id: Some(op.op_id.clone()),
        version_id: operation_version_id(op),
        base_version: op.base_version,
        last_error: op.last_error.clone(),
    })
}

pub(super) fn classify_review_operation(op: &PendingOperation) -> (&'static str, &'static str, String, &'static str) {
    let raw_error = op.last_error.as_deref().unwrap_or("");
    let error = raw_error.to_ascii_lowercase();
    let metadata = op.metadata_json.as_deref().unwrap_or("").to_ascii_lowercase();
    let stale_base = error.contains("stale")
        || error.contains("base version")
        || metadata.contains("base_version_identifier")
        || op.base_version.is_some();

    if matches!(op.kind, OperationKind::RestoreVersion) {
        return (
            "restore",
            "restore review",
            "Restore is queued or failed; the server restore endpoint creates a new current version when it succeeds."
                .to_string(),
            "restore_review",
        );
    }

    // Auth failures take priority over quota/permission/stale-base: an
    // expired session's 401 unblocks nothing until the user signs in again,
    // and "sign in again" is a strictly more actionable message than "your
    // storage is full" or "permission denied" would be for the same root
    // cause. Reuses `classify_operation_error`'s stripped-URL substring
    // matching (task 1252 — a reqwest error's `" for url (…)"` suffix can
    // embed a port number containing "401") instead of re-deriving a second,
    // looser copy of the same check here (task 1546 finding 3).
    if matches!(classify_operation_error(raw_error), OperationFailureClass::Auth) {
        return (
            "auth_failure",
            "sign-in needed",
            "Your session has expired. Sign in again to resume syncing.".to_string(),
            "sign_in_again",
        );
    }
    if error.contains("quota") || error.contains("insufficient storage") {
        return (
            "quota_failure",
            "quota blocked",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Upload is blocked by account storage quota.".to_string()),
            "review_upload",
        );
    }
    if error.contains("permission")
        || error.contains("forbidden")
        || error.contains("read-only")
        || error.contains("403")
    {
        return (
            "permission_failure",
            "permission blocked",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Write is blocked by folder permissions.".to_string()),
            "review_upload",
        );
    }
    if stale_base && matches!(op.kind, OperationKind::UploadVersion) {
        return (
            "stale_base",
            "stale base kept local",
            "Local bytes are preserved in the durable queue and need version review before retry.".to_string(),
            "review_upload",
        );
    }

    match op.kind {
        OperationKind::UploadVersion | OperationKind::UploadFile => (
            "failed_upload",
            "upload review",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Upload is queued for the encrypted transfer worker.".to_string()),
            "review_upload",
        ),
        OperationKind::RenameFile | OperationKind::MoveFile => (
            "metadata",
            "metadata review",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Rename or move is queued for metadata sync.".to_string()),
            "review_upload",
        ),
        OperationKind::TrashFile => (
            "delete",
            "delete review",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Delete is queued as a trash/soft-delete operation.".to_string()),
            "review_upload",
        ),
        _ => (
            "failed_upload",
            "queued review",
            op.last_error
                .clone()
                .unwrap_or_else(|| "Operation is queued for a worker that is not attached yet.".to_string()),
            "review_upload",
        ),
    }
}

/// Spec D5 (task 1888): after the pass committed, the directories of the scaffolding it dropped (sync-root-relative)
/// are removed deepest first, each with `std::fs::remove_dir`: never recursive, so one that holds anything errors and
/// stays, as does one already gone. A path that is empty, absolute or climbs out of the sync root is skipped. The
/// paths are never logged.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(super) fn remove_scaffold_dirs(sync_root: &Path, dirs: &[String]) {
    let mut inside: Vec<&Path> = dirs
        .iter()
        .map(Path::new)
        .filter(|rel| {
            rel.components().next().is_some()
                && rel
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
        })
        .collect();
    inside.sort_by_key(|rel| std::cmp::Reverse(rel.components().count()));
    for rel in inside {
        let path = sync_root.join(rel);
        // Task 2407 (core 2404): never a different (empty) folder the name reaches, e.g. a Windows 8.3 short name.
        if matches!(crate::name_alias::conflict_below(sync_root, &path), Ok(None)) {
            let _ = std::fs::remove_dir(path);
        }
    }
}

/// Task 1640 (review P1-1, N2, N6): remove the local entry of a file or folder the server no
/// longer lists (a web trash, a snapshot prune) without losing a change that was never sent.
///
/// A file in sync, or with no bytes on disk, is removed. A file wholly on disk (out of sync after
/// an edit, or a plain file an atomic save left) is removed when its bytes are `agreed` (the
/// row's agreed content); other bytes are copied first, with [`copy_as_this_pc`], into
/// `copy_dir` (a folder the copy can upload from), and only then is the original removed. A file
/// partly on disk is left: reading it would fetch the rest from inside the tick, and the server's
/// own bytes would come back as a copy. A folder is removed only when it is empty.
///
/// Task 2407 (core 2404): a `path` (below `sync_root`) whose name reaches a different entry, e.g. a Windows 8.3 short
/// name, is not this item's entry: nothing is removed or copied, and nothing of the item is on disk (`Gone`). A check
/// that cannot finish removes nothing either; the entry is kept for the retry.
#[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
pub(crate) fn remove_pruned_entry(
    sync_root: &Path,
    path: &Path,
    kind: RemovedKind,
    agreed: Option<&str>,
    local: LocalEntry,
    copy_dir: &Path,
) -> PrunedEntry {
    match crate::name_alias::conflict_below(sync_root, path) {
        Ok(None) => {}
        Ok(Some(_)) => {
            // Zero-knowledge: never the path.
            tracing::warn!(
                "remote-deletion reconcile: the name reaches a different entry (8.3 alias); it is left alone"
            );
            return PrunedEntry::Gone;
        }
        Err(error) => {
            tracing::warn!(kind = ?error.kind(), "remote-deletion reconcile: the name could not be checked; nothing is removed");
            return if kind == RemovedKind::Dir {
                PrunedEntry::FolderKept
            } else {
                PrunedEntry::KeptInPlace(KeptReason::InUse)
            };
        }
    }
    if std::fs::symlink_metadata(path).is_err() {
        return PrunedEntry::Gone;
    }
    let removed = |result: std::io::Result<()>, kept: PrunedEntry| match result {
        Ok(()) => PrunedEntry::Removed,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => PrunedEntry::Gone,
        Err(_) => kept,
    };
    let in_use = || PrunedEntry::KeptInPlace(KeptReason::InUse);
    if kind == RemovedKind::Dir {
        return removed(engine_remove(path, RemovedKind::Dir), PrunedEntry::FolderKept);
    }
    match local {
        LocalEntry::InSync | LocalEntry::NothingLocal => removed(engine_remove(path, RemovedKind::File), in_use()),
        LocalEntry::PartlyLocal => PrunedEntry::KeptInPlace(KeptReason::PartlyLocal),
        LocalEntry::Unreadable => in_use(),
        LocalEntry::WhollyLocal => {
            let Ok(hash) = sha256_file_hex(path) else {
                return in_use();
            };
            if agreed == Some(hash.as_str()) {
                return removed(engine_remove(path, RemovedKind::File), in_use());
            }
            match copy_as_this_pc(path, copy_dir) {
                Ok((copy, copied_hash)) => {
                    let removal = match engine_remove(path, RemovedKind::File) {
                        Ok(()) => OriginalRemoval::Removed,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => OriginalRemoval::Removed,
                        Err(_) => OriginalRemoval::InPlace,
                    };
                    PrunedEntry::KeptCopy {
                        copy,
                        copied_hash,
                        removal,
                    }
                }
                Err(_) => PrunedEntry::KeptInPlace(KeptReason::CopyFailed),
            }
        }
    }
}

/// Task 1640 (review-151d P2-2): once `original` is removed, the Office owner file it left (`~$`
/// and its name, or `~$` and its name without the first two characters) goes too: it belongs to
/// that file, and would keep its folder. No other file is touched: names the sync folder ignores
/// are never synced, so they would be lost for good.
///
/// Task 2407 (core 2404): each owner name is checked from `sync_root`, every folder on the way included, so a name
/// that reaches a different file or folder (a Windows 8.3 short name) never removes that one's owner file.
#[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
pub(super) fn remove_office_owner_files(sync_root: &Path, original: &Path) {
    let (Some(dir), Some(name)) = (original.parent(), original.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    let mut owners = vec![format!("~${name}")];
    if let Some((cut, _)) = name.char_indices().nth(2) {
        owners.push(format!("~${}", &name[cut..]));
    }
    for owner in owners {
        let path = dir.join(owner);
        if !matches!(crate::name_alias::conflict_below(sync_root, &path), Ok(None)) {
            continue;
        }
        if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Task 1640 (review P1-1, N5): copy `path` into `dir` as `<stem> (this PC)<ext>`, then
/// `<stem> (this PC 2)<ext>` and so on, never over an existing file. The bytes are written and
/// flushed under an ignored temporary name first and linked to the final name only then, so the
/// upload driver never sees a partial copy. Returns the copy and the SHA-256 of what it holds.
#[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
pub fn copy_as_this_pc(path: &Path, dir: &Path) -> std::io::Result<(PathBuf, String)> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|s| s.to_str());
    let temp = dir.join(format!(".{stem}.{}.tmp", uuid::Uuid::new_v4()));
    let written = (|| -> std::io::Result<String> {
        let mut source = std::fs::File::open(path)?;
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            out.write_all(&buffer[..read])?;
        }
        out.sync_all()?;
        Ok(format!("{:x}", hasher.finalize()))
    })();
    let hash = match written {
        Ok(hash) => hash,
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            return Err(e);
        }
    };
    for n in 1..=99 {
        let suffix = if n == 1 {
            " (this PC)".to_string()
        } else {
            format!(" (this PC {n})")
        };
        let name = match ext {
            Some(ext) => format!("{stem}{suffix}.{ext}"),
            None => format!("{stem}{suffix}"),
        };
        let copy = dir.join(name);
        match move_no_replace(&temp, &copy) {
            Ok(()) => return Ok((copy, hash)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                let _ = std::fs::remove_file(&temp);
                return Err(e);
            }
        }
    }
    let _ = std::fs::remove_file(&temp);
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free name for the copy",
    ))
}

/// Task 1640 (review N3): where a copy of `path` can upload from: its folder, or the nearest
/// folder above it that is neither being removed now (`removed_dirs`) nor in the trash, else
/// the sync root. Inside a folder trashed on the web the copy would name that folder as its
/// parent, which the server refuses.
#[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
pub(super) fn kept_copy_dir(db: &StateDb, sync_root: &Path, path: &Path, removed_dirs: &HashSet<PathBuf>) -> PathBuf {
    let mut dir = path.parent().unwrap_or(sync_root).to_path_buf();
    while dir != sync_root && dir.starts_with(sync_root) {
        let trashed = relative_db_path(sync_root, &dir)
            .and_then(|rel| db.get_file_by_path(&rel).ok().flatten())
            .is_some_and(|row| row.status == FileStatus::Trashing);
        if !removed_dirs.contains(&dir) && !trashed {
            return dir;
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => break,
        }
    }
    sync_root.to_path_buf()
}

/// Record a kept copy (review N9, N13): one review entry until it is dismissed, a tray activity
/// row, and, while its original could not be removed, the retry's record. `copy: None` is a file
/// kept where it is (`reason`, review F5); `copied_hash` is then its agreed content, if any, for
/// the retry to compare with. `existing`: the entry to update where it is (review F6), not a new one.
#[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
pub(super) fn record_kept_copy(
    db: &StateDb,
    sync_root: &Path,
    file_id: &str,
    original: &Path,
    copy: Option<&Path>,
    copied_hash: &str,
    removal: OriginalRemoval,
    reason: KeptReason,
    existing: Option<i64>,
) {
    let now = now_secs();
    let original_path = rel_under(sync_root, original);
    let copy_path = copy.map(|copy| rel_under(sync_root, copy)).unwrap_or_default();
    let input = crate::state_db::KeptCopyInput {
        original_file_id: file_id,
        original_path: &original_path,
        copy_path: &copy_path,
        copied_hash,
        original_removed: removal.is_removed(),
        reason,
        kept_at: now,
    };
    let recorded = match existing {
        Some(id) => db.update_kept_copy(id, &input),
        None => db.record_kept_copy(&input).map(|_| ()),
    }
    .and_then(|()| match copy {
        Some(copy) => db.record_local_activity(LocalActivityEventInput {
            event_type: LocalActivityKind::KeptLocalCopy,
            file_id: Some(file_id.to_string()),
            file_name: copy
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string(),
            rel_path: Some(copy_path.clone()),
            occurred_at: now,
        }),
        None => Ok(()),
    });
    if let Err(e) = recorded {
        tracing::warn!(file_id = %file_id, error = %e, "remote-deletion reconcile: could not record the kept copy");
    }
}

/// Task 1640 (review F3): the folders deleted elsewhere that stay on disk for what they hold, as
/// paths under `sync_root`.
#[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
pub(super) fn removed_folder_paths(db: &StateDb, sync_root: &Path) -> HashSet<PathBuf> {
    db.list_removed_folders()
        .unwrap_or_default()
        .iter()
        .filter_map(|rel| EngineBridge::placeholder_path_under(sync_root, rel))
        .collect()
}

/// Remove the on-disk entries of `rows` (a remote deletion), each through [`remove_pruned_entry`],
/// with `local` the platform's view of each file.
#[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
pub(super) fn remove_pruned_entries(
    db: &StateDb,
    sync_root: &Path,
    rows: &[crate::state_db::PrunedRow],
    local: &dyn Fn(&Path, bool) -> LocalEntry,
) {
    let mut removed_dirs: HashSet<PathBuf> = rows
        .iter()
        .filter(|row| row.is_dir)
        .filter_map(|row| EngineBridge::placeholder_path_under(sync_root, &row.path))
        .collect();
    removed_dirs.extend(removed_folder_paths(db, sync_root));
    for row in rows {
        let Some(path) = EngineBridge::placeholder_path_under(sync_root, &row.path) else {
            continue;
        };
        let rel = rel_under(sync_root, &path);
        // Review F4: a file whose kept entry still waits (a copy made, the original in use; a
        // file kept where it is) is the retry's. A second remote op (the trash emptied after a
        // web trash, a folder trash after a file trash) copies nothing again. Unreadable: kept.
        if !row.is_dir && db.pending_kept_original(&rel).map_or(true, |pending| pending.is_some()) {
            continue;
        }
        // The remove itself registers with the watcher right before it happens (`engine_remove`),
        // so the NOTIFY_DELETE_COMPLETION it fires queues no trash.
        let agreed = row.local_hash.as_deref().or(row.fetched_hash.as_deref());
        // Task 2191 (G-engine, review-114 F-P1-4): the platform's in-sync state is trusted only
        // for a file with agreed content; one of unknown content is read by its size on disk.
        let state = if row.is_dir {
            LocalEntry::InSync
        } else {
            local(&path, agreed.is_some())
        };
        let copy_dir = kept_copy_dir(db, sync_root, &path, &removed_dirs);
        let kind = if row.is_dir {
            RemovedKind::Dir
        } else {
            RemovedKind::File
        };
        match remove_pruned_entry(sync_root, &path, kind, agreed, state, &copy_dir) {
            PrunedEntry::Removed if !row.is_dir => remove_office_owner_files(sync_root, &path),
            PrunedEntry::Removed | PrunedEntry::Gone => {}
            PrunedEntry::KeptCopy {
                copy,
                copied_hash,
                removal,
            } => {
                if removal.is_removed() {
                    remove_office_owner_files(sync_root, &path);
                }
                // Zero-knowledge: never log the path, only the file_id.
                tracing::warn!(
                    file_id = %row.file_id,
                    original_removed = removal.is_removed(),
                    "remote-deletion reconcile: the file held changes that were not sent; they are kept as a copy"
                );
                record_kept_copy(
                    db,
                    sync_root,
                    &row.file_id,
                    &path,
                    Some(&copy),
                    &copied_hash,
                    removal,
                    KeptReason::Copy,
                    None,
                );
            }
            PrunedEntry::KeptInPlace(reason) => {
                tracing::warn!(
                    file_id = %row.file_id,
                    reason = reason.as_str(),
                    "remote-deletion reconcile: kept the file where it is"
                );
                // Condition 5: ONE entry per file (a second remote op skips it, above); the retry
                // resolves it once it can be removed, copied or is gone.
                record_kept_copy(
                    db,
                    sync_root,
                    &row.file_id,
                    &path,
                    None,
                    agreed.unwrap_or_default(),
                    OriginalRemoval::InPlace,
                    reason,
                    None,
                );
            }
            PrunedEntry::FolderKept => {
                tracing::warn!(file_id = %row.file_id, "remote-deletion reconcile: kept a folder that still holds something");
                // Review F3: never created again from this PC while it only holds what is kept.
                if let Err(e) = db.record_removed_folder(&rel, now_secs()) {
                    tracing::warn!(file_id = %row.file_id, error = %e, "remote-deletion reconcile: could not record the kept folder");
                }
            }
        }
    }
}

/// Task 1640 (review N13, F2, F6): every file kept from a remote deletion whose original still
/// waits (it was in use, partly on disk, or its copy failed) is retried: the original is removed
/// when it still holds the copied bytes; one the person changed since is a new change, kept as a
/// new copy, and then removed. A copy is never made twice of the same bytes. The entry is updated
/// where it is, or goes when nothing is kept. An original whose place belongs to a file the
/// server lists again (restored on the web, or another file moved there) is left alone and no
/// longer waits. Then every kept folder that is empty now is removed (review F3).
#[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
pub fn retry_kept_originals(db: &StateDb, sync_root: &Path, local: &dyn Fn(&Path, bool) -> LocalEntry) {
    // Review-151d P2-1: a dismissed entry's tombstone goes once its file is gone, or once the place
    // is the server's again.
    for (id, rel) in db.list_dismissed_kept_originals().unwrap_or_default() {
        let gone = EngineBridge::placeholder_path_under(sync_root, &rel)
            .is_none_or(|path| std::fs::symlink_metadata(path).is_err());
        let listed = db
            .get_file_by_path(&rel)
            .is_ok_and(|row| row.is_some_and(|row| row.status != FileStatus::Trashing));
        if gone || listed {
            let _ = db.delete_kept_copy(id);
        }
    }
    let Ok(kept) = db.list_kept_copies() else {
        return;
    };
    let removed_dirs = removed_folder_paths(db, sync_root);
    for copy in kept.into_iter().filter(|copy| !copy.original_removed) {
        // Resolved with nothing left to wait for: a copy's entry stays (dismissable); a file kept
        // where it is had nothing kept, so its entry goes.
        let resolve = |copy: &crate::state_db::KeptCopy| {
            let _ = if copy.copy_path.is_empty() {
                db.delete_kept_copy(copy.id)
            } else {
                db.mark_kept_original_removed(copy.id)
            };
        };
        let Some(original) = EngineBridge::placeholder_path_under(sync_root, &copy.original_path) else {
            resolve(&copy);
            continue;
        };
        if std::fs::symlink_metadata(&original).is_err() {
            resolve(&copy);
            continue;
        }
        // Review F2: the file was restored on the web (its own row, live again): it is tracked by
        // its row, never removed or trashed again. Any other row at the place (another file moved
        // there) does not own these bytes: they are still the deleted file's placeholder, which would
        // keep the other file's from being created, so they go as below (a change copied first); the
        // remove is the engine's own, so nothing is trashed (self-review #4).
        match db.get_file_by_path(&copy.original_path) {
            Ok(Some(row)) if row.file_id == copy.original_file_id && row.status != FileStatus::Trashing => {
                resolve(&copy);
                continue;
            }
            Ok(_) => {}
            Err(_) => continue,
        }
        let copy_dir = kept_copy_dir(db, sync_root, &original, &removed_dirs);
        let agreed = Some(copy.copied_hash.as_str()).filter(|hash| !hash.is_empty());
        // Review F2: the engine's own remove queues no trash (`engine_remove` registers it).
        // Task 2191 (review-114 F-P1-4): an entry with no agreed content never trusts in sync.
        match remove_pruned_entry(
            sync_root,
            &original,
            RemovedKind::File,
            agreed,
            local(&original, agreed.is_some()),
            &copy_dir,
        ) {
            PrunedEntry::Removed | PrunedEntry::Gone => {
                remove_office_owner_files(sync_root, &original);
                resolve(&copy);
            }
            PrunedEntry::KeptCopy {
                copy: new_copy,
                copied_hash,
                removal,
            } => {
                if removal.is_removed() {
                    remove_office_owner_files(sync_root, &original);
                }
                // Changed since (or wholly on disk now, with changes): a new change, in the same entry.
                record_kept_copy(
                    db,
                    sync_root,
                    &copy.original_file_id,
                    &original,
                    Some(&new_copy),
                    &copied_hash,
                    removal,
                    KeptReason::Copy,
                    Some(copy.id),
                );
            }
            PrunedEntry::KeptInPlace(reason) => {
                // Self-review #5: an entry whose changes could not be copied was read wholly once; it
                // never becomes "partly on disk", which the person may dismiss.
                let downgrade = copy.reason == KeptReason::CopyFailed && reason == KeptReason::PartlyLocal;
                if copy.copy_path.is_empty() && reason != copy.reason && !downgrade {
                    record_kept_copy(
                        db,
                        sync_root,
                        &copy.original_file_id,
                        &original,
                        None,
                        &copy.copied_hash,
                        OriginalRemoval::InPlace,
                        reason,
                        Some(copy.id),
                    );
                }
            }
            PrunedEntry::FolderKept => {}
        }
    }
    remove_empty_kept_folders(db, sync_root);
}

/// Task 1640 (review F3): remove every kept folder (deepest first) that holds nothing now, and
/// forget every one that is gone or that the server lists again.
#[cfg_attr(not(any(target_os = "windows", test, feature = "test-support")), allow(dead_code))]
pub(super) fn remove_empty_kept_folders(db: &StateDb, sync_root: &Path) {
    for rel in db.list_removed_folders().unwrap_or_default() {
        let Some(path) = EngineBridge::placeholder_path_under(sync_root, &rel) else {
            let _ = db.forget_removed_folder(&rel);
            continue;
        };
        match db.get_file_by_path(&rel) {
            // Listed again (restored on the web, or another folder moved there): it is not removed.
            Ok(Some(row)) if row.status != FileStatus::Trashing => {
                let _ = db.forget_removed_folder(&rel);
                continue;
            }
            Ok(_) => {}
            Err(_) => continue,
        }
        // Task 2407 (core 2404): a name that reaches a different folder (a Windows 8.3 short name) is not the kept
        // folder, which is gone; a check that cannot finish is retried.
        match crate::name_alias::conflict_below(sync_root, &path) {
            Ok(None) => {}
            Ok(Some(_)) => {
                let _ = db.forget_removed_folder(&rel);
                continue;
            }
            Err(_) => continue,
        }
        // Review-151d P2-2: only an empty folder goes. One that keeps a file the sync folder ignores
        // (never synced, so the person's only copy) stays, and is not created again on the server.
        if std::fs::read_dir(&path).is_ok_and(|mut entries| entries.next().is_some()) {
            continue;
        }
        match engine_remove(&path, RemovedKind::Dir) {
            Ok(()) => {
                let _ = db.forget_removed_folder(&rel);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let _ = db.forget_removed_folder(&rel);
            }
            Err(_) => {}
        }
    }
}

#[cfg(target_os = "windows")]
pub(super) fn remove_pruned_placeholders(db: &StateDb, sync_root: &Path, rows: &[crate::state_db::PrunedRow]) {
    remove_pruned_entries(
        db,
        sync_root,
        rows,
        &crate::windows_cf::placeholders::placeholder_local_entry_trusting,
    );
}

/// No-op stand-in on non-Windows builds so the call sites stay platform-agnostic.
#[cfg(not(target_os = "windows"))]
pub(super) fn remove_pruned_placeholders(_db: &StateDb, _sync_root: &Path, _rows: &[crate::state_db::PrunedRow]) {}
