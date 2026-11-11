//! Task 2407, the desktop half of core's task 2404. Windows looks a name up among a folder's long names AND its 8.3
//! short names, and strips a trailing dot or space, so a name such as `REPORT~1.PDF` can reach an existing
//! `Report for 2026.pdf`. A create, truncate, replace, rename, copy or delete by that name would touch a file it never
//! named. Every desktop site that does one of those to a sync-folder entry, by a name it did not read from the folder
//! itself, asks [`conflict_below`] first. On a conflict it takes its conflict path, never the entry the name reaches.
//!
//! The answer is core's `path_safety::short_name_alias` (core v0.2.7). It reads the existing entry's own long name from
//! the file system, never the shape of the requested name: real files have `~` in their names. Other hosts never
//! resolve a name to a differently named entry; there core answers without touching the disk.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

/// A name that reaches a different existing entry: a name conflict, never an I/O failure. Its text names no path and no
/// file name (zero-knowledge), so it may be logged and shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameConflict;

impl std::fmt::Display for NameConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "name conflict: the name reaches a different existing entry (an 8.3 short name or another alias); \
             it was not touched",
        )
    }
}

impl std::error::Error for NameConflict {}

/// Whether creating, truncating, replacing, renaming, copying or deleting `path` would touch a DIFFERENT existing entry
/// that a component of `path` below `base` reaches through another of its names.
///
/// `base` is trusted (the sync root, or a folder already checked); every component of `path` below it is looked up,
/// and the first that does not exist ends the walk (a create makes exactly that name).
/// - `Ok(None)`: no conflict.
/// - `Ok(Some(NameConflict))`: take the conflict path; never open, truncate, replace, rename or delete `path`.
/// - `Err(_)`: the check could not finish, or `path` is not a path of plain names below `base` (`..`, a root or a
///   prefix). Refuse.
pub fn conflict_below(base: &Path, path: &Path) -> io::Result<Option<NameConflict>> {
    let rel = path
        .strip_prefix(base)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "the path is not below its base"))?;
    #[cfg(any(test, feature = "test-support"))]
    if let Some(answer) = stub::answer(base, rel) {
        return answer;
    }
    beebeeb_core::path_safety::short_name_alias(base, rel)
        .map(|found| found.map(|_| NameConflict))
        // Zero-knowledge: core's error text can carry the plaintext path; only its kind travels on.
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                "the name could not be checked against the folder's entries",
            )
        })
}

/// [`conflict_below`] as one answer for a site that only proceeds when it is clear: `Ok(())` when nothing conflicts,
/// else an `io::Error` whose kind is `AlreadyExists` for a conflict (its inner error is the [`NameConflict`]) and the
/// check's own error when it could not finish.
pub fn refuse_conflict_below(base: &Path, path: &Path) -> io::Result<()> {
    match conflict_below(base, path)? {
        None => Ok(()),
        Some(conflict) => Err(io::Error::new(io::ErrorKind::AlreadyExists, conflict)),
    }
}

/// [`refuse_conflict_below`] for a destination that one of `roots` allows: from the root it was built on (a lexical
/// prefix), else from its own folder, which checks its name alone (as core's own writers do).
pub fn refuse_conflict_under_roots(dest: &Path, roots: &[&Path]) -> io::Result<()> {
    let base = roots
        .iter()
        .copied()
        .find(|root| dest.starts_with(root))
        .or_else(|| dest.parent())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the destination has no folder"))?;
    refuse_conflict_below(base, dest)
}

/// Where a Windows reclaim (free up space, a selective-sync exclusion) may dehydrate a row's file: its place below
/// `sync_root`, built from the row's '/'-joined path. `None` for an empty path, and for a path whose name reaches a
/// different file or folder (a Windows 8.3 short name; `canonicalize` would follow it to that file's long name) or
/// that cannot be checked: the file stays local.
#[cfg_attr(
    not(any(target_os = "windows", any(test, feature = "test-support"))),
    allow(dead_code)
)]
pub fn reclaim_place(sync_root: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.trim_matches('/');
    if rel.is_empty() {
        return None;
    }
    let place = sync_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    matches!(conflict_below(sync_root, &place), Ok(None)).then_some(place)
}

