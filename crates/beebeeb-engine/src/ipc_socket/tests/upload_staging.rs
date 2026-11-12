//! Upload staging admission, tree removal and purge tests.

use super::*;

// ── Upload staging: contents validation, cleanup, purge ──────────────
//
// Pure functions over injected temp dirs: never the real App Group
// container. Not macOS-gated, so they also run on the Linux CI job.

/// A staging dir that is a CHILD of a temp dir, so a file can sit right
/// next to it (outside it).
/// What [`open_staged_contents`] accepted (its journal label) or refused.
fn validate(staging: &std::path::Path, candidate: &str) -> Result<std::path::PathBuf, ContentsRefusal> {
    open_staged_contents(staging, candidate).map(|contents| contents.path)
}

#[test]
fn staged_contents_directly_inside_the_staging_dir_are_accepted_and_left_intact() {
    let (_root, staging) = upload_staging_fixture();
    let candidate = write_file(&staging.join("6f1c0d1e-copy"), b"contents");
    let accepted = validate(&staging, &candidate).expect("a staged regular file is accepted");
    assert_eq!(
        accepted,
        std::fs::canonicalize(&staging).unwrap().join("6f1c0d1e-copy"),
        "the accepted path is rebuilt from the canonical staging dir"
    );
    assert_eq!(
        std::fs::read(&candidate).unwrap(),
        b"contents",
        "validation must not touch the file"
    );
}

#[test]
fn staged_contents_outside_the_staging_dir_are_refused() {
    let (root, staging) = upload_staging_fixture();
    let outside = write_file(&root.path().join("outside.txt"), b"x");
    assert_eq!(validate(&staging, &outside), Err(ContentsRefusal::OutsideStaging));
    // A subdirectory of the staging dir is outside it too: the extension
    // only ever writes directly into it.
    std::fs::create_dir(staging.join("sub")).unwrap();
    let nested = write_file(&staging.join("sub").join("copy"), b"x");
    assert_eq!(validate(&staging, &nested), Err(ContentsRefusal::OutsideStaging));
    // A parent directory that does not exist is outside too.
    let ghost = root.path().join("ghost").join("copy").to_string_lossy().into_owned();
    assert_eq!(validate(&staging, &ghost), Err(ContentsRefusal::OutsideStaging));
    assert!(
        std::path::Path::new(&outside).exists(),
        "a refused path is never deleted"
    );
}

#[test]
fn staged_contents_with_dot_segments_are_refused_even_when_they_resolve_inside() {
    let (root, staging) = upload_staging_fixture();
    write_file(&root.path().join("outside.txt"), b"x");
    write_file(&staging.join("copy"), b"x");
    let escape = format!("{}/../outside.txt", staging.display());
    let loops_back = format!("{}/../upload-staging/copy", staging.display());
    let dot = format!("{}/./copy", staging.display());
    for candidate in [escape, loops_back, dot] {
        assert_eq!(
            validate(&staging, &candidate),
            Err(ContentsRefusal::Traversal),
            "{candidate}"
        );
    }
}

#[test]
fn staged_contents_that_are_a_symlink_are_refused_and_the_target_is_untouched() {
    let (root, staging) = upload_staging_fixture();
    let target = root.path().join("target.txt");
    write_file(&target, b"target");
    let link = staging.join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(
        validate(&staging, &link.to_string_lossy()),
        Err(ContentsRefusal::Symlink)
    );
    // A symlink to a file INSIDE the staging dir is refused as well.
    write_file(&staging.join("real"), b"x");
    let inner = staging.join("inner-link");
    std::os::unix::fs::symlink(staging.join("real"), &inner).unwrap();
    assert_eq!(
        validate(&staging, &inner.to_string_lossy()),
        Err(ContentsRefusal::Symlink)
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"target");
}

