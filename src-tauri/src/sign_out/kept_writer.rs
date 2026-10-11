//! Spec 2026-10-10 §7.2 "How" (task 1887, macOS): copy a staged copy into the kept folder. Nothing can overwrite:
//! every destination is made exclusively, and the volume, not a string comparison, decides a collision.

use std::collections::HashMap;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use super::kept_names::{MAX_NAME_BYTES, with_suffix};

/// `sys/clonefile.h:33`. `libc` 0.2.189 has `clonefile` but not this flag.
#[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 7
pub(crate) const CLONE_NOFOLLOW: u32 = 0x0001;
/// `sys/syslimits.h:103`, counting the terminating NUL: a path of 1024 bytes or more cannot be made.
pub(crate) const PATH_MAX_BYTES: usize = 1024;
/// The attribute that keeps an item out of Time Machine. A kept file is the person's, so it is backed up.
pub(crate) const BACKUP_EXCLUSION_ATTRIBUTE: &str = "com.apple.metadata:com_apple_backup_excludeItem";

pub(crate) trait CloneFile {
    /// Make `dst` as a copy of `src`. Never replaces an existing `dst`: that is `AlreadyExists`.
    fn clone_file(&self, src: &Path, dst: &Path) -> io::Result<()>;
}

/// The production copy: an APFS clone, or, where the volume cannot clone, a new file and a byte copy.
#[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 7
pub(crate) struct SystemClone;

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::new(ErrorKind::InvalidInput, "a path holds a NUL"))
}

impl CloneFile for SystemClone {
    fn clone_file(&self, src: &Path, dst: &Path) -> io::Result<()> {
        let (src_c, dst_c) = (c_path(src)?, c_path(dst)?);
        // Bound first, so the call's argument list holds no parenthesis and U40 can read its flag.
        let (src_ptr, dst_ptr) = (src_c.as_ptr(), dst_c.as_ptr());
        // SAFETY: both pointers are NUL-terminated C strings that live until the call returns.
        let rc = unsafe { libc::clonefile(src_ptr, dst_ptr, CLONE_NOFOLLOW) };
        if rc == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if falls_back(error.raw_os_error()) {
            copy_new(src, dst)
        } else {
            Err(error)
        }
    }
}

/// §7.2 "How", step 2: only a volume that cannot clone sends the copy to the fallback.
#[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 7
fn falls_back(errno: Option<i32>) -> bool {
    matches!(errno, Some(libc::ENOTSUP) | Some(libc::EXDEV))
}

/// The fallback: `create_new` (never truncates), private as a clone of a staged copy is, the bytes, then the staged
/// copy's modification time. The source is opened without following a link. A copy that fails partway removes its
/// file, which is this call's own (`create_new`): a partial plaintext copy never outlives the failed copy.
#[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 7
fn copy_new(src: &Path, dst: &Path) -> io::Result<()> {
    let mut from = open_no_follow(src)?;
    let mut to = OpenOptions::new().write(true).create_new(true).mode(0o600).open(dst)?;
    let copied = io::copy(&mut from, &mut to)
        .and_then(|_| from.metadata()?.modified())
        .and_then(|at| to.set_modified(at));
    if let Err(error) = copied {
        drop(to);
        let _ = std::fs::remove_file(dst);
        return Err(error);
    }
    Ok(())
}

/// `path` opened for reading, never through a link in its last component (`O_NOFOLLOW`: a link answers `ELOOP`).
fn open_no_follow(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(path)
}

fn remove_backup_exclusion(path: &Path) -> io::Result<()> {
    let (path_c, name_c) = (
        c_path(path)?,
        CString::new(BACKUP_EXCLUSION_ATTRIBUTE).expect("a constant without NUL"),
    );
    // SAFETY: both pointers are NUL-terminated C strings that live until the call returns.
    let rc = unsafe { libc::removexattr(path_c.as_ptr(), name_c.as_ptr(), libc::XATTR_NOFOLLOW) };
    let error = io::Error::last_os_error();
    if rc == 0 || error.raw_os_error() == Some(libc::ENOATTR) {
        Ok(())
    } else {
        Err(error)
    }
}

