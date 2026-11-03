//! `meta` items split out of `engine_bridge.rs`.

use super::*;

/// Where a Finder fetch was served from (spec §7.4, §11: the device plan counts these).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrateSource {
    /// The staged copy of the held write, still queued: the bytes its token names.
    Queue,
    /// The server, as every fetch did before.
    Server,
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
impl HydrateSource {
    pub fn as_str(self) -> &'static str {
        match self {
            HydrateSource::Queue => "queue",
            HydrateSource::Server => "server",
        }
    }
}

/// The two fields of a line that logs an id from the wire (M5): the id field (`Some` when it
/// is a UUID) and `id_kind` (`Some("non_uuid")` otherwise). Exactly one is `Some`; `tracing`
/// writes nothing for a `None` field.
pub(crate) fn wire_id_fields(id: &str) -> (Option<tracing::field::DisplayValue<&str>>, Option<&'static str>) {
    match loggable_id(id) {
        NON_UUID => (None, Some(NON_UUID)),
        id => (Some(tracing::field::display(id)), None),
    }
}

/// The device plan's count of fetches that reached the daemon (spec §11, §14 instrument 2).
/// The id is the request's (M5).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn log_hydrate_served(file_id: &str, source: HydrateSource) {
    let (file_id, id_kind) = wire_id_fields(file_id);
    tracing::info!(file_id, id_kind, source = source.as_str(), "Finder hydrate served");
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedRootRefreshOutcome {
    pub active_shared_root_ids: Vec<String>,
    pub removed_shared_file_ids: Vec<String>,
    pub removed_cache_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SharedRootMapping {
    pub(super) invite_id: String,
    pub(super) file_id: String,
    pub(super) display_name: String,
    pub(super) is_folder: bool,
    pub(super) size_bytes: i64,
    pub(super) content_type: Option<String>,
    pub(super) owner_email: Option<String>,
    pub(super) sender_public_key: Option<String>,
    pub(super) encrypted_file_key: Option<String>,
    pub(super) encrypted_folder_key: Option<String>,
    pub(super) file_name_encrypted: Option<String>,
    pub(super) permission_bits: i64,
    pub(super) approved_at: Option<i64>,
}

pub(super) fn owned_key_and_pin(db: &StateDb, master: &[u8; 32], id: &str)
    -> Result<(beebeeb_core::kdf::FileKey, beebeeb_core::chunk_stream::ExpectedChunkFormat), EngineError> {
    if let Some(record) = db.get_owned_key_record(id)? {
        let (key, metadata) = crate::owner_keys::record_key(
            &beebeeb_core::kdf::MasterKey::from_bytes(*master),
            &record,
            crate::owner_keys::kind_in_db(db, id),
        )?;
        return Ok((key, metadata.chunk_format));
    }
    // P2-8: a foreign id has no derived key. Its read twin refuses it already;
    // deriving here would seal the write under a key the attacker can pick.
    if beebeeb_core::item_id::item_class(id).ok() == Some(beebeeb_core::item_id::ItemClass::Foreign) {
        return Err(EngineError::from(anyhow::anyhow!("item key is unavailable")));
    }
    Ok((file_key_for(master, id), beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded))
}

pub(super) fn owned_metadata_for_name(db: &StateDb, master: &[u8; 32], id: &str, name: &str, mime: Option<&str>) -> Result<String, EngineError> {
    let (key, pin) = owned_key_and_pin(db, master, id)?;
    Ok(beebeeb_core::encrypt::encrypt_name_for(&key, name, mime, pin)?)
}

pub(super) fn encrypted_metadata_for_name(
    master_key_bytes: &[u8; 32],
    file_id: &str,
    filename: &str,
    mime_type: Option<&str>,
) -> Result<String, EngineError> {
    let master_key = beebeeb_core::kdf::MasterKey::from_bytes(*master_key_bytes);
    beebeeb_core::encrypt::encrypt_name(&master_key, file_id, filename, mime_type)
        .map_err(|e| EngineError::from(anyhow::anyhow!("encrypt Finder metadata: {e}")))
}

/// Task 2190: a held edit's reasons (`held_edits.reason`).
pub(super) const HELD_BASE_UNKNOWN: &str = "base_unknown";

pub(super) const HELD_CONTENT_UNKNOWN: &str = "content_unknown";

/// [`SyncFolderContent`] of `file_id`'s bytes at `path`. `os_in_sync` is the Cloud Files in-sync
/// state of its placeholder (Windows; `None` elsewhere): the platform clears it when the bytes
/// change, and this engine sets it only on agreed content, so `Some(true)` is agreed without
/// reading the file. The file is read only when an agreed hash is recorded.
/// Task 2191: whether `path` holds bytes on this PC (a placeholder partly or wholly on disk, or
/// a plain file). Unreadable counts as holding bytes.
pub(super) fn place_holds_bytes(path: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        !matches!(
            crate::windows_cf::placeholders::placeholder_local_entry_trusting(path, false),
            LocalEntry::NothingLocal
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::fs::metadata(path).is_ok_and(|meta| meta.len() > 0)
    }
}

/// Task 1640 (review N1, condition 2): make a file's candidate its agreed content. Only for a
/// file the platform reports in sync with no modified data (`local`), wholly on disk, whose bytes
/// read under one exclusive handle (`hash_under_handle`) are the candidate's, with a candidate of
/// ONE version (a mixed one is none), and only while the row is still `Local` with that same
/// candidate (re-checked in the write). `Ok(true)`: promoted.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(crate) fn promote_fetched_content(
    db: &StateDb,
    file_id: &str,
    local: LocalEntry,
    wholly_on_disk: bool,
    hash_under_handle: &mut dyn FnMut() -> std::io::Result<String>,
) -> Result<bool, EngineError> {
    if local != LocalEntry::InSync || !wholly_on_disk {
        return Ok(false);
    }
    let Some(row) = db.get_file(file_id)? else {
        return Ok(false);
    };
    if row.status != FileStatus::Local
        || db.get_file_contract_state(file_id)?.is_some_and(|contract| contract.local_hash.is_some())
    {
        return Ok(false);
    }
    let Some((version, Some(candidate))) = db.fetched_content(file_id)? else {
        return Ok(false);
    };
    if hash_under_handle()? != candidate {
        return Ok(false);
    }
    Ok(db.promote_fetched_content(file_id, version, &candidate)?)
}

