//! `paths` items split out of `held_queue.rs`.

use super::*;

/// `a ≡ b` for two leaf names (spec §3).
pub fn same_leaf(a: &str, b: &str, case_insensitive: bool) -> bool {
    if case_insensitive {
        a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

/// A parent 404: the error text before `" for url ("` contains `404` (spec §3), so a UUID with "404" in the URL never
/// counts.
pub fn is_parent_404(last_error: &str) -> bool {
    last_error
        .split(" for url (")
        .next()
        .unwrap_or_default()
        .contains("404")
}

pub(super) fn leaf(path: &str) -> &str {
    path.trim_matches('/').rsplit('/').next().unwrap_or_default()
}

pub(super) fn depth(path: &str) -> usize {
    path.trim_matches('/').matches('/').count()
}

/// The pass decides about an op that is not paused, paused for `auth` or held by an earlier pass. Any other pause
/// (2267's `found_while_paused`, quota, a replaced key) waits for the person, and the pass leaves it alone.
pub(super) fn pass_decides(op: &Op) -> bool {
    matches!(op.paused_reason.as_deref(), None | Some("auth") | Some(NEEDS_REVIEW))
}

/// What the plan does with a ghost; its children read it.
#[derive(Clone, Copy)]
pub(super) enum Fate<'a> {
    Remapped(&'a str),
    Replay,
    /// Held, left for the person, or waiting on one of those: it stays where it is.
    Stays,
    /// D5: in a scaffolding subtree the plan drops whole.
    Dropped,
}

/// D5: where the backup mirror wrote when it could not read this PC's name (D9).
pub(super) const BACKUPS_UNKNOWN: &str = "Backups/unknown";

/// `path` is `root` or lies under it, whole segment by segment under `≡`.
pub(super) fn at_or_under(path: &str, root: &str, case_insensitive: bool) -> bool {
    let mut segments = path.trim_matches('/').split('/');
    root.trim_matches('/')
        .split('/')
        .all(|seg| segments.next().is_some_and(|s| same_leaf(s, seg, case_insensitive)))
}

/// `dirs` in an order the caller can remove them in, one at a time: deepest first (then by path, so the order is the
/// same whichever way they were gathered), each once.
pub(super) fn deepest_first(dirs: &mut Vec<String>) {
    dirs.sort_by(|a, b| depth(b).cmp(&depth(a)).then_with(|| a.cmp(b)));
    dirs.dedup();
}
