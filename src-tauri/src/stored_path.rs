//! Task 2413: the one rule for the path of a server row, and the one map from a stored path onto the sync root.
//!
//! A row the server lists is stored at its parent's stored path and its own name, '/'-joined. The state DB writes it
//! only when every segment is a name this host holds as exactly one entry: core's `path_safety::check_component_for`
//! with the host's rules ([`check_name_for`]). A row that fails is refused: nothing about it is written, so a row
//! stored before stays exactly as it was, and is never removed for it.
//!
//! The rules are the HOST's on purpose (core's `Rules::Host`, which core's docs name for sync): sync mirrors a tree
//! both ways, and a name that is an ordinary file name on Linux or macOS (`aux.c`, `10:30.txt`, `a\b`) must keep
//! round-tripping there. Rewriting a name (core's `sanitize_component`) is never used for a stored path: the stored
//! path is the key the local scan compares with the name on disk, so a rewritten one reads as another file.
//!
//! [`path_under`] maps a stored path onto the sync root for the Windows placeholder code. It applies the Windows rules
//! on every host, splits on both separators, and checks containment component by component.

use std::path::{Component, Path, PathBuf};

use beebeeb_core::path_safety::{UnsafeName, check_component_for};

#[cfg(test)]
thread_local! {
    static TEST_HOST_WINDOWS: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Whether this host applies Windows path rules to a stored path: the build target, unless a test runs a block with
/// another host's rules ([`HostRules`]).
pub(crate) fn host_is_windows() -> bool {
    #[cfg(test)]
    if let Some(windows) = TEST_HOST_WINDOWS.with(std::cell::Cell::get) {
        return windows;
    }
    cfg!(target_os = "windows")
}

/// While alive, [`host_is_windows`] answers `windows` on this thread, so a test on any host runs the store with the
/// Windows rules (or without them). Dropping it restores the build target's answer.
#[cfg(test)]
pub(crate) struct HostRules(());

#[cfg(test)]
impl HostRules {
    pub(crate) fn windows(windows: bool) -> Self {
        TEST_HOST_WINDOWS.with(|cell| cell.set(Some(windows)));
        Self(())
    }
}

#[cfg(test)]
impl Drop for HostRules {
    fn drop(&mut self) {
        TEST_HOST_WINDOWS.with(|cell| cell.set(None));
    }
}

/// What the state DB did with a server row (`StateDb::upsert_server_file_row`, `StateDb::upsert_shared_server_row`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerRowWrite {
    /// Written, at this path.
    Written(String),
    /// Not written, because a segment of its path breaks this rule. A row stored before is exactly as it was.
    Refused(UnsafeName),
}

/// One name, a single segment of a stored path. `windows`: the host applies Windows path rules.
///
/// Refused on every host: an empty name (or one only of invisible characters), `.` and `..`, a `/`, a NUL, more than
/// 255 bytes. With the Windows rules also: a `\`, a drive prefix, a `:` anywhere, `< > " | ? *`, control and
/// bidi-control characters, a trailing dot or space, and a reserved device name (`CON`, `NUL`, `COM1.txt` ...).
pub(crate) fn check_name_for(name: &str, windows: bool) -> Result<(), UnsafeName> {
    check_component_for(name, false, windows)
}

/// A stored path: every '/'-separated segment passes [`check_name_for`]. Leading `/`s are the stored form some older
/// rows have (every reader trims them), so they are allowed; an empty path, a trailing `/` and a doubled `/` leave an
/// empty segment and are refused.
pub(crate) fn check_path_for(path: &str, windows: bool) -> Result<(), UnsafeName> {
    path.trim_start_matches('/')
        .split('/')
        .try_for_each(|segment| check_name_for(segment, windows))
}

/// [`check_path_for`] with this host's rules: the check the state DB runs before it writes a server row.
pub(crate) fn check_path(path: &str) -> Result<(), UnsafeName> {
    check_path_for(path, host_is_windows())
}

