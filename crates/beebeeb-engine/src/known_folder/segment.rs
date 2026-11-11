//! The one pure part of the desktop's `known_folder`: a device name as a safe path segment. Moved out of
//! `src-tauri/src/known_folder.rs` unchanged; the Windows known-folder lookups stay there.

/// Sanitize a raw device name into a single safe path segment for the backup
/// destination `<sync_root>\Backups\<device>\<DisplayName>`.
///
/// The device name comes from `runner::device_name()` — normally a tame
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