/// Measured on a resolved path only: `create` resolves the root, so every path built from `self.event` is the one the
/// kernel checks.
fn too_long(path: &Path) -> bool {
    path.as_os_str().len() >= PATH_MAX_BYTES
}

/// The directories `create_dir_all(path)` is about to make, deepest first.
fn missing_ancestors(path: &Path) -> Vec<PathBuf> {
    path.ancestors()
        .take_while(|dir| !dir.exists())
        .map(Path::to_path_buf)
        .collect()
}

/// `name`, then `name (2)`, `name (3)`…, cut to 255 bytes. Directories are never split at a dot.
fn directory_candidate(name: &str, n: u32) -> String {
    let suffix = if n == 1 { String::new() } else { format!(" ({n})") };
    let mut stem = name.to_string();
    while stem.len() + suffix.len() > MAX_NAME_BYTES {
        stem.pop();
    }
    format!("{stem}{suffix}")
}

/// One event folder being written.
pub(crate) struct KeptWriter<'a> {
    /// Resolved (`canonicalize`), so every length below is the kernel's.
    root: PathBuf,
    event: PathBuf,
    clone: &'a dyn CloneFile,
    /// The vault directory (its components) → the directory this keep made or merged into for it.
    directories: HashMap<Vec<String>, PathBuf>,
    made: Vec<PathBuf>,
    /// The root and its ancestors that `create` made, resolved, deepest first.
    made_root: Vec<PathBuf>,
    files: usize,
}

#[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 5
pub(crate) struct KeptFolder {
    #[allow(dead_code)] // first production caller: Task 5; no test reads it before then
    pub path: PathBuf,
    pub files: usize,
}

