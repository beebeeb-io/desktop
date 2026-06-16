//! Known-folder backup — OneDrive-style "Manage backup" (task 0797, Model 2).
//!
//! Decision 0797 chose **Model 2**: a one-way mirror, NOT an OS redirect. We do
//! NOT call `SHSetKnownFolderPath` and we never move or touch the user's real
//! Desktop / Documents / Pictures / Music / Videos / Downloads. Instead, for
//! each folder the user opts in, we create
//! `<sync_root>\Backup\<device>\<DisplayName>` and copy NEW or CHANGED files
//! from the real known folder into that vault subfolder. `<device>` is this
//! machine's hostname (the SAME value it registers with the server), so a
//! multi-device account keeps each machine's backup in its own subtree. The
//! existing enumeration scan (`watcher::scan_loop`) then encrypts + uploads
//! whatever lands under the sync root — so the only new engine code is this
//! source→vault copy loop. Originals stay untouched, so this coexists with
//! OneDrive (no redirect fight).
//!
//! We resolve the source folders to their DEFAULT, NON-redirected user-profile
//! location (`C:\Users\<u>\Desktop`, …) even when OneDrive Known-Folder-Move has
//! redirected them — see [`resolve_known_folder`].
//!
//! ## Guarantees (v1)
//! - **One-way only**: we ONLY read the real known folder and ONLY write into
//!   `<sync_root>\Backup\<device>\<DisplayName>`. We never write back to the
//!   source.
//! - **No source deletes, ever**: the source is opened read-only (copy reads).
//! - **No vault deletes (v1)**: a file removed from the source is left in the
//!   vault copy. Mirroring deletions into the vault would, via the upload
//!   scan, delete it from the user's cloud vault — a destructive action we do
//!   not take silently in v1. (TODO(0797-prune): an explicit, opt-in
//!   "remove from backup when removed locally" mode is a follow-up.)
//! - **Bounded**: files larger than [`MAX_MIRROR_FILE_BYTES`] are skipped this
//!   pass (logged as a count only) so a single huge file can't stall a tick.
//! - **Zero-knowledge logging**: we log COUNTS only — never file names, paths,
//!   or contents — at info level. (Paths only ever appear at `trace`.)
//!
//! ## Platform split
//! The **catalog** ([`KNOWN_FOLDERS`]) and the pure **diff classifier**
//! ([`classify_copy`], [`needs_copy`]) compile and unit-test on every platform.
//! The Win32 resolution ([`resolve_known_folder`]) and the actual copy loop
//! ([`mirror_enabled_known_folders`]) are `#[cfg(target_os = "windows")]` — they
//! call `SHGetKnownFolderPath`, which only exists on Windows.

// The copy-diff classifier ([`classify_copy`], [`FileStat`], [`stat_path`], the
// per-pass size cap) is consumed by the Windows mirror loop and the unit tests.
// On a NON-Windows, NON-test build those consumers are absent, so the items read
// as dead code there even though they are live on Windows + under `cargo test`.
// Scope the allow precisely so a genuinely-unused helper on Windows/test still
// warns.
#![cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]

use std::path::Path;

/// Per-file size cap for a single mirror pass. Files at or above this are
/// skipped (and re-evaluated next pass — the cap is a pass-level guard, not a
/// permanent exclusion). 4 GiB keeps a tick bounded; very large media is the
/// rare case and the user can still sync it by other means. Documented in the
/// task report so the cap is visible, not silent.
pub const MAX_MIRROR_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// A backupable Windows known folder.
///
/// `key` is the stable identifier persisted in `desktop.toml`
/// (`known_folder_backup`) and sent over IPC — it never changes. `display_name`
/// is the subfolder name created under the sync root (and shown in the UI). The
/// `folder_id` string names the `FOLDERID_*` GUID resolved on Windows; it is
/// inert on other platforms (kept here so the catalog is one cross-platform
/// list and the Windows resolver is a single match over `key`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownFolder {
    /// Stable key persisted + sent over IPC, e.g. `"documents"`.
    pub key: &'static str,
    /// Subfolder name under the sync root + UI label, e.g. `"Documents"`.
    pub display_name: &'static str,
}

