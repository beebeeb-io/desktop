//! `macos_staging` items split out of `ipc_socket.rs`.

use super::*;

#[cfg(target_os = "macos")]
pub(super) fn macos_upload_staging_dir_in(home_dir: &std::path::Path) -> std::path::PathBuf {
    home_dir
        .join("Library")
        .join("Group Containers")
        .join(MACOS_APP_GROUP_ID)
        .join(MACOS_UPLOAD_STAGING_DIRNAME)
}

/// Why a staging directory itself was refused. Logged as this fixed
/// category, never with the directory's path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StagingDirRefusal {
    /// Nothing at the path.
    Missing,
    /// A symlink, not a directory, or owned by another user.
    NotPrivate,
    /// It could not be opened or inspected.
    Unavailable(std::io::ErrorKind),
}

impl StagingDirRefusal {
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub(crate) fn category(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::NotPrivate => "not_private",
            Self::Unavailable(_) => "unavailable",
        }
    }

    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub(crate) fn into_io_error(self) -> std::io::Error {
        match self {
            Self::Missing => std::io::ErrorKind::NotFound.into(),
            Self::NotPrivate => std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "staging directory is not a private directory",
            ),
            Self::Unavailable(kind) => kind.into(),
        }
    }
}

/// A staging directory opened once, without following a symlink, and checked
/// to be a real directory owned by this user. Listing and deleting its
/// entries goes through this descriptor (`fstatat`, `unlinkat`), so renaming
/// the directory away and putting a symlink in its place cannot redirect
/// them to another folder.
pub(crate) struct StagingDir {
    pub(super) fd: std::os::fd::OwnedFd,
    /// Device and inode of the opened directory, to recognise it by.
    pub(super) dev: u64,
    pub(super) ino: u64,
}

impl StagingDir {
    /// Open `path` as a staging directory owned by this process's user.
    pub(crate) fn open(path: &std::path::Path) -> Result<Self, StagingDirRefusal> {
        // SAFETY: `geteuid` has no preconditions and cannot fail.
        Self::open_owned_by(path, unsafe { libc::geteuid() })
    }

    /// [`StagingDir::open`] with the expected owner as a parameter, so the
    /// owner check is testable without a second account.
    pub(super) fn open_owned_by(path: &std::path::Path, owner: libc::uid_t) -> Result<Self, StagingDirRefusal> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;

        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| StagingDirRefusal::Unavailable(std::io::ErrorKind::InvalidInput))?;
        // SAFETY: `c_path` is NUL-terminated and lives for the call.
        let raw = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            let error = std::io::Error::last_os_error();
            return Err(match error.raw_os_error() {
                Some(libc::ENOENT) => StagingDirRefusal::Missing,
                // ELOOP: the final component is a symlink (O_NOFOLLOW).
                // ENOTDIR: it is not a directory.
                Some(libc::ELOOP) | Some(libc::ENOTDIR) => StagingDirRefusal::NotPrivate,
                _ => StagingDirRefusal::Unavailable(error.kind()),
            });
        }
        // SAFETY: `raw` is a descriptor we just opened and own.
        let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
        let stat = fstat_fd(fd.as_raw_fd()).map_err(|e| StagingDirRefusal::Unavailable(e.kind()))?;
        if !is_dir_mode(stat.st_mode) || stat.st_uid != owner {
            return Err(StagingDirRefusal::NotPrivate);
        }
        // `as u64` matches `MetadataExt::dev()` / `ino()` on every unix.
        #[allow(clippy::unnecessary_cast)]
        Ok(Self {
            fd,
            dev: stat.st_dev as u64,
            ino: stat.st_ino as u64,
        })
    }

    /// Force the directory to owner-only `0o700`, through the descriptor.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(super) fn restrict_to_owner(&self) -> std::io::Result<()> {
        use std::os::fd::AsRawFd;
        // SAFETY: the descriptor is open for the call.
        if unsafe { libc::fchmod(self.fd.as_raw_fd(), 0o700) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// The names directly inside, without `.` and `..`.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub(crate) fn entry_names(&self) -> std::io::Result<Vec<std::ffi::OsString>> {
        use std::os::fd::AsRawFd;
        list_dir_fd(self.fd.as_raw_fd())
    }

    /// `lstat` of one entry: a symlink is described, never followed.
    pub(crate) fn stat_entry(&self, name: &std::ffi::OsStr) -> std::io::Result<libc::stat> {
        use std::os::fd::AsRawFd;
        fstatat_nofollow(self.fd.as_raw_fd(), name)
    }

    /// Unlink one non-directory entry (a symlink itself, never its target).
    pub(crate) fn unlink_entry(&self, name: &std::ffi::OsStr) -> std::io::Result<()> {
        use std::os::fd::AsRawFd;
        unlink_at(self.fd.as_raw_fd(), name, 0)
    }

    /// Remove one entry: a file or a symlink is unlinked (a symlink itself,
    /// never its target); a directory is removed with everything below it,
    /// through descriptors only ([`remove_tree_at`]).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub(super) fn remove_entry(&self, name: &std::ffi::OsStr, stat: &libc::stat) -> std::io::Result<()> {
        use std::os::fd::AsRawFd;
        if is_dir_mode(stat.st_mode) {
            remove_tree_at(self.fd.as_raw_fd(), name, 0)
        } else {
            unlink_at(self.fd.as_raw_fd(), name, 0)
        }
    }
}