/// The folders one pass has found to be their own names, so a pass over many rows looks each folder up once
/// (`windows_cf::populate_placeholders` runs every tick, over every cloud-only row).
#[cfg_attr(
    not(any(target_os = "windows", any(test, feature = "test-support"))),
    allow(dead_code)
)]
pub struct CheckedDirs<'a> {
    base: &'a Path,
    own: HashSet<PathBuf>,
}

#[cfg_attr(
    not(any(target_os = "windows", any(test, feature = "test-support"))),
    allow(dead_code)
)]
impl<'a> CheckedDirs<'a> {
    pub fn new(base: &'a Path) -> Self {
        Self {
            base,
            own: HashSet::new(),
        }
    }

    /// [`conflict_below`] for the folder `dir`, looking up only the components below the deepest folder this pass has
    /// already found to be its own name. A clear answer is remembered for the rest of the pass.
    pub fn check(&mut self, dir: &Path) -> io::Result<Option<NameConflict>> {
        if dir == self.base || self.own.contains(dir) {
            return Ok(None);
        }
        let trusted = dir
            .ancestors()
            .skip(1)
            .take_while(|ancestor| *ancestor != self.base)
            .find(|ancestor| self.own.contains(*ancestor))
            .unwrap_or(self.base)
            .to_path_buf();
        let found = conflict_below(&trusted, dir)?;
        if found.is_none() {
            // Every folder between `trusted` and `dir` is its own name too (or did not exist, and the caller creates
            // it under exactly that name).
            for ancestor in dir.ancestors().take_while(|ancestor| *ancestor != trusted) {
                self.own.insert(ancestor.to_path_buf());
            }
        }
        Ok(found)
    }

    /// `dir` was just created under exactly its name (a create never takes another entry's name), or checked.
    pub fn mark_own(&mut self, dir: PathBuf) {
        self.own.insert(dir);
    }
}

/// Tests only: answer [`conflict_below`] for chosen paths as Windows would for an 8.3 short name, so each site's
/// decision runs on any host. On Linux the stubbed path is an ordinary entry, standing in for the different file the
/// name reaches: a site that ignores the answer creates, replaces or deletes it, which the test sees.
#[cfg(any(test, feature = "test-support"))]
pub mod stub {
    use super::NameConflict;
    use std::io;
    use std::path::{Component, Path, PathBuf};
    use std::sync::Mutex;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Answer {
        Alias,
        Fails,
    }

    /// Every stubbed path with its answer. Tests run in parallel, each under its own temp dir, so one list serves all.
    static STUBS: Mutex<Vec<(PathBuf, Answer)>> = Mutex::new(Vec::new());

    /// The stub for one path; dropped, the path is answered by core again.
    #[must_use = "the stub lasts while this is alive"]
    pub struct Stubbed(PathBuf);

    impl Drop for Stubbed {
        fn drop(&mut self) {
            let mut stubs = STUBS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            stubs.retain(|(path, _)| path != &self.0);
        }
    }

    fn stub(path: &Path, answer: Answer) -> Stubbed {
        STUBS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((path.to_path_buf(), answer));
        Stubbed(path.to_path_buf())
    }

    /// `path` (or anything below it) reaches a different entry, as `REPORT~1.PDF` reaches `Report for 2026.pdf`.
    pub fn alias(path: &Path) -> Stubbed {
        stub(path, Answer::Alias)
    }

    /// The check of `path` (or anything below it) cannot finish (a folder that cannot be read).
    pub fn failing(path: &Path) -> Stubbed {
        stub(path, Answer::Fails)
    }