#[test]
fn staged_contents_that_are_missing_or_not_a_file_or_malformed_are_refused() {
    let (_root, staging) = upload_staging_fixture();
    let missing = staging.join("never-staged").to_string_lossy().into_owned();
    assert_eq!(validate(&staging, &missing), Err(ContentsRefusal::Missing));
    std::fs::create_dir(staging.join("a-dir")).unwrap();
    let dir = staging.join("a-dir").to_string_lossy().into_owned();
    assert_eq!(validate(&staging, &dir), Err(ContentsRefusal::NotAFile));
    for malformed in ["", "relative/copy", "copy", "/tmp/with\0nul"] {
        assert_eq!(
            validate(&staging, malformed),
            Err(ContentsRefusal::MalformedPath),
            "{malformed:?}"
        );
    }
}

#[test]
fn staged_contents_are_refused_when_the_staging_dir_is_missing_or_a_symlink() {
    let (root, staging) = upload_staging_fixture();
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let candidate = write_file(&elsewhere.join("copy"), b"x");
    // The configured staging dir replaced by a symlink to another folder:
    // its files must NOT become acceptable (the daemon deletes what it accepts).
    std::fs::remove_dir(&staging).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &staging).unwrap();
    let through_link = staging.join("copy").to_string_lossy().into_owned();
    assert_eq!(
        validate(&staging, &through_link),
        Err(ContentsRefusal::StagingUnavailable)
    );
    assert_eq!(validate(&staging, &candidate), Err(ContentsRefusal::StagingUnavailable));
    let gone = root.path().join("gone");
    assert_eq!(
        validate(&gone, &gone.join("copy").to_string_lossy()),
        Err(ContentsRefusal::StagingUnavailable)
    );
}

#[test]
fn a_hard_link_in_the_staging_dir_is_refused() {
    // A hard link names the same file as an entry elsewhere: reading it
    // would read a file the extension never copied.
    let (root, staging) = upload_staging_fixture();
    let elsewhere = root.path().join("private.db");
    write_file(&elsewhere, b"private");
    let link = staging.join("hard-link");
    std::fs::hard_link(&elsewhere, &link).unwrap();
    assert_eq!(
        validate(&staging, &link.to_string_lossy()),
        Err(ContentsRefusal::HardLinked)
    );
    assert_eq!(std::fs::read(&elsewhere).unwrap(), b"private");
}

#[test]
fn the_checks_on_the_opened_file_refuse_anything_but_a_single_link_regular_file_of_ours() {
    // SAFETY: `stat` is plain old data; all-zero is a valid value.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: no preconditions.
    let me = unsafe { libc::geteuid() };
    stat.st_mode = libc::S_IFREG | 0o600;
    stat.st_nlink = 1;
    stat.st_uid = me;
    assert_eq!(check_staged_contents_stat(&stat, me), Ok(()));
    stat.st_nlink = 2;
    assert_eq!(check_staged_contents_stat(&stat, me), Err(ContentsRefusal::HardLinked));
    stat.st_nlink = 1;
    stat.st_uid = me.wrapping_add(1);
    assert_eq!(
        check_staged_contents_stat(&stat, me),
        Err(ContentsRefusal::ForeignOwner)
    );
    stat.st_uid = me;
    for kind in [
        libc::S_IFIFO,
        libc::S_IFDIR,
        libc::S_IFCHR,
        libc::S_IFSOCK,
        libc::S_IFLNK,
    ] {
        stat.st_mode = kind | 0o600;
        assert_eq!(
            check_staged_contents_stat(&stat, me),
            Err(ContentsRefusal::NotAFile),
            "mode {kind:o}"
        );
    }
}

#[test]
fn a_fifo_in_the_staging_dir_is_refused_without_blocking() {
    use std::os::unix::ffi::OsStrExt;
    let (_root, staging) = upload_staging_fixture();
    let fifo = staging.join("fifo");
    let c_fifo = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: NUL-terminated path that lives for the call.
    assert_eq!(unsafe { libc::mkfifo(c_fifo.as_ptr(), 0o600) }, 0);
    let (tx, rx) = std::sync::mpsc::channel();
    let candidate = fifo.to_string_lossy().into_owned();
    std::thread::spawn(move || {
        let _ = tx.send(validate(&staging, &candidate));
    });
    // A blocking open would wait here for a writer that never comes.
    let result = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("opening a FIFO must not block");
    assert_eq!(result, Err(ContentsRefusal::NotAFile));
}

