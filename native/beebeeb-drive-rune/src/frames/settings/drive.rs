//! What Drive's own settings panes show (Storage, Storage insights, Sync and File Explorer,
//! Thumbnails and previews, Bandwidth, Diagnostics): `DriveSettings`, and the fixture that
//! holds exactly the board's sample content.
//!
//! A pane never invents a value: every figure, name and state it draws comes from here, as
//! the words the board shows. The engine lane replaces `BoardDrive` (which call each method
//! needs is listed beside it and in the pull request).

use super::bandwidth_data::{Bandwidth, BoardBandwidth, Direction, SpeedTest};

/// Which of the two per-PC limits (DW-0713, DW-0714).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kept {
    /// Downloaded files (DW-0714).
    Downloaded,
    /// Thumbnails and previews (DW-0713).
    Previews,
}

impl Kept {
    pub fn title(self) -> &'static str {
        match self {
            Self::Downloaded => "Downloaded files",
            Self::Previews => "Thumbnails and previews",
        }
    }
}

/// A limit on what Beebeeb keeps on this PC, and the range it may be set in (DW-0713: "from
/// 1 GB to half of the free space on this PC").
#[derive(Clone, PartialEq, Debug)]
pub struct Limit {
    pub gb: u32,
    pub min_gb: u32,
    pub max_gb: u32,
    /// Where the board draws the knob for `gb`, 0 to 1.
    pub at: f32,
    /// What the limit means right now, under the slider.
    pub says: String,
}

impl Limit {
    /// Where the knob is for `gb`: the stored limit's own place, else along the range.
    pub fn at_for(&self, gb: u32) -> f32 {
        if gb == self.gb || self.max_gb <= self.min_gb {
            return self.at;
        }
        (gb.saturating_sub(self.min_gb) as f32 / (self.max_gb - self.min_gb) as f32).clamp(0.0, 1.0)
    }
    /// The limit a knob at `at` (0 to 1) stands for.
    pub fn gb_at(&self, at: f32) -> u32 {
        let span = self.max_gb.saturating_sub(self.min_gb) as f32;
        (self.min_gb as f32 + at.clamp(0.0, 1.0) * span).round() as u32
    }
}

/// One thing Beebeeb keeps on this PC: how much it holds now, and its limit.
#[derive(Clone, PartialEq, Debug)]
pub struct KeptOnPc {
    /// "18.4 GB".
    pub used: String,
    pub limit: Limit,
}

/// What a row of the account's use stands for (its glyph).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UseKind {
    Photos,
    Files,
    Previews,
    Versions,
}

/// The Storage pane (DW-0712).
#[derive(Clone, PartialEq, Debug)]
pub struct Storage {
    /// "240 GB" of "1 TB".
    pub used: String,
    pub total: String,
    /// The meter's two parts, as shares of the whole: files and photos, then the rest.
    pub shares: [f32; 2],
    pub kinds: Vec<(UseKind, &'static str, String)>,
    pub trash: String,
    pub downloaded: KeptOnPc,
    pub previews: KeptOnPc,
    pub offline: String,
}

/// One kind of file in Storage insights' By type (DW-0783).
#[derive(Clone, PartialEq, Debug)]
pub struct KindShare {
    pub name: String,
    /// "221.6 GB · 3,140 files".
    pub says: String,
    /// Its bar against the largest kind's, 0 to 1.
    pub share: f32,
}

/// One of the largest files (DW-0783).
#[derive(Clone, PartialEq, Debug)]
pub struct LargeFile {
    /// The whole path (the row's name for assistive technology and its tooltip).
    pub path: String,
    /// The path as the row shows it, cut in the middle so the file name stays readable.
    pub shown: String,
    pub kind: String,
    pub size: String,
}

/// Storage insights, once loaded (DW-0783).
#[derive(Clone, PartialEq, Debug)]
pub struct InsightsData {
    pub used: String,
    pub total: String,
    pub plan: String,
    pub share: f32,
    pub used_says: String,
    pub free_says: String,
    pub totals: String,
    pub kinds: Vec<KindShare>,
    pub largest_says: String,
    pub largest: Vec<LargeFile>,
    /// "And 43 more, down to " and "48 MB".
    pub more: Option<(String, String)>,
}

/// Where Storage insights stands (DW-0783, b, c, d).
#[derive(Clone, PartialEq, Debug)]
pub enum Insights {
    Loading,
    /// Could not load: the reason, in the engine's words.
    Failed(String),
    /// No files and no bytes.
    Empty,
    Ready(Box<InsightsData>),
}

/// Where sync stands, as the Status row says it (DW-0746, DW-0746e).
#[derive(Clone, PartialEq, Debug)]
pub enum SyncStatus {
    /// Running: the row's words ("Up to date · last change 2 min ago").
    Running(String),
    /// Paused by the person.
    Paused,
    /// Waiting because this PC is on a metered connection and sync on metered connections
    /// is off: the row's words.
    WaitingMetered(String),
}

/// Where a repair of Beebeeb in File Explorer stands (DW-0746c, DW-0746d).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Repair {
    Idle,
    Running,
    /// The last repair did not finish; nothing needs undoing.
    Failed,
}

