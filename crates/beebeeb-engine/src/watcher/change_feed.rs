//! dc-166: what changed under the sync root since the scan last looked, so an idle pass opens no file.
//!
//! At 41,510 placeholders the full walk every [`SCAN_INTERVAL`] cost 4,300-4,600 file operations a second
//! (one listing per folder and one attribute open per skip candidate). On Windows the volume's USN change
//! journal names every create, write, rename and delete NTFS records, Cloud Files filter or not (the filter
//! sits above NTFS; the `notify`/ReadDirectoryChanges route does not fire on a connected root, see the
//! watcher module docs). The scan then visits only the named paths; a full walk still runs every
//! [`FULL_RECHECK_INTERVAL`] (an hour) as the backstop, and at once whenever the feed cannot say
//! ([`Changes::Unknown`]: a journal gap or overflow) or the app resumes from sleep.
//! Elsewhere, or when the journal cannot be opened, there is no feed and every pass is a full walk, as before.
//!
//! RC2-1: read without administrator rights (what an installed app gets), the journal's records carry no
//! file name. A record then named only its parent folder: a file in a subfolder was found because that
//! folder was walked, a file in the top level became "the sync root itself", which the scan passes over,
//! so it waited for the hourly walk. A nameless record is now resolved through the file's own id; one
//! whose file is gone names its folder, and the scan lists the top level when the root itself is named.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};

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
                "enumeration scan: following the change journal; full walk every 60 min"
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

/// RC5-1: how often the scan loop reads the feed between scans, so the status knows of a change long before the
/// scan (every [`SCAN_INTERVAL`], which stays: it lets a copied file finish landing) looks at it.
pub(super) const GATHER_INTERVAL: Duration = Duration::from_millis(500);

/// RC5-1: the feed named a change under the sync root that no scan has looked at yet. Read by the status pulse:
/// in the seconds between a copy and the scan the status said "Up to date" with the copied files waiting. One
/// engine runs a watcher at a time (as with the Cloud Files notify sender), so one flag.
static CHANGES_PENDING: AtomicBool = AtomicBool::new(false);

/// RC5-1: has the change feed named something the scan has not looked at yet? `false` where there is no feed.
pub fn changes_pending() -> bool {
    CHANGES_PENDING.load(Ordering::Relaxed)
}

pub(super) fn set_changes_pending(pending: bool) {
    CHANGES_PENDING.store(pending, Ordering::Relaxed);
}

/// RC5-1: a feed read often, its changes held for the scan. [`Gathered::gather`] reads the inner feed and keeps what
/// it named; [`ChangeFeed::take`] hands everything held since the last take to the scan. [`Gathered::named`] is the
/// status signal: a take that named paths since the scan's last take. A feed that cannot say ([`Changes::Unknown`],
/// which [`NoFeed`] always answers) names nothing, so no feed never reads as a change waiting.
pub struct Gathered {
    feed: Box<dyn ChangeFeed>,
    held: Changes,
    named: bool,
}

impl Gathered {
    pub fn new(feed: Box<dyn ChangeFeed>) -> Self {
        Self {
            feed,
            held: Changes::Nothing,
            named: false,
        }
    }

    /// Read the inner feed and hold what it says until the scan takes it.
    pub fn gather(&mut self) {
        let more = self.feed.take();
        if matches!(more, Changes::Paths(_)) {
            self.named = true;
        }
        self.held = match (std::mem::replace(&mut self.held, Changes::Nothing), more) {
            (Changes::Unknown, _) | (_, Changes::Unknown) => Changes::Unknown,
            (Changes::Nothing, other) | (other, Changes::Nothing) => other,
            (Changes::Paths(mut held), Changes::Paths(more)) => {
                held.extend(more);
                if held.len() > MAX_PATHS_PER_TAKE {
                    Changes::Unknown
                } else {
                    Changes::Paths(held)
                }
            }
        };
    }

    /// The feed named a change since the scan last took.
    pub fn named(&self) -> bool {
        self.named
    }
}

impl ChangeFeed for Gathered {
    fn take(&mut self) -> Changes {
        self.gather();
        self.named = false;
        std::mem::replace(&mut self.held, Changes::Nothing)
    }
}