#[test]
fn contents_refusal_categories_are_fixed_and_distinct() {
    let all = [
        ContentsRefusal::MalformedPath,
        ContentsRefusal::Traversal,
        ContentsRefusal::StagingUnavailable,
        ContentsRefusal::OutsideStaging,
        ContentsRefusal::Missing,
        ContentsRefusal::Symlink,
        ContentsRefusal::NotAFile,
        ContentsRefusal::HardLinked,
        ContentsRefusal::ForeignOwner,
        ContentsRefusal::Unreadable,
    ];
    let categories: std::collections::HashSet<&str> = all.iter().map(|r| r.category()).collect();
    assert_eq!(categories.len(), all.len(), "every refusal has its own category");
    assert!(
        categories
            .iter()
            .all(|c| c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
    );
}

#[test]
fn dropping_admitted_staged_contents_deletes_exactly_that_copy() {
    let (_root, staging) = upload_staging_fixture();
    let keep = write_file(&staging.join("other-request"), b"other");
    let candidate = write_file(&staging.join("this-request"), b"mine");
    let policy = WriteContentsPolicy::StagingDir(staging.clone());
    let (path, guard) = match admit_write_contents(&policy, "create", Some(candidate.clone())) {
        Ok(admitted) => admitted,
        Err(refusal) => panic!("a staged copy must be admitted, got {refusal:?}"),
    };
    let path = path.expect("the engine gets a contents path");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"mine",
        "the engine reads the handed-over copy"
    );
    drop(guard);
    assert!(
        !std::path::Path::new(&candidate).exists(),
        "the copy is deleted once the request is answered"
    );
    assert_eq!(
        std::fs::read(&keep).unwrap(),
        b"other",
        "another request's copy is untouched"
    );
}

#[test]
fn any_path_policy_passes_contents_through_and_deletes_nothing() {
    let dir = tempdir().unwrap();
    let candidate = write_file(&dir.path().join("a.txt"), b"x");
    let (path, guard) = match admit_write_contents(&WriteContentsPolicy::AnyPath, "create", Some(candidate.clone())) {
        Ok(admitted) => admitted,
        Err(refusal) => panic!("AnyPath refuses nothing, got {refusal:?}"),
    };
    assert_eq!(path.as_deref(), Some(candidate.as_str()));
    assert!(guard.is_none());
    assert!(std::path::Path::new(&candidate).exists());
}

#[test]
fn a_refused_staged_contents_path_is_an_error_reply_with_the_category() {
    let (root, staging) = upload_staging_fixture();
    let outside = write_file(&root.path().join("outside.txt"), b"x");
    let policy = WriteContentsPolicy::StagingDir(staging);
    let refusal = match admit_write_contents(&policy, "modify", Some(outside.clone())) {
        Err(refusal) => refusal,
        Ok((path, _)) => panic!("expected a refusal, got {path:?}"),
    };
    assert_eq!(refusal, ContentsRefusal::OutsideStaging);
    match contents_refusal_response(refusal) {
        IpcResponse::Error { message } => {
            assert!(message.contains("outside_staging"), "got {message}");
            assert!(
                !message.contains("outside.txt"),
                "the reply must not echo the path: {message}"
            );
        }
        other => panic!("expected an Error reply, got {other:?}"),
    }
    assert!(std::path::Path::new(&outside).exists());
}

fn set_mtime(path: &std::path::Path, when: std::time::SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn upload_staging_purge_removes_entries_older_than_the_bound_and_never_leaves_the_dir() {
    // Age comes from each entry's ctime, which a test cannot set back:
    // entries are made now and the sweeps pass a `now` of their own.
    let (root, staging) = upload_staging_fixture();
    let now = std::time::SystemTime::now();
    let hour = std::time::Duration::from_secs(60 * 60);
    let file = staging.join("orphan");
    write_file(&file, b"orphan");
    let tree = staging.join("stale-dir");
    std::fs::create_dir(&tree).unwrap();
    write_file(&tree.join("inside"), b"x");
    let outside = root.path().join("outside.txt");
    write_file(&outside, b"never purged");
    set_mtime(&outside, now - 10 * hour);

    assert_eq!(
        purge_stale_upload_staging(&staging, UPLOAD_STAGING_MAX_AGE, now),
        Ok(0),
        "nothing has existed for an hour yet"
    );
    assert!(file.exists() && tree.exists());

    let later = now + 3 * hour;
    assert_eq!(
        purge_stale_upload_staging(&staging, UPLOAD_STAGING_MAX_AGE, later),
        Ok(2),
        "the file and the directory are stale three hours later"
    );
    assert!(!file.exists() && !tree.exists());
    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"never purged",
        "the purge never leaves the dir"
    );
    assert!(staging.exists(), "the staging dir itself remains");
}

