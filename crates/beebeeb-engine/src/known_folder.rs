//! The known folders a backup can be turned on for (Windows' Desktop, Documents, Pictures, Music, Videos and
//! Downloads), and what is left of the old mirror's layout.
//!
//! Until LD-0113 a turned-on folder was mirrored, one way, into `<sync_root>\Backups\<device>\<DisplayName>`
//! (task 0797) and uploaded from there. The mirror is gone: a backup now reads each folder in place and uploads
//! into `Backups/<device>/<category>` on the server (`crate::backup`), and nothing is copied into the sync root.
//! What stays here:
//!
//! - the catalog ([`KNOWN_FOLDERS`]): the keys persisted in `desktop.toml` (`known_folder_backup`), which are core's
//!   `Category::key()`s, and their display names;
//! - [`resolve_known_folder`] (Windows): a folder's default, un-redirected location;
//! - the old mirror's paths: [`sanitize_device_segment`] named `Backups/<device>` (core's identity rule adopts that
//!   folder, `backup::device`), and the `backup_source_key_*` functions still tag queue ops for files under it.

/// A backupable Windows known folder.
///
/// `key` is the stable identifier persisted in `desktop.toml`
/// (`known_folder_backup`) and sent over IPC — it never changes. `display_name`
/// is the category folder's name under `Backups/<device>` (and the UI label). The
/// `folder_id` string names the `FOLDERID_*` GUID resolved on Windows; it is
/// inert on other platforms (kept here so the catalog is one cross-platform
/// list and the Windows resolver is a single match over `key`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownFolder {
    /// Stable key persisted + sent over IPC, e.g. `"documents"`.
    pub key: &'static str,
    /// The category folder's name and the UI label, e.g. `"Documents"`.
    pub display_name: &'static str,
}

/// The six known folders OneDrive offers under "Manage backup", in the order
/// the UI lists them. `Downloads` is included even though OneDrive historically
/// did not back it up — decision 0797 lists it explicitly.
pub const KNOWN_FOLDERS: &[KnownFolder] = &[
    KnownFolder {
        key: "desktop",
        display_name: "Desktop",
    },
    KnownFolder {
        key: "documents",
        display_name: "Documents",
    },
    KnownFolder {
        key: "pictures",
        display_name: "Pictures",
    },
    KnownFolder {
        key: "music",
        display_name: "Music",
    },
    KnownFolder {
        key: "videos",
        display_name: "Videos",
    },
    KnownFolder {
        key: "downloads",
        display_name: "Downloads",
    },
];

/// Look up a known folder by its stable key. `None` for an unknown key (e.g. a
/// stale entry in a hand-edited config).
pub fn known_folder_by_key(key: &str) -> Option<&'static KnownFolder> {
    KNOWN_FOLDERS.iter().find(|f| f.key == key)
}

/// Reverse of the catalog: resolve a known folder's stable `key` from the
/// `display_name` segment the mirror writes into the vault path. `None` for a
/// name that is not one of the catalog display names.
fn known_folder_by_display_name(display_name: &str) -> Option<&'static KnownFolder> {
    KNOWN_FOLDERS.iter().find(|f| f.display_name == display_name)
}

