//! Task 2526: the unix reclaim removes a sync folder copy only when its bytes are the agreed content, and
//! checks again right before it deletes. Linux only: the write lease exists nowhere else.

#[cfg(target_os = "linux")]
mod linux {
    use super::super::*;
    use crate::state_db::{FileEntry, FileStatus, ItemKind, OperationKind, PendingOperation, StateDb};
    use sha2::{Digest, Sha256};
    use std::cell::{Cell, RefCell};
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    struct Root {
        _dir: tempfile::TempDir,
        root: PathBuf,
        db: StateDb,
        identity: &'static crate::identity::EngineIdentity,
    }

    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn kept(id: &str, reason: &'static str) -> NotFreed {
        NotFreed {
            file_id: id.into(),
            reason,
        }
    }

    /// A clock far enough ahead that a file written a moment ago counts as settled (step 1).
    fn later() -> SystemTime {
        SystemTime::now() + Duration::from_secs(10)
    }

    /// A write must land in a later timer tick than the file's last change to be seen by its fingerprint.
    fn next_tick() {
        std::thread::sleep(Duration::from_millis(50));
    }

    /// These tests need a user that cannot open a mode 000 file; root can, so they fail loudly rather than pass.
    fn require_non_root(path: &Path) {
        assert!(
            std::fs::File::open(path).is_err(),
            "this test needs a user that cannot read a mode 000 path (not root): it proves nothing here"
        );
    }

    impl Root {
        fn new() -> Self {
            Self::with_identity(&crate::identity::TEST)
        }

        fn with_identity(identity: &'static crate::identity::EngineIdentity) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("sync");
            std::fs::create_dir(&root).unwrap();
            let db = StateDb::open(dir.path().join("state.db")).unwrap();
            Root {
                _dir: dir,
                root,
                db,
                identity,
            }
        }

        /// A kept-then-unkept file: a `Local`, unpinned row whose cache copy is the sync folder file `name`
        /// holding `on_disk`, with `agreed` as the row's agreed content.
        fn seed(&self, id: &str, name: &str, on_disk: &[u8], agreed: Option<String>) -> PathBuf {
            let path = self.root.join(name);
            std::fs::write(&path, on_disk).unwrap();
            self.seed_row(id, name, &path, on_disk.len(), agreed);
            path
        }

        fn seed_row(&self, id: &str, name: &str, cache_path: &Path, len: usize, agreed: Option<String>) {
            self.db
                .upsert_file_with_contract(
                    &FileEntry {
                        file_id: id.into(),
                        path: format!("/{name}"),
                        status: FileStatus::Local,
                        size_bytes: len as i64,
                        modified_at: 0,
                        content_hash: None,
                        remote_updated_at: 0,
                        parent_id: None,
                        item_kind: ItemKind::File,
                    },
                    |contract| contract.local_hash = agreed,
                )
                .unwrap();
            self.db
                .mark_cached(id, &cache_path.to_string_lossy(), len as i64, 1)
                .unwrap();
        }

        fn status(&self, id: &str) -> FileStatus {
            self.db.get_file(id).unwrap().unwrap().status
        }

        fn queue_upload(&self, id: &str) {
            self.db
                .enqueue_operation(&PendingOperation {
                    op_id: format!("op-{id}"),
                    kind: OperationKind::UploadVersion,
                    file_id: Some(id.into()),
                    parent_id: None,
                    target_path: Some("x.txt".into()),
                    metadata_json: None,
                    payload_path: None,
                    base_version: Some(1),
                    base_object_version_id: None,
                    attempts: 0,
                    max_attempts: 5,
                    next_retry_at: 0,
                    last_error: None,
                    backup_source_key: None,
                    created_at: 1,
                    updated_at: 1,
                })
                .unwrap();
        }

        /// The reclaim with the clock, the unlink, `go_on` and an act at two points of a copy's check all chosen.
        fn free_full(
            &self,
            go_on: &dyn Fn() -> bool,
            remove: &dyn Fn(&Path) -> std::io::Result<()>,
            at: &dyn Fn(&str, &str),
            now: &dyn Fn() -> SystemTime,
        ) -> FreeUpSpaceResult {
            reclaim_unix(self.identity, &self.db, &self.root, go_on, &Seams { remove, at, now }).unwrap()
        }

        fn free_with(
            &self,
            go_on: &dyn Fn() -> bool,
            remove: &dyn Fn(&Path) -> std::io::Result<()>,
            at: &dyn Fn(&str, &str),
        ) -> FreeUpSpaceResult {
            self.free_full(go_on, remove, at, &later)
        }