/// The `init` request for one upload attempt. `proposed_id` is the item id this
/// device has chosen for the operation — a create's own (possibly freshly
/// minted) id, a replace's row id — and it is always sent: the server never
/// picks an id for this client (SF-0001 L13).
pub(super) fn upload_init_request_for_operation(
    proposed_id: &str,
    name_encrypted: &str,
    content_type: Option<String>,
    parent_id: Option<String>,
    plaintext_size: u64,
    base_version: Option<i64>,
) -> DesktopUploadInitRequest {
    let plan = beebeeb_types::plan_chunks(plaintext_size, beebeeb_types::ChunkProfile::Desktop);
    DesktopUploadInitRequest {
        file_id: Some(proposed_id.to_string()),
        file_name: name_encrypted.to_string(),
        file_size_bytes: plaintext_size,
        mime_type: None,
        parent_id,
        profile: "desktop".to_string(),
        is_media: beebeeb_core::media::is_media(content_type.as_deref()),
        chunk_size_bytes: Some(plan.chunk_size_bytes),
        chunk_count: Some(plan.chunk_count),
        base_version_number: base_version.and_then(|version| i32::try_from(version).ok()),
    }
}

pub(super) fn sign_pow(value: f64, exp: f64) -> f64 {
    value.abs().powf(exp).copysign(value)
}

/// `op`'s metadata with its encrypted name (if it carries one) re-encrypted
/// for `file_id`. The plaintext name is the op's display name; the MIME hint
/// is the op's content type, else the one guessed from the name, as the
/// Finder write path does.
pub(super) fn metadata_rekeyed_with_material(master_key: &[u8; 32], op: &PendingOperation, file_id: &str, material: Option<(&beebeeb_core::kdf::FileKey, beebeeb_core::chunk_stream::ExpectedChunkFormat)>) -> Result<Option<String>, EngineError> {
    let Some(raw) = op.metadata_json.as_deref() else {
        return Ok(None);
    };
    let mut metadata: serde_json::Value = serde_json::from_str(raw)?;
    if metadata.get("name_encrypted").and_then(|v| v.as_str()).is_none() {
        return Ok(Some(raw.to_string()));
    }
    let Some(name) = metadata_display_name(&metadata, op) else {
        return Err(anyhow::anyhow!(
            "queued operation {} has an encrypted name but no display name",
            op.op_id
        ).into());
    };
    let mime = metadata["content_type"]
        .as_str()
        .map(str::to_string)
        .or_else(|| beebeeb_core::media::guess_mime_type(&name).map(str::to_string));
    metadata["name_encrypted"] = serde_json::json!(match material {
        Some((key, pin)) => beebeeb_core::encrypt::encrypt_name_for(key, &name, mime.as_deref(), pin)?,
        None => encrypted_metadata_for_name(master_key, file_id, &name, mime.as_deref())?,
    });
    Ok(Some(serde_json::to_string(&metadata)?))
}