#[test]
fn upload_staging_purge_removes_a_stale_symlink_without_touching_its_target() {
    let (root, staging) = upload_staging_fixture();
    let target = root.path().join("target.txt");
    write_file(&target, b"target");
    std::os::unix::fs::symlink(&target, staging.join("link")).unwrap();
    let later = std::time::SystemTime::now() + 3 * UPLOAD_STAGING_MAX_AGE;
    assert_eq!(
        purge_stale_upload_staging(&staging, UPLOAD_STAGING_MAX_AGE, later).unwrap(),
        1
    );
    assert!(
        std::fs::symlink_metadata(staging.join("link")).is_err(),
        "the link itself is removed"
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"target", "its target is not");
}

#[test]
fn upload_staging_purge_of_a_missing_dir_is_a_no_op() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("upload-staging");
    assert_eq!(
        purge_stale_upload_staging(&missing, UPLOAD_STAGING_MAX_AGE, std::time::SystemTime::now()).unwrap(),
        0
    );
}

/// The configured staging dir replaced by a symlink to another folder that
/// holds files the daemon must never lose (its own queued uploads, say).
fn symlinked_staging_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let (root, staging) = upload_staging_fixture();
    std::fs::remove_dir(&staging).unwrap();
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    write_file(&elsewhere.join("queued-upload"), b"the only copy");
    std::fs::create_dir(elsewhere.join("a-folder")).unwrap();
    write_file(&elsewhere.join("a-folder").join("inside"), b"x");
    std::os::unix::fs::symlink(&elsewhere, &staging).unwrap();
    (root, staging, elsewhere)
}

fn assert_elsewhere_untouched(elsewhere: &std::path::Path) {
    assert_eq!(
        std::fs::read(elsewhere.join("queued-upload")).unwrap(),
        b"the only copy",
        "a purge through a symlinked staging dir must delete nothing"
    );
    assert_eq!(std::fs::read(elsewhere.join("a-folder").join("inside")).unwrap(), b"x");
}

#[test]
fn upload_staging_purge_refuses_a_symlinked_staging_dir_and_deletes_nothing() {
    let (_root, staging, elsewhere) = symlinked_staging_fixture();
    let later = std::time::SystemTime::now() + 3 * UPLOAD_STAGING_MAX_AGE;
    assert_eq!(
        purge_stale_upload_staging(&staging, UPLOAD_STAGING_MAX_AGE, later),
        Err(StagingDirRefusal::NotPrivate),
        "a symlinked staging dir must be refused, not followed"
    );
    assert_elsewhere_untouched(&elsewhere);
}

#[test]
fn a_staging_dir_must_be_a_real_directory_owned_by_this_user() {
    let (root, staging) = upload_staging_fixture();
    // SAFETY: no preconditions.
    let me = unsafe { libc::geteuid() };
    assert!(
        StagingDir::open_owned_by(&staging, me).is_ok(),
        "our own dir is accepted"
    );
    let refusal = |result: Result<StagingDir, StagingDirRefusal>| result.err();
    assert_eq!(
        refusal(StagingDir::open_owned_by(&staging, me.wrapping_add(1))),
        Some(StagingDirRefusal::NotPrivate),
        "a dir owned by another user must be refused"
    );
    let file = root.path().join("a-file");
    write_file(&file, b"x");
    assert_eq!(refusal(StagingDir::open(&file)), Some(StagingDirRefusal::NotPrivate));
    let link = root.path().join("link-to-staging");
    std::os::unix::fs::symlink(&staging, &link).unwrap();
    assert_eq!(
        refusal(StagingDir::open(&link)),
        Some(StagingDirRefusal::NotPrivate),
        "a symlink to a real staging dir is refused too"
    );
    assert_eq!(
        refusal(StagingDir::open(&root.path().join("gone"))),
        Some(StagingDirRefusal::Missing)
    );
}