#[cfg(target_os = "windows")]
mod usn {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAGS_AND_ATTRIBUTES, FILE_ID_DESCRIPTOR,
        FILE_ID_DESCRIPTOR_0, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FileIdType, GETFINALPATHNAMEBYHANDLE_FLAGS, GetFinalPathNameByHandleW, OpenFileById,
        VOLUME_NAME_DOS,
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
            Self::open_by(sync_root, None)
        }

        /// [`UsnFeed::open`], through the `only`-th way of reading the journal alone (tests).
        pub(super) fn open_by(sync_root: &Path, only: Option<usize>) -> Result<Self, &'static str> {
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
                tried.push((0, v, FSCTL_READ_USN_JOURNAL));
            }
            if let Ok(v) = open_volume(FILE_READ_ATTRIBUTES.0) {
                tried.push((1, v, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL));
            }
            if let Ok(d) = open_dir() {
                tried.push((2, d, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL));
            }
            for (way, journal, read_code) in tried {
                if only.is_some_and(|only| only != way) {
                    continue;
                }
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
            let resolved = self.under_root(resolve(&self.root_dir, parent, FILE_FLAG_BACKUP_SEMANTICS));
            self.parents.insert(parent, resolved.clone());
            resolved
        }

        /// `full` (as the system spells it) as a path under the registered sync root; the root itself for
        /// the root. `None`: elsewhere.
        fn under_root(&self, full: Option<PathBuf>) -> Option<PathBuf> {
            let full = full?;
            let rel = full.strip_prefix(&self.canonical_root).ok()?;
            Some(if rel.as_os_str().is_empty() {
                self.sync_root.clone()
            } else {
                self.sync_root.join(rel)
            })
        }

        /// RC2-1: where the file or folder with id `file` is now, under the sync root, for a record that
        /// carries no name. One attribute-only open that follows no link and reads no data. `None`: gone,
        /// or elsewhere.
        fn own_path(&self, file: u64) -> Option<PathBuf> {
            self.under_root(resolve(
                &self.root_dir,
                file,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            ))
        }
    }

    impl ChangeFeed for UsnFeed {
        fn take(&mut self) -> Changes {
            let mut buf = vec![0u8; BUF];
            let mut paths = BTreeSet::new();
            let mut forget_parents = false;
            // RC2-1: nameless records by file id, so a file with many records in one take is opened once.
            let mut nameless: HashMap<u64, Option<PathBuf>> = HashMap::new();
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
                        let file = u64::from_le_bytes(rec[8..16].try_into().unwrap());
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
                        if name_len == 0 {
                            // RC2-1: the read without administrator rights names no file. The file's own
                            // id says where it is; a file that is gone names the folder it was in.
                            let dir = self.parent_path(parent);
                            let own = match nameless.get(&file) {
                                Some(known) => known.clone(),
                                None => {
                                    let own = self.own_path(file);
                                    nameless.insert(file, own.clone());
                                    own
                                }
                            };
                            if let Some(path) = own.or(dir) {
                                paths.insert(path);
                            }
                        } else if name_at + name_len <= len
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
    fn resolve(hint: &std::fs::File, id: u64, flags: FILE_FLAGS_AND_ATTRIBUTES) -> Option<PathBuf> {
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
                flags,
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

    fn paths(names: &[&str]) -> Changes {
        Changes::Paths(names.iter().map(PathBuf::from).collect())
    }

    /// RC5-1: what the feed names between scans is held for the scan (none of it lost) and is the status signal
    /// until the scan takes it; a feed that cannot say is never a change waiting.
    #[test]
    fn gathered_changes_wait_for_the_scan_and_signal_until_taken() {
        let told = Told([paths(&["a"]), Changes::Nothing, paths(&["b"])].into());
        let mut feed = Gathered::new(Box::new(told));
        assert!(!feed.named());
        feed.gather();
        assert!(feed.named(), "the journal named a copied file: a change waits");
        feed.gather();
        assert!(feed.named(), "still waits: the scan has not looked");
        assert_eq!(feed.take(), paths(&["a", "b"]), "the scan gets all of it");
        assert!(!feed.named(), "taken");
        assert_eq!(feed.take(), Changes::Nothing);

        let mut none = Gathered::new(Box::new(NoFeed));
        none.gather();
        assert!(!none.named(), "no feed: never a change waiting");
        assert_eq!(none.take(), Changes::Unknown, "and the scan still walks everything");

        let mut gap = Gathered::new(Box::new(Told([paths(&["a"]), Changes::Unknown].into())));
        gap.gather();
        gap.gather();
        assert!(gap.named());
        assert_eq!(
            gap.take(),
            Changes::Unknown,
            "a gap after a change still asks for the full walk"
        );
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

    /// dc-174 red test: no full walk within 60 minutes of idle; a journal overflow walks at once.
    #[test]
    fn full_walk_is_hourly_and_an_overflow_walks_at_once() {
        let f = fixture();
        let mut memo = ScanMemo::default();
        let mut feed = Told(Default::default());
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);
        memo.age_by(Duration::from_secs(59 * 60));
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);
        assert_eq!(memo.visited, 0, "no full walk 59 minutes into idle");
        let mut feed = Told([Changes::Unknown].into());
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);
        assert!(
            memo.visited >= 6,
            "an overflow walks everything at once: {}",
            memo.visited
        );
        memo.age_by(Duration::from_secs(60 * 60));
        let mut feed = Told(Default::default());
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);
        assert!(memo.visited >= 6, "the hourly backstop walks");
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

    /// RC2-1: a new file in the top level that the feed names is queued like a nested one.
    #[test]
    fn root_level_new_file_named_by_the_feed_is_queued() {
        let f = fixture();
        let mut memo = ScanMemo::default();
        let mut feed = Told(Default::default());
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);

        let new = f.root.join("root-new.txt");
        std::fs::write(&new, b"dropped in the top level").unwrap();
        feed.0.push_back(Changes::Paths([new].into()));
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);
        assert_eq!(memo.visited, 1);
        assert!(
            f.db.get_file_by_path("root-new.txt").unwrap().is_some(),
            "the named top-level file is queued"
        );
    }

    /// RC2-1 red test: a feed that can only say "something in the sync root's top level" (a nameless
    /// journal record whose file cannot be resolved) names the root itself. The top level is listed, its
    /// new file queued once, and no subfolder is walked.
    #[test]
    fn root_level_new_file_is_queued_when_the_feed_names_the_root_itself() {
        let f = fixture();
        let mut memo = ScanMemo::default();
        let mut feed = Told(Default::default());
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);

        std::fs::write(f.root.join("root-new.txt"), b"dropped in the top level").unwrap();
        std::fs::write(f.root.join("root-two.txt"), b"and another").unwrap();
        feed.0.push_back(Changes::Paths(
            [f.root.clone(), f.root.join(""), f.root.join("root-two.txt")].into(),
        ));
        scan_tick(&f.bridge, &f.root, &mut memo, &mut feed);
        assert!(f.db.get_file_by_path("root-new.txt").unwrap().is_some());
        assert!(f.db.get_file_by_path("root-two.txt").unwrap().is_some());
        assert_eq!(
            memo.visited, 3,
            "a.txt and the two new files, each once; nothing under Photos"
        );
    }

    /// RC2-1 red test on a real NTFS folder: every way the journal can be read (with administrator
    /// rights; without them on the volume or through the folder, where records carry no name) names a new
    /// file in the top level, written, copied in or renamed in, as it names a nested one. Before the fix
    /// the read without rights named `sub` and `newdir` and, for the top level, nothing.
    #[cfg(target_os = "windows")]
    #[test]
    fn root_level_new_files_are_named_by_every_way_of_reading_the_journal() {
        let mut opened = Vec::new();
        for way in 0..3 {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("sync-root");
            std::fs::create_dir_all(root.join("sub")).unwrap();
            let outside = temp.path().join("outside.txt");
            std::fs::write(&outside, b"made elsewhere").unwrap();
            let Ok(mut feed) = usn::UsnFeed::open_by(&root, Some(way)) else {
                continue;
            };
            opened.push(way);
            let _ = feed.take();
            std::fs::write(root.join("written.txt"), b"written").unwrap();
            std::fs::copy(&outside, root.join("copied.txt")).unwrap();
            std::fs::rename(&outside, root.join("renamed-in.txt")).unwrap();
            std::fs::write(root.join("sub/nested.txt"), b"nested").unwrap();
            std::fs::create_dir(root.join("newdir")).unwrap();
            std::fs::write(root.join("newdir/inside.txt"), b"inside").unwrap();
            std::fs::write(root.join("gone.txt"), b"gone").unwrap();
            std::fs::remove_file(root.join("gone.txt")).unwrap();
            let Changes::Paths(named) = feed.take() else {
                panic!("way {way}: the feed names paths");
            };
            for rel in [
                "written.txt",
                "copied.txt",
                "renamed-in.txt",
                "sub/nested.txt",
                "newdir/inside.txt",
            ] {
                assert!(
                    named.contains(&root.join(rel)),
                    "way {way}: {rel} is named in {named:?}"
                );
            }
            assert!(
                named.iter().all(|p| p.starts_with(&root)),
                "way {way}: nothing outside the root: {named:?}"
            );
        }
        println!("RC2-1 ways of reading the journal opened here: {opened:?}");
        assert!(!opened.is_empty(), "no way of reading the journal opened");
    }

    /// RC2-1 on a real NTFS folder, journal to queue: new files in the top level and in a subfolder are
    /// queued by the tick after they appear, through every way of reading the journal.
    #[cfg(target_os = "windows")]
    #[test]
    fn root_level_new_files_from_the_journal_are_queued() {
        for way in 0..3 {
            let f = fixture();
            let Ok(feed) = usn::UsnFeed::open_by(&f.root, Some(way)) else {
                continue;
            };
            let mut feed: Box<dyn ChangeFeed> = Box::new(feed);
            let mut memo = ScanMemo::default();
            scan_tick(&f.bridge, &f.root, &mut memo, feed.as_mut());
            // Whatever the first walk's own queueing left in the journal is taken here.
            scan_tick(&f.bridge, &f.root, &mut memo, feed.as_mut());

            for n in 0..18 {
                std::fs::write(f.root.join(format!("root-{n:02}.txt")), format!("top level {n}")).unwrap();
            }
            std::fs::write(f.root.join("Photos/nested-new.jpg"), b"nested").unwrap();
            scan_tick(&f.bridge, &f.root, &mut memo, feed.as_mut());
            for n in 0..18 {
                assert!(
                    f.db.get_file_by_path(&format!("root-{n:02}.txt")).unwrap().is_some(),
                    "way {way}: top-level file {n} is queued"
                );
            }
            assert!(
                f.db.get_file_by_path("Photos/nested-new.jpg").unwrap().is_some(),
                "way {way}: the nested file is queued"
            );
        }
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
