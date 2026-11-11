//! `GlobalSource`: what the global elements ask of the engine, and the board's fixture.
//!
//! The whole window's state in one value ([`GlobalStatus`]): the connection, the session, the
//! account's space, what could not sync or back up, the first list still loading, an update that
//! is ready, the app lock. The shell asks for it; the window draws at most one banner from it
//! (`super::banner_for`), in the board's order.

/// When the list on this PC was last brought up to date: "today at 09:12".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedAt {
    /// "today", "yesterday", or a date ("12 Oct").
    pub day: String,
    /// A 24-hour clock time ("09:12").
    pub time: String,
}

/// The account's space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Storage {
    pub used_bytes: u64,
    pub quota_bytes: u64,
}

/// How full the account is, as the banners know it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StorageLevel {
    Roomy,
    /// 90 %, 95 % or 99 % (the banner comes back at each).
    AlmostFull(u8),
    Full,
}

impl Storage {
    pub fn level(self) -> StorageLevel {
        if self.quota_bytes == 0 {
            return StorageLevel::Roomy;
        }
        if self.used_bytes >= self.quota_bytes {
            return StorageLevel::Full;
        }
        let percent = self.used_bytes.saturating_mul(100) / self.quota_bytes;
        match percent {
            99.. => StorageLevel::AlmostFull(99),
            95.. => StorageLevel::AlmostFull(95),
            90.. => StorageLevel::AlmostFull(90),
            _ => StorageLevel::Roomy,
        }
    }
}

/// A size as the board writes it: whole units, decimal ("950 GB", "1 TB", "0 B").
pub fn size_words(bytes: u64) -> String {
    const UNITS: [(&str, u64); 4] = [
        ("TB", 1_000_000_000_000),
        ("GB", 1_000_000_000),
        ("MB", 1_000_000),
        ("KB", 1_000),
    ];
    for (unit, size) in UNITS {
        if bytes >= size {
            let tenths = bytes.saturating_mul(10) / size;
            return if tenths % 10 == 0 || tenths >= 100 {
                format!("{} {unit}", tenths / 10)
            } else {
                format!("{}.{} {unit}", tenths / 10, tenths % 10)
            };
        }
    }
    format!("{bytes} B")
}

/// A step of the first load (DW-0118: "File list: In progress · Thumbnails: Waiting for the file list").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    InProgress,
    WaitingForFileList,
    /// The engine's to report; the board draws the two states above.
    #[cfg_attr(not(test), allow(dead_code))]
    Done,
}

impl Step {
    pub fn words(self) -> &'static str {
        match self {
            Step::InProgress => "In progress",
            Step::WaitingForFileList => "Waiting for the file list",
            Step::Done => "Done",
        }
    }
}

/// The first list still loading in the background.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loading {
    pub listed: u64,
    pub total: u64,
    /// "about 1 min"; `None` while there is no estimate yet.
    pub left: Option<String>,
    pub file_list: Step,
    pub thumbnails: Step,
}

/// The state of the whole window. `Default` is a quiet one: nothing to say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlobalStatus {
    /// The session ended (90 days without use); the key and the list are still on this PC.
    pub signed_out: bool,
    pub storage: Option<Storage>,
    /// Items that can't sync.
    pub sync_problems: u32,
    /// Backup items that failed.
    pub backup_problems: u32,
    /// `Some`: there is no connection, and this is how old the list is.
    pub offline: Option<ListedAt>,
    pub loading: Option<Loading>,
    /// An update is downloaded and waits for a restart: its version. The window draws nothing
    /// for it (the board puts it in the b's flyout, DW-1210); the shell reads it from here.
    pub update_ready: Option<String>,
    /// The app lock is on. The lock screen is the sign-in group's (DW-0009); a banner never
    /// shows over it.
    pub locked: bool,
}

pub trait GlobalSource: Send + 'static {
    fn status(&self) -> GlobalStatus;
    /// Ask again whether there is a connection (Try again on a preview that needs one).
    /// `true`: there is one now.
    fn check_connection(&mut self) -> bool;
}

/// Nothing is connected that knows: a quiet window, and never a connection.
pub struct Quiet;

impl GlobalSource for Quiet {
    fn status(&self) -> GlobalStatus {
        GlobalStatus::default()
    }
    fn check_connection(&mut self) -> bool {
        false
    }
}

/// The board's samples, answered in memory.
#[derive(Default)]
pub struct GlobalFixture {
    pub status: GlobalStatus,
    /// What `check_connection` answers; a `true` also ends the offline state.
    pub back_online: bool,
    /// How often the connection was checked.
    pub checks: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

impl GlobalFixture {
    pub fn of(status: GlobalStatus) -> Self {
        GlobalFixture {
            status,
            ..GlobalFixture::default()
        }
    }
}

impl GlobalSource for GlobalFixture {
    fn status(&self) -> GlobalStatus {
        self.status.clone()
    }
    fn check_connection(&mut self) -> bool {
        self.checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.back_online {
            self.status.offline = None;
        }
        self.back_online
    }
}

/// The board's sample states, one per frame.
pub mod board {
    use super::{GlobalStatus, ListedAt, Loading, Step, Storage};

    const TB: u64 = 1_000_000_000_000;

    /// DW-0101: "This list is from today at 09:12."
    pub fn offline() -> GlobalStatus {
        GlobalStatus {
            offline: Some(ListedAt {
                day: "today".into(),
                time: "09:12".into(),
            }),
            ..GlobalStatus::default()
        }
    }
    /// DW-0102: "3 items can't sync."
    pub fn sync_problem() -> GlobalStatus {
        GlobalStatus {
            sync_problems: 3,
            ..GlobalStatus::default()
        }
    }
    /// DW-0103: "950 GB of 1 TB used."
    pub fn storage_almost_full() -> GlobalStatus {
        GlobalStatus {
            storage: Some(Storage {
                used_bytes: 950_000_000_000,
                quota_bytes: TB,
            }),
            ..GlobalStatus::default()
        }
    }
    /// DW-0104.
    pub fn storage_full() -> GlobalStatus {
        GlobalStatus {
            storage: Some(Storage {
                used_bytes: TB,
                quota_bytes: TB,
            }),
            ..GlobalStatus::default()
        }
    }
    /// DW-0105.
    pub fn signed_out() -> GlobalStatus {
        GlobalStatus {
            signed_out: true,
            ..GlobalStatus::default()
        }
    }
    /// DW-0106: "3 items couldn't back up."
    pub fn backup_problem() -> GlobalStatus {
        GlobalStatus {
            backup_problems: 3,
            ..GlobalStatus::default()
        }
    }
    /// DW-0118: "18,240 of 40,112 · about 1 min left".
    pub fn loading() -> GlobalStatus {
        GlobalStatus {
            loading: Some(Loading {
                listed: 18_240,
                total: 40_112,
                left: Some("about 1 min".into()),
                file_list: Step::InProgress,
                thumbnails: Step::WaitingForFileList,
            }),
            ..GlobalStatus::default()
        }
    }
}