#[test]
fn upload_staging_purge_removes_a_stale_tree_without_following_links_inside_it() {
    let (root, staging) = upload_staging_fixture();
    let outside_file = root.path().join("outside.txt");
    write_file(&outside_file, b"never purged");
    let outside_dir = root.path().join("outside-dir");
    std::fs::create_dir(&outside_dir).unwrap();
    write_file(&outside_dir.join("kept"), b"kept");
    let tree = staging.join("stray-tree");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    write_file(&tree.join("file"), b"x");
    write_file(&tree.join("sub").join("deeper"), b"x");
    std::os::unix::fs::symlink(&outside_file, tree.join("link-to-file")).unwrap();
    std::os::unix::fs::symlink(&outside_dir, tree.join("sub").join("link-to-dir")).unwrap();
    let later = std::time::SystemTime::now() + 3 * UPLOAD_STAGING_MAX_AGE;
    assert_eq!(
        purge_stale_upload_staging(&staging, UPLOAD_STAGING_MAX_AGE, later),
        Ok(1)
    );
    assert!(std::fs::symlink_metadata(&tree).is_err(), "the whole tree is removed");
    assert_eq!(std::fs::read(&outside_file).unwrap(), b"never purged");
    assert_eq!(
        std::fs::read(outside_dir.join("kept")).unwrap(),
        b"kept",
        "a link inside the tree is unlinked, never followed"
    );
    assert!(staging.is_dir(), "the staging dir itself remains");
}

