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
pub enum SourceRefusal {
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
                format!(
                    r"\\{}\{}",
                    server.to_string_lossy().to_lowercase(),
                    fold(&share.to_string_lossy())
                )
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
#[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
pub fn check_source(source: &Path, sync_root: &Path) -> Result<(), SourceRefusal> {
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
#[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
pub fn pass_excludes(sync_root: &Path) -> Option<Vec<PathBuf>> {
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

/// Whether `a` and `b` name the same folder: compared as [`check_source`] compares (component by component, without
/// case on Windows and macOS, as given and as resolved).
pub fn same_folder(a: &Path, b: &Path) -> bool {
    match (spellings(a), spellings(b)) {
        (Some(a), Some(b)) => a.iter().any(|spelling| b.contains(spelling)),
        _ => a == b,
    }
}

/// What reading the sources found: the folders to pass now, and the folders whose question was asked just now
/// (Amendment 9: once per moved folder).
#[derive(Debug, Default)]
pub struct SourcesRead {
    pub sources: Vec<super::scheduler::BackupSource>,
    pub newly_asked: Vec<String>,
}

/// The folders this device backs up now, from the settings' switched-on keys, each with where it is read from.
/// Windows reads the known folder's default location (`known_folder::resolve_known_folder`), or, for a folder
/// OneDrive's Known Folder Move has moved, the one the person chose (Amendment 9): a moved folder with no answer is
/// not passed and its question is asked once. The other systems have no sources before Task D5.
pub fn backup_sources(
    db: &crate::state_db::StateDb,
    enabled_keys: &[String],
    choices: &std::collections::BTreeMap<String, crate::config::KnownFolderSource>,
) -> Vec<super::scheduler::BackupSource> {
    backup_sources_with(Some(db), enabled_keys, choices, super::kfm::known_folder_paths).sources
}

/// [`backup_sources`] without asking or settling any question: a read only (the drive protection's).
pub fn backup_sources_quiet(
    enabled_keys: &[String],
    choices: &std::collections::BTreeMap<String, crate::config::KnownFolderSource>,
) -> Vec<super::scheduler::BackupSource> {
    backup_sources_with(None, enabled_keys, choices, super::kfm::known_folder_paths).sources
}

/// [`backup_sources`] with the known folders' locations from `paths` (the tests' folder pairs). `questions`: where
/// the questions are asked and settled; `None` reads only.
pub fn backup_sources_with(
    questions: Option<&crate::state_db::StateDb>,
    enabled_keys: &[String],
    choices: &std::collections::BTreeMap<String, crate::config::KnownFolderSource>,
    paths: impl Fn(&str) -> super::kfm::KnownFolderPaths,
) -> SourcesRead {
    use super::kfm::SourceResolution;
    let mut read = SourcesRead::default();
    for folder in crate::known_folder::KNOWN_FOLDERS {
        let Some(category) = beebeeb_types::backup::Category::from_key(folder.key) else {
            continue;
        };
        let enabled = enabled_keys.iter().any(|key| key == folder.key);
        let resolution = if enabled {
            super::kfm::resolve(&paths(folder.key), choices.get(folder.key).copied())
        } else {
            SourceResolution::Unavailable
        };
        match resolution {
            SourceResolution::Read(path) => {
                settle_question(questions, folder.key);
                read.sources.push(super::scheduler::BackupSource { category, path });
            }
            SourceResolution::Waiting { default, moved } => {
                let Some(db) = questions else {
                    continue;
                };
                let at_ms = chrono::Utc::now().timestamp_millis();
                match db.ask_backup_source_question(
                    folder.key,
                    &default.to_string_lossy(),
                    &moved.to_string_lossy(),
                    at_ms,
                ) {
                    Ok(true) => {
                        tracing::info!(
                            category = folder.key,
                            "backup: OneDrive moved this folder; asking which to back up"
                        );
                        read.newly_asked.push(folder.key.to_string());
                    }
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(category = folder.key, %error, "backup: the question was not recorded")
                    }
                }
            }
            SourceResolution::Unavailable => settle_question(questions, folder.key),
        }
    }
    read
}

fn settle_question(questions: Option<&crate::state_db::StateDb>, key: &str) {
    let Some(db) = questions else {
        return;
    };
    if let Err(error) = db.settle_backup_source_question(key) {
        tracing::warn!(category = key, %error, "backup: a settled question was not cleared");
    }
}

/// Bounded, best-effort count of regular files under `root` (≤ this many before
/// we stop — the panel only needs an "about this many" signal, not an exact
/// total, and an exact recursive count of a huge Documents folder would block
/// the IPC thread). Windows-only (the only caller is gated).
#[cfg(target_os = "windows")]
pub fn count_items_bounded(root: &std::path::Path) -> Option<u64> {
    const CAP: u64 = 10_000;
    fn walk(dir: &std::path::Path, depth: usize, count: &mut u64) {
        if depth > 32 || *count >= CAP {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if *count >= CAP {
                return;
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                walk(&entry.path(), depth + 1, count);
            } else if ft.is_file() {
                *count += 1;
            }
        }
    }
    let mut count = 0u64;
    walk(root, 0, &mut count);
    Some(count)
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
        assert_eq!(
            check_source(&sibling, &sync_root),
            Ok(()),
            "a name prefix is not a folder"
        );
        assert_eq!(
            check_source(dir.path(), &sync_root),
            Ok(()),
            "a source holding the sync root walks around it"
        );
        // Not yet created: compared as given.
        assert_eq!(
            check_source(&sync_root.join("Later"), &sync_root),
            Err(SourceRefusal::InsideSyncRoot)
        );
        assert_eq!(
            check_source(Path::new("relative"), &sync_root),
            Err(SourceRefusal::Unclear)
        );
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