/// How deep [`remove_tree_at`] descends before it gives up on a tree. The
/// extension only ever stages single files; this only bounds a stray tree.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) const STAGING_TREE_MAX_DEPTH: usize = 32;

pub(super) fn is_dir_mode(mode: libc::mode_t) -> bool {
    mode & libc::S_IFMT == libc::S_IFDIR
}

pub(super) fn fstat_fd(fd: std::os::fd::RawFd) -> std::io::Result<libc::stat> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `stat` is writable storage of the right type; the descriptor is
    // open for the call.
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fstat` returned 0, so it filled `stat` in.
    Ok(unsafe { stat.assume_init() })
}

pub(super) fn entry_cstring(name: &std::ffi::OsStr) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(name.as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "name contains a NUL byte"))
}

pub(super) fn fstatat_nofollow(dir_fd: std::os::fd::RawFd, name: &std::ffi::OsStr) -> std::io::Result<libc::stat> {
    let c_name = entry_cstring(name)?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `c_name` is NUL-terminated and lives for the call; `stat` is
    // writable storage of the right type; the descriptor is open.
    if unsafe { libc::fstatat(dir_fd, c_name.as_ptr(), stat.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fstatat` returned 0, so it filled `stat` in.
    Ok(unsafe { stat.assume_init() })
}