/// The six known folders OneDrive offers under "Manage backup", in the order
/// the UI lists them. `Downloads` is included even though OneDrive historically
/// did not back it up — decision 0797 lists it explicitly.
pub const KNOWN_FOLDERS: &[KnownFolder] = &[
    KnownFolder { key: "desktop", display_name: "Desktop" },
    KnownFolder { key: "documents", display_name: "Documents" },
    KnownFolder { key: "pictures", display_name: "Pictures" },
    KnownFolder { key: "music", display_name: "Music" },
    KnownFolder { key: "videos", display_name: "Videos" },
    KnownFolder { key: "downloads", display_name: "Downloads" },
];

/// Look up a known folder by its stable key. `None` for an unknown key (e.g. a
/// stale entry in a hand-edited config).
pub fn known_folder_by_key(key: &str) -> Option<&'static KnownFolder> {
    KNOWN_FOLDERS.iter().find(|f| f.key == key)
}

// ── Pure diff classifier (cross-platform, unit-tested everywhere) ────────────

/// The decision for one source file in a mirror pass. Kept as a pure value so
/// the new/changed/skip logic is testable without touching the filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyDecision {
    /// No vault copy exists yet — copy it (a NEW file).
    New,
    /// A vault copy exists but the source differs (size or mtime) — overwrite
    /// it (a CHANGED file).
    Changed,
    /// Vault copy matches the source (same size + mtime) — leave it (UNCHANGED).
    Skip,
    /// Source is at/over the per-pass size cap — defer to a later pass.
    TooLarge,
}

/// Minimal stat used by the classifier: the comparison is mtime + size, the
/// same cheap signal rsync/OneDrive use. (Content hashing would be correct but
/// far more expensive; for a one-way mirror feeding an upload scan, mtime+size
/// is the right tradeoff — a false "Skip" only delays propagation until the
/// next real change, and the upload path itself de-dupes by content.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    pub size: u64,
    /// Modification time as whole seconds since the Unix epoch. Whole seconds
    /// (not nanos) so a copy that rounds the mtime still compares equal.
    pub mtime_secs: i64,
}

/// Classify one source file against the vault copy's stat (if any).
///
/// - `src`            — the source file's stat.
/// - `dst`            — the vault copy's stat, or `None` if it doesn't exist.
/// - `max_file_bytes` — per-pass size cap (see [`MAX_MIRROR_FILE_BYTES`]).
///
/// Pure + total: no I/O, no panics. This is the unit-tested heart of the mirror.
pub fn classify_copy(src: FileStat, dst: Option<FileStat>, max_file_bytes: u64) -> CopyDecision {
    if src.size >= max_file_bytes {
        return CopyDecision::TooLarge;
    }
    match dst {
        None => CopyDecision::New,
        Some(d) if d.size == src.size && d.mtime_secs == src.mtime_secs => CopyDecision::Skip,
        Some(_) => CopyDecision::Changed,
    }
}

/// `true` if [`classify_copy`] says a byte copy is needed (New or Changed).
pub fn needs_copy(decision: CopyDecision) -> bool {
    matches!(decision, CopyDecision::New | CopyDecision::Changed)
}

// ── Backup destination path (cross-platform, unit-tested everywhere) ─────────

/// Sanitize a raw device name into a single safe path segment for the backup
/// destination `<sync_root>\Backup\<device>\<DisplayName>`.
///
/// The device name comes from `runner::hostname_or_unknown()` — normally a tame
/// hostname, but we never trust it to be path-safe. We collapse it to a single
/// segment that cannot escape its parent or split into nested folders:
///
/// - any path separator (`/` or `\`) becomes `_` (no nesting, no traversal);
/// - any other path-special / control char (`:`, `*`, `?`, `"`, `<`, `>`, `|`,
///   and ASCII control bytes) becomes `_` (so the segment is a valid Windows
///   filename);
/// - leading/trailing dots and whitespace are trimmed (Windows strips trailing
///   dots/spaces, and a leading dot would hide the folder);
/// - the reserved `.`/`..` traversal names, or an empty result, fall back to
///   the literal `"device"`.
///
/// Pure + total: no I/O, no panics. Unit-tested on every platform.
pub fn sanitize_device_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let mapped = match ch {
            '/' | '\\' => '_',
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        };
        out.push(mapped);
    }
    // Trim leading/trailing dots and whitespace (a leading '.' would hide the
    // folder; trailing dots/spaces are stripped by the Windows filesystem and
    // would otherwise create a name that doesn't round-trip).
    let trimmed = out.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return "device".to_string();
    }
    trimmed.to_string()
}