/// Sync and File Explorer (DW-0746).
#[derive(Clone, PartialEq, Debug)]
pub struct SyncPane {
    /// Beebeeb has its location in File Explorer's navigation pane.
    pub in_explorer: bool,
    pub status_on_files: bool,
    pub repair: Repair,
    pub status: SyncStatus,
    pub open_at_login: bool,
    /// "18.4 GB" downloaded, against a limit of so many GB.
    pub downloaded: String,
    pub limit_gb: u32,
    /// "3 items · 1.4 GB".
    pub offline: String,
    /// Files made in File Explorer that have not uploaded yet: how many, and their size
    /// (DW-0746b counts them before the location is turned off).
    pub not_uploaded: Option<(u32, String)>,
}

/// What Drive's settings panes read and change.
pub trait DriveSettings {
    /// `false`: nothing answers for these panes yet; they draw the shell's "not built yet"
    /// frame instead of any value.
    fn wired(&self) -> bool {
        true
    }
    /// The account's use and what this PC keeps. Engine: account usage by kind, the Trash
    /// size, the two caches' sizes and limits, the pinned (available offline) size.
    fn storage(&self) -> Storage;
    /// Store a limit. Engine: set the cache limit; lowering it below what is in use removes
    /// the oldest copies straight away (DW-0714).
    fn set_limit(&mut self, which: Kept, gb: u32);
    /// Remove every copy of `which` from this PC (never an Available offline file, never an
    /// open one). Engine: clear the cache; File Explorer then shows the files as online only.
    fn clear(&mut self, which: Kept);
    /// Sync and File Explorer. Engine: the cloud-files registration, the shell status
    /// setting, the sync state, the login item, the cache size and limit, the pinned set.
    fn sync(&self) -> SyncPane;
    /// Put the PC column's states on files in File Explorer, or not.
    fn set_status_on_files(&mut self, on: bool);
    /// Open Beebeeb when the person logs in (a login item), or not.
    fn set_open_at_login(&mut self, on: bool);
    /// Pause or resume syncing.
    fn set_paused(&mut self, paused: bool);
    /// Sync on this metered connection until the PC leaves it; the setting stays off (DW-0746e).
    fn sync_anyway(&mut self);
    /// Repair Beebeeb in File Explorer (DW-0746c): the row says Repairing… while it runs.
    fn repair(&mut self);
    /// Remove Beebeeb from File Explorer (DW-0746b): Windows unregisters its folder.
    fn turn_off_explorer(&mut self);
    /// Storage insights. Engine: usage plus the breakdown by kind and the 50 largest files,
    /// worked out on this PC from the decrypted file list. Lent, never copied: a page asks
    /// on every draw, and the answer holds the 50 largest files.
    fn insights(&self) -> &Insights;
    /// Ask for the insights again (DW-0783c, Try again): the page loads (DW-0783b).
    fn reload_insights(&mut self);
    /// Bandwidth (DW-0785). Engine: the live figures (every 10 s while the pane is open), the
    /// 20 hours of traffic kept on this PC, each device's heartbeat, the two limits, the
    /// metered switch, and where the speed test stands. Unanswered: the empty pane.
    fn bandwidth(&self) -> Bandwidth {
        Bandwidth::default()
    }
    /// Limit a direction to so many megabits per second on this PC, or not at all. It
    /// applies straight away and stays until changed (DW-0785c).
    fn set_speed_limit(&mut self, _direction: Direction, _mbps: Option<u32>) {}
    /// Sync on networks Windows marks as metered, or wait on them (DW-0785f).
    fn set_sync_on_metered(&mut self, _on: bool) {}
    /// Start the speed test against Beebeeb's own servers; one test at a time (DW-0786).
    fn run_speed_test(&mut self) {}
    /// The engine's host says where the speed test stands now: each step as it starts, the
    /// result, or the failure (DW-0786 build note). The shell passes it on.
    fn speed_test_reported(&mut self, _now: SpeedTest) {}
    /// Called at the start of a frame with the window's clock, for what arrives between
    /// presses (the speed test's steps). Answers (something changed, more is coming: draw
    /// again soon).
    fn pump(&mut self, _seconds: f64) -> (bool, bool) {
        (false, false)
    }
}