/// The path a server row is stored at: `parent` (a stored path, "" at the top level) and the row's own `name`, which
/// must be ONE segment. An empty `name` (it did not resolve) gives "", a row that is stored but never placed, as
/// before. The joined form is the one the pull always wrote, so a row's stored path does not change with this check.
pub(crate) fn compose_for(parent: &str, name: &str, windows: bool) -> Result<String, UnsafeName> {
    if name.is_empty() {
        return Ok(String::new());
    }
    check_name_for(name, windows)?;
    let path = if parent.is_empty() {
        name.to_string()
    } else {
        format!("{}/{name}", parent.trim_end_matches('/'))
    };
    check_path_for(&path, windows)?;
    Ok(path)
}

/// [`compose_for`] with this host's rules.
pub(crate) fn compose(parent: &str, name: &str) -> Result<String, UnsafeName> {
    compose_for(parent, name, host_is_windows())
}

/// `rel_path` (a stored path) as a path under `root`, or `None`.
///
/// The Windows rules apply on every host (its production callers are Windows's placeholder code, and the same rules
/// everywhere let a Linux test show what Windows does): the path is split on both `/` and `\`, and every segment must
/// pass [`check_name_for`] with them. Then the joined path must be `root` followed by exactly one plain component per
/// segment: containment by component, never by string prefix. Leading and trailing `/`s are trimmed first (older
/// rows); an empty path is `None`.
pub(crate) fn path_under(root: &Path, rel_path: &str) -> Option<PathBuf> {
    let rel = rel_path.trim_matches('/');
    if rel.is_empty() {
        return None;
    }
    let mut candidate = root.to_path_buf();
    let mut segments = 0usize;
    for segment in rel.split(['/', '\\']) {
        check_name_for(segment, true).ok()?;
        candidate.push(segment);
        segments += 1;
    }
    let below = candidate.strip_prefix(root).ok()?;
    let mut plain = 0usize;
    for component in below.components() {
        if !matches!(component, Component::Normal(_)) {
            return None;
        }
        plain += 1;
    }
    (plain == segments).then_some(candidate)
}