pub(super) fn unlink_at(dir_fd: std::os::fd::RawFd, name: &std::ffi::OsStr, flags: libc::c_int) -> std::io::Result<()> {
    let c_name = entry_cstring(name)?;
    // SAFETY: `c_name` is NUL-terminated and lives for the call; the
    // descriptor is open.
    if unsafe { libc::unlinkat(dir_fd, c_name.as_ptr(), flags) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// The names directly inside the directory open at `dir_fd`, without `.`
/// and `..`. Reads a duplicate of the descriptor, so `dir_fd` stays usable.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn list_dir_fd(dir_fd: std::os::fd::RawFd) -> std::io::Result<Vec<std::ffi::OsString>> {
    use std::os::unix::ffi::OsStrExt;

    // SAFETY: duplicating an open descriptor has no other preconditions.
    let dup = unsafe { libc::fcntl(dir_fd, libc::F_DUPFD_CLOEXEC, 0) };
    if dup < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `dup` is an open directory descriptor we own; on success
    // `fdopendir` takes it over and `closedir` below closes it.
    let dirp = unsafe { libc::fdopendir(dup) };
    if dirp.is_null() {
        let error = std::io::Error::last_os_error();
        // SAFETY: `fdopendir` failed, so `dup` is still ours to close.
        unsafe { libc::close(dup) };
        return Err(error);
    }
    // The duplicate shares the read position with `dir_fd`: start over.
    // SAFETY: `dirp` is a valid stream until `closedir`.
    unsafe { libc::rewinddir(dirp) };
    let mut names = Vec::new();
    loop {
        // SAFETY: `dirp` is a valid stream; the entry it returns stays valid
        // until the next `readdir` on it, and is copied out before that.
        let entry = unsafe { libc::readdir(dirp) };
        if entry.is_null() {
            break;
        }
        // SAFETY: `d_name` is NUL-terminated within the entry.
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name != b"." && name != b".." {
            names.push(std::ffi::OsStr::from_bytes(name).to_os_string());
        }
    }
    // SAFETY: `dirp` came from `fdopendir` and is closed exactly once.
    unsafe { libc::closedir(dirp) };
    Ok(names)
}

/// Remove the directory `name` inside `parent_fd` and everything below it,
/// through descriptors only: each level is opened with `O_NOFOLLOW`, and a
/// symlink anywhere in the tree is unlinked, never followed.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn remove_tree_at(parent_fd: std::os::fd::RawFd, name: &std::ffi::OsStr, depth: usize) -> std::io::Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd};

    if depth >= STAGING_TREE_MAX_DEPTH {
        return Err(std::io::Error::other("staging tree too deep"));
    }
    let c_name = entry_cstring(name)?;
    // SAFETY: `c_name` is NUL-terminated and lives for the call; the parent
    // descriptor is open.
    let raw = unsafe {
        libc::openat(
            parent_fd,
            c_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `raw` is a descriptor we just opened and own.
    let dir = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
    // A directory without its search bit (an older extension set a copied
    // package to 0600) lets nothing inside it be inspected or unlinked. We
    // own it, so give it back to ourselves first.
    // SAFETY: the descriptor is open for the call.
    if unsafe { libc::fchmod(dir.as_raw_fd(), 0o700) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    for child in list_dir_fd(dir.as_raw_fd())? {
        let stat = fstatat_nofollow(dir.as_raw_fd(), &child)?;
        if is_dir_mode(stat.st_mode) {
            remove_tree_at(dir.as_raw_fd(), &child, depth + 1)?;
        } else {
            unlink_at(dir.as_raw_fd(), &child, 0)?;
        }
    }
    drop(dir);
    unlink_at(parent_fd, name, libc::AT_REMOVEDIR)
}

/// `SystemTime` of a `stat` timestamp; `None` before the epoch.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn stat_time(secs: libc::time_t, nanos: libc::c_long) -> Option<std::time::SystemTime> {
    let secs = u64::try_from(secs).ok()?;
    let nanos = u32::try_from(nanos).ok()?;
    std::time::UNIX_EPOCH.checked_add(std::time::Duration::new(secs, nanos))
}

/// Where `QueueFinderCreate` / `QueueFinderModify` may take file contents from.
#[derive(Debug, Clone)]
pub(crate) enum WriteContentsPolicy {
    /// Any path the daemon can read, left in place. Linux (the daemon is not
    /// sandboxed and no client sends writes over this socket) and tests that
    /// predate the staging directory.
    #[cfg_attr(all(target_os = "macos", not(test)), allow(dead_code))]
    AnyPath,
    /// Only a regular file directly inside this directory, deleted by the
    /// daemon once the request is answered. macOS (and the socket tests on
    /// every platform).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    StagingDir(std::path::PathBuf),
}

impl WriteContentsPolicy {
    pub(crate) fn platform_default() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::StagingDir(macos_upload_staging_dir())
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self::AnyPath
        }
    }
}