pub(super) fn metadata_display_name(metadata: &serde_json::Value, op: &PendingOperation) -> Option<String> {
    metadata["display_name"]
        .as_str()
        .map(str::to_string)
        .or_else(|| op.target_path.as_deref().map(display_name_for_path))
}

#[cfg(any(target_os = "windows", test))]
pub(super) struct HydrationChunkRangePlan {
    pub(super) first_chunk: u32,
    pub(super) last_chunk_exclusive: u32,
    pub(super) range_start_bytes: u64,
    pub(super) covering_span_hint: usize,
    pub(super) covers_whole_file: bool,
}

#[cfg(any(target_os = "windows", test))]
pub(super) fn plan_hydration_chunk_range(
    size_bytes: u64,
    chunk_count: u64,
    chunk_size_bytes: u64,
    required_offset: u64,
    required_length: u64,
) -> Result<Option<HydrationChunkRangePlan>, EngineError> {
    if !chunk_layout_matches_metadata(size_bytes, chunk_count, chunk_size_bytes) {
        return Ok(None);
    }

    let range_end = required_offset.saturating_add(required_length).min(size_bytes);
    if range_end <= required_offset {
        return Ok(None);
    }

    let first_chunk = required_offset / chunk_size_bytes;
    let last_chunk = (range_end - 1) / chunk_size_bytes;
    if last_chunk >= chunk_count {
        return Ok(None);
    }

    let first_chunk_u32 =
        u32::try_from(first_chunk).map_err(|_| anyhow::anyhow!("chunk index {first_chunk} exceeds u32 range"))?;
    let last_chunk_u32 =
        u32::try_from(last_chunk).map_err(|_| anyhow::anyhow!("chunk index {last_chunk} exceeds u32 range"))?;
    let last_chunk_exclusive = last_chunk_u32
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("chunk range end exceeds u32 range"))?;
    let chunk_size_usize = usize::try_from(chunk_size_bytes)
        .map_err(|_| anyhow::anyhow!("chunk_size_bytes {chunk_size_bytes} exceeds usize range"))?;
    let chunk_count_in_range = usize::try_from(last_chunk_u32 - first_chunk_u32 + 1)
        .map_err(|_| anyhow::anyhow!("covering chunk count exceeds usize range"))?;
    let covering_span_hint = chunk_count_in_range
        .checked_mul(chunk_size_usize)
        .ok_or_else(|| anyhow::anyhow!("covering range size exceeds usize range"))?;

    Ok(Some(HydrationChunkRangePlan {
        first_chunk: first_chunk_u32,
        last_chunk_exclusive,
        range_start_bytes: first_chunk * chunk_size_bytes,
        covering_span_hint,
        covers_whole_file: first_chunk == 0 && last_chunk == chunk_count - 1,
    }))
}

/// `progress(done_bytes, total_bytes)` callback for [`EngineBridge::hydrate_file_with_progress`].
/// Called synchronously from the download loop, so it must be cheap and must
/// not block (the IPC handler just pushes onto a channel).
pub(crate) type HydrateProgressFn = dyn Fn(u64, u64) + Send + Sync;

/// Where a direct download (task 2173) reports: the same wire counter, transfer board and
/// progress callback the API path's chunk loop feeds.
pub(super) struct HydrateReport<'a> {
    pub(super) bridge: &'a EngineBridge,
    pub(super) file_id: &'a str,
    /// The server's plaintext size (the API path's `total`).
    pub(super) total: u64,
    pub(super) progress: Option<&'a HydrateProgressFn>,
}

impl crate::direct_download::Progress for HydrateReport<'_> {
    fn received(&self, wire_bytes: u64) {
        self.bridge.wire.download_bytes.fetch_add(wire_bytes, Ordering::Relaxed);
    }

    fn decrypted(&self, plaintext_bytes: u64) {
        self.decrypted_bytes(plaintext_bytes);
    }
}