impl<'a> KeptWriter<'a> {
    /// Make `<root>/<event_name>`, or `… (2)`, `… (3)` when the name is taken (§7.1). The folder is new and empty.
    /// The root is resolved once it exists: a symlink anywhere in its prefix (`/var` → `private/var` on a Mac) makes
    /// the kernel's path longer than the one given, and §7.2's 1024-byte rule is about the kernel's.
    #[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 5
    pub(crate) fn create(root: &Path, event_name: &str, clone: &'a dyn CloneFile) -> io::Result<Self> {
        let missing = missing_ancestors(root);
        std::fs::create_dir_all(root)?;
        let made_root = missing
            .iter()
            .map(std::fs::canonicalize)
            .collect::<io::Result<Vec<_>>>()?;
        let root = std::fs::canonicalize(root)?;
        let mut n = 1;
        let event = loop {
            let candidate = root.join(directory_candidate(event_name, n));
            match std::fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => n += 1,
                Err(error) => return Err(error),
            }
        };
        Ok(Self {
            root,
            event,
            clone,
            directories: HashMap::new(),
            made: Vec::new(),
            made_root,
            files: 0,
        })
    }

    #[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 5
    pub(crate) fn event(&self) -> &Path {
        &self.event
    }

    /// The directory on disk for the vault directory `components`, made one component at a time (never
    /// `create_dir_all`, which fails when a file holds the name). A directory already there is merged into (a
    /// case-only or normalization-only twin); a file there gives the next " (n)". `None`: a directory this file
    /// needs would be 1024 bytes or more after its " (n)", so the file goes to the top (no name can fail a
    /// sign-out). Directories already made for it stay, and may be empty (§7.2).
    fn directory_for(&mut self, components: &[String]) -> io::Result<Option<PathBuf>> {
        let mut parent = self.event.clone();
        for depth in 0..components.len() {
            let key = components[..=depth].to_vec();
            if let Some(known) = self.directories.get(&key) {
                parent = known.clone();
                continue;
            }
            let mut n = 1;
            let made = loop {
                let candidate = parent.join(directory_candidate(&components[depth], n));
                if too_long(&candidate) {
                    return Ok(None);
                }
                match std::fs::create_dir(&candidate) {
                    Ok(()) => {
                        self.made.push(candidate.clone());
                        break candidate;
                    }
                    Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                        let there = std::fs::symlink_metadata(&candidate)?;
                        if there.is_dir() {
                            break candidate;
                        }
                        n += 1;
                    }
                    Err(error) => return Err(error),
                }
            };
            self.directories.insert(key, made.clone());
            parent = made;
        }
        Ok(Some(parent))
    }

    /// Copy `source` to the vault path `components` (the last is the file name) and flush it. A destination of 1024
    /// bytes or more, planned or after any " (n)" on its directories or its name, goes to the top of the event folder
    /// under its last component.
    #[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 5
    pub(crate) fn keep(&mut self, components: &[String], source: &Path) -> io::Result<PathBuf> {
        let name = components
            .last()
            .expect("kept_relative_path never returns an empty path")
            .clone();
        // §7.2 "How", step 3: a staged copy that is not a regular file (a link, a pipe) is a failed copy (W8). It is
        // refused before anything is made for it, so it is never kept as a link and never counted.
        if !std::fs::symlink_metadata(source)?.file_type().is_file() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "a staged copy is not a regular file",
            ));
        }
        let planned = components.iter().fold(self.event.clone(), |path, part| path.join(part));
        let mut directory = if too_long(&planned) {
            self.event.clone()
        } else {
            self.directory_for(&components[..components.len() - 1])?
                .unwrap_or_else(|| self.event.clone())
        };
        let mut n = 1;
        let kept = loop {
            let file_name = if n == 1 {
                name.clone()
            } else {
                with_suffix(&name, &format!(" ({n})"))
            };
            let destination = directory.join(&file_name);
            if too_long(&destination) && directory != self.event {
                directory = self.event.clone();
                n = 1;
                continue;
            }
            match self.clone.clone_file(source, &destination) {
                Ok(()) => break destination,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => n += 1,
                Err(error) => return Err(error),
            }
        };
        remove_backup_exclusion(&kept)?;
        open_no_follow(&kept)?.sync_all()?;
        self.files += 1;
        Ok(kept)
    }

    /// What `finish` flushes, in order: every directory this keep made (deepest first), the event folder, the root,
    /// then the parent of each directory `create` made, which holds that directory's entry.
    pub(crate) fn flush_list(&self) -> Vec<PathBuf> {
        let parents_of_made_root = self
            .made_root
            .iter()
            .filter_map(|dir| dir.parent().map(Path::to_path_buf));
        self.made
            .iter()
            .rev()
            .cloned()
            .chain([self.event.clone(), self.root.clone()])
            .chain(parents_of_made_root)
            .collect()
    }

    /// Flush [`Self::flush_list`], then hand back the folder.
    #[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 5
    pub(crate) fn finish(self) -> io::Result<KeptFolder> {
        for directory in self.flush_list() {
            File::open(&directory)?.sync_all()?;
        }
        Ok(KeptFolder {
            path: self.event,
            files: self.files,
        })
    }

    /// A failed keep (W8): the half-made folder holds only copies of bytes still in the queue, so it goes.
    #[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 5
    pub(crate) fn abandon(self) {
        if let Err(error) = std::fs::remove_dir_all(&self.event) {
            tracing::warn!(kind = %error.kind(), "could not remove a half-made kept folder");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn sha(path: &Path) -> Vec<u8> {
        Sha256::digest(std::fs::read(path).unwrap()).to_vec()
    }

    fn staged(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join("staging").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn parts(path: &str) -> Vec<String> {
        crate::sign_out::kept_names::kept_relative_path(Some(path), "op", crate::sign_out::kept_names::KeptRole::Newest)
    }

    /// The disk rules need a case- and normalization-insensitive volume, as the default APFS is. Fail, never skip.
    fn assert_insensitive_volume(dir: &Path) {
        for (written, other_spelling, needs) in [("a", "A", "case"), ("\u{e9}", "e\u{301}", "normalization")] {
            std::fs::write(dir.join(written), b"").unwrap();
            assert!(
                dir.join(other_spelling).exists(),
                "this test needs a {needs}-insensitive volume for its temporary directory"
            );
            std::fs::remove_file(dir.join(written)).unwrap();
        }
    }

    /// U11, the disk half.
    #[test]
    fn kept_names_follow_the_rule_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        assert_insensitive_volume(dir.path());
        let root = dir.path().join("Changes not sent");
        let mut writer = KeptWriter::create(&root, "2026-10-10 at 14.05.12", &SystemClone).unwrap();
        let cases: [(&str, &str); 5] = [
            ("Work/Report.docx", "Work/report.docx"),
            ("Caf\u{e9}.txt", "Cafe\u{301}.txt"),
            ("Notes", "notes/a.txt"),
            ("dir-first/a.txt", "Dir-First"),
            ("Docs/a.txt", "docs/b.txt"),
        ];
        let mut seconds = Vec::new();
        for (index, (first, second)) in cases.iter().enumerate() {
            let a = staged(dir.path(), &format!("{index}-a"), format!("first {index}").as_bytes());
            let b = staged(dir.path(), &format!("{index}-b"), format!("second {index}").as_bytes());
            let kept_a = writer.keep(&parts(first), &a).unwrap();
            let kept_b = writer.keep(&parts(second), &b).unwrap();
            assert_ne!(kept_a, kept_b, "{first} / {second}");
            assert_eq!(sha(&kept_a), sha(&a), "{first}");
            assert_eq!(sha(&kept_b), sha(&b), "{second}");
            seconds.push(kept_b);
        }
        let event = writer.event().to_path_buf();
        // Two files the volume takes for one: the second is " (2)".
        assert_eq!(
            seconds[1],
            event.join("Cafe\u{301} (2).txt"),
            "the normalization-only twin"
        );
        assert_eq!(
            seconds[0],
            event.join("Work").join("report (2).docx"),
            "the case-only twin"
        );
        // Two directories the volume takes for one are one directory: the second file lands beside the first.
        assert!(
            event.join("Docs").join("b.txt").is_file(),
            "the directory twin is merged into"
        );
        assert!(
            !event.join("docs (2)").exists(),
            "the directory twin is not a new directory"
        );
        // A link in the event folder is never merged into: its file takes " (2)", and nothing is written through it.
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, event.join("Linked")).unwrap();
        let linked = staged(dir.path(), "linked", b"linked");
        assert_eq!(
            writer.keep(&parts("Linked/a.txt"), &linked).unwrap(),
            event.join("Linked (2)").join("a.txt"),
            "a link is never a directory to merge into"
        );
        assert_eq!(
            std::fs::read_dir(&outside).unwrap().count(),
            0,
            "nothing lands outside the event folder"
        );
        // File first: the directory takes " (2)", and a later file of that directory lands beside the first.
        assert!(event.join("notes (2)").join("a.txt").exists());
        let b2 = staged(dir.path(), "notes-b", b"b");
        assert_eq!(
            writer.keep(&parts("notes/b.txt"), &b2).unwrap(),
            event.join("notes (2)").join("b.txt")
        );
        // Directory first: the file takes " (2)".
        assert!(event.join("Dir-First (2)").is_file());
        // A 255-byte name that collides keeps its " (2)" and its extension.
        let long = format!("{}.docx", "y".repeat(250));
        let one = staged(dir.path(), "long-1", b"1");
        let two = staged(dir.path(), "long-2", b"2");
        writer.keep(&parts(&long), &one).unwrap();
        let second = writer.keep(&parts(&long), &two).unwrap();
        let name = second.file_name().unwrap().to_str().unwrap();
        assert_eq!(name.len(), 255);
        assert!(name.ends_with(" (2).docx"), "{name}");
        let kept = writer.finish().unwrap();
        assert_eq!(kept.files, 14);
    }

    /// `event/<path>` exactly `total` bytes long, from components of at most 200 bytes.
    fn padded(event: &Path, total: usize, tag: char) -> String {
        let mut remaining = total - event.as_os_str().len() - 1;
        let mut parts = Vec::new();
        while remaining > 205 {
            parts.push(tag.to_string().repeat(200));
            remaining -= 201;
        }
        parts.push(format!("{}.bin", tag.to_string().repeat(remaining - 4)));
        parts.join("/")
    }

    /// U11, path length: 1023 bytes in place, 1024 at the top, 1022 + " (2)" at the top. The test's
    /// temporary directory sits under `/var`, a symlink to `private/var`, and the kernel measures a path with every
    /// link resolved, so the writer measures the resolved path, and so does the padding here.
    #[test]
    fn long_paths_go_to_the_top_of_the_event_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Changes not sent");
        let mut writer = KeptWriter::create(&root, "2026-10-10 at 14.05.12", &SystemClone).unwrap();
        let event = writer.event().to_path_buf();
        assert_eq!(
            event,
            std::fs::canonicalize(&event).unwrap(),
            "the writer measures where the kernel measures"
        );
        let fits = padded(&event, 1023, 'a');
        let too_long = padded(&event, 1024, 'b');
        let grows = padded(&event, 1022, 'c');
        let source = |name: &str| staged(dir.path(), name, name.as_bytes());
        let kept = writer.keep(&parts(&fits), &source("fits")).unwrap();
        assert_eq!(kept, event.join(&fits), "1023 bytes is made in place");
        assert_eq!(kept.as_os_str().len(), 1023);
        assert_eq!(
            sha(&kept),
            sha(&dir.path().join("staging").join("fits")),
            "the 1023-byte file's bytes"
        );
        let kept = writer.keep(&parts(&too_long), &source("too-long")).unwrap();
        assert_eq!(kept.parent().unwrap(), event, "1024 bytes goes to the top");
        assert_eq!(
            sha(&kept),
            sha(&dir.path().join("staging").join("too-long")),
            "the 1024-byte file's bytes"
        );
        writer.keep(&parts(&grows), &source("grows-1")).unwrap();
        let kept = writer.keep(&parts(&grows), &source("grows-2")).unwrap();
        assert_eq!(
            kept.parent().unwrap(),
            event,
            "1022 bytes plus \" (2)\" goes to the top"
        );
        assert_eq!(sha(&kept), sha(&dir.path().join("staging").join("grows-2")));
        // A file holds the name of a directory a later file needs, and that directory's " (2)" would
        // pass 1023 bytes. The later file goes to the top; the sign-out never fails on a name.
        let file_first = padded(&event, 1021, 'd');
        writer.keep(&parts(&file_first), &source("file-first")).unwrap();
        let under_it = format!("{file_first}/x");
        assert_eq!(
            event.join(&under_it).as_os_str().len(),
            1023,
            "planned in place, under a directory that cannot be made"
        );
        let kept = writer.keep(&parts(&under_it), &source("under-it")).unwrap();
        assert_eq!(
            kept,
            event.join("x"),
            "the directory's \" (2)\" passes the limit, so the file goes to the top"
        );
        assert_eq!(sha(&kept), sha(&dir.path().join("staging").join("under-it")));
        writer.finish().unwrap();
    }

    /// U11, path length through a symlink: a path that is 1023 bytes through a short link is longer
    /// once the link is resolved. Every file is still kept, each under 1024 bytes as the kernel measures it.
    #[test]
    fn a_symlinked_root_still_keeps_every_file() {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let real = base.join("a-real-directory-whose-name-is-longer-than-its-link");
        std::fs::create_dir(&real).unwrap();
        let link = base.join("l");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut writer =
            KeptWriter::create(&link.join("Changes not sent"), "2026-10-10 at 14.05.12", &SystemClone).unwrap();
        let resolved = real.join("Changes not sent").join("2026-10-10 at 14.05.12");
        assert_eq!(writer.event(), resolved, "the event folder is the resolved one");
        let through_link = link.join("Changes not sent").join("2026-10-10 at 14.05.12");
        let mut kept_files = Vec::new();
        for (tag, total) in [('a', 1023), ('b', 1000), ('c', 900)] {
            let source = staged(&base, &format!("link-{tag}"), format!("bytes {tag}").as_bytes());
            let kept = writer
                .keep(&parts(&padded(&through_link, total, tag)), &source)
                .unwrap();
            assert_eq!(sha(&kept), sha(&source), "{tag}");
            assert!(
                kept.as_os_str().len() < PATH_MAX_BYTES,
                "{tag}: {} bytes",
                kept.as_os_str().len()
            );
            kept_files.push(kept);
        }
        assert_eq!(
            kept_files[0].parent().unwrap(),
            resolved,
            "1023 bytes through the link is 1073 on disk: the top"
        );
        assert!(
            kept_files[2].parent().unwrap() != resolved,
            "900 bytes through the link fits in place on disk"
        );
        assert_eq!(writer.finish().unwrap().files, 3);
    }

    /// §7.2 "Every directory the step made is fsynced": when this keep made the root, the entries
    /// that name it are flushed too, in the root's parent and in every parent `create` made.
    #[test]
    fn a_root_the_keep_made_is_flushed_in_its_parent() {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let root = base.join("Documents").join("Changes not sent");
        let mut made = KeptWriter::create(&root, "event", &SystemClone).unwrap();
        made.keep(&parts("a/b/c.txt"), &staged(&base, "nested", b"nested"))
            .unwrap();
        let event = made.event().to_path_buf();
        let flushed = made.flush_list();
        assert_eq!(
            flushed[..4],
            [event.join("a").join("b"), event.join("a"), event.clone(), root.clone()],
            "the directories this keep made, deepest first, then the event folder and the root"
        );
        for entry_holder in [&base, &base.join("Documents"), &root] {
            assert!(
                flushed.contains(entry_holder),
                "{} is flushed: {flushed:?}",
                entry_holder.display()
            );
        }
        made.finish().unwrap();
        let again = KeptWriter::create(&root, "event", &SystemClone).unwrap();
        let flushed = again.flush_list();
        assert!(flushed.contains(&root), "the root holds the new event folder's entry");
        assert!(
            !flushed.contains(&base.join("Documents")),
            "a root that was there already is not this keep's to flush in its parent"
        );
        again.abandon();
    }

    /// U28.
    #[test]
    fn kept_copies_carry_no_backup_exclusion() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let source = staged(dir.path(), "excluded", b"bytes");
        let path = CString::new(source.as_os_str().as_bytes()).unwrap();
        let name = CString::new(BACKUP_EXCLUSION_ATTRIBUTE).unwrap();
        let value = b"com.apple.backupd";
        // SAFETY: both strings are NUL-terminated and outlive the call; `value` is a valid buffer of its length.
        let set = unsafe { libc::setxattr(path.as_ptr(), name.as_ptr(), value.as_ptr().cast(), value.len(), 0, 0) };
        assert_eq!(set, 0, "set the attribute on the staged copy");
        let mut writer = KeptWriter::create(&dir.path().join("kept"), "event", &SystemClone).unwrap();
        let kept = writer.keep(&parts("a.txt"), &source).unwrap();
        let kept_c = CString::new(kept.as_os_str().as_bytes()).unwrap();
        // SAFETY: as above; a null buffer asks only for the size.
        let size = unsafe { libc::getxattr(kept_c.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0, 0, 0) };
        assert_eq!(size, -1, "the kept copy has no backup exclusion");
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ENOATTR));
    }

    /// U40.
    #[test]
    fn clone_nofollow_is_the_headers_value() {
        assert_eq!(CLONE_NOFOLLOW, 0x0001, "sys/clonefile.h:33");
        // The code above the tests, without its comment lines: a comment that quotes the call never satisfies the pin.
        let source = include_str!("kept_writer.rs");
        let code: String = source[..source.find("#[cfg(test)]").expect("the test module")]
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(code.matches("libc::clonefile(").count(), 1, "one clone call");
        let call = &code[code.find("libc::clonefile(").unwrap()..];
        let call = &call[..call.find(')').unwrap()];
        assert!(call.contains("CLONE_NOFOLLOW"), "{call}");
    }

    /// §7.2 "How", step 3: an error other than `EEXIST` is a failed copy (W8). A staged copy that is a link, to a
    /// file or to nothing, is one: it is never kept as a link and never counted.
    #[test]
    fn a_linked_staged_copy_is_a_failed_copy() {
        let dir = tempfile::tempdir().unwrap();
        let target = staged(dir.path(), "target", b"bytes");
        for (name, points_to) in [("to-a-file", target.clone()), ("to-nothing", dir.path().join("gone"))] {
            let link = dir.path().join("staging").join(name);
            std::os::unix::fs::symlink(&points_to, &link).unwrap();
            let mut writer = KeptWriter::create(&dir.path().join("kept"), name, &SystemClone).unwrap();
            let event = writer.event().to_path_buf();
            assert!(
                writer.keep(&parts("a.txt"), &link).is_err(),
                "{name}: a link is a failed copy"
            );
            assert!(
                std::fs::symlink_metadata(event.join("a.txt")).is_err(),
                "{name}: nothing is made for it"
            );
            assert_eq!(writer.files, 0, "{name}: not counted");
            writer.abandon();
            assert!(!event.exists(), "{name}: the half-made folder goes");
        }
        // A clone that makes a link, as a source swapped for one after the check would: the flush never follows it.
        struct LinkingClone;
        impl CloneFile for LinkingClone {
            fn clone_file(&self, src: &Path, dst: &Path) -> io::Result<()> {
                std::os::unix::fs::symlink(src, dst)
            }
        }
        let mut writer = KeptWriter::create(&dir.path().join("kept"), "linking", &LinkingClone).unwrap();
        assert!(
            writer.keep(&parts("a.txt"), &target).is_err(),
            "a link made at the destination is never flushed"
        );
        assert_eq!(writer.files, 0, "and never counted");
        writer.abandon();
    }

    /// §7.2 "How", step 2: the fallback makes a new file only, with the staged copy's bytes and time, private as a
    /// clone of a staged copy is. It follows no link, leaves nothing behind when the copy fails, and only `ENOTSUP`
    /// and `EXDEV` reach it.
    #[test]
    fn the_fallback_copy_makes_a_new_file_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let source = staged(dir.path(), "source", b"new bytes");
        let at = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_modified(at)
            .unwrap();
        let made = dir.path().join("made");
        copy_new(&source, &made).unwrap();
        let metadata = std::fs::metadata(&made).unwrap();
        assert_eq!(sha(&made), sha(&source), "the staged copy's bytes");
        assert_eq!(metadata.modified().unwrap(), at, "the staged copy's time");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600, "private");
        let taken = staged(dir.path(), "taken", b"first");
        assert_eq!(copy_new(&source, &taken).unwrap_err().kind(), ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read(&taken).unwrap(),
            b"first",
            "an existing file is never replaced"
        );
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&source, &link).unwrap();
        for (from, why) in [(link.as_path(), "a link"), (dir.path(), "a read that fails")] {
            let to = dir.path().join(format!("from {why}"));
            assert!(copy_new(from, &to).is_err(), "{why} is a failed copy");
            assert!(std::fs::symlink_metadata(&to).is_err(), "{why}: nothing is left behind");
        }
        for (errno, falls) in [(libc::ENOTSUP, true), (libc::EXDEV, true), (libc::EEXIST, false)] {
            assert_eq!(falls_back(Some(errno)), falls, "errno {errno}");
        }
    }

    /// A keep right after a failed one, in the same second, gets a folder of its own.
    #[test]
    fn two_keeps_in_the_same_second_make_two_folders() {
        let dir = tempfile::tempdir().unwrap();
        // Resolved first: the writer's paths are the resolved ones.
        let root = std::fs::canonicalize(dir.path()).unwrap().join("Changes not sent");
        let first = KeptWriter::create(&root, "2026-10-10 at 14.05.12", &SystemClone).unwrap();
        let first_event = first.event().to_path_buf();
        let second = KeptWriter::create(&root, "2026-10-10 at 14.05.12", &SystemClone).unwrap();
        assert_eq!(
            second.event(),
            root.join("2026-10-10 at 14.05.12 (2)"),
            "never split at the dots"
        );
        first.abandon();
        assert!(!first_event.exists(), "a failed keep removes its half-made folder");
        let third = KeptWriter::create(&root, "2026-10-10 at 14.05.12", &SystemClone).unwrap();
        assert_eq!(
            third.event(),
            first_event,
            "the name is free again, and the folder is new and empty"
        );
        assert_eq!(std::fs::read_dir(third.event()).unwrap().count(), 0);
    }
}