/// The walk from `root` to `dest` a file write takes: `root` itself, every folder below it on the way, and `dest` when
/// it exists must not be a link. A link is std's `FileType::is_symlink`: a Unix symlink, and on Windows a reparse point
/// whose tag is a name surrogate (a symlink or a junction). A Cloud Files or OneDrive placeholder carries a tag that is
/// not a name surrogate, so it passes, as in core's backup walk. `dest` must be `root` followed by plain components;
/// the walk ends at the first entry that is not there yet (nothing below it can be a link). `PermissionDenied` when
/// refused.
#[cfg_attr(unix, allow(dead_code))]
pub(crate) fn refuse_links_below(root: &Path, dest: &Path) -> std::io::Result<()> {
    let deny = |message: &'static str| std::io::Error::new(std::io::ErrorKind::PermissionDenied, message);
    let rel = dest
        .strip_prefix(root)
        .map_err(|_| deny("write destination is not within an allowed root"))?;
    // `Ok(true)` when `path` is there and is no link, `Ok(false)` when it is not there.
    let present = |path: &Path| match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(deny("write destination goes through a link")),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    };
    let mut at = root.to_path_buf();
    if !present(&at)? {
        return Ok(());
    }
    for component in rel.components() {
        let Component::Normal(name) = component else {
            return Err(deny("write destination has a non-normal path component"));
        };
        at.push(name);
        if !present(&at)? {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names each class of the rule covers, with whether the Linux/macOS rules and the Windows rules accept them.
    const NAMES: &[(&str, bool, bool)] = &[
        ("report.pdf", true, true),
        ("Notes 2026", true, true),
        ("élan.txt", true, true),
        (" leading space.txt", true, true),
        ("", false, false),
        (".", false, false),
        ("..", false, false),
        ("a/b", false, false),
        ("/top/level.txt", false, false),
        ("a\0b", false, false),
        ("a\\b", true, false),
        ("..\\outside.txt", true, false),
        ("\\\\server\\share", true, false),
        ("C:x", true, false),
        ("C:\\x", true, false),
        ("a:b", true, false),
        ("report.txt:stream", true, false),
        ("CON", true, false),
        ("nul.txt", true, false),
        ("COM1.txt", true, false),
        ("Lpt9", true, false),
        ("trailing.", true, false),
        ("trailing ", true, false),
        ("star*", true, false),
        ("bell\u{7}", true, false),
    ];

    #[test]
    fn task_2413_each_name_class_meets_the_host_rule() {
        let mut checked = 0;
        for (name, unix, windows) in NAMES {
            assert_eq!(check_name_for(name, false).is_ok(), *unix, "{name:?} without the Windows rules");
            assert_eq!(check_name_for(name, true).is_ok(), *windows, "{name:?} with the Windows rules");
            checked += 1;
        }
        assert_eq!(checked, 25);
    }

    #[test]
    fn task_2413_compose_keeps_one_name_under_its_parent() {
        for windows in [false, true] {
            assert_eq!(compose_for("", "a.txt", windows).as_deref(), Ok("a.txt"));
            assert_eq!(compose_for("docs", "a.txt", windows).as_deref(), Ok("docs/a.txt"));
            assert_eq!(compose_for("docs/", "a.txt", windows).as_deref(), Ok("docs/a.txt"));
            // An older parent's leading slash stays as the pull always wrote it.
            assert_eq!(compose_for("/docs", "a.txt", windows).as_deref(), Ok("/docs/a.txt"));
            assert_eq!(compose_for("docs", "", windows).as_deref(), Ok(""), "a name that did not resolve");
            for name in ["..", ".", "a/b", "/a", "a/", "../../x"] {
                assert!(compose_for("docs", name, windows).is_err(), "{name:?} is not one name");
            }
            for parent in ["../up", "docs/../..", "docs//x"] {
                assert!(compose_for(parent, "a.txt", windows).is_err(), "{parent:?} as a parent");
            }
        }
        assert_eq!(compose_for("docs", "CON", false).as_deref(), Ok("docs/CON"));
        assert!(compose_for("docs", "CON", true).is_err());
        assert!(compose_for("a:b", "x.txt", true).is_err(), "a parent segment with the Windows rules");
    }

    /// The walk refuses a link at the root, at a folder on the way and at the destination, and passes plain entries
    /// and a destination that is not there yet. (Windows: the junction test in `stored_path_2413_windows`.)
    #[cfg(unix)]
    #[test]
    fn task_2413_a_write_walk_refuses_a_link_anywhere_from_the_root_down() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("kept.txt"), b"kept").unwrap();
        std::fs::write(root.join("docs/own.txt"), b"own").unwrap();
        symlink(&outside, root.join("docs/link")).unwrap();
        symlink(outside.join("kept.txt"), root.join("docs/file-link.txt")).unwrap();
        symlink(&root, dir.path().join("root-link")).unwrap();
        let denied = |dest: &Path, base: &Path| {
            let error = refuse_links_below(base, dest).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied, "{}: {error}", dest.display());
        };
        denied(&root.join("docs/link/kept.txt"), &root);
        denied(&root.join("docs/file-link.txt"), &root);
        denied(&dir.path().join("root-link/docs/own.txt"), &dir.path().join("root-link"));
        denied(&dir.path().join("elsewhere.txt"), &root);
        refuse_links_below(&root, &root.join("docs/own.txt")).unwrap();
        refuse_links_below(&root, &root.join("docs/new.txt")).unwrap();
        refuse_links_below(&root, &root.join("new-folder/new.txt")).unwrap();
    }

    #[test]
    fn task_2413_the_test_host_rules_end_with_their_guard() {
        let built = host_is_windows();
        {
            let _rules = HostRules::windows(!built);
            assert_eq!(host_is_windows(), !built);
        }
        assert_eq!(host_is_windows(), built);
    }
}