    /// The stubbed answer for `rel` below `base`, walking it one component at a time as core does; `None` when no
    /// component is stubbed.
    pub(super) fn answer(base: &Path, rel: &Path) -> Option<io::Result<Option<NameConflict>>> {
        let stubs = STUBS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if stubs.is_empty() {
            return None;
        }
        let mut at = base.to_path_buf();
        for component in rel.components() {
            if !matches!(component, Component::Normal(_)) {
                return None;
            }
            at.push(component);
            match stubs.iter().find(|(path, _)| path == &at).map(|(_, answer)| *answer) {
                Some(Answer::Alias) => return Some(Ok(Some(NameConflict))),
                Some(Answer::Fails) => return Some(Err(io::Error::other("stubbed: the check could not finish"))),
                None => {}
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stub answers like core: a conflict in any segment, a failure as an error; core answers everything else
    /// (no conflict on this host, a refusal for `..`).
    #[test]
    fn the_check_answers_a_conflict_a_failure_and_a_path_outside_its_base() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let _alias = stub::alias(&base.join("PHOTOS~1"));
        let _failing = stub::failing(&base.join("locked"));
        assert_eq!(
            conflict_below(base, &base.join("PHOTOS~1")).unwrap(),
            Some(NameConflict)
        );
        assert_eq!(
            conflict_below(base, &base.join("PHOTOS~1").join("a.jpg")).unwrap(),
            Some(NameConflict)
        );
        assert!(conflict_below(base, &base.join("locked").join("a.txt")).is_err());
        assert_eq!(
            conflict_below(base, &base.join("Photos 2026").join("a.jpg")).unwrap(),
            None
        );
        assert_eq!(conflict_below(base, base).unwrap(), None);
        assert!(
            conflict_below(base, Path::new("/elsewhere/a.txt")).is_err(),
            "not below the base"
        );
        assert!(
            conflict_below(base, &base.join("..").join("a.txt")).is_err(),
            "not a path of plain names"
        );
        let refused = refuse_conflict_below(base, &base.join("PHOTOS~1")).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::AlreadyExists);
        assert!(refused.get_ref().is_some_and(|inner| inner.is::<NameConflict>()));
        assert!(
            !NameConflict.to_string().contains("PHOTOS"),
            "the conflict names no file"
        );
        let other_root = tempfile::tempdir().unwrap();
        let under = base.join("PHOTOS~1").join("a.jpg");
        assert!(
            refuse_conflict_under_roots(&under, &[other_root.path(), base]).is_err(),
            "checked from its root"
        );
        let _leaf = stub::alias(&base.join("REPORT~1.PDF"));
        assert!(
            refuse_conflict_under_roots(&base.join("REPORT~1.PDF"), &[other_root.path()]).is_err(),
            "no root is its prefix: its own name is still checked"
        );
        refuse_conflict_under_roots(&base.join("new.pdf"), &[base]).unwrap();
    }

    /// The pass remembers a clear folder (and the folders above it) and looks below them only; a conflicting folder
    /// is answered again for each row under it, and a folder the pass created is trusted.
    #[test]
    fn checked_dirs_look_each_folder_up_once() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let mut dirs = CheckedDirs::new(base);
        assert_eq!(dirs.check(&base.join("Trip").join("Day 1")).unwrap(), None);
        // Stubbed only now: the pass already found `Trip` clear and never walks it again.
        let _late = stub::alias(&base.join("Trip"));
        assert_eq!(dirs.check(&base.join("Trip").join("Day 1")).unwrap(), None);
        assert_eq!(dirs.check(&base.join("Trip").join("Day 2")).unwrap(), None);
        let _day3 = stub::alias(&base.join("Trip").join("Day 3"));
        assert_eq!(
            dirs.check(&base.join("Trip").join("Day 3").join("Morning")).unwrap(),
            Some(NameConflict),
            "below the clear `Trip`, `Day 3` is still looked up"
        );
        let _alias = stub::alias(&base.join("PHOTOS~1"));
        for _ in 0..2 {
            assert_eq!(dirs.check(&base.join("PHOTOS~1")).unwrap(), Some(NameConflict));
        }
        dirs.mark_own(base.join("New"));
        let _new = stub::alias(&base.join("New"));
        assert_eq!(
            dirs.check(&base.join("New")).unwrap(),
            None,
            "created by this pass under its own name"
        );
    }
}