/// Build the vault destination root for one known folder:
/// `<sync_root>\Backup\<sanitized_device>\<display_name>`.
///
/// Factored cross-platform (pure `Path` joins) so the path layout — and the
/// device-name sanitization feeding it — is unit-testable without Windows. The
/// Windows mirror loop calls this with `runner::hostname_or_unknown()` as
/// `device_name`; the same value this device registers with the server.
pub fn backup_dest_root(sync_root: &Path, device_name: &str, display_name: &str) -> std::path::PathBuf {
    sync_root
        .join("Backup")
        .join(sanitize_device_segment(device_name))
        .join(display_name)
}

/// Read a `FileStat` from a path, mapping any error (missing file, permission)
/// to `None`. Cross-platform; used by the Windows mirror loop and by tests.
pub fn stat_path(path: &Path) -> Option<FileStat> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mtime_secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some(FileStat { size: meta.len(), mtime_secs })
}

// ── Windows: resolve + mirror ────────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::*;
    use std::path::PathBuf;

    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music,
        FOLDERID_Pictures, FOLDERID_Videos, KF_FLAG_DEFAULT_PATH, KF_FLAG_DONT_VERIFY,
        SHGetKnownFolderPath,
    };
    use windows::core::GUID;

    /// Map a known-folder key to its Win32 `FOLDERID_*` GUID. `None` for an
    /// unknown key.
    fn folder_id_for(key: &str) -> Option<GUID> {
        match key {
            "desktop" => Some(FOLDERID_Desktop),
            "documents" => Some(FOLDERID_Documents),
            "pictures" => Some(FOLDERID_Pictures),
            "music" => Some(FOLDERID_Music),
            "videos" => Some(FOLDERID_Videos),
            "downloads" => Some(FOLDERID_Downloads),
            _ => None,
        }
    }

    /// Resolve a known folder's DEFAULT user-profile path on this machine via
    /// `SHGetKnownFolderPath`. Returns `None` for an unknown key or if the
    /// shell call fails (e.g. the folder is not provisioned for this user).
    ///
    /// **Default (non-redirected) location, by design (founder fix).** We pass
    /// `KF_FLAG_DEFAULT_PATH` (NOT `KF_FLAG_DEFAULT`) so the call returns the
    /// known folder's DEFAULT location *ignoring* any OneDrive Known-Folder-Move
    /// redirection. With plain `KF_FLAG_DEFAULT`, an account with OneDrive KFM
    /// enabled resolves Desktop/Documents/Pictures to
    /// `C:\Users\<u>\OneDrive - <org>\Desktop` etc.; `KF_FLAG_DEFAULT_PATH`
    /// resolves the un-redirected `C:\Users\<u>\Desktop`. This stays
    /// localization-safe — it is still the known-folder API resolving the right
    /// per-user path, NOT a hard-coded `"Desktop"` string. We OR in
    /// `KF_FLAG_DONT_VERIFY` so the call still returns the default path on a
    /// machine where that directory does not physically exist (without it the
    /// shell verifies existence and can fail). The flag pair is a no-op for the
    /// never-redirected Music/Videos/Downloads, so all six folders use it
    /// uniformly.
    ///
    /// Memory: `SHGetKnownFolderPath` allocates the path string with
    /// `CoTaskMemAlloc`; we copy it into a Rust `String` and then `CoTaskMemFree`
    /// the original, so nothing leaks.
    pub fn resolve_known_folder(key: &str) -> Option<PathBuf> {
        let rfid = folder_id_for(key)?;
        // SAFETY: `rfid` is a valid &GUID; `KF_FLAG_DEFAULT_PATH | KF_FLAG_DONT_VERIFY`
        // is a valid `KNOWN_FOLDER_FLAG` bitset (`BitOr` is implemented), and the
        // null (`HANDLE::default()`) access token is the documented "current
        // user" argument — exactly what we want. On success the out-param holds a
        // CoTaskMem-allocated NUL-terminated wide string we own and must free
        // with CoTaskMemFree. (We pass an explicit `HANDLE::default()` rather
        // than a bare `None` so the `P0: Param<HANDLE>` type is unambiguous.)
        unsafe {
            let pwstr =
                SHGetKnownFolderPath(&rfid, KF_FLAG_DEFAULT_PATH | KF_FLAG_DONT_VERIFY, HANDLE::default())
                    .ok()?;
            if pwstr.is_null() {
                return None;
            }
            let s = pwstr.to_string().ok();
            CoTaskMemFree(Some(pwstr.0 as *const _));
            s.map(PathBuf::from)
        }
    }

    /// Counts from one mirror pass — what we log (and only this; never names).
    #[derive(Debug, Default, Clone, Copy)]
    pub struct MirrorStats {
        pub copied_new: usize,
        pub copied_changed: usize,
        pub skipped_unchanged: usize,
        pub skipped_too_large: usize,
        pub errors: usize,
    }

    impl MirrorStats {
        fn add(&mut self, other: MirrorStats) {
            self.copied_new += other.copied_new;
            self.copied_changed += other.copied_changed;
            self.skipped_unchanged += other.skipped_unchanged;
            self.skipped_too_large += other.skipped_too_large;
            self.errors += other.errors;
        }
        fn touched(&self) -> bool {
            self.copied_new + self.copied_changed + self.skipped_too_large + self.errors > 0
        }
    }

    /// Mirror every enabled known folder into the sync root. One-way:
    /// source → `<sync_root>\Backup\<device>\<DisplayName>`. Best-effort +
    /// idempotent — a per-file error logs (count only) and the pass continues.
    ///
    /// `<device>` is `runner::hostname_or_unknown()` (the SAME value this device
    /// registers with the server) sanitized to one safe path segment, so a
    /// multi-device account keeps each machine's backup in its own subtree under
    /// `Backup/`.
    ///
    /// Safety invariants enforced here:
    /// - We only ever WRITE under `<sync_root>\Backup\<device>\<DisplayName>`
    ///   (`dest_root`), built from the trusted sync root + the literal `Backup`
    ///   segment + a sanitized single-segment device name + a hard-coded display
    ///   name. None of those components can traverse out of the sync root.
    /// - We never write to, rename, or delete anything under the SOURCE.
    /// - We refuse to mirror a source that resolves to (or under) the sync root
    ///   itself, which would be a copy-into-itself loop. This also covers a
    ///   source that somehow resolves under `<sync_root>\Backup\...` — it is
    ///   under the sync root, so the same guard refuses it.
    pub fn mirror_enabled_known_folders(sync_root: &Path, enabled_keys: &[String]) {
        if enabled_keys.is_empty() {
            return;
        }
        // Resolve the device name ONCE per pass: the SAME hostname this device
        // registers with the server, sanitized inside `backup_dest_root` to a
        // single safe path segment.
        let device_name = crate::runner::hostname_or_unknown();
        let mut total = MirrorStats::default();
        for key in enabled_keys {
            let Some(kf) = known_folder_by_key(key) else {
                continue;
            };
            let Some(source) = resolve_known_folder(key) else {
                tracing::debug!(folder = %kf.key, "known-folder backup: source not resolvable; skipping");
                continue;
            };
            // Refuse a self-referential mirror (source inside the sync root, or
            // the sync root inside the source) — either would recurse. A source
            // under `<sync_root>\Backup\...` is under `sync_root`, so this guard
            // also refuses backing up the backup.
            if source.starts_with(sync_root) || sync_root.starts_with(&source) {
                tracing::warn!(folder = %kf.key, "known-folder backup: source overlaps sync root; skipping");
                continue;
            }
            let dest_root = backup_dest_root(sync_root, &device_name, kf.display_name);
            if let Err(e) = std::fs::create_dir_all(&dest_root) {
                tracing::warn!(folder = %kf.key, error = %e, "known-folder backup: could not create vault folder");
                total.errors += 1;
                continue;
            }
            let stats = mirror_one_folder(&source, &dest_root);
            total.add(stats);
        }

        // Zero-knowledge: counts ONLY. Quiet when nothing changed.
        if total.touched() {
            tracing::info!(
                copied_new = total.copied_new,
                copied_changed = total.copied_changed,
                skipped_too_large = total.skipped_too_large,
                errors = total.errors,
                "known-folder backup mirror pass complete"
            );
        } else {
            tracing::debug!(
                skipped_unchanged = total.skipped_unchanged,
                "known-folder backup mirror pass: nothing to copy"
            );
        }
    }

    /// Mirror a single `source` tree into `dest_root` (already created). Walks
    /// the source recursively (bounded depth, no symlink follow), classifies
    /// each file, and copies New/Changed ones. Pure `std::fs` so it never needs
    /// the Cloud Files API — the file simply appears under the sync root and the
    /// enumeration scan picks it up.
    fn mirror_one_folder(source: &Path, dest_root: &Path) -> MirrorStats {
        let mut stats = MirrorStats::default();
        walk_and_mirror(source, source, dest_root, 0, &mut stats);
        stats
    }

    /// Bound recursion the same way the upload scan does (64 levels) so a
    /// pathological tree can't blow the stack or stall a tick forever.
    const MAX_MIRROR_DEPTH: usize = 64;

    fn walk_and_mirror(src_root: &Path, src_dir: &Path, dest_root: &Path, depth: usize, stats: &mut MirrorStats) {
        if depth > MAX_MIRROR_DEPTH {
            return;
        }
        let entries = match std::fs::read_dir(src_dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::trace!(error = %e, "known-folder backup: read_dir failed");
                stats.errors += 1;
                return;
            }
        };
        for entry in entries.flatten() {
            let src_path = entry.path();
            // file_type() avoids a follow; we skip symlinks so the mirror can't
            // be walked out of the source tree via a link.
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            // Reconstruct the relative path so the vault mirror preserves the
            // source's subfolder structure.
            let Ok(rel) = src_path.strip_prefix(src_root) else {
                continue;
            };
            let dest_path = dest_root.join(rel);

            if ft.is_dir() {
                if let Err(e) = std::fs::create_dir_all(&dest_path) {
                    tracing::trace!(error = %e, "known-folder backup: mkdir failed");
                    stats.errors += 1;
                    continue;
                }
                walk_and_mirror(src_root, &src_path, dest_root, depth + 1, stats);
                continue;
            }
            if !ft.is_file() {
                continue;
            }

            let Some(src_stat) = stat_path(&src_path) else {
                continue;
            };
            let dst_stat = stat_path(&dest_path);
            match classify_copy(src_stat, dst_stat, MAX_MIRROR_FILE_BYTES) {
                CopyDecision::New | CopyDecision::Changed => {
                    let is_new = dst_stat.is_none();
                    // SAFETY: dest_path is `dest_root` (under the sync root) joined
                    // with a relative path strip_prefix'd from the source, so the
                    // copy can only ever write inside the vault folder.
                    match copy_preserving_mtime(&src_path, &dest_path) {
                        Ok(()) => {
                            if is_new {
                                stats.copied_new += 1;
                            } else {
                                stats.copied_changed += 1;
                            }
                        }
                        Err(e) => {
                            tracing::trace!(error = %e, "known-folder backup: copy failed");
                            stats.errors += 1;
                        }
                    }
                }
                CopyDecision::Skip => stats.skipped_unchanged += 1,
                CopyDecision::TooLarge => stats.skipped_too_large += 1,
            }
        }
    }

    /// Copy `src` → `dst`, then stamp `dst`'s mtime to match `src` so the next
    /// pass's mtime+size compare sees them as equal (otherwise every pass would
    /// re-copy unchanged files). Best-effort on the mtime stamp — a failure
    /// there only causes one redundant re-copy next pass, never data loss.
    fn copy_preserving_mtime(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::fs::copy(src, dst)?;
        if let Ok(meta) = std::fs::metadata(src) {
            if let Ok(mtime) = meta.modified() {
                let ft = filetime_from_systemtime(mtime);
                let _ = set_file_mtime(dst, ft);
            }
        }
        Ok(())
    }

    /// Set a file's modification time. Implemented with the Windows
    /// `SetFileTime` API (no extra crate) — converts a Unix-epoch `SystemTime`
    /// to a Windows `FILETIME` (100ns ticks since 1601-01-01).
    fn set_file_mtime(path: &Path, ft: FileTimeTicks) -> std::io::Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, SetFileTime};

        let file = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(path)?;
        let handle = windows::Win32::Foundation::HANDLE(
            std::os::windows::io::AsRawHandle::as_raw_handle(&file) as _,
        );
        let filetime = FILETIME {
            dwLowDateTime: (ft.0 & 0xFFFF_FFFF) as u32,
            dwHighDateTime: (ft.0 >> 32) as u32,
        };
        // SAFETY: handle is valid (owned by `file`, alive for the call). We pass
        // the same FILETIME for last-write; creation/access left unchanged (None).
        unsafe { SetFileTime(handle, None, None, Some(&filetime)).map_err(std::io::Error::other) }
    }

    /// Windows FILETIME as a single 64-bit tick count (100ns since 1601).
    #[derive(Clone, Copy)]
    struct FileTimeTicks(u64);

    /// 100ns intervals between 1601-01-01 and 1970-01-01.
    const EPOCH_DIFF_100NS: u64 = 116_444_736_000_000_000;

    fn filetime_from_systemtime(t: std::time::SystemTime) -> FileTimeTicks {
        let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let ticks = dur.as_secs() * 10_000_000 + (dur.subsec_nanos() as u64) / 100;
        FileTimeTicks(ticks + EPOCH_DIFF_100NS)
    }
}