/// The board's sample content for Drive's settings panes.
pub struct BoardDrive {
    storage: Storage,
    insights: Insights,
    sync: SyncPane,
    bandwidth: BoardBandwidth,
}

impl Default for BoardDrive {
    fn default() -> Self {
        Self::new()
    }
}

impl BoardDrive {
    pub fn new() -> Self {
        let large = |path: &str, shown: &str, kind: &str, size: &str| LargeFile {
            path: path.into(),
            shown: shown.into(),
            kind: kind.into(),
            size: size.into(),
        };
        let same = |path: &str, kind: &str, size: &str| large(path, path, kind, size);
        Self {
            storage: Storage {
                used: "240 GB".into(),
                total: "1 TB".into(),
                shares: [0.216, 0.024],
                kinds: vec![
                    (UseKind::Photos, "Photos and videos", "216 GB".into()),
                    (UseKind::Files, "Files", "19.5 GB".into()),
                    (UseKind::Previews, "Thumbnails and previews", "3.1 GB".into()),
                    (UseKind::Versions, "Older versions", "1.2 GB".into()),
                ],
                trash: "240 MB".into(),
                downloaded: KeptOnPc {
                    used: "18.4 GB".into(),
                    limit: Limit {
                        gb: 50,
                        min_gb: 1,
                        max_gb: 186,
                        at: 0.27,
                        says: "Using 18.4 GB now, 372 GB free on this PC. Files you open are kept here so they open faster. Over the limit, the oldest are removed. Available offline files don't count.".into(),
                    },
                },
                previews: KeptOnPc {
                    used: "3.1 GB".into(),
                    limit: Limit {
                        gb: 20,
                        min_gb: 1,
                        max_gb: 93,
                        at: 0.20,
                        says: "All 2,431 photos fit, about 3.1 GB. Over the limit, the oldest are removed from this PC and download again when needed. They stay in Beebeeb.".into(),
                    },
                },
                offline: "1.4 GB".into(),
            },
            sync: SyncPane {
                in_explorer: true,
                status_on_files: true,
                repair: Repair::Idle,
                status: SyncStatus::Running("Up to date · last change 2 min ago".into()),
                open_at_login: true,
                downloaded: "18.4 GB".into(),
                limit_gb: 50,
                offline: "3 items · 1.4 GB".into(),
                not_uploaded: Some((4, "12 MB".into())),
            },
            insights: Insights::Ready(Box::new(InsightsData {
                used: "240 GB".into(),
                total: "1 TB".into(),
                plan: "Drive plan · 1 TB".into(),
                share: 0.24,
                used_says: "24% used".into(),
                free_says: "784 GB free".into(),
                totals: "4,424 files · 240 GB".into(),
                kinds: vec![
                    KindShare {
                        name: "Media".into(),
                        says: "221.6 GB · 3,140 files".into(),
                        share: 1.0,
                    },
                    KindShare {
                        name: "Documents".into(),
                        says: "13.9 GB · 1,032 files".into(),
                        share: 0.063,
                    },
                    KindShare {
                        name: "Other".into(),
                        says: "4.5 GB · 252 files".into(),
                        share: 0.02,
                    },
                ],
                largest_says: "top 50".into(),
                largest: vec![
                    same("Backups/Anna's iPhone/Photos/2026/IMG_4410.MOV", "Media", "4.2 GB"),
                    same("Shoot selects/Harbourlight/MVI_0087.mov", "Media", "3.8 GB"),
                    large(
                        "Backups/Anna-Laptop/Documents/Archive 2024.zip",
                        "Backups/Anna's…ook Pro/Documents/Archive 2024.zip",
                        "Other",
                        "2.6 GB",
                    ),
                    same("Lisbon trip/Day 2/DJI_0412.mp4", "Media", "1.9 GB"),
                    same("Shoot selects/Raw/A001_C014.braw", "Media", "1.2 GB"),
                    large(
                        "Backups/Anna-Laptop/Desktop/Screen recording 14 Oct.mov",
                        "Backups/Anna's…esktop/Screen recording 14 Oct.mov",
                        "Media",
                        "940 MB",
                    ),
                    same("Documents/Taxes/2025 receipts.pdf", "Documents", "212 MB"),
                ],
                more: Some(("And 43 more, down to ".into(), "48 MB".into())),
            })),
            bandwidth: BoardBandwidth::default(),
        }
    }

