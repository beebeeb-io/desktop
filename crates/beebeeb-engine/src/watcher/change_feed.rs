//! dc-166: what changed under the sync root since the scan last looked, so an idle pass opens no file.
//!
//! At 41,510 placeholders the full walk every [`SCAN_INTERVAL`] cost 4,300-4,600 file operations a second
//! (one listing per folder and one attribute open per skip candidate). On Windows the volume's USN change
//! journal names every create, write, rename and delete NTFS records, Cloud Files filter or not (the filter
//! sits above NTFS; the `notify`/ReadDirectoryChanges route does not fire on a connected root, see the
//! watcher module docs). The scan then visits only the named paths; a full walk still runs every
//! [`FULL_RECHECK_INTERVAL`] as the backstop, and whenever the feed cannot say ([`Changes::Unknown`]).
//! Elsewhere, or when the journal cannot be opened, there is no feed and every pass is a full walk, as before.

use std::collections::BTreeSet;

use super::*;

/// What changed since the last [`ChangeFeed::take`].
#[derive(Debug, PartialEq, Eq)]
pub enum Changes {
    /// Nothing under the sync root.
    Nothing,
    /// These paths (files or folders, under the sync root) were created, written, renamed to or deleted.
    Paths(BTreeSet<PathBuf>),
    /// The feed cannot say (no feed, journal reset or overflow): walk everything.
    Unknown,
}

/// A source of [`Changes`].
pub trait ChangeFeed: Send {
    fn take(&mut self) -> Changes;
}

/// No feed: every pass walks the whole tree.
pub struct NoFeed;

impl ChangeFeed for NoFeed {
    fn take(&mut self) -> Changes {
        Changes::Unknown
    }
}

/// The best feed this platform has for `sync_root`.
pub fn open(sync_root: &Path) -> Box<dyn ChangeFeed> {
    #[cfg(target_os = "windows")]
    match usn::UsnFeed::open(sync_root) {
        Ok(feed) => {
            tracing::info!(
                mode = feed.mode(),
                "enumeration scan: following the change journal; full walk every 10 min"
            );
            return Box::new(feed);
        }
        Err(why) => {
            // Zero-knowledge: a reason word, never a path.
            tracing::warn!(
                why,
                "enumeration scan: no change journal; walking the whole folder every pass"
            );
        }
    }
    let _ = sync_root;
    Box::new(NoFeed)
}

/// Above this many changed paths in one take, a full walk is cheaper than visiting them one by one.
pub(super) const MAX_PATHS_PER_TAKE: usize = 20_000;