#[cfg(target_os = "windows")]
pub use windows_impl::{mirror_enabled_known_folders, resolve_known_folder};

// ── Tests (pure diff logic + catalog — run on every platform) ────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_six_unique_keys() {
        assert_eq!(KNOWN_FOLDERS.len(), 6);
        let mut keys: Vec<&str> = KNOWN_FOLDERS.iter().map(|f| f.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 6, "keys must be unique");
        // Lookups resolve.
        assert_eq!(known_folder_by_key("documents").map(|f| f.display_name), Some("Documents"));
        assert!(known_folder_by_key("nope").is_none());
    }

    #[test]
    fn classify_new_when_no_dest() {
        let src = FileStat { size: 100, mtime_secs: 1_700_000_000 };
        assert_eq!(classify_copy(src, None, MAX_MIRROR_FILE_BYTES), CopyDecision::New);
        assert!(needs_copy(classify_copy(src, None, MAX_MIRROR_FILE_BYTES)));
    }

    #[test]
    fn classify_skip_when_size_and_mtime_match() {
        let src = FileStat { size: 100, mtime_secs: 1_700_000_000 };
        let dst = FileStat { size: 100, mtime_secs: 1_700_000_000 };
        let d = classify_copy(src, Some(dst), MAX_MIRROR_FILE_BYTES);
        assert_eq!(d, CopyDecision::Skip);
        assert!(!needs_copy(d));
    }

    #[test]
    fn classify_changed_on_size_diff() {
        let src = FileStat { size: 101, mtime_secs: 1_700_000_000 };
        let dst = FileStat { size: 100, mtime_secs: 1_700_000_000 };
        assert_eq!(classify_copy(src, Some(dst), MAX_MIRROR_FILE_BYTES), CopyDecision::Changed);
    }

    #[test]
    fn classify_changed_on_mtime_diff() {
        let src = FileStat { size: 100, mtime_secs: 1_700_000_500 };
        let dst = FileStat { size: 100, mtime_secs: 1_700_000_000 };
        assert_eq!(classify_copy(src, Some(dst), MAX_MIRROR_FILE_BYTES), CopyDecision::Changed);
    }

    #[test]
    fn classify_too_large_at_or_over_cap() {
        let cap = 1_000;
        let at_cap = FileStat { size: 1_000, mtime_secs: 1 };
        let over_cap = FileStat { size: 5_000, mtime_secs: 1 };
        let under_cap = FileStat { size: 999, mtime_secs: 1 };
        assert_eq!(classify_copy(at_cap, None, cap), CopyDecision::TooLarge);
        assert_eq!(classify_copy(over_cap, None, cap), CopyDecision::TooLarge);
        assert_eq!(classify_copy(under_cap, None, cap), CopyDecision::New);
        // Size cap wins even over a would-be Skip.
        assert_eq!(
            classify_copy(at_cap, Some(at_cap), cap),
            CopyDecision::TooLarge
        );
    }

    // ── Backup destination path + device-name sanitization ───────────────────
    //
    // The Win32 source resolver (`resolve_known_folder`) is `cfg(windows)`-only,
    // so it is not exercised here; the path LAYOUT and the device-name
    // sanitization that feed the destination are cross-platform and ARE tested.

    #[test]
    fn dest_root_builds_backup_device_folder() {
        // `<sync_root>/Backup/<device>/<DisplayName>`, in that order.
        let sync_root = Path::new("/sync");
        let dest = backup_dest_root(sync_root, "my-laptop", "Documents");
        // Compare component-wise so the test is separator-agnostic across OSes.
        let comps: Vec<_> = dest
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        assert!(comps.iter().any(|c| c == "Backup"), "must contain a Backup segment: {comps:?}");
        // The tail is …/Backup/my-laptop/Documents.
        let n = comps.len();
        assert_eq!(comps[n - 3], "Backup");
        assert_eq!(comps[n - 2], "my-laptop");
        assert_eq!(comps[n - 1], "Documents");
    }

    #[test]
    fn dest_root_sanitizes_device_into_one_segment() {
        // A device name carrying separators / traversal must NOT add nesting or
        // escape upward — it collapses to a single sanitized segment.
        let dest = backup_dest_root(Path::new("/sync"), "../../evil\\name", "Pictures");
        let comps: Vec<_> = dest
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let n = comps.len();
        // Still exactly Backup / <one device segment> / Pictures.
        assert_eq!(comps[n - 3], "Backup");
        assert_eq!(comps[n - 1], "Pictures");
        let device_seg = &comps[n - 2];
        assert!(!device_seg.contains('/'), "no '/' in device segment: {device_seg}");
        assert!(!device_seg.contains('\\'), "no '\\\\' in device segment: {device_seg}");
        assert_ne!(device_seg, "..", "device segment must never be the traversal token");
        // Each separator became '_', so the traversal `../../` flattens to
        // `.._.._` (no real `..` component survives); the leading dots are then
        // trimmed and the tail `evil\name` becomes `evil_name`, yielding the
        // single inert segment `_.._evil_name` — it cannot walk up or split into
        // nested dirs.
        assert_eq!(device_seg, "_.._evil_name");
        assert!(!device_seg.contains(std::path::MAIN_SEPARATOR), "single inert segment");
    }

    #[test]
    fn sanitize_replaces_separators_and_path_specials() {
        assert_eq!(sanitize_device_segment("a/b\\c"), "a_b_c");
        // Windows-reserved filename chars all map to '_'.
        assert_eq!(sanitize_device_segment("a:b*c?d\"e<f>g|h"), "a_b_c_d_e_f_g_h");
        // A plain hostname is preserved verbatim.
        assert_eq!(sanitize_device_segment("Guus-PC"), "Guus-PC");
    }

    #[test]
    fn sanitize_strips_control_chars() {
        // A NUL / control byte (e.g. from a corrupt hostname) becomes '_'.
        let raw = "host\u{0}name\u{1f}";
        // Trailing control char is mapped to '_' (not whitespace), so it stays.
        assert_eq!(sanitize_device_segment(raw), "host_name_");
    }

    #[test]
    fn sanitize_trims_leading_trailing_dots_and_spaces() {
        // Leading dot would hide the folder; trailing dots/spaces are stripped
        // by Windows anyway.
        assert_eq!(sanitize_device_segment("  .hidden.  "), "hidden");
        assert_eq!(sanitize_device_segment("name..."), "name");
        assert_eq!(sanitize_device_segment("...name"), "name");
    }

    #[test]
    fn sanitize_falls_back_to_device_on_empty_or_traversal() {
        assert_eq!(sanitize_device_segment(""), "device");
        assert_eq!(sanitize_device_segment("   "), "device");
        assert_eq!(sanitize_device_segment("."), "device");
        assert_eq!(sanitize_device_segment(".."), "device");
        // A name that is ONLY dots/spaces trims to empty → fallback.
        assert_eq!(sanitize_device_segment(". . ."), "device");
    }

    #[test]
    fn stat_path_reads_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, b"hello").unwrap();
        let s = stat_path(&f).expect("file should stat");
        assert_eq!(s.size, 5);
        // A directory is not a file → None.
        assert!(stat_path(dir.path()).is_none());
        // Missing → None.
        assert!(stat_path(&dir.path().join("missing")).is_none());
    }
}