        fn free_at(&self, at: &dyn Fn(&str, &str)) -> FreeUpSpaceResult {
            self.free_with(&|| true, &|p| std::fs::remove_file(p), at)
        }

        fn free(&self) -> FreeUpSpaceResult {
            self.free_at(&|_, _| {})
        }
    }

    /// Red before the fix: an edit nobody detected (no watcher) in a copy that is no longer kept is deleted.
    #[test]
    fn an_edited_unkept_copy_survives_free_up_space() {
        let r = Root::new();
        let path = r.seed(
            "edited",
            "edited.txt",
            b"my edit, never sent",
            Some(sha(b"version one")),
        );
        let result = r.free();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"my edit, never sent",
            "the edit is still on disk"
        );
        assert_eq!(r.status("edited"), FileStatus::Local, "the row still says it is here");
        assert_eq!(result.bytes_freed, 0);
        assert_eq!(result.not_freed, vec![kept("edited", KEPT_EDITED)]);
    }

    #[test]
    fn an_agreed_copy_is_freed() {
        let r = Root::new();
        let path = r.seed("agreed", "agreed.txt", b"version one", Some(sha(b"version one")));
        let result = r.free();
        assert!(!path.exists(), "the agreed copy is removed");
        assert_eq!(r.status("agreed"), FileStatus::CloudOnly);
        assert_eq!(result.bytes_freed, b"version one".len() as u64);
        assert!(result.not_freed.is_empty());
    }

    #[test]
    fn a_copy_with_no_agreed_content_survives() {
        let r = Root::new();
        let path = r.seed("unknown", "unknown.txt", b"anything", None);
        let result = r.free();
        assert_eq!(std::fs::read(&path).unwrap(), b"anything");
        assert_eq!(r.status("unknown"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("unknown", KEPT_NO_AGREED)]);
    }

    /// One run, three rows: the kept ones do not hold back the agreed one, and the counts say which is which.
    #[test]
    fn a_mixed_run_frees_exactly_the_agreed_copies() {
        let r = Root::new();
        let good = r.seed("good", "good.txt", b"same", Some(sha(b"same")));
        let bad = r.seed("bad", "bad.txt", b"edited", Some(sha(b"same")));
        let none = r.seed("none", "none.txt", b"same", None);
        let result = r.free();
        assert_eq!(
            (good.exists(), bad.exists(), none.exists()),
            (false, true, true),
            "freed, kept, kept"
        );
        assert_eq!(result.not_freed.len(), 2);
        assert_eq!(result.bytes_freed, 4);
    }

    /// An agreed copy with a change of the file waiting to be sent is not the agreed content any more.
    #[test]
    fn an_agreed_copy_with_an_upload_queued_survives() {
        let r = Root::new();
        let path = r.seed("queued", "queued.txt", b"same", Some(sha(b"same")));
        r.queue_upload("queued");
        let result = r.free();
        assert!(path.exists());
        assert_eq!(r.status("queued"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("queued", KEPT_UPLOAD_QUEUED)]);
    }

    /// An upload queued while the run is under way (after the first look): the second look keeps the copy.
    #[test]
    fn an_upload_queued_during_the_run_keeps_the_copy() {
        let r = Root::new();
        let path = r.seed("late", "late.txt", b"same", Some(sha(b"same")));
        let result = r.free_at(&|stage, id| {
            if stage == "delete" {
                r.queue_upload(id);
            }
        });
        assert!(path.exists());
        assert_eq!(r.status("late"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("late", KEPT_UPLOAD_QUEUED)]);
    }

    /// A same-size write while the bytes are being read (the hash is of the old bytes and matches).
    #[test]
    fn a_write_while_the_copy_is_read_keeps_it() {
        let r = Root::new();
        let path = r.seed("busy", "busy.txt", b"version one", Some(sha(b"version one")));
        let result = r.free_at(&|stage, _| {
            if stage == "read" {
                next_tick();
                std::fs::write(&path, b"version two").unwrap();
            }
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"version two");
        assert_eq!(r.status("busy"), FileStatus::Local);
        assert_eq!(result.bytes_freed, 0);
        assert_eq!(result.not_freed, vec![kept("busy", KEPT_CHANGED)]);
    }

    /// A write that keeps the size and sets the modification time back, after the hash and before the delete.
    #[test]
    fn a_same_size_write_with_the_times_set_back_before_the_delete_keeps_it() {
        let r = Root::new();
        let path = r.seed("sneaky", "sneaky.txt", b"version one", Some(sha(b"version one")));
        let meta = std::fs::metadata(&path).unwrap();
        let result = r.free_at(&|stage, _| {
            if stage == "delete" {
                next_tick();
                let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
                use std::io::Write;
                (&file).write_all(b"version two").unwrap();
                file.set_modified(meta.modified().unwrap()).unwrap();
            }
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"version two", "the edit is on disk");
        assert_eq!(r.status("sneaky"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("sneaky", KEPT_CHANGED)]);
    }

    /// An editor that saves to a temp file and renames it over the path, after the hash.
    #[test]
    fn a_save_by_rename_before_the_delete_keeps_the_new_file() {
        let r = Root::new();
        let path = r.seed("renamed", "renamed.txt", b"version one", Some(sha(b"version one")));
        let result = r.free_at(&|stage, _| {
            if stage == "delete" {
                let temp = path.with_extension("tmp");
                std::fs::write(&temp, b"my edit").unwrap();
                std::fs::rename(&temp, &path).unwrap();
            }
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"my edit");
        assert_eq!(r.status("renamed"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("renamed", KEPT_CHANGED)]);
    }

    /// The same save during the read: the handle still holds the old file, the path names the new one.
    #[test]
    fn a_save_by_rename_during_the_read_keeps_the_new_file() {
        let r = Root::new();
        let path = r.seed("renamed2", "renamed2.txt", b"version one", Some(sha(b"version one")));
        let result = r.free_at(&|stage, _| {
            if stage == "read" {
                let temp = path.with_extension("tmp");
                std::fs::write(&temp, b"version one").unwrap();
                std::fs::rename(&temp, &path).unwrap();
            }
        });
        assert!(path.exists(), "the file now at the path is not the one that was hashed");
        assert_eq!(r.status("renamed2"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("renamed2", KEPT_CHANGED)]);
    }

    /// A keep made during the run (the row is pinned after the snapshot) keeps its file, and the row.
    #[test]
    fn a_keep_made_during_the_run_keeps_the_file() {
        let r = Root::new();
        let first = r.seed("a-first", "a.txt", b"aaaa", Some(sha(b"aaaa")));
        let second = r.seed("b-second", "b.txt", b"bbbb", Some(sha(b"bbbb")));
        let calls = Cell::new(0);
        let result = r.free_with(
            &|| {
                calls.set(calls.get() + 1);
                if calls.get() == 2 {
                    r.db.set_recursive_pin("b-second", true, 5).unwrap();
                }
                true
            },
            &|p| std::fs::remove_file(p),
            &|_, _| {},
        );
        assert!(!first.exists(), "the first copy was freed");
        assert!(second.exists(), "the copy kept during the run stays");
        assert_eq!(r.status("b-second"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("b-second", KEPT_NOW_KEPT)]);
        assert_eq!(result.bytes_freed, 4);
    }

    /// The same keep, for a copy in the engine's own cache directory: its file stays, its row stays `Local`.
    #[test]
    fn a_keep_made_during_the_run_keeps_an_engine_cache_copy() {
        let r = Root::with_identity(&crate::identity::TEST);
        let cache = dirs::cache_dir()
            .expect("a cache dir")
            .join(crate::identity::TEST.config_dir_name());
        std::fs::create_dir_all(&cache).unwrap();
        let dir = tempfile::Builder::new().prefix("free-up-").tempdir_in(&cache).unwrap();
        let first = r.seed("a-first", "a.txt", b"aaaa", Some(sha(b"aaaa")));
        let staged = dir.path().join("staged.bin");
        std::fs::write(&staged, b"staged copy").unwrap();
        r.seed_row("b-staged", "staged.bin", &staged, 11, None);
        let calls = Cell::new(0);
        let result = r.free_with(
            &|| {
                calls.set(calls.get() + 1);
                if calls.get() == 2 {
                    r.db.set_recursive_pin("b-staged", true, 5).unwrap();
                }
                true
            },
            &|p| std::fs::remove_file(p),
            &|_, _| {},
        );
        assert!(!first.exists());
        assert!(
            staged.exists(),
            "the engine cache copy of a file kept during the run stays"
        );
        assert_eq!(
            r.status("b-staged"),
            FileStatus::Local,
            "and its row is consistent with that"
        );
        assert_eq!(result.not_freed, vec![kept("b-staged", KEPT_NOW_KEPT)]);
    }

    /// A row that left `Local` during the run (here: trashing) keeps its copy, in the words for that.
    #[test]
    fn a_row_that_changed_state_during_the_run_keeps_its_copy() {
        let r = Root::new();
        let path = r.seed("moved", "moved.txt", b"same", Some(sha(b"same")));
        let result = r.free_at(&|stage, id| {
            if stage == "delete" {
                r.db.set_status(id, FileStatus::Trashing).unwrap();
            }
        });
        assert!(path.exists());
        assert_eq!(r.status("moved"), FileStatus::Trashing);
        assert_eq!(result.not_freed, vec![kept("moved", KEPT_STATE_CHANGED)]);
    }

    /// A delete the system refuses leaves the copy, and its row says it is here.
    #[test]
    fn a_refused_delete_leaves_the_row_local() {
        let r = Root::new();
        let path = r.seed("refused", "refused.txt", b"same", Some(sha(b"same")));
        let result = r.free_with(
            &|| true,
            &|_| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            &|_, _| {},
        );
        assert!(path.exists());
        assert_eq!(r.status("refused"), FileStatus::Local);
        assert_eq!(result.bytes_freed, 0);
        assert_eq!(result.not_freed, vec![kept("refused", KEPT_REMOVE_FAILED)]);
    }

    /// A session that ended stops the run before the next file.
    #[test]
    fn a_session_that_ends_stops_the_run_before_the_next_file() {
        let r = Root::new();
        let first = r.seed("a-first", "a.txt", b"aaaa", Some(sha(b"aaaa")));
        let second = r.seed("b-second", "b.txt", b"bbbb", Some(sha(b"bbbb")));
        // Per row: the loop's ask, then the ask right before the unlink (so the first row takes calls 1 and 2).
        let calls = Cell::new(0);
        let result = r.free_with(
            &|| {
                calls.set(calls.get() + 1);
                calls.get() < 3
            },
            &|p| std::fs::remove_file(p),
            &|_, _| {},
        );
        assert_eq!((first.exists(), second.exists()), (false, true));
        assert_eq!(r.status("b-second"), FileStatus::Local);
        assert!(
            result.not_freed.is_empty(),
            "nothing was looked at, so nothing is reported"
        );
        assert_eq!(result.bytes_freed, 4);
    }

    /// A session that ended while one long copy was hashed keeps that copy: asked once more before the unlink.
    #[test]
    fn a_session_that_ends_during_the_read_keeps_that_copy() {
        let r = Root::new();
        let path = r.seed("long", "long.txt", b"same", Some(sha(b"same")));
        let calls = Cell::new(0);
        let result = r.free_with(
            &|| {
                calls.set(calls.get() + 1);
                calls.get() < 2
            },
            &|p| std::fs::remove_file(p),
            &|_, _| {},
        );
        assert!(path.exists());
        assert_eq!(r.status("long"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("long", KEPT_STOPPED)]);
    }

    /// A link at the copy's place is not followed: its target (another file) is not deleted.
    #[test]
    fn a_link_in_the_sync_folder_is_never_followed() {
        let r = Root::new();
        let target = r.root.join("other.txt");
        std::fs::write(&target, b"same").unwrap();
        let link = r.root.join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        r.seed_row("linked", "link.txt", &link, 4, Some(sha(b"same")));
        let result = r.free();
        assert_eq!(std::fs::read(&target).unwrap(), b"same", "the target is untouched");
        assert!(link.symlink_metadata().is_ok(), "the link is untouched");
        assert_eq!(r.status("linked"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("linked", KEPT_NOT_A_FILE)]);
    }

    /// A copy that cannot be opened is kept, in the words for that.
    #[test]
    fn an_unopenable_copy_is_kept() {
        use std::os::unix::fs::PermissionsExt;
        let r = Root::new();
        let path = r.seed("locked", "locked.txt", b"same", Some(sha(b"same")));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        require_non_root(&path);
        let result = r.free();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(path.exists());
        assert_eq!(r.status("locked"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("locked", KEPT_UNREADABLE)]);
    }

    /// A path that cannot be resolved for any reason but "not there" is kept, not treated as gone.
    #[test]
    fn a_path_that_cannot_be_resolved_is_kept_not_gone() {
        use std::os::unix::fs::PermissionsExt;
        let r = Root::new();
        let folder = r.root.join("denied");
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("a.txt");
        std::fs::write(&path, b"same").unwrap();
        r.seed_row("denied", "denied/a.txt", &path, 4, Some(sha(b"same")));
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o000)).unwrap();
        require_non_root(&path);
        let result = r.free();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(path.exists(), "the copy is still there");
        assert_eq!(r.status("denied"), FileStatus::Local, "and its row still says so");
        assert_eq!(result.not_freed, vec![kept("denied", KEPT_UNREADABLE)]);
    }

    /// A copy whose file is gone (NotFound, confirmed) has its row marked online only: nothing to remove.
    #[test]
    fn a_copy_that_is_gone_has_its_row_marked_online_only() {
        let r = Root::new();
        let path = r.seed("gone", "gone.txt", b"same", Some(sha(b"same")));
        std::fs::remove_file(&path).unwrap();
        let result = r.free();
        assert_eq!(r.status("gone"), FileStatus::CloudOnly);
        assert!(result.not_freed.is_empty());
        assert_eq!(result.bytes_freed, 0);
    }

    /// The claim that the engine's own cache directory is freed with no byte check (no agreed content).
    #[test]
    fn a_copy_in_the_engines_own_cache_directory_is_still_freed() {
        let r = Root::with_identity(&crate::identity::TEST);
        let cache = dirs::cache_dir()
            .expect("a cache dir")
            .join(crate::identity::TEST.config_dir_name());
        std::fs::create_dir_all(&cache).unwrap();
        let dir = tempfile::Builder::new().prefix("free-up-").tempdir_in(&cache).unwrap();
        let path = dir.path().join("staged.bin");
        std::fs::write(&path, b"staged copy").unwrap();
        r.seed_row("staged", "staged.bin", &path, 11, None);
        let result = r.free();
        assert!(!path.exists());
        assert_eq!(r.status("staged"), FileStatus::CloudOnly);
        assert_eq!(result.bytes_freed, 11);
        assert!(result.not_freed.is_empty());
    }

    /// A copy written moments ago is kept: its times cannot tell a later write in the same tick. The clock is the
    /// real one here, so the copy this test just wrote is younger than `HASH_CACHE_SETTLED`.
    #[test]
    fn a_copy_written_moments_ago_is_kept() {
        let r = Root::new();
        let path = r.seed("fresh", "fresh.txt", b"same", Some(sha(b"same")));
        let result = r.free_full(&|| true, &|p| std::fs::remove_file(p), &|_, _| {}, &SystemTime::now);
        assert!(path.exists());
        assert_eq!(r.status("fresh"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("fresh", KEPT_JUST_WRITTEN)]);
    }

    /// Another descriptor on the copy (here a second `open` in the same process; any program would do the same):
    /// the write lease is refused and the copy is kept.
    #[test]
    fn a_copy_another_descriptor_has_open_is_kept() {
        let r = Root::new();
        let path = r.seed("open", "open.txt", b"same", Some(sha(b"same")));
        let holder: RefCell<Option<std::fs::File>> = RefCell::new(None);
        let result = r.free_at(&|stage, _| {
            if stage == "delete" {
                *holder.borrow_mut() = Some(std::fs::File::open(&path).unwrap());
            }
        });
        assert!(holder.borrow().is_some(), "the second descriptor was opened");
        assert!(path.exists());
        assert_eq!(r.status("open"), FileStatus::Local);
        assert_eq!(result.not_freed, vec![kept("open", KEPT_OPEN_ELSEWHERE)]);
    }
}