#[test]
fn upload_staging_purge_removes_a_stale_tree_whose_directory_lost_its_search_bit() {
    // An older extension copied a package as a directory and then set it
    // to 0600: no search bit, so its children could not be unlinked and
    // the tree stayed forever. The purge must still remove it.
    use std::os::unix::fs::PermissionsExt;
    let (_root, staging) = upload_staging_fixture();
    let tree = staging.join("package-copy");
    std::fs::create_dir_all(tree.join("Contents")).unwrap();
    write_file(&tree.join("TXT.rtf"), b"plaintext");
    write_file(&tree.join("Contents").join("image.png"), b"plaintext");
    std::fs::set_permissions(tree.join("Contents"), std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::set_permissions(&tree, std::fs::Permissions::from_mode(0o600)).unwrap();
    let later = std::time::SystemTime::now() + 3 * UPLOAD_STAGING_MAX_AGE;
    let removed = purge_stale_upload_staging(&staging, UPLOAD_STAGING_MAX_AGE, later);
    // Leave nothing undeletable behind for the temp dir's own cleanup.
    let _ = std::fs::set_permissions(&tree, std::fs::Permissions::from_mode(0o700));
    let _ = std::fs::set_permissions(tree.join("Contents"), std::fs::Permissions::from_mode(0o700));
    assert_eq!(removed, Ok(1), "a 0600 tree must still be purged");
    assert!(std::fs::symlink_metadata(&tree).is_err(), "the whole tree is gone");
}

#[test]
fn a_fresh_copy_that_kept_an_old_mtime_is_not_purged() {
    // `copyItem` keeps the source's mtime, and anyone who can write the
    // directory can set an mtime back. Neither may make a copy that was
    // just made look orphaned.
    let (_root, staging) = upload_staging_fixture();
    let now = std::time::SystemTime::now();
    let copy = staging.join("just-staged");
    write_file(&copy, b"in flight");
    set_mtime(&copy, now - 10 * UPLOAD_STAGING_MAX_AGE);
    assert_eq!(purge_stale_upload_staging(&staging, UPLOAD_STAGING_MAX_AGE, now), Ok(0));
    assert!(
        copy.exists(),
        "a copy made just now is in flight, whatever its mtime says"
    );
}

#[test]
fn a_copy_is_purged_once_it_has_existed_for_the_bound() {
    let (_root, staging) = upload_staging_fixture();
    let made = std::time::SystemTime::now();
    let copy = staging.join("orphan");
    write_file(&copy, b"orphan");
    let minute = std::time::Duration::from_secs(60);
    assert_eq!(
        purge_stale_upload_staging(&staging, UPLOAD_STAGING_MAX_AGE, made + UPLOAD_STAGING_MAX_AGE - minute),
        Ok(0),
        "younger than the bound: kept"
    );
    assert!(copy.exists());
    assert_eq!(
        purge_stale_upload_staging(&staging, UPLOAD_STAGING_MAX_AGE, made + UPLOAD_STAGING_MAX_AGE + minute),
        Ok(1),
        "older than the bound: purged"
    );
    assert!(!copy.exists());
}

#[test]
fn dropping_the_guard_after_the_staging_dir_was_swapped_for_a_symlink_deletes_nothing_outside() {
    let (root, staging) = upload_staging_fixture();
    let candidate = write_file(&staging.join("handed-over"), b"mine");
    let policy = WriteContentsPolicy::StagingDir(staging.clone());
    let Ok((_, guard)) = admit_write_contents(&policy, "create", Some(candidate)) else {
        panic!("a staged copy must be admitted");
    };
    // After admission, upload-staging is renamed away and a symlink to a
    // folder holding a file of the same name is put in its place.
    let moved = root.path().join("moved-staging");
    std::fs::rename(&staging, &moved).unwrap();
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    write_file(&elsewhere.join("handed-over"), b"not ours to delete");
    std::os::unix::fs::symlink(&elsewhere, &staging).unwrap();
    drop(guard);
    assert_eq!(
        std::fs::read(elsewhere.join("handed-over")).unwrap(),
        b"not ours to delete",
        "the deletion must not follow a symlink put in the staging dir's place"
    );
    assert!(
        !moved.join("handed-over").exists(),
        "the handed-over copy itself is deleted, through the directory that was checked"
    );
}

#[test]
fn dropping_the_guard_leaves_a_different_file_that_took_the_copys_name() {
    let (_root, staging) = upload_staging_fixture();
    let candidate = write_file(&staging.join("handed-over"), b"mine");
    let policy = WriteContentsPolicy::StagingDir(staging.clone());
    let Ok((_, guard)) = admit_write_contents(&policy, "create", Some(candidate.clone())) else {
        panic!("a staged copy must be admitted");
    };
    std::fs::rename(&candidate, staging.join("moved-away")).unwrap();
    write_file(&staging.join("handed-over"), b"another file");
    drop(guard);
    assert_eq!(
        std::fs::read(staging.join("handed-over")).unwrap(),
        b"another file",
        "only the file that was handed over is deleted"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_failed_backup_exclusion_is_logged_without_the_path() {
    let parent = tempdir().unwrap();
    let dir = parent.path().join("upload-staging");
    let logs = capture_logs(|| {
        let result = macos_open_private_staging_dir_with(&dir, |_| Err(std::io::Error::from_raw_os_error(libc::EPERM)));
        assert!(result.is_ok(), "a failed exclusion is best-effort");
    });
    let warns: Vec<&str> = logs
        .lines()
        .filter(|line| line.contains("could not exclude a macOS staging dir from backups"))
        .collect();
    assert_eq!(warns.len(), 1, "one warning, got:\n{logs}");
    let parent_text = parent.path().to_string_lossy();
    assert!(
        !logs.contains(parent_text.as_ref()) && !logs.contains("upload-staging"),
        "the warning must not carry the path:\n{logs}"
    );
}

#[test]
fn a_refused_upload_staging_purge_is_logged_by_category_without_the_path() {
    let (root, staging, elsewhere) = symlinked_staging_fixture();
    let later = std::time::SystemTime::now() + 3 * UPLOAD_STAGING_MAX_AGE;
    let logs = capture_logs(|| sweep_upload_staging_at(&staging, "periodic", later));
    let warns: Vec<&str> = logs
        .lines()
        .filter(|line| line.contains("upload-staging purge refused"))
        .collect();
    assert_eq!(warns.len(), 1, "one warning for the refused purge, got:\n{logs}");
    assert!(
        warns[0].contains("not_private"),
        "the warning names the category: {}",
        warns[0]
    );
    let root_text = root.path().to_string_lossy();
    assert!(
        !logs.contains(root_text.as_ref()),
        "the warning must not carry the path:\n{logs}"
    );
    assert_elsewhere_untouched(&elsewhere);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_prepare_upload_staging_dir_refuses_a_symlink_and_never_touches_its_target() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    let (_root, staging, elsewhere) = symlinked_staging_fixture();
    std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o755)).unwrap();
    let later = std::time::SystemTime::now() + 3 * UPLOAD_STAGING_MAX_AGE;
    assert_eq!(
        macos_prepare_upload_staging_dir(&staging, UPLOAD_STAGING_MAX_AGE, later),
        Err(StagingDirRefusal::NotPrivate)
    );
    let mode = std::fs::metadata(&elsewhere).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o755,
        "the hardening must not chmod the symlink's target, got {mode:o}"
    );
    let attr_name = std::ffi::CString::new("com.apple.metadata:com_apple_backup_excludeItem").unwrap();
    let path_c = std::ffi::CString::new(elsewhere.as_os_str().as_bytes()).unwrap();
    // SAFETY: NUL-terminated strings that live for the call; a NULL buffer asks for the size.
    let n = unsafe { libc::getxattr(path_c.as_ptr(), attr_name.as_ptr(), std::ptr::null_mut(), 0, 0, 0) };
    assert!(n < 0, "the backup exclusion must not land on the symlink's target");
    assert_elsewhere_untouched(&elsewhere);
}