/// Why a write's `contents_path` was refused. Logged and returned by
/// [`ContentsRefusal::category`] only: never the path itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentsRefusal {
    /// Empty, relative, or containing a NUL byte.
    MalformedPath,
    /// Contains a `..` or `.` segment.
    Traversal,
    /// The staging directory is missing, is a symlink, or cannot be resolved.
    StagingUnavailable,
    /// Resolves outside the staging directory, or below a subdirectory of it.
    OutsideStaging,
    /// Nothing at the path.
    Missing,
    /// The path is a symbolic link.
    Symlink,
    /// Not a regular file (a directory, FIFO, socket, ...).
    NotAFile,
    /// A regular file with more than one link: a hard link to a file
    /// elsewhere, not a copy the extension made.
    HardLinked,
    /// A regular file owned by another user.
    ForeignOwner,
    /// The entry could not be inspected.
    Unreadable,
}

impl ContentsRefusal {
    pub(crate) fn category(self) -> &'static str {
        match self {
            Self::MalformedPath => "malformed_path",
            Self::Traversal => "traversal",
            Self::StagingUnavailable => "staging_unavailable",
            Self::OutsideStaging => "outside_staging",
            Self::Missing => "missing",
            Self::Symlink => "symlink",
            Self::NotAFile => "not_a_file",
            Self::HardLinked => "hard_linked",
            Self::ForeignOwner => "foreign_owner",
            Self::Unreadable => "unreadable",
        }
    }
}

/// Open `candidate` only if it names a regular file directly inside
/// `staging_dir`, and hold it open.
///
/// The staging directory is opened first ([`StagingDir::open`]: never
/// through a symlink, owned by this user). The candidate's parent must be
/// that same directory (device and inode), and the leaf is then opened
/// RELATIVE to the held descriptor with `O_NOFOLLOW | O_NONBLOCK`: a symlink
/// is refused at the open itself and a FIFO cannot block it. Every check that
/// decides acceptance runs on the opened descriptor
/// ([`check_staged_contents_stat`]), and the daemon reads the contents from
/// that descriptor only. Swapping the entry after this returns therefore
/// changes nothing the daemon reads.
pub(crate) fn open_staged_contents(
    staging_dir: &std::path::Path,
    candidate: &str,
) -> Result<OpenedContents, ContentsRefusal> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::MetadataExt;

    let path = std::path::Path::new(candidate);
    if candidate.is_empty() || candidate.contains('\0') || !path.is_absolute() {
        return Err(ContentsRefusal::MalformedPath);
    }
    // Checked on the raw text: `Path::components` silently drops interior
    // `.` segments, and the extension never sends either.
    if candidate.split('/').any(|segment| segment == ".." || segment == ".") {
        return Err(ContentsRefusal::Traversal);
    }
    // The configured directory itself must be a real directory owned by this
    // user: if it were replaced by a symlink, another folder's files would
    // become acceptable, and the daemon deletes what it accepts.
    let dir = StagingDir::open(staging_dir).map_err(|_| ContentsRefusal::StagingUnavailable)?;
    let (Some(parent), Some(leaf)) = (path.parent(), path.file_name()) else {
        return Err(ContentsRefusal::OutsideStaging);
    };
    // Directly inside only: the parent must be the held directory itself.
    match std::fs::metadata(parent) {
        Ok(meta) if meta.dev() == dir.dev && meta.ino() == dir.ino => {}
        _ => return Err(ContentsRefusal::OutsideStaging),
    }
    let c_leaf = entry_cstring(leaf).map_err(|_| ContentsRefusal::MalformedPath)?;
    // SAFETY: `c_leaf` is NUL-terminated and lives for the call; the
    // directory descriptor is open.
    let raw = unsafe {
        libc::openat(
            dir.fd.as_raw_fd(),
            c_leaf.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        let error = std::io::Error::last_os_error();
        return Err(match error.raw_os_error() {
            Some(libc::ENOENT) => ContentsRefusal::Missing,
            // O_NOFOLLOW on a symlink.
            Some(libc::ELOOP) => ContentsRefusal::Symlink,
            // A socket cannot be opened.
            Some(libc::ENXIO) | Some(libc::EOPNOTSUPP) => ContentsRefusal::NotAFile,
            _ => ContentsRefusal::Unreadable,
        });
    }
    // SAFETY: `raw` is a descriptor we just opened and own.
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
    let stat = fstat_fd(fd.as_raw_fd()).map_err(|_| ContentsRefusal::Unreadable)?;
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    check_staged_contents_stat(&stat, unsafe { libc::geteuid() })?;
    // A label for the engine's journal, rebuilt from the canonical staging
    // dir: the daemon never reads through it.
    let label = std::fs::canonicalize(staging_dir)
        .map_err(|_| ContentsRefusal::StagingUnavailable)?
        .join(leaf);
    #[allow(clippy::unnecessary_cast)]
    let identity = (stat.st_dev as u64, stat.st_ino as u64);
    Ok(OpenedContents {
        path: label,
        file: std::fs::File::from(fd),
        dir,
        leaf: leaf.to_os_string(),
        identity,
    })
}