impl HydrateReport<'_> {
    pub(super) fn decrypted_bytes(&self, done: u64) {
        self.bridge.transfers.report(self.file_id, done, self.total);
        if let Some(report) = self.progress {
            report(done, self.total);
        }
    }
}

/// Task 1698: the status a SUCCESSFUL hydrate leaves the row in. A `Trashing`
/// row (the user is viewing a file from the macOS Trash view) must KEEP its
/// marker — flipping it to `Local` would reparent the item back under its
/// folder in the replica's working set (the trash view would lose it). Any
/// other row completes normally as `Local` (the content is now on disk).
pub(crate) fn hydrate_final_status(previous: FileStatus) -> FileStatus {
    match previous {
        FileStatus::Trashing => FileStatus::Trashing,
        _ => FileStatus::Local,
    }
}

/// Task 1247: is `dest_path` a safe hydration target — i.e. inside (or about to
/// be created inside) at least one of the caller's trusted `allowed_roots`?
///
/// Pure and free-standing so the containment decision is directly unit-testable
/// without a live `EngineBridge`. Mirrors the two-layer containment pattern
/// already used by `lib.rs::reveal_and_open_file` (see its documented 3-layer
/// comment block):
///   - If `dest_path` already exists, canonicalize both and require containment.
///   - If it does not exist yet (the normal hydrate case — we are about to
///     create it), require the PARENT directory to be contained AND the file
///     name to be a single normal component with no embedded separator, so a
///     traversal like `<root>/../evil` cannot slip past the parent check.
///
/// Returns true if ANY root passes; false if none do. An empty `allowed_roots`
/// slice returns false (fail-closed), never vacuously true.
pub(crate) fn hydrate_dest_is_allowed(dest_path: &Path, allowed_roots: &[&Path]) -> bool {
    for &root in allowed_roots {
        let ok = if dest_path.exists() {
            crate::is_contained(root, dest_path)
        } else {
            let parent_ok = dest_path
                .parent()
                .map(|p| crate::is_contained(root, p))
                .unwrap_or(false);
            let name_ok = dest_path
                .file_name()
                .map(|n| {
                    let s = n.to_string_lossy();
                    !s.contains('/') && !s.contains('\\')
                })
                .unwrap_or(false);
            parent_ok && name_ok
        };
        if ok {
            return true;
        }
    }
    false
}

/// Task 1670 round 4 (Codex P2 on PR #75, review thread on `runner.rs:1044`):
/// register `dest_path` as this file's durable cache entry (`mark_cached`,
/// same as before this round) — UNLESS it is a macOS Finder `fetchContents`
/// HANDOFF staging file (i.e. [`is_macos_finder_handoff_staging_path`] against
/// `macos_hydrate_dir`), in which case this deliberately does nothing.
///
/// **Why:** `FileProviderExtension.fetchContents` (round 4,
/// `BeebeebFileProvider/FileProviderExtension.swift`) copies a staged file
/// like this one into the SYSTEM's own
/// `NSFileProviderManager.temporaryDirectoryURL()` and deletes OUR copy at
/// `dest_path` immediately after — so `dest_path` can already be gone by the
/// time anything reads `files.cache_path` again. Registering it anyway
/// pointed `desktop_storage_summary` (`cache_bytes_by_effective_pin`,
/// `state_db.rs`) and smart-cache eviction
/// (`evict_unpinned_cache_until_under`, `enforce_configured_cache_limit`) at
/// a file that stops existing almost immediately — both sum/act on ANY row
/// with `cache_path IS NOT NULL AND cache_bytes > 0` with no existence
/// check — so every Finder peek left a phantom cache-usage entry, and a
/// target for a pointless eviction of a file that was never really
/// "cached" at all. [`Self::hydrate_file`]'s `set_status(..., Local)` call
/// (just before this one runs) still flips Finder's own downloaded badge
/// (`FileProviderItem.isDownloaded` reads `status == "local"`,
/// `BeebeebFileProvider/FileProviderItem.swift`) — that is the one real UI
/// signal a momentary Finder peek should produce; it should not also claim
/// a durable cache footprint the system, not us, now owns.
///
/// `macos_hydrate_dir` is a parameter — production passes the REAL
/// `crate::ipc_socket`'s `macos_hydrate_cache_dir()` (see the call site in
/// [`Self::hydrate_file`]); tests pass a throwaway tempdir, so this whole
/// decision is exercised end to end without ever touching the real,
/// singleton App-Group directory a live Beebeeb.app/Finder extension share
/// on this same machine.
#[cfg(target_os = "macos")]
pub(super) fn record_hydration_cache_state(
    db: &StateDb,
    file_id: &str,
    dest_path: &Path,
    macos_hydrate_dir: &Path,
) -> Result<(), EngineError> {
    if is_macos_finder_handoff_staging_path(dest_path, macos_hydrate_dir) {
        return Ok(());
    }
    let cache_bytes = std::fs::metadata(dest_path).map(|m| m.len() as i64).unwrap_or(0);
    db.mark_cached(file_id, &dest_path.to_string_lossy(), cache_bytes, now_secs())?;
    Ok(())
}

