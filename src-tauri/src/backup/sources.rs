//! What a backup reads from (plan Amendment 1 item 11, host duties). A backup never reads the sync root:
//!
//! - a source that is the sync root or lies inside it is refused (`check_source`): those files are in the drive
//!   already, and the drive's own `Backups` would back itself up;
//! - a sync root inside a source is left out of the walk (`pass_excludes`, core's `walk_source` `exclude`).
//!
//! Paths are compared component by component, without case on Windows and macOS, each as given and as resolved
//! (links followed), the rule core's walk applies to its excludes.

use std::path::{Component, Path, PathBuf, Prefix};

/// Why a folder is not backed up from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceRefusal {
    /// The folder is the sync root or lies inside it.
    InsideSyncRoot,
    /// The path is not absolute, or names no folder this app can place (`..` that does not resolve).
    Unclear,
}

const CASE_INSENSITIVE: bool = cfg!(any(target_os = "windows", target_os = "macos"));

fn fold(text: &str) -> String {
    if CASE_INSENSITIVE {
        text.to_lowercase()
    } else {
        text.to_string()
    }
}

/// One component as compared: a drive prefix the same whether spelled `C:` or `\\?\C:`.
fn component_key(component: Component<'_>) -> String {
    match component {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => format!("{}:", (drive as char).to_ascii_lowercase()),
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                format!(r"\\{}\{}", server.to_string_lossy().to_lowercase(), fold(&share.to_string_lossy()))
            }
            _ => prefix.as_os_str().to_string_lossy().into_owned(),
        },
        other => fold(&other.as_os_str().to_string_lossy()),
    }
}

fn keys(path: &Path) -> Vec<String> {
    path.components().map(component_key).collect()
}

/// `path` as given (made absolute) and as resolved, when it exists. `None` when neither is a clean absolute path.
fn spellings(path: &Path) -> Option<Vec<Vec<String>>> {
    let clean = |p: &Path| p.is_absolute() && !p.components().any(|c| c == Component::ParentDir);
    let mut found = Vec::new();
    if let Ok(given) = std::path::absolute(path) {
        if clean(&given) {
            found.push(keys(&given));
        }
    }
    if let Ok(real) = std::fs::canonicalize(path) {
        let real = keys(&real);
        if !found.contains(&real) {
            found.push(real);
        }
    }
    (!found.is_empty()).then_some(found)
}

/// Whether `source` may be backed up while `sync_root` is this app's sync root. Checked when the person chooses a
/// folder and again before each pass (a sync root can move). A relative path is never guessed against the working
/// directory.
// LD-0113 Task D4: the folder picker and the backup scheduler are its production callers.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn check_source(source: &Path, sync_root: &Path) -> Result<(), SourceRefusal> {
    if !source.is_absolute() || !sync_root.is_absolute() {
        return Err(SourceRefusal::Unclear);
    }
    let (Some(sources), Some(roots)) = (spellings(source), spellings(sync_root)) else {
        return Err(SourceRefusal::Unclear);
    };
    let inside = sources
        .iter()
        .any(|source| roots.iter().any(|root| !root.is_empty() && source.starts_with(root)));
    if inside {
        Err(SourceRefusal::InsideSyncRoot)
    } else {
        Ok(())
    }
}

/// The `exclude` of every pass: the sync root, absolute and without `..`, as core's walk requires. `None` when the
/// sync root cannot be spelled so: then no pass runs, because a pass without its exclude could walk the drive.
// LD-0113 Task D4: the backup scheduler (`run_pass`) is its production caller.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn pass_excludes(sync_root: &Path) -> Option<Vec<PathBuf>> {
    let clean = |p: &Path| p.is_absolute() && !p.components().any(|c| c == Component::ParentDir);
    let mut excludes = Vec::new();
    if let Ok(given) = std::path::absolute(sync_root) {
        if clean(&given) {
            excludes.push(given);
        }
    }
    if let Ok(real) = std::fs::canonicalize(sync_root) {
        if clean(&real) && !excludes.contains(&real) {
            excludes.push(real);
        }
    }
    (!excludes.is_empty()).then_some(excludes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beebeeb_core::backup::SkipReason;

    /// A source that is the sync root or lies inside it is refused; a sibling whose name only starts the same way,
    /// and a folder that holds the sync root, are not.
    #[test]
    fn a_source_under_the_sync_root_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("Beebeeb");
        let inside = sync_root.join("Photos");
        let sibling = dir.path().join("Beebeeb Photos");
        for folder in [&inside, &sibling] {
            std::fs::create_dir_all(folder).unwrap();
        }
        assert_eq!(check_source(&sync_root, &sync_root), Err(SourceRefusal::InsideSyncRoot));
        assert_eq!(check_source(&inside, &sync_root), Err(SourceRefusal::InsideSyncRoot));
        assert_eq!(check_source(&sibling, &sync_root), Ok(()), "a name prefix is not a folder");
        assert_eq!(check_source(dir.path(), &sync_root), Ok(()), "a source holding the sync root walks around it");
        // Not yet created: compared as given.
        assert_eq!(check_source(&sync_root.join("Later"), &sync_root), Err(SourceRefusal::InsideSyncRoot));
        assert_eq!(check_source(Path::new("relative"), &sync_root), Err(SourceRefusal::Unclear));
        if CASE_INSENSITIVE {
            let shouted = PathBuf::from(inside.to_string_lossy().to_uppercase());
            assert_eq!(check_source(&shouted, &sync_root), Err(SourceRefusal::InsideSyncRoot));
        }
        #[cfg(unix)]
        {
            let link = dir.path().join("Linked");
            std::os::unix::fs::symlink(&sync_root, &link).unwrap();
            assert_eq!(
                check_source(&link.join("Photos"), &sync_root),
                Err(SourceRefusal::InsideSyncRoot),
                "a link into the sync root"
            );
        }
    }

    /// A sync root inside a backed-up folder is left out of the walk: nothing under it is listed, and it is reported
    /// as `Excluded`.
    #[test]
    fn the_sync_root_inside_a_source_is_excluded_from_the_walk() {
        let dir = tempfile::tempdir().unwrap();
        let documents = dir.path().join("Documents");
        let sync_root = documents.join("Beebeeb");
        std::fs::create_dir_all(sync_root.join("Backups")).unwrap();
        std::fs::write(documents.join("letter.txt"), b"the person's").unwrap();
        std::fs::write(sync_root.join("drive.txt"), b"already in the drive").unwrap();
        std::fs::write(sync_root.join("Backups").join("copy.txt"), b"a backup").unwrap();

        let excludes = pass_excludes(&sync_root).expect("the sync root has a clean spelling");
        let walk = beebeeb_backup::walk_source(&documents, &excludes).unwrap();
        let keys: Vec<&str> = walk.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["letter.txt"], "nothing of the sync root is listed");
        assert_eq!(walk.skipped, vec![("Beebeeb".to_string(), SkipReason::Excluded)]);
        assert!(pass_excludes(Path::new("")).is_none(), "no clean spelling, no pass");
    }
}