    /// The last repair did not finish (DW-0746d).
    pub fn with_repair_failed(mut self) -> Self {
        self.sync.repair = Repair::Failed;
        self
    }

    /// Sync waits on a metered connection (DW-0746e).
    pub fn with_metered_wait(mut self) -> Self {
        self.sync.status =
            SyncStatus::WaitingMetered("Waiting · this PC is on a metered connection · 7 changes wait".into());
        self.bandwidth = self.bandwidth.metered_wait();
        self
    }

    /// The speed test `seconds` after Run test, standing still there (DW-0786, b, c, d).
    pub fn with_speed_test_at(mut self, seconds: f64) -> Self {
        self.bandwidth = self.bandwidth.test_at(seconds);
        self
    }

    /// This PC cannot reach Beebeeb: a speed test ends in the failure (DW-0786d).
    pub fn with_beebeeb_unreachable(mut self) -> Self {
        self.bandwidth = self.bandwidth.unreachable();
        self
    }

    /// Bandwidth with nothing moving (DW-0785b).
    pub fn with_bandwidth_idle(mut self) -> Self {
        self.bandwidth = self.bandwidth.idle();
        self
    }

    /// Bandwidth right after sign-in: nothing recorded yet (DW-0785e).
    pub fn with_no_traffic_recorded(mut self) -> Self {
        self.bandwidth = self.bandwidth.new_pc();
        self
    }

    /// Storage insights in another state (DW-0783b, c, d).
    pub fn with_insights(mut self, insights: Insights) -> Self {
        self.insights = insights;
        self
    }
}