/// A handed-over contents file accepted by [`open_staged_contents`], held
/// open. Opening it changes nothing on disk.
pub(crate) struct OpenedContents {
    /// A label for the engine's journal; the daemon never reads through it.
    pub(super) path: std::path::PathBuf,
    /// The opened file: what the engine reads.
    pub(super) file: std::fs::File,
    /// The staging directory it was opened in, and its name there: the
    /// deletion goes through these, never through a path.
    pub(super) dir: StagingDir,
    pub(super) leaf: std::ffi::OsString,
    /// Device and inode of the opened file.
    pub(super) identity: (u64, u64),
}

/// The checks on an opened handed-over file: a regular file with exactly one
/// link (the extension's copy is always a fresh file with one), owned by
/// `owner`. A hard link to another file, a FIFO, a device or a directory is
/// refused. Split out so the checks are testable on a constructed `stat`.
pub(super) fn check_staged_contents_stat(stat: &libc::stat, owner: libc::uid_t) -> Result<(), ContentsRefusal> {
    if stat.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(ContentsRefusal::NotAFile);
    }
    if stat.st_nlink != 1 {
        return Err(ContentsRefusal::HardLinked);
    }
    if stat.st_uid != owner {
        return Err(ContentsRefusal::ForeignOwner);
    }
    Ok(())
}

/// An admitted handed-over contents file. The engine copies from
/// [`StagedContents::file`], never by path. Dropping it deletes the file: by
/// then the daemon holds its own copy (`StagedPayload::copy_from_file`), or
/// the request failed and the extension's retry stages a fresh one.
///
/// The deletion is `unlinkat` on the held staging-directory descriptor, and
/// only while the name still refers to the file that was handed over (same
/// device and inode, checked with `fstatat` without following a link).
/// Renaming the staging directory away and putting a symlink in its place,
/// or putting another file under the copy's name, therefore deletes
/// nothing else; a name left behind is the age-bound purge's.
pub(crate) struct StagedContents {
    pub(super) contents: OpenedContents,
}

impl StagedContents {
    pub(super) fn path_string(&self) -> String {
        self.contents.path.to_string_lossy().into_owned()
    }

    /// The opened file: what the engine reads.
    pub(crate) fn file(&self) -> &std::fs::File {
        &self.contents.file
    }
}

impl Drop for StagedContents {
    fn drop(&mut self) {
        let contents = &self.contents;
        let result = match contents.dir.stat_entry(&contents.leaf) {
            #[allow(clippy::unnecessary_cast)]
            Ok(stat) if (stat.st_dev as u64, stat.st_ino as u64) == contents.identity => {
                contents.dir.unlink_entry(&contents.leaf)
            }
            // The name now refers to something else: not ours to delete.
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => {}
            // The extension deletes its copy after the reply too; either side may be first.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(
                error_kind = ?e.kind(),
                "could not delete a handed-over upload copy; the age-bound purge will"
            ),
        }
    }
}