/// Task 1247: write hydrated plaintext to `dest_path`, enforcing that it lands
/// inside an allowed root even against an actively-racing same-UID attacker.
///
/// The threat: `hydrate_file` validates the destination *before* the network
/// download/decrypt (`do_hydrate`), but the write happens *after* — a real
/// wall-clock window. Earlier rounds re-checked containment and then re-opened
/// the parent by PATH; those are two independent, non-atomic path resolutions,
/// so a plain directory `rename()` swap between them (no symlink needed) still
/// escaped: `O_NOFOLLOW` on the second open only refuses a *symlink* at that
/// name, it says nothing about whether the name still resolves to the SAME
/// inode that was validated.
///
/// The fix eliminates the second path resolution entirely. We descend from a
/// trusted allowed root to the parent directory ONE COMPONENT AT A TIME, purely
/// via `openat` relative to already-open directory fds, never touching an
/// absolute path string again after the first (root) open. Each component's
/// `openat` IS its own validation — it happens exactly once, atomically, and
/// once a directory fd is open a rename of its name elsewhere in the tree can no
/// longer affect that fd. `O_NOFOLLOW` on every hop refuses symlinks, and
/// because each inode is reached only as a direct child entry of an
/// already-in-root directory fd, nothing can escape the root subtree.
///
/// Also (unchanged from the prior round): the leaf is created with `O_NOFOLLOW`
/// and then `fchmod`'d to `0o600` unconditionally — POSIX applies the `O_CREAT`
/// mode only to a newly created inode, so an attacker-planted pre-existing
/// `0o644` file would otherwise keep its perms and leak the plaintext.
///
/// Non-unix writes with `File::create` (`openat`/`fchmod`/`O_NOFOLLOW` are Unix-only,
/// and the Windows Cloud Files path never writes plaintext to disk via this fn —
/// it uses `hydrate_file_to_memory`).
///
/// **Task 1670 round 2 addition:** the plaintext is staged under a per-call,
/// randomly-suffixed temp name in the SAME directory, then published onto the
/// real `leaf` name with one atomic `renameat` — anchored to the SAME
/// already-validated `dir_fd` for the temp create, the write, AND the rename,
/// never re-resolving by path (this keeps the anchored-descent invariant
/// `create_leaf_relative_is_anchored_to_original_dir_fd_across_rename_swap`
/// pins). Two consequences:
/// - Any reader racing this write (another same-UID process, or the macOS
///   hydrate-cache TTL sweep running concurrently in a different IPC
///   connection) can never observe a file at `dest_path` that exists but is
///   only partially written — it either isn't there yet, or it's complete.
/// - On ANY failure after the temp file is created, that temp file is removed
///   before the error is returned, so nothing new is ever left behind at
///   `dest_path` — there is no separate "clean up the half-written file" step
///   for a caller to remember.
///
/// The leaf-is-currently-a-symlink refusal is preserved with the EXACT same
/// observable behavior as before (a probe `open(O_NOFOLLOW)`, not
/// `fstat`/`lstat`, so a symlink leaf still fails closed with `ELOOP`) even
/// though the final publish step is now a `renameat`, which — unlike
/// `open`+`O_TRUNC` — does not dereference a symlink destination at all and so
/// could never be tricked into writing THROUGH one to an outside target on its
/// own. Keeping the probe is about not silently replacing a foreign symlink
/// with a real file, not about a plaintext-leak risk `renameat` doesn't have.
///
/// Known residual (documented follow-up, not closed here): a same-filesystem
/// HARD link to a file outside all allowed roots bypasses containment (the walk
/// sees a regular in-root leaf), and `O_TRUNC` would overwrite the linked inode.
/// Noted in the task file.
pub(crate) fn write_hydrated_plaintext(dest_path: &Path, allowed_roots: &[&Path], buf: &[u8]) -> std::io::Result<()> {
    write_hydrated_from_reader(dest_path, allowed_roots, &mut &buf[..]).map(|_| ())
}