#[cfg(target_os = "windows")]
mod usn {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_DESCRIPTOR, FILE_ID_DESCRIPTOR_0, FILE_NAME_NORMALIZED,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdType,
        GETFINALPATHNAMEBYHANDLE_FLAGS, GetFinalPathNameByHandleW, OpenFileById, VOLUME_NAME_DOS,
    };
    use windows::Win32::System::IO::DeviceIoControl;

    const FSCTL_QUERY_USN_JOURNAL: u32 = 0x000900f4;
    const FSCTL_READ_USN_JOURNAL: u32 = 0x000900bb;
    const FSCTL_READ_UNPRIVILEGED_USN_JOURNAL: u32 = 0x000903ab;
    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const USN_REASON_FILE_DELETE: u32 = 0x0000_0200;
    const USN_REASON_RENAME_OLD_NAME: u32 = 0x0000_1000;
    const USN_REASON_RENAME_NEW_NAME: u32 = 0x0000_2000;
    /// Reads per take before the feed gives up and asks for a full walk (64 KB each).
    const MAX_READS_PER_TAKE: usize = 256;
    const BUF: usize = 64 * 1024;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct JournalData {
        id: u64,
        first_usn: i64,
        next_usn: i64,
        lowest_valid_usn: i64,
        max_usn: i64,
        maximum_size: u64,
        allocation_delta: u64,
    }

    #[repr(C)]
    struct ReadJournal {
        start_usn: i64,
        reason_mask: u32,
        return_only_on_close: u32,
        timeout: u64,
        bytes_to_wait_for: u64,
        journal_id: u64,
    }

    pub struct UsnFeed {
        /// Handle the journal is read through (the volume, or the root folder for the unprivileged read).
        journal: std::fs::File,
        /// The root folder, the volume hint for resolving a parent folder's id.
        root_dir: std::fs::File,
        read_code: u32,
        journal_id: u64,
        next_usn: i64,
        sync_root: PathBuf,
        /// `sync_root` as the system spells it (`\\?\C:\…`), to compare resolved paths against.
        canonical_root: PathBuf,
        /// Parent folder id to its path under the sync root (`None`: elsewhere or unreadable).
        parents: HashMap<u64, Option<PathBuf>>,
    }

    impl UsnFeed {
        pub fn open(sync_root: &Path) -> Result<Self, &'static str> {
            let canonical_root = std::fs::canonicalize(sync_root).map_err(|_| "root unreadable")?;
            let letter = canonical_root
                .components()
                .next()
                .and_then(|c| match c {
                    std::path::Component::Prefix(p) => match p.kind() {
                        std::path::Prefix::VerbatimDisk(d) | std::path::Prefix::Disk(d) => Some(d as char),
                        _ => None,
                    },
                    _ => None,
                })
                .ok_or("not a local disk")?;
            let share = FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0;
            let open_dir = || {
                std::fs::OpenOptions::new()
                    .access_mode(FILE_READ_ATTRIBUTES.0)
                    .share_mode(share)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
                    .open(&canonical_root)
            };
            let root_dir = open_dir().map_err(|_| "root unopenable")?;
            let volume = format!(r"\\.\{letter}:");
            let open_volume = |access: u32| {
                std::fs::OpenOptions::new()
                    .access_mode(access)
                    .share_mode(share)
                    .open(&volume)
            };
            // Admin: the volume read for reading; otherwise the unprivileged read, on the volume or the root.
            let mut tried = Vec::new();
            if let Ok(v) = open_volume(GENERIC_READ) {
                tried.push((v, FSCTL_READ_USN_JOURNAL));
            }
            if let Ok(v) = open_volume(FILE_READ_ATTRIBUTES.0) {
                tried.push((v, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL));
            }
            if let Ok(d) = open_dir() {
                tried.push((d, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL));
            }
            for (journal, read_code) in tried {
                let Some(data) = query(&journal) else { continue };
                let mut feed = UsnFeed {
                    journal,
                    root_dir: open_dir().map_err(|_| "root unopenable")?,
                    read_code,
                    journal_id: data.id,
                    next_usn: data.next_usn,
                    sync_root: sync_root.to_path_buf(),
                    canonical_root: canonical_root.clone(),
                    parents: HashMap::new(),
                };
                let mut buf = vec![0u8; BUF];
                if feed.read(&mut buf).is_some() {
                    drop(root_dir);
                    return Ok(feed);
                }
            }
            Err("journal unreadable")
        }

        pub fn mode(&self) -> &'static str {
            if self.read_code == FSCTL_READ_USN_JOURNAL {
                "volume"
            } else {
                "unprivileged"
            }
        }

        /// One read from `next_usn`; the bytes returned (the first 8 are the next USN). `None` on error.
        fn read(&mut self, buf: &mut [u8]) -> Option<usize> {
            let input = ReadJournal {
                start_usn: self.next_usn,
                reason_mask: u32::MAX,
                return_only_on_close: 0,
                timeout: 0,
                bytes_to_wait_for: 0,
                journal_id: self.journal_id,
            };
            let mut got = 0u32;
            // SAFETY: `input` and `buf` outlive the synchronous call; their sizes are passed.
            unsafe {
                DeviceIoControl(
                    HANDLE(self.journal.as_raw_handle()),
                    self.read_code,
                    Some((&input as *const ReadJournal).cast()),
                    std::mem::size_of::<ReadJournal>() as u32,
                    Some(buf.as_mut_ptr().cast()),
                    buf.len() as u32,
                    Some(&mut got),
                    None,
                )
                .ok()?;
            }
            Some(got as usize)
        }

        /// Restart from the journal's present end (after a wrap, a reset or an overflow).
        fn resync(&mut self) {
            if let Some(data) = query(&self.journal) {
                self.journal_id = data.id;
                self.next_usn = data.next_usn;
            }
            self.parents.clear();
        }

        /// The path under the sync root of the folder with file id `parent`, cached.
        fn parent_path(&mut self, parent: u64) -> Option<PathBuf> {
            if self.parents.len() > 100_000 {
                self.parents.clear();
            }
            if let Some(known) = self.parents.get(&parent) {
                return known.clone();
            }
            let resolved = resolve(&self.root_dir, parent).and_then(|full| {
                full.strip_prefix(&self.canonical_root)
                    .ok()
                    .map(|rel| self.sync_root.join(rel))
            });
            self.parents.insert(parent, resolved.clone());
            resolved
        }
    }

    impl ChangeFeed for UsnFeed {
        fn take(&mut self) -> Changes {
            let mut buf = vec![0u8; BUF];
            let mut paths = BTreeSet::new();
            let mut forget_parents = false;
            for _ in 0..MAX_READS_PER_TAKE {
                let Some(got) = self.read(&mut buf) else {
                    // Wrapped, deleted or reset journal: what happened in between is lost.
                    self.resync();
                    return Changes::Unknown;
                };
                if got < 8 {
                    self.resync();
                    return Changes::Unknown;
                }
                let next = i64::from_le_bytes(buf[0..8].try_into().unwrap());
                if got == 8 {
                    if forget_parents {
                        self.parents.clear();
                    }
                    return if paths.is_empty() {
                        Changes::Nothing
                    } else {
                        Changes::Paths(paths)
                    };
                }
                let mut at = 8usize;
                while at + 60 <= got {
                    let rec = &buf[at..got];
                    let len = u32::from_le_bytes(rec[0..4].try_into().unwrap()) as usize;
                    if len < 60 || len > rec.len() {
                        break;
                    }
                    let major = u16::from_le_bytes(rec[4..6].try_into().unwrap());
                    if major == 2 {
                        let parent = u64::from_le_bytes(rec[16..24].try_into().unwrap());
                        let reason = u32::from_le_bytes(rec[40..44].try_into().unwrap());
                        let attributes = u32::from_le_bytes(rec[52..56].try_into().unwrap());
                        let name_len = u16::from_le_bytes(rec[56..58].try_into().unwrap()) as usize;
                        let name_at = u16::from_le_bytes(rec[58..60].try_into().unwrap()) as usize;
                        if attributes & FILE_ATTRIBUTE_DIRECTORY != 0
                            && reason
                                & (USN_REASON_RENAME_OLD_NAME | USN_REASON_RENAME_NEW_NAME | USN_REASON_FILE_DELETE)
                                != 0
                        {
                            // A renamed or removed folder leaves stale paths for what is cached under it.
                            forget_parents = true;
                        }
                        if name_at + name_len <= len
                            && let Some(dir) = self.parent_path(parent)
                        {
                            let name: Vec<u16> = rec[name_at..name_at + name_len]
                                .as_chunks::<2>()
                                .0
                                .iter()
                                .map(|c| u16::from_le_bytes(*c))
                                .collect();
                            paths.insert(dir.join(String::from_utf16_lossy(&name)));
                        }
                    }
                    at += len;
                }
                self.next_usn = next;
                if forget_parents {
                    self.parents.clear();
                    forget_parents = false;
                }
                if paths.len() > MAX_PATHS_PER_TAKE {
                    self.resync();
                    return Changes::Unknown;
                }
            }
            // More records than one take reads: walk everything and start from the present end.
            self.resync();
            Changes::Unknown
        }
    }

    fn query(handle: &std::fs::File) -> Option<JournalData> {
        let mut data = JournalData::default();
        let mut got = 0u32;
        // SAFETY: `data` outlives the synchronous call; its size is passed.
        unsafe {
            DeviceIoControl(
                HANDLE(handle.as_raw_handle()),
                FSCTL_QUERY_USN_JOURNAL,
                None,
                0,
                Some((&mut data as *mut JournalData).cast()),
                std::mem::size_of::<JournalData>() as u32,
                Some(&mut got),
                None,
            )
            .ok()?;
        }
        (got as usize >= std::mem::size_of::<JournalData>()).then_some(data)
    }

    /// The full path of the file or folder with NTFS file id `id` on the volume of `hint`.
    fn resolve(hint: &std::fs::File, id: u64) -> Option<PathBuf> {
        let descriptor = FILE_ID_DESCRIPTOR {
            dwSize: std::mem::size_of::<FILE_ID_DESCRIPTOR>() as u32,
            Type: FileIdType,
            Anonymous: FILE_ID_DESCRIPTOR_0 { FileId: id as i64 },
        };
        // SAFETY: `descriptor` outlives the call; the returned handle is closed below.
        let handle = unsafe {
            OpenFileById(
                HANDLE(hint.as_raw_handle()),
                &descriptor,
                FILE_READ_ATTRIBUTES.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                FILE_FLAG_BACKUP_SEMANTICS,
            )
        }
        .ok()?;
        let mut buf = vec![0u16; 1024];
        // SAFETY: `handle` is open; `buf` is the output slice.
        let mut n = unsafe {
            GetFinalPathNameByHandleW(
                handle,
                &mut buf,
                GETFINALPATHNAMEBYHANDLE_FLAGS(FILE_NAME_NORMALIZED.0 | VOLUME_NAME_DOS.0),
            )
        } as usize;
        if n > buf.len() {
            buf = vec![0u16; n];
            // SAFETY: as above, with the size the first call asked for.
            n = unsafe {
                GetFinalPathNameByHandleW(
                    handle,
                    &mut buf,
                    GETFINALPATHNAMEBYHANDLE_FLAGS(FILE_NAME_NORMALIZED.0 | VOLUME_NAME_DOS.0),
                )
            } as usize;
        }
        // SAFETY: `handle` came from OpenFileById above and is closed once.
        let _ = unsafe { CloseHandle(handle) };
        (n > 0 && n <= buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::ApiClient;
    use crate::state_db::StateDb;

    /// A feed that answers what the test told it.
    struct Told(std::collections::VecDeque<Changes>);

    impl ChangeFeed for Told {
        fn take(&mut self) -> Changes {
            self.0.pop_front().unwrap_or(Changes::Nothing)
        }
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        root: PathBuf,
        db: Arc<StateDb>,
        bridge: EngineBridge,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sync-root");
        let state = temp.path().join("state");
        std::fs::create_dir_all(root.join("Photos/2026")).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        for (i, rel) in ["a.txt", "Photos/b.jpg", "Photos/2026/c.jpg", "Photos/2026/d.jpg"]
            .iter()
            .enumerate()
        {
            std::fs::write(root.join(rel), vec![b'x'; 10 + i]).unwrap();
        }
        let db = Arc::new(StateDb::open(state.join("state.db")).unwrap());
        // The first file list is in, so folders are scaffolded and no pass is owed for them.
        db.set_sync_cursor(1).unwrap();
        let api = Arc::new(ApiClient::new(
            "https://api.beebeeb.io".into(),
            "token".into(),
            [17u8; 32],
        ));
        let bridge = EngineBridge::new(&crate::identity::TEST, db.clone(), api);
        Fixture {
            _temp: temp,
            root,
            db,
            bridge,
        }
    }

    /// dc-166 red test: at 41,510 placeholders the idle scan did 4,300-4,600 file operations a second.
    /// With a feed that names no change, a tick after the first full walk looks at nothing.
    #[test]
    fn an_idle_tick_with_no_change_opens_no_file() {
        let f = fixture();
        let mut memo = ScanMemo::default();
        let mut feed = Told(Default::default());
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);
        assert!(memo.visited >= 6, "the first tick walks everything: {}", memo.visited);
        for _ in 0..3 {
            scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);
            assert_eq!(
                (memo.visited, memo.classified, memo.skipped),
                (0, 0, 0),
                "an idle tick opens nothing"
            );
        }
        // The backstop: once FULL_RECHECK_INTERVAL has passed, everything is walked again.
        memo.age_by(FULL_RECHECK_INTERVAL);
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);
        assert!(memo.visited >= 6, "the backstop walks everything: {}", memo.visited);
    }

    /// A file the feed names is queued, and only it is looked at; a named folder is walked whole.
    #[test]
    fn a_named_change_is_all_a_tick_looks_at() {
        let f = fixture();
        let mut memo = ScanMemo::default();
        let mut feed = Told(Default::default());
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);

        let new = f.root.join("Photos/new.jpg");
        std::fs::write(&new, b"dropped by explorer").unwrap();
        feed.0.push_back(Changes::Paths([new.clone()].into()));
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);
        assert_eq!(memo.visited, 1);
        assert!(
            f.db.get_file_by_path("Photos/new.jpg").unwrap().is_some(),
            "the named file is queued"
        );

        std::fs::create_dir_all(f.root.join("Moved/in")).unwrap();
        std::fs::write(f.root.join("Moved/in/e.txt"), b"e").unwrap();
        std::fs::write(f.root.join("Moved/f.txt"), b"f").unwrap();
        // The folder and something under it named: the folder's walk covers both, each once.
        feed.0.push_back(Changes::Paths(
            [f.root.join("Moved"), f.root.join("Moved/f.txt")].into(),
        ));
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);
        assert_eq!(memo.visited, 4, "Moved, Moved/in, e.txt, f.txt");
        assert!(f.db.get_file_by_path("Moved/in/e.txt").unwrap().is_some());

        // Gone, engine-internal and outside paths are passed over.
        std::fs::create_dir_all(f.root.join(".beebeeb")).unwrap();
        std::fs::write(f.root.join(".beebeeb/state.db-wal"), b"w").unwrap();
        feed.0.push_back(Changes::Paths(
            [
                f.root.join("gone.txt"),
                f.root.join(".beebeeb/state.db-wal"),
                PathBuf::from("/elsewhere/x"),
            ]
            .into(),
        ));
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);
        assert_eq!(memo.visited, 0);
    }

    /// A feed that cannot say walks everything, as does no feed at all.
    #[test]
    fn an_unknown_answer_walks_everything() {
        let f = fixture();
        let mut memo = ScanMemo::default();
        scan_tick(&f.bridge, &f.root, &mut memo, &mut NoFeed);
        let all = memo.visited;
        scan_tick(&f.bridge, &f.root, &mut memo, &mut NoFeed);
        assert_eq!(memo.visited, all);
        let mut feed = Told([Changes::Nothing, Changes::Unknown].into());
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);
        assert_eq!(memo.visited, 0);
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);
        assert_eq!(memo.visited, all);
    }
}