#[test]
fn write_refusal_categories_classify_without_the_error_text() {
    let io: anyhow::Error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "/Users/x/secret.txt").into();
    assert_eq!(write_refusal_category(&io), "io");
    let stopping = anyhow::anyhow!("engine is stopping; refusing to enqueue a new local write");
    assert_eq!(write_refusal_category(&stopping), "engine_stopping");
    let not_a_file = anyhow::anyhow!("Upload source is not a file");
    assert_eq!(write_refusal_category(&not_a_file), "contents_not_a_file");
    let policy = anyhow::anyhow!("read-only shared folder cannot accept new Finder items");
    assert_eq!(write_refusal_category(&policy), "write_policy");
    let db: anyhow::Error = rusqlite::Error::QueryReturnedNoRows.into();
    assert_eq!(write_refusal_category(&db), "database");
    let in_trash = anyhow::Error::new(crate::engine_bridge::ItemInTrash);
    assert_eq!(write_refusal_category(&in_trash), "in_trash");
    let other = anyhow::anyhow!("something else about notes.txt");
    assert_eq!(write_refusal_category(&other), "other");
}

#[cfg(target_os = "macos")]
#[test]
fn macos_upload_staging_dir_is_inside_the_group_container() {
    let home = std::path::Path::new("/Users/someone");
    assert_eq!(
        macos_upload_staging_dir_in(&crate::identity::REAL, home),
        std::path::PathBuf::from(
            "/Users/someone/Library/Group Containers/R8352WDJJR.io.beebeeb.app.fileprovider/upload-staging"
        )
    );
    assert_ne!(
        macos_upload_staging_dir_in(&crate::identity::REAL, home),
        macos_hydrate_cache_dir_in(&crate::identity::REAL, home),
        "upload staging and hydrate staging are different directories"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_prepare_upload_staging_dir_hardens_it_and_purges_stale_copies() {
    use std::os::unix::fs::PermissionsExt;
    let parent = tempdir().unwrap();
    let dir = parent.path().join("upload-staging");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let stale = dir.join("stale");
    write_file(&stale, b"orphan");
    let now = std::time::SystemTime::now();
    let later = now + 2 * UPLOAD_STAGING_MAX_AGE;
    let removed = macos_prepare_upload_staging_dir(&dir, UPLOAD_STAGING_MAX_AGE, later).unwrap();
    assert_eq!(removed, 1);
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "upload-staging must be owner-only, got {mode:o}");
    // Created when missing, too.
    let fresh = parent.path().join("fresh-staging");
    assert_eq!(
        macos_prepare_upload_staging_dir(&fresh, UPLOAD_STAGING_MAX_AGE, now).unwrap(),
        0
    );
    assert!(fresh.is_dir());
}