/// [`write_hydrated_plaintext`]'s core, reading the plaintext from `reader` (spec §7.4: a
/// fetch served from a queued write's staged copy streams it). Every step is the same:
/// the anchored descent, the `O_NOFOLLOW` temp file, the atomic rename. Returns the number
/// of bytes written.
#[cfg(unix)]
pub(crate) fn write_hydrated_from_reader(
    dest_path: &Path,
    allowed_roots: &[&Path],
    reader: &mut dyn std::io::Read,
) -> std::io::Result<u64> {
    use std::os::unix::io::AsRawFd;
    use std::path::Component;

    let deny = |m: &'static str| std::io::Error::new(std::io::ErrorKind::PermissionDenied, m);

    let parent = match dest_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => return Err(deny("hydrate destination has no parent directory")),
    };
    let leaf = dest_path
        .file_name()
        .ok_or_else(|| deny("hydrate destination has no file name"))?;

    // Pick the allowed root that is a lexical prefix of the parent, and the
    // component chain from that root to the parent. This is a pure string
    // operation (no filesystem access), so there is nothing to race here — the
    // openat descent below is the real, atomic enforcement.
    let (root, rel) = allowed_roots
        .iter()
        .find_map(|&root| parent.strip_prefix(root).ok().map(|rel| (root, rel)))
        .ok_or_else(|| deny("hydrate destination is not within an allowed root"))?;

    // Every relative component must be a plain name — reject `.`/`..`/root/prefix
    // (defense in depth; a legit dest built by root.join(rel) never has these).
    let mut components: Vec<&std::ffi::OsStr> = Vec::new();
    for comp in rel.components() {
        match comp {
            Component::Normal(c) => components.push(c),
            _ => return Err(deny("hydrate destination has a non-normal path component")),
        }
    }

    // Descend from the trusted root to the immediate parent, one component at a
    // time, entirely via fd-relative opens. Each `OwnedFd` reassignment drops the
    // previous one.
    let mut dir_fd = open_dir_no_follow(root)?;
    for comp in components {
        dir_fd = open_dir_relative_no_follow(dir_fd.as_raw_fd(), comp)?;
    }

    // Refuse (without creating or writing anything) if the leaf currently
    // exists as a symlink — same ELOOP-producing O_NOFOLLOW semantics the
    // previous direct-open implementation used, reproduced here via a
    // read-only probe open so the observable failure mode for
    // `write_hydrated_plaintext_refuses_symlink_and_sets_0600` is unchanged.
    // ENOENT (no existing leaf — the common, fresh-hydrate case) is fine; the
    // rename below creates it. Any other pre-existing entry (a regular file)
    // is also fine — it gets atomically replaced below, same as before.
    let leaf_c = path_to_cstring(leaf)?;
    // SAFETY: `dir_fd` is a live, already-validated directory fd; `leaf_c` is
    // NUL-terminated. We only inspect the syscall's return value / errno.
    let probe = unsafe {
        libc::openat(
            dir_fd.as_raw_fd(),
            leaf_c.as_ptr(),
            libc::O_NOFOLLOW | libc::O_RDONLY | libc::O_CLOEXEC,
        )
    };
    if probe < 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ENOENT) {
            return Err(err);
        }
    } else {
        // SAFETY: `probe` is a valid fd just returned by the openat above.
        unsafe { libc::close(probe) };
    }

    // Stage under a per-call, randomly-suffixed temp name in the same
    // directory (task 1670 round 2 — see the fn doc comment).
    let temp_leaf_name = format!(".{}.{}.part", leaf.to_string_lossy(), uuid::Uuid::new_v4());
    let temp_leaf = std::ffi::OsStr::new(&temp_leaf_name);
    let temp_leaf_c = path_to_cstring(temp_leaf)?;
    let remove_temp = || {
        // SAFETY: `dir_fd` is still live (we only ever drop it by falling out
        // of this function); `temp_leaf_c` is NUL-terminated and owned for the
        // duration of this closure's use. Best-effort — errors are ignored,
        // matching every other cleanup-on-failure path in this module.
        let _ = unsafe { libc::unlinkat(dir_fd.as_raw_fd(), temp_leaf_c.as_ptr(), 0) };
    };

    // Create/open the temp leaf relative to the anchored parent fd, refusing a
    // symlink at the leaf itself (it never pre-exists — the name is fresh —
    // but O_NOFOLLOW costs nothing and matches the invariant every other leaf
    // open in this function keeps), then force owner-only perms on the fd
    // unconditionally (before any plaintext is written).
    let file_fd = create_leaf_relative(dir_fd.as_raw_fd(), temp_leaf)?;
    if unsafe { libc::fchmod(file_fd.as_raw_fd(), 0o600 as libc::mode_t) } != 0 {
        let err = std::io::Error::last_os_error();
        remove_temp();
        return Err(err);
    }

    let written = match std::io::copy(reader, &mut std::fs::File::from(file_fd)) {
        Ok(written) => written,
        Err(e) => {
            remove_temp();
            return Err(e);
        }
    };

    // Atomically publish: same `dir_fd` for both sides, so this is anchored to
    // the already-validated directory inode, not a fresh path resolution.
    // SAFETY: both name arguments are NUL-terminated `CString`s alive for this
    // call; `dir_fd` is a live, already-validated directory fd.
    let rc = unsafe {
        libc::renameat(
            dir_fd.as_raw_fd(),
            temp_leaf_c.as_ptr(),
            dir_fd.as_raw_fd(),
            leaf_c.as_ptr(),
        )
    };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        remove_temp();
        return Err(err);
    }

    Ok(written)
}