/// Decide which known-folder backup (if any) a queued upload/folder op belongs
/// to, from its server-relative `rel_path` (the `/`-joined key the upload path
/// stores and threads as `target_path`). Backup files live under
/// `Backups/<device>/<DisplayName>/<subdirs…>`; we match that exact prefix shape
/// AND require segment 2 to equal `expected_device` (the sanitized device
/// segment THIS machine's mirror writes — see [`sanitize_device_segment`]), then
/// reverse-map `<DisplayName>` to its catalog key. Returns the key (e.g.
/// `"music"`) for a backup-originated path, or `None` for any normal (non-backup)
/// path.
///
/// **Device check (task 0811 review fix).** Without it, ANY vault path of shape
/// `Backups/<anySegment>/<CatalogDisplayName>/…` matched — so a user who manually
/// created e.g. `Backups/Foo/Documents/notes.txt` in their own vault would have
/// that upload tagged `documents`, and disabling the Documents known-folder
/// backup would then PURGE their unrelated pending upload. Pinning segment 2 to
/// this device's own sanitized name (the only value the mirror ever writes)
/// eliminates that false-positive: a user folder under a different second
/// segment no longer collides, and a path that happens to reuse the real device
/// name AND a catalog display name is the mirror's own subtree anyway.
///
/// Tagging at the path layer (not threaded through every enqueue call site) keeps
/// the change surgical: every backup op already carries its full `rel_path`, and
/// the catalog is the single source of truth for which display names are ours.
///
/// Pure + total: a leading `/` is tolerated; case-sensitive segment compares
/// (the mirror writes exact catalog display names + a fixed device segment).
/// Unit-tested on every platform.
pub fn backup_source_key_for_rel_path(rel_path: &str, expected_device: &str) -> Option<String> {
    let mut segments = rel_path.trim_start_matches('/').split('/');
    // Segment 1 must be the literal `Backups` root the mirror writes under.
    if segments.next()? != "Backups" {
        return None;
    }
    // Segment 2 must be THIS device's sanitized name — the only value the mirror
    // writes. Rejecting any other segment is what prevents a user's own
    // `Backups/<other>/<CatalogName>/…` file from being mis-tagged (and then
    // wrongly purged on a folder disable).
    if segments.next()? != expected_device {
        return None;
    }
    let display_name = segments.next()?;
    // Segment 4+ (the file or its subdirs) must exist — a path that stops at the
    // display-name folder itself is the folder root, still ours to tag.
    known_folder_by_display_name(display_name).map(|kf| kf.key.to_string())
}

/// Convenience wrapper over [`backup_source_key_for_rel_path`] that resolves
/// `expected_device` from this machine's name, `runner::device_name()` (the SAME
/// value the mirror writes under `Backups/<device>/…`), sanitized to a single
/// segment. Use this at op-enqueue and purge-decision sites; the explicit-device
/// form stays pure for unit tests.
pub fn backup_source_key_for_this_device(rel_path: &str) -> Option<String> {
    // macOS keeps its fallback name (spec §2: no macOS change). It has no backup mirror, and the tag only drives the
    // purge when a backup is turned off.
    #[cfg(target_os = "macos")]
    let device = Some(crate::runner::hostname_or_unknown());
    #[cfg(not(target_os = "macos"))]
    let device = crate::runner::device_name();
    backup_source_key_for_device(rel_path, device.as_deref())
}

/// Spec D9 (tasks 1888, 2226): `None` when this PC's name cannot be read. The mirror makes no backup folder then, so
/// no path is one, `Backups/unknown` included.
fn backup_source_key_for_device(rel_path: &str, device: Option<&str>) -> Option<String> {
    backup_source_key_for_rel_path(rel_path, &sanitize_device_segment(device?))
}

// ── The old mirror's device segment ────────────────────────────────────────

mod segment;
pub use segment::sanitize_device_segment;

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::path::PathBuf;

    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Videos,
        KF_FLAG_DEFAULT_PATH, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath,
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
        resolve_with(key, KF_FLAG_DEFAULT_PATH | KF_FLAG_DONT_VERIFY)
    }

    /// LD-0113 Amendment 9: the known folder's CURRENT location, OneDrive's Known Folder Move redirection included
    /// (`KF_FLAG_DEFAULT`, with `KF_FLAG_DONT_VERIFY`). Where it differs from [`resolve_known_folder`] the folder was
    /// moved, and the person is asked once which one to back up.
    pub fn resolve_known_folder_current(key: &str) -> Option<PathBuf> {
        resolve_with(key, KF_FLAG_DONT_VERIFY)
    }

    fn resolve_with(key: &str, flags: windows::Win32::UI::Shell::KNOWN_FOLDER_FLAG) -> Option<PathBuf> {
        let rfid = folder_id_for(key)?;
        // SAFETY: `rfid` is a valid &GUID; `KF_FLAG_DEFAULT_PATH | KF_FLAG_DONT_VERIFY`
        // is a valid `KNOWN_FOLDER_FLAG` bitset (`BitOr` is implemented), and the
        // null (`HANDLE::default()`) access token is the documented "current
        // user" argument — exactly what we want. On success the out-param holds a
        // CoTaskMem-allocated NUL-terminated wide string we own and must free
        // with CoTaskMemFree. (We pass an explicit `HANDLE::default()` rather
        // than a bare `None` so the `P0: Param<HANDLE>` type is unambiguous.)
        unsafe {
            let pwstr = SHGetKnownFolderPath(&rfid, flags, HANDLE::default()).ok()?;
            if pwstr.is_null() {
                return None;
            }
            let s = pwstr.to_string().ok();
            CoTaskMemFree(Some(pwstr.0 as *const _));
            s.map(PathBuf::from)
        }
    }
}