/// Apply `policy` to one write request's `contents_path`. `Ok` carries the
/// path to hand the engine and, under [`WriteContentsPolicy::StagingDir`],
/// the guard that deletes the handed-over copy when dropped. `Err` is the
/// refusal, already logged; [`contents_refusal_response`] is its reply.
pub(super) fn admit_write_contents(
    policy: &WriteContentsPolicy,
    op: &'static str,
    contents_path: Option<String>,
) -> Result<(Option<String>, Option<StagedContents>), ContentsRefusal> {
    let (WriteContentsPolicy::StagingDir(staging_dir), Some(candidate)) = (policy, contents_path.as_deref()) else {
        return Ok((contents_path, None));
    };
    match open_staged_contents(staging_dir, candidate) {
        Ok(contents) => {
            let staged = StagedContents { contents };
            Ok((Some(staged.path_string()), Some(staged)))
        }
        Err(refusal) => {
            log_refused_write(op, refusal.category());
            Err(refusal)
        }
    }
}

/// The reply for a refused `contents_path`: the category, never the path.
pub(super) fn contents_refusal_response(refusal: ContentsRefusal) -> IpcResponse {
    IpcResponse::Error {
        message: format!("Upload contents refused ({})", refusal.category()),
    }
}

/// [`purge_upload_staging`] on a directory already held open.
#[cfg(any(target_os = "macos", test))]
pub(super) fn purge_upload_staging_in(staging: &StagingDir, scope: UploadStagingPurge) -> Result<usize, StagingDirRefusal> {
    let names = staging
        .entry_names()
        .map_err(|e| StagingDirRefusal::Unavailable(e.kind()))?;
    let mut removed = 0usize;
    for name in names {
        // `lstat` of the entry itself: a symlink is described, not followed.
        let Ok(stat) = staging.stat_entry(&name) else {
            continue;
        };
        let stale = match scope {
            UploadStagingPurge::Everything => true,
            // Aged by the status-change time (ctime), never the mtime: the
            // kernel sets ctime when the copy is made (an APFS clone keeps
            // the source's mtime), and no process can set it back. A future
            // ctime (clock change) counts as fresh; a later sweep gets it.
            UploadStagingPurge::OlderThan { max_age, now } => stat_time(stat.st_ctime, stat.st_ctime_nsec)
                .and_then(|changed| now.duration_since(changed).ok())
                .is_some_and(|age| age >= max_age),
        };
        if !stale {
            continue;
        }
        match staging.remove_entry(&name, &stat) {
            Ok(()) => removed += 1,
            Err(e) => tracing::warn!(error_kind = ?e.kind(), "upload-staging purge: could not remove an entry"),
        }
    }
    Ok(removed)
}

/// Age-bound purge of `dir`, logged. Split from the real-directory caller so a
/// test can run it on a temp dir.
#[cfg(any(target_os = "macos", test))]
pub(super) fn sweep_upload_staging_at(dir: &std::path::Path, context: &'static str, now: std::time::SystemTime) {
    let result = purge_stale_upload_staging(dir, UPLOAD_STAGING_MAX_AGE, now);
    log_upload_staging_purge(context, result);
}

/// Log a purge: a count, or the refusal's fixed category. Never a path.
#[cfg(any(target_os = "macos", test))]
pub(super) fn log_upload_staging_purge(context: &'static str, result: Result<usize, StagingDirRefusal>) {
    match result {
        Ok(removed) if removed > 0 => {
            tracing::info!(removed, context, "purged upload-staging copies");
        }
        Ok(_) => {}
        Err(refusal) => {
            tracing::warn!(
                reason = refusal.category(),
                context,
                "upload-staging purge refused or failed; nothing was removed"
            );
        }
    }
}

/// Log one refused Finder write: the operation and a fixed reason category,
/// never a file name, path or contents.
pub(super) fn log_refused_write(op: &'static str, reason: &'static str) {
    tracing::warn!(op, reason, "Finder write refused");
}

/// [`log_refused_write`] for an id with no row and no alias (spec §11, "Unknown id"): it also
/// logs the id the request named, which comes from the wire (M5: as `id` when it is a UUID,
/// else as `id_kind = "non_uuid"`).
pub(super) fn log_refused_unknown_item(op: &'static str, requested_id: &str) {
    let (id, id_kind) = crate::engine_bridge::wire_id_fields(requested_id);
    tracing::warn!(op, reason = "unknown_item", id, id_kind, "Finder write refused");
}