impl DriveSettings for BoardDrive {
    fn storage(&self) -> Storage {
        self.storage.clone()
    }
    fn set_limit(&mut self, which: Kept, gb: u32) {
        let limit = match which {
            Kept::Downloaded => &mut self.storage.downloaded.limit,
            Kept::Previews => &mut self.storage.previews.limit,
        };
        limit.at = limit.at_for(gb);
        limit.gb = gb.clamp(limit.min_gb, limit.max_gb);
    }
    fn clear(&mut self, which: Kept) {
        match which {
            Kept::Downloaded => self.storage.downloaded.used = "0 GB".into(),
            Kept::Previews => self.storage.previews.used = "0 GB".into(),
        }
    }
    fn sync(&self) -> SyncPane {
        SyncPane {
            downloaded: self.storage.downloaded.used.clone(),
            limit_gb: self.storage.downloaded.limit.gb,
            ..self.sync.clone()
        }
    }
    fn set_status_on_files(&mut self, on: bool) {
        self.sync.status_on_files = on;
    }
    fn set_open_at_login(&mut self, on: bool) {
        self.sync.open_at_login = on;
    }
    fn set_paused(&mut self, paused: bool) {
        self.sync.status = if paused {
            SyncStatus::Paused
        } else {
            SyncStatus::Running("Up to date · last change 2 min ago".into())
        };
    }
    fn sync_anyway(&mut self) {
        self.sync.status = SyncStatus::Running("Syncing on this connection".into());
        self.bandwidth.sync_anyway();
    }
    fn repair(&mut self) {
        self.sync.repair = Repair::Running;
    }
    fn turn_off_explorer(&mut self) {
        self.sync.in_explorer = false;
        self.sync.not_uploaded = None;
    }
    fn insights(&self) -> &Insights {
        &self.insights
    }
    fn reload_insights(&mut self) {
        self.insights = Insights::Loading;
    }
    fn bandwidth(&self) -> Bandwidth {
        self.bandwidth.bandwidth()
    }
    fn set_speed_limit(&mut self, direction: Direction, mbps: Option<u32>) {
        self.bandwidth.set_limit(direction, mbps);
    }
    fn set_sync_on_metered(&mut self, on: bool) {
        let waited = self.bandwidth.waiting_metered();
        self.bandwidth.set_sync_on_metered(on);
        // Sync resumes at once (DW-0785f), and waits again when the switch goes back off.
        if waited && !self.bandwidth.waiting_metered() {
            self.sync.status = SyncStatus::Running("Up to date · last change 2 min ago".into());
        } else if !waited && self.bandwidth.waiting_metered() {
            self.sync.status =
                SyncStatus::WaitingMetered("Waiting · this PC is on a metered connection · 7 changes wait".into());
        }
    }
    fn run_speed_test(&mut self) {
        self.bandwidth.run_test();
    }
    fn speed_test_reported(&mut self, now: SpeedTest) {
        self.bandwidth.report_test(now);
    }
    fn pump(&mut self, seconds: f64) -> (bool, bool) {
        self.bandwidth.pump(seconds)
    }
}

/// The board's content, held by a build that ships before the engine lane wires these panes:
/// `wired()` is `false`, so no pane ever draws a value of it.
pub struct UnwiredDrive(BoardDrive);

impl UnwiredDrive {
    pub fn new() -> Self {
        Self(BoardDrive::new())
    }
}

impl DriveSettings for UnwiredDrive {
    fn wired(&self) -> bool {
        false
    }
    fn storage(&self) -> Storage {
        self.0.storage()
    }
    fn set_limit(&mut self, _: Kept, _: u32) {}
    fn clear(&mut self, _: Kept) {}
    fn sync(&self) -> SyncPane {
        self.0.sync()
    }
    fn set_status_on_files(&mut self, _: bool) {}
    fn set_open_at_login(&mut self, _: bool) {}
    fn set_paused(&mut self, _: bool) {}
    fn sync_anyway(&mut self) {}
    fn repair(&mut self) {}
    fn turn_off_explorer(&mut self) {}
    fn insights(&self) -> &Insights {
        static NOTHING: Insights = Insights::Loading;
        &NOTHING
    }
    fn reload_insights(&mut self) {}
}

/// What Drive's settings panes start on: the board's sample content in a development build,
/// nothing in a build that ships (as `source::first`).
pub fn first(dev_build: bool) -> Box<dyn DriveSettings> {
    if dev_build {
        Box::new(BoardDrive::new())
    } else {
        Box::new(UnwiredDrive(BoardDrive::new()))
    }
}