#[cfg(target_os = "windows")]
pub use windows_impl::{resolve_known_folder, resolve_known_folder_current};

// ── Tests (catalog, legacy paths, sanitizer — every platform) ───────────────

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
        assert_eq!(
            known_folder_by_key("documents").map(|f| f.display_name),
            Some("Documents")
        );
        assert!(known_folder_by_key("nope").is_none());
    }

    #[test]
    fn backup_source_key_maps_vault_paths_to_origin_key() {
        // A nested backup file under THIS device resolves to its catalog key.
        assert_eq!(
            backup_source_key_for_rel_path("Backups/Legion/Music/Artist/Album/song.mp3", "Legion").as_deref(),
            Some("music")
        );
        // The display-name folder root itself is still tagged.
        assert_eq!(
            backup_source_key_for_rel_path("Backups/Legion/Pictures", "Legion").as_deref(),
            Some("pictures")
        );
        // A leading slash is tolerated.
        assert_eq!(
            backup_source_key_for_rel_path("/Backups/My-PC/Documents/cv.pdf", "My-PC").as_deref(),
            Some("documents")
        );
    }

    #[test]
    fn backup_source_key_rejects_non_backup_paths() {
        // A normal top-level user file.
        assert_eq!(backup_source_key_for_rel_path("report.pdf", "Legion"), None);
        // A user folder that merely happens to be named `Backups` but doesn't
        // match the device/display-name shape (display name isn't a catalog one).
        assert_eq!(
            backup_source_key_for_rel_path("Backups/Legion/Holiday/pic.jpg", "Legion"),
            None
        );
        // `Backups` with too few segments to identify a folder.
        assert_eq!(backup_source_key_for_rel_path("Backups/Legion", "Legion"), None);
        assert_eq!(backup_source_key_for_rel_path("Backups", "Legion"), None);
        // A different first segment.
        assert_eq!(
            backup_source_key_for_rel_path("Documents/Music/song.mp3", "Legion"),
            None
        );
    }

    #[test]
    fn backup_source_key_requires_this_devices_segment() {
        // Review fix: a user-created `Backups/<other>/<CatalogName>/…` path under a
        // DIFFERENT device segment must NOT be tagged — otherwise disabling that
        // catalog folder would purge the user's unrelated upload. Only the real
        // device segment this machine's mirror writes qualifies.
        assert_eq!(
            backup_source_key_for_rel_path("Backups/SomeoneElse/Documents/notes.txt", "Legion"),
            None
        );
        // Same path WITH the matching device segment is the mirror's own subtree
        // and IS tagged.
        assert_eq!(
            backup_source_key_for_rel_path("Backups/Legion/Documents/notes.txt", "Legion").as_deref(),
            Some("documents")
        );
    }

    /// Spec D9 (tasks 1888, 2226): with no readable name this PC has no backup folder, so no path is tagged as one,
    /// `Backups/unknown` included.
    #[test]
    fn an_unreadable_hostname_tags_no_backup_path() {
        assert_eq!(
            backup_source_key_for_device("Backups/unknown/Documents/a.txt", None),
            None
        );
        assert_eq!(backup_source_key_for_device("Backups/PC-1/Documents/a.txt", None), None);
        assert_eq!(
            backup_source_key_for_device("Backups/PC-1/Documents/a.txt", Some("PC-1")).as_deref(),
            Some("documents")
        );
    }

    /// Spec D9: when this PC's name cannot be read, the mirror makes nothing under the sync root, so a lossy name
    /// never starts a `Backups/unknown` folder. Windows only: the mirror resolves the real known folders.
    #[cfg(target_os = "windows")]
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
}