#[cfg(not(unix))]
pub(crate) fn write_hydrated_from_reader(
    dest_path: &Path,
    _allowed_roots: &[&Path],
    reader: &mut dyn std::io::Read,
) -> std::io::Result<u64> {
    std::io::copy(reader, &mut std::fs::File::create(dest_path)?)
}

/// The content version the File Provider extension holds for an item
/// (`docs/IPC_PROTOCOL.md`, "Version identifiers"): the server's version
/// number, then the content hash when the row has one.
///
/// The extension hands it back as the base of its next write, and
/// [`parse_base_version_number`] reads the first segment as the
/// `base_version_number` the server compares with the file's current version.
/// It never carries a wall-clock value: a re-stamp of the row without a new
/// server version (the `/sync/ops` echo of an upload, a finished upload's
/// `remote_updated_at`) is not a content change, and must neither force a
/// re-download nor turn into a base the server refuses.
pub(crate) fn item_content_version(current_version: i64, content_hash: Option<&str>) -> String {
    match content_hash {
        Some(hash) => format!("{current_version}:{hash}"),
        None => current_version.to_string(),
    }
}

/// The item's full version identifier: server version, then modification
/// time and size. Older extensions fall back to it when the payload has no
/// content version, so it leads with the server version too.
pub(crate) fn item_version_identifier(current_version: i64, modified_at: i64, size_bytes: i64) -> String {
    format!("{current_version}:{modified_at}:{size_bytes}")
}

/// The server version a Finder content modify is based on, from the content
/// version the system held when the user saved.
///
/// Builds before the server-version-led format told the system identifiers
/// led by `max(current_version, remote_updated_at)`: a wall-clock second once
/// the row had been re-stamped. The system keeps such an identifier for an
/// item until it reads the item again, and sends it as the base of the next
/// write (also when it re-sends a write that failed earlier). Parsed as is,
/// that base is a timestamp the server refuses as stale.
///
/// Rule: an identifier that is EXACTLY what the old formula gives for the row
/// as it is now ([`legacy_item_identifiers`]: the old content version, with
/// the row's hash when it has one, or the old full identifier) describes the
/// row's current content, so it is based on `current_version`. Every other
/// identifier is parsed as before: its first segment is the base, and a base
/// the server does not hold is refused as stale. An old identifier for older
/// content never equals the row's current one: content changes move
/// `current_version`, and every re-stamp moves `remote_updated_at`.
pub(super) fn modify_base_version(
    version_identifier: Option<&str>,
    current: Option<(&FileEntry, &FileContractState)>,
) -> Option<i64> {
    if let (Some(identifier), Some((entry, contract))) = (version_identifier, current)
        && legacy_item_identifiers(entry, contract)
            .iter()
            .any(|legacy| legacy == identifier)
    {
        return Some(contract.current_version).filter(|version| *version > 0);
    }
    parse_base_version_number(version_identifier)
}