/// Off Linux nothing can show that no other program has a sync folder copy open, so none is removed.
#[cfg(not(target_os = "linux"))]
mod no_lease {
    use super::super::*;
    use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
    use sha2::{Digest, Sha256};

    #[test]
    fn an_agreed_sync_folder_copy_is_kept_where_there_are_no_leases() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sync");
        std::fs::create_dir(&root).unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let path = root.join("a.txt");
        std::fs::write(&path, b"same").unwrap();
        db.upsert_file_with_contract(
            &FileEntry {
                file_id: "a".into(),
                path: "/a.txt".into(),
                status: FileStatus::Local,
                size_bytes: 4,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            },
            |contract| contract.local_hash = Some(format!("{:x}", Sha256::digest(b"same"))),
        )
        .unwrap();
        db.mark_cached("a", &path.to_string_lossy(), 4, 1).unwrap();
        let result = reclaim_unix(
            &crate::identity::TEST,
            &db,
            &root,
            &|| true,
            &Seams {
                remove: &|p| std::fs::remove_file(p),
                at: &|_, _| {},
                now: &|| std::time::SystemTime::now() + std::time::Duration::from_secs(10),
            },
        )
        .unwrap();
        assert!(path.exists());
        assert_eq!(
            result.not_freed,
            vec![NotFreed {
                file_id: "a".into(),
                reason: KEPT_NO_LEASE
            }]
        );
    }
}
