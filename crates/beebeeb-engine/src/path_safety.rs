//! Path guards for a relative path that arrives from outside (an IPC call, a server row): no traversal, and
//! containment under a root. Pure and platform-deterministic. Must not touch the network or the state database.

/// Layer 1: Pure, cross-platform path-traversal guard.
///
/// Validates `rel_path` (as received from the IPC call, before any separator
/// normalisation) against every traversal vector we know of. Must be kept
/// platform-deterministic — it must catch Windows-style attacks even when
/// compiled and tested on Linux.
///
/// Returns `Err(reason)` on any dangerous input.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn reject_unsafe_rel_path(rel_path: &str) -> Result<(), String> {
    // 1. Reject empty or whitespace-only input.
    if rel_path.trim().is_empty() {
        return Err("rel_path must not be empty".to_string());
    }

    // 2. Reject UNC paths (\\server\share or //server/share).
    if rel_path.starts_with("\\\\") || rel_path.starts_with("//") {
        return Err("rel_path must not be a UNC path".to_string());
    }

    // 3. Reject absolute Unix paths (leading '/') and root-relative Windows
    //    paths (leading '\').
    if rel_path.starts_with('/') || rel_path.starts_with('\\') {
        return Err("rel_path must not be absolute".to_string());
    }

    // 4. Reject Windows drive-letter prefixes: 'C:' (followed by anything,
    //    including just 'C:foo' or 'C:\foo'). On Linux `std::path::Component`
    //    never returns `Prefix`, so we must catch this with string inspection.
    {
        let bytes = rel_path.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            return Err("rel_path must not contain a drive-letter prefix".to_string());
        }
    }

    // 5. Inspect every path component (split on BOTH '/' and '\') for
    //    traversal and ambiguous components in any separator convention.
    for component in rel_path.split(['/', '\\']) {
        if component.is_empty() || component == "." {
            return Err("rel_path must not contain empty or '.' components".to_string());
        }
        if component == ".." {
            return Err("rel_path must not contain '..' components".to_string());
        }
    }

    // 6. Defense-in-depth: also run the platform's path parser.  On Windows
    //    this catches `Component::Prefix` (drive letters, UNC) and
    //    `Component::RootDir`; on Linux it catches `Component::RootDir` and
    //    `Component::ParentDir` for Unix-style paths. Cross-platform: catches
    //    `Component::ParentDir` regardless of OS.
    use std::path::{Component, Path};
    for component in Path::new(rel_path).components() {
        match component {
            Component::ParentDir => {
                return Err("rel_path must not contain '..' (ParentDir) components".to_string());
            }
            Component::RootDir => {
                return Err("rel_path must not be rooted".to_string());
            }
            Component::Prefix(_) => {
                return Err("rel_path must not contain a drive/UNC prefix".to_string());
            }
            Component::CurDir => {
                return Err("rel_path must not contain '.' (CurDir) components".to_string());
            }
            Component::Normal(_) => {}
        }
    }

    Ok(())
}

/// Layer 2: Canonicalization containment check.
///
/// Returns true iff `full` is strictly inside (or equal to) `root` after
/// both paths are canonicalized with the same function — keeping the `\\?\`
/// verbatim prefix consistent on Windows so `starts_with` works correctly.
///
/// `pub(crate)` and no longer `allow(dead_code)`: it is now used cross-platform
/// by `engine_bridge::hydrate_dest_is_allowed` (task 1247 — validating the
/// IPC-supplied hydrate destination), not just by the Windows reveal path.
pub fn is_contained(root: &std::path::Path, full: &std::path::Path) -> bool {
    let Ok(root_canon) = std::fs::canonicalize(root) else {
        return false;
    };
    let Ok(full_canon) = std::fs::canonicalize(full) else {
        return false;
    };
    full_canon.starts_with(&root_canon)
}
