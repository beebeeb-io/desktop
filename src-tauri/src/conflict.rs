//! Conflict detection primitives — pure functions over `VersionInfo`s,
//! plus the policy timestamps Task 13's auto-resolution timer reads.
//!
//! Spec: `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`
//! Phase 4 Task 10 + Phase 5 Task 13.
//!
//! Nothing in this module touches I/O or state. The point is to keep
//! the conflict definition unit-testable without a SQLite/HTTP test
//! harness — any "is this divergence a conflict?" question stays a
//! pure-function decision. Side-effecting wire-up (set status, emit
//! events, call open_conflict_window) lives in `engine_bridge` /
//! `runner` which import these primitives.
//!
//! ## What counts as a conflict
//!
//! A file has a conflict when **both** local and remote have moved
//! past the last shared base version, in different directions:
//!
//! - `local.hash != base.hash` → local user touched it.
//! - `remote.hash != base.hash` → another device touched it.
//! - `local.hash != remote.hash` → those touches diverged (otherwise
//!   the two devices coincidentally produced the same edit and there's
//!   nothing to resolve).
//!
//! We never auto-pick a winner — divergent edits go through the user.
//! Auto-resolution is a *time-based fallback* (Task 13) that materialises
//! a "Keep Both" if the user ignores the conflict for 24 h, so we don't
//! leave the file in a status that blocks sync forever.

/// One side's view of a file at a given moment in time.
///
/// `hash` is whatever opaque blob the daemon uses to represent
/// "version identity" — for the local side this is the plaintext
/// content hash from `state_db::FileEntry::content_hash`; for the
/// remote side it's a synthetic hash derived from server metadata
/// (`updated_at` + `size_bytes`) until the API exposes a stable
/// content hash on file metadata.
///
/// `modified_at` is seconds since the Unix epoch — the value the
/// daemon stores in `state_db::FileEntry::modified_at`. It's the
/// later of "user touched local" or "remote `updated_at`", which is
/// what the auto-resolution deadline reads (see
/// [`auto_resolution_deadline`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInfo {
    pub hash: String,
    pub modified_at: u64,
}

/// `true` when both sides moved away from the last shared base AND
/// they moved to different states. The order of checks doesn't
/// matter — they all have to hold for a real conflict.
pub fn is_conflict(local: &VersionInfo, remote: &VersionInfo, base: &VersionInfo) -> bool {
    local.hash != base.hash && remote.hash != base.hash && local.hash != remote.hash
}

/// What the daemon needs to remember about an unresolved conflict.
/// Persisted indirectly via the `Conflict` status row in
/// `state_db::files` — the file_id is the key, the rest is reconstructed
/// from the row + a fresh remote fetch when the resolution UI opens.
#[derive(Debug, Clone)]
#[allow(dead_code)] // used by future engine_bridge::resolve_conflict wiring
pub struct ConflictRecord {
    pub file_id: String,
    pub file_name: String,
    pub local_version: VersionInfo,
    pub remote_version: VersionInfo,
    /// Seconds since Unix epoch. Anchors the auto-resolution clock.
    pub detected_at: u64,
}

/// User's choice from the conflict window. Mirrors the IPC string
/// values `"local"` / `"remote"` / `"both"` accepted by
/// [`crate::resolve_conflict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // used by future engine_bridge::resolve_conflict wiring
pub enum Resolution {
    KeepLocal,
    KeepRemote,
    KeepBoth,
}

/// Twenty-four hours after detection, expressed in seconds since the
/// Unix epoch. The runner's tick loop compares `now` against this —
/// when `now >= deadline` it applies "Keep Both" automatically.
///
/// The 24 h figure isn't load-bearing — it's "long enough that a user
/// who's actively using their machine will have seen the conflict
/// notification, short enough that the file isn't in limbo for days
/// if they're on holiday." Adjust if telemetry says otherwise.
pub fn auto_resolution_deadline(detected_at: u64) -> u64 {
    detected_at + 24 * 3600
}

/// Heuristic: should the conflict window render a side-by-side diff
/// view (text mode) or a side-by-side metadata card (binary mode)?
/// Picked from the file extension — same shortlist Spotlight indexes
/// without specialised plug-ins, which is a reasonable proxy for
/// "things humans expect to read line by line."
pub fn is_text_file(filename: &str) -> bool {
    matches!(
        std::path::Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some(
            "txt" | "md" | "csv" | "json" | "yaml" | "yml" | "toml"
            | "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "rb"
            | "c" | "h" | "cpp" | "hpp" | "java" | "kt" | "swift"
            | "html" | "css" | "scss" | "sh" | "xml" | "ini" | "log"
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(hash: &str, t: u64) -> VersionInfo {
        VersionInfo {
            hash: hash.into(),
            modified_at: t,
        }
    }

    #[test]
    fn test_conflict_detected_when_both_sides_changed() {
        // Plan §1494 — the canonical positive case.
        let local = v("abc", 1000);
        let remote = v("xyz", 1001);
        let base = v("old", 900);
        assert!(is_conflict(&local, &remote, &base));
    }

    #[test]
    fn test_no_conflict_when_only_remote_changed() {
        // Plan §1501 — local stayed at base, only remote moved. We
        // can apply remote without consulting the user.
        let local = v("old", 900);
        let remote = v("xyz", 1001);
        let base = v("old", 900);
        assert!(!is_conflict(&local, &remote, &base));
    }

    #[test]
    fn test_no_conflict_when_only_local_changed() {
        // Symmetric to the previous case — local moved, remote
        // stayed. We can upload local without consulting the user.
        let local = v("abc", 1000);
        let remote = v("old", 900);
        let base = v("old", 900);
        assert!(!is_conflict(&local, &remote, &base));
    }

    #[test]
    fn test_no_conflict_when_both_sides_landed_on_same_hash() {
        // Pathological but possible — two devices independently
        // produced byte-identical content (e.g. both saved a fresh
        // `git status` output). Nothing to resolve.
        let local = v("same", 1000);
        let remote = v("same", 1001);
        let base = v("old", 900);
        assert!(!is_conflict(&local, &remote, &base));
    }

    #[test]
    fn test_auto_resolution_deadline_is_24h() {
        // Plan §1794. 86400 seconds = 24 h.
        assert_eq!(auto_resolution_deadline(0), 86_400);
        assert_eq!(auto_resolution_deadline(1_700_000_000), 1_700_086_400);
    }

    #[test]
    fn test_auto_resolve_fires_after_24h() {
        // Plan §1794.
        let detected_at = 0u64;
        let now = 24 * 3600 + 1;
        assert!(now >= auto_resolution_deadline(detected_at));
    }

    #[test]
    fn test_auto_resolve_does_not_fire_early() {
        // Plan §1801.
        let detected_at = 0u64;
        let now = 24 * 3600 - 1;
        assert!(now < auto_resolution_deadline(detected_at));
    }

    #[test]
    fn test_text_classification() {
        assert!(is_text_file("notes.md"));
        assert!(is_text_file("Cargo.toml"));
        assert!(is_text_file("App.tsx"));
        // Case-insensitive — users on Windows pick up MD / TXT.
        assert!(is_text_file("README.MD"));

        assert!(!is_text_file("photo.jpg"));
        assert!(!is_text_file("archive.zip"));
        // Bare names with no extension fall through to "binary" — we
        // err on the side of NOT trying to render arbitrary bytes
        // through a diff view.
        assert!(!is_text_file("Makefile"));
    }
}
