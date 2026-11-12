//! What the Bandwidth pane shows (DW-0785 and its states, the speed test DW-0786): the
//! types `DriveSettings::bandwidth` answers with, and the fixture that holds exactly the
//! board's sample content.
//!
//! Speeds are bytes per second as words ("3.2", "MB/s"); limits and the speed test are
//! megabits per second (DW-0785, rules). A device's current file name is never part of this
//! (the engine leaves it out for privacy).

/// A direction of traffic. Upload is the encryption step, so it is the Amber one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Upload,
    Download,
}

impl Direction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Upload => "Upload",
            Self::Download => "Download",
        }
    }
}

/// The limits a person can choose, in megabits per second; `None` is No limit (the default).
pub const PRESETS: [Option<u32>; 8] = [None, Some(1), Some(2), Some(5), Some(10), Some(20), Some(50), Some(100)];

/// The preset limits as the menu and the pop-up button say them, in `PRESETS`' order.
pub const PRESET_LABELS: [&str; 8] = [
    "No limit", "1 Mbps", "2 Mbps", "5 Mbps", "10 Mbps", "20 Mbps", "50 Mbps", "100 Mbps",
];

/// A limit in words. A limit an older version saved that is not a preset is said as it is.
pub fn limit_label(limit: Option<u32>) -> String {
    match limit {
        None => "No limit".into(),
        Some(mbps) => format!("{mbps} Mbps"),
    }
}

/// One direction, right now.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Flow {
    /// The figure and its unit ("3.2", "MB/s"). `None`: nothing is moving, drawn as a dash
    /// and never as "0 B/s" (DW-0785b).
    pub speed: Option<(String, String)>,
    /// What is moving, or why nothing is.
    pub says: String,
}

/// The last 20 hours: data moved per 20 minutes, oldest first.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct History {
    /// Megabytes sent per bucket. Empty: nothing recorded yet (DW-0785e).
    pub up_mb: Vec<f32>,
    /// Megabytes received per bucket; as long as `up_mb`.
    pub down_mb: Vec<f32>,
    /// "5.2 GB", "1.3 GB".
    pub up_total: String,
    pub down_total: String,
    /// The top of the one axis, in megabytes (400).
    pub top_mb: f32,
    /// When the first bucket starts, in minutes after midnight (13:40 is 820).
    pub starts_at: u32,
    /// How long a bucket is, in minutes (20).
    pub bucket_minutes: u32,
}

impl History {
    pub fn is_empty(&self) -> bool {
        self.up_mb.is_empty()
    }

    /// "06:20" for a count of minutes after the first bucket's start.
    fn clock(&self, minutes: u32) -> String {
        let at = (self.starts_at + minutes) % (24 * 60);
        format!("{:02}:{:02}", at / 60, at % 60)
    }

    /// The six marks under the chart: five times, and Now.
    pub fn marks(&self) -> Vec<String> {
        let whole = self.up_mb.len() as u32 * self.bucket_minutes;
        let mut marks: Vec<String> = (0..5).map(|i| self.clock(whole * i / 5)).collect();
        marks.push("Now".into());
        marks
    }

    /// What the chart says about bucket `i`: its time, and what moved in it.
    pub fn tip(&self, i: usize) -> Option<(String, String)> {
        let (up, down) = (*self.up_mb.get(i)?, *self.down_mb.get(i)?);
        let from = i as u32 * self.bucket_minutes;
        // The chart ends now, so a bucket that starts after midnight is today's.
        let day = if self.starts_at + from >= 24 * 60 {
            "Today"
        } else {
            "Yesterday"
        };
        Some((
            format!(
                "{day} {} \u{2013} {}",
                self.clock(from),
                self.clock(from + self.bucket_minutes)
            ),
            format!("Upload {} MB \u{b7} Download {} MB", up.round(), down.round()),
        ))
    }
}

/// What kind of device a row is (its glyph).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceKind {
    Computer,
    Phone,
    Tablet,
}

/// Where a device's sync stands (its pill).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceState {
    Syncing,
    UpToDate,
    Paused,
    Failed,
}

impl DeviceState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Syncing => "Syncing",
            Self::UpToDate => "Up to date",
            Self::Paused => "Paused",
            Self::Failed => "Couldn't sync",
        }
    }
}

/// One device's sync session, from its heartbeat.
#[derive(Clone, PartialEq, Debug)]
pub struct Device {
    pub kind: DeviceKind,
    pub name: String,
    /// "This PC", "iOS".
    pub tag: String,
    pub state: DeviceState,
    /// How far its sync is, 0 to 1, while it syncs and says so.
    pub progress: Option<f32>,
    /// Files and bytes done, its speed, when it last reported.
    pub says: String,
}

/// One step of the speed test.
#[derive(Clone, PartialEq, Debug)]
pub enum Step {
    /// Not started: why ("Waits for the uploads").
    Waiting(String),
    /// Running: "2 of 3", and how far along, 0 to 1.
    Measuring(String, f32),
    /// Finished: the figure, its unit, and the line under it ("24", "ms", "18 to 41 ms").
    Done(String, String, String),
}

/// Where the speed test stands (DW-0786, b, c, d). The three steps are latency, upload,
/// download, in the order they run.
#[derive(Clone, PartialEq, Debug, Default)]
pub enum SpeedTest {
    /// Never run, or its figures were cleared (a sign-out, a lock).
    #[default]
    Idle,
    Running([Step; 3]),
    /// The result, and when and what it measured.
    Done([Step; 3], String),
    /// It could not run: the sentence names the step that failed.
    Failed(String),
}

impl SpeedTest {
    pub fn running(&self) -> bool {
        matches!(self, Self::Running(_))
    }
}

/// The Bandwidth pane. `Default` is the state of a pane nothing answers for: no figures, no
/// history, no devices, no limits.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Bandwidth {
    /// "2 of 3 devices syncing".
    pub now: String,
    pub upload: Flow,
    pub download: Flow,
    pub history: History,
    pub devices: Vec<Device>,
    pub upload_limit: Option<u32>,
    pub download_limit: Option<u32>,
    /// Sync on metered connections (on by default).
    pub sync_on_metered: bool,
    /// Sync waits right now because this PC is on a metered connection (DW-0785f).
    pub waiting_metered: bool,
    pub test: SpeedTest,
}

/// The board's 60 buckets, in units of 100 MB (`BW_UP`, `BW_DN`).
const UP: [f32; 60] = [
    0.1, 0.1, 0.3, 1.2, 2.6, 3.1, 2.4, 0.8, 0.2, 0.1, 0.1, 0.4, 1.8, 2.2, 0.6, 0.1, 0.1, 0.1, 0.1, 0.0, 0.0, 0.0, 0.0,
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.1, 0.1, 0.2, 0.1, 0.1, 0.6, 1.4, 0.9,
    0.2, 0.1, 0.3, 1.1, 2.4, 3.4, 3.0, 2.2, 2.8, 3.6, 3.3, 2.9, 3.2, 3.2,
];
const DOWN: [f32; 60] = [
    0.2, 0.4, 0.3, 0.2, 0.1, 0.2, 0.3, 0.6, 1.2, 0.8, 0.3, 0.1, 0.1, 0.2, 0.1, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2, 0.5, 0.9, 0.6, 0.3, 0.2, 0.1, 0.2, 0.4,
    0.3, 0.2, 0.1, 0.2, 0.3, 0.4, 0.5, 0.3, 0.2, 0.4, 0.5, 0.4, 0.4, 0.4,
];

/// What the board's account is doing, apart from the limits and the metered switch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Activity {
    /// Files are moving (DW-0785).
    Busy,
    /// Nothing is moving (DW-0785b).
    Idle,
    /// Right after sign-in: nothing recorded yet (DW-0785e).
    New,
}

/// The board's sample content for the Bandwidth pane, and what its controls change.
#[derive(Clone, PartialEq, Debug)]
pub struct BoardBandwidth {
    activity: Activity,
    upload_limit: Option<u32>,
    download_limit: Option<u32>,
    sync_on_metered: bool,
    /// This PC is on a network Windows marks as metered.
    on_metered: bool,
    /// Sync anyway was chosen for this connection (DW-0746e): the setting stays off.
    anyway: bool,
    test: SpeedTest,
}

impl Default for BoardBandwidth {
    fn default() -> Self {
        Self {
            activity: Activity::Busy,
            upload_limit: None,
            download_limit: None,
            sync_on_metered: true,
            on_metered: false,
            anyway: false,
            test: SpeedTest::Idle,
        }
    }
}

fn done(figure: &str, unit: &str, says: &str) -> Step {
    Step::Done(figure.into(), unit.into(), says.into())
}

fn total(buckets: &[f32]) -> String {
    format!("{:.1} GB", buckets.iter().sum::<f32>() / 10.0)
}

impl BoardBandwidth {
    pub fn idle(mut self) -> Self {
        self.activity = Activity::Idle;
        self
    }
    pub fn new_pc(mut self) -> Self {
        self.activity = Activity::New;
        self
    }
    /// On a metered connection with Sync on metered connections off (DW-0785f, DW-0746e).
    pub fn metered_wait(mut self) -> Self {
        self.on_metered = true;
        self.sync_on_metered = false;
        self
    }

    /// The test's states as the board draws them: uploading (DW-0786).
    pub fn test_uploading() -> SpeedTest {
        SpeedTest::Running([
            done("24", "ms", "18 to 41 ms"),
            Step::Measuring("2 of 3".into(), 0.55),
            Step::Waiting("Waits for the uploads".into()),
        ])
    }
    /// Downloading (DW-0786b).
    pub fn test_downloading() -> SpeedTest {
        SpeedTest::Running([
            done("24", "ms", "18 to 41 ms"),
            done("35.2", "Mbps", "3 of 3 \u{b7} 30 MB"),
            Step::Measuring("1 of 3".into(), 0.3),
        ])
    }
    /// The result (DW-0786c).
    pub fn test_done() -> SpeedTest {
        SpeedTest::Done(
            [
                done("24", "ms", "18 to 41 ms"),
                done("35.2", "Mbps", "3 of 3 \u{b7} 30 MB"),
                done("94.0", "Mbps", "3 of 3 \u{b7} 30 MB"),
            ],
            "Measured just now, from this PC to Beebeeb's servers in Europe. The test moved 60 MB.".into(),
        )
    }
    /// No ping answered (DW-0786d).
    pub fn test_failed() -> SpeedTest {
        SpeedTest::Failed(
            "Beebeeb didn't answer any of the 5 pings. Check your internet connection and try again.".into(),
        )
    }

    pub fn set_limit(&mut self, direction: Direction, mbps: Option<u32>) {
        match direction {
            Direction::Upload => self.upload_limit = mbps,
            Direction::Download => self.download_limit = mbps,
        }
    }
    pub fn set_sync_on_metered(&mut self, on: bool) {
        self.sync_on_metered = on;
    }
    /// Sync on this metered connection until the PC leaves it; the setting stays as it is.
    pub fn sync_anyway(&mut self) {
        self.anyway = true;
    }
    /// One test at a time: a running test is not started again.
    pub fn run_test(&mut self) {
        if !self.test.running() {
            self.test = Self::test_uploading();
        }
    }
    pub fn report_test(&mut self, now: SpeedTest) {
        self.test = now;
    }

    /// Sync waits: on a metered connection, the setting off, and no Sync anyway.
    pub fn waiting_metered(&self) -> bool {
        self.on_metered && !self.sync_on_metered && !self.anyway
    }

    /// The upload speed a limit leaves, of the board's 3.2 MB/s: a megabit per second is
    /// 125 KB/s.
    fn upload_speed(&self) -> (String, String) {
        match self.upload_limit {
            Some(mbps) if mbps * 125 < 3200 && mbps * 125 < 1000 => ((mbps * 125).to_string(), "KB/s".into()),
            Some(mbps) if mbps * 125 < 3200 => (format!("{:.1}", mbps as f32 * 0.125), "MB/s".into()),
            _ => ("3.2".into(), "MB/s".into()),
        }
    }

    fn history(&self, waiting: bool) -> History {
        if self.activity == Activity::New {
            return History::default();
        }
        // A limit flattens the chart since 07:00 (a megabit per second is 150 MB in 20 minutes);
        // on a metered connection the last hour sends nothing.
        let cap = self.upload_limit.map(|mbps| mbps as f32 * 1.5);
        let up: Vec<f32> = UP
            .iter()
            .enumerate()
            .map(|(i, v)| match cap {
                _ if waiting && i >= 57 => 0.0,
                Some(cap) if i >= 52 => v.min(cap),
                _ => *v,
            })
            .collect();
        History {
            up_total: total(&up),
            down_total: total(&DOWN),
            up_mb: up.iter().map(|v| v * 100.0).collect(),
            down_mb: DOWN.iter().map(|v| v * 100.0).collect(),
            top_mb: 400.0,
            starts_at: 13 * 60 + 40,
            bucket_minutes: 20,
        }
    }

    pub fn bandwidth(&self) -> Bandwidth {
        let waiting = self.waiting_metered();
        let moving = self.activity == Activity::Busy && !waiting;
        let flow = |speed: Option<(&str, &str)>, says: &str| Flow {
            speed: speed.map(|(figure, unit)| (figure.to_string(), unit.to_string())),
            says: says.into(),
        };
        let (speed, unit) = self.upload_speed();
        let limited = self.upload_limit.filter(|_| unit == "KB/s" || speed != "3.2");
        let (now, upload, download) = if waiting {
            (
                "Waiting for another network",
                flow(
                    None,
                    "Waiting: this PC is on a metered connection \u{b7} 7 changes wait",
                ),
                flow(None, "Files you open still download"),
            )
        } else if !moving {
            (
                "Nothing is moving",
                flow(None, "Up to date \u{b7} last change 2 min ago"),
                flow(None, "Files download when you open them"),
            )
        } else {
            let says = match limited {
                Some(mbps) => format!("Limited to {mbps} Mbps \u{b7} 12 files, 48 MB left"),
                // The board keeps "48 MB" together when the line wraps.
                None => "Encrypting and uploading 12 files \u{b7} 48\u{a0}MB left".into(),
            };
            (
                "2 of 3 devices syncing",
                flow(Some((&speed, &unit)), &says),
                flow(Some(("410", "KB/s")), "2 files from Anna's iPhone"),
            )
        };
        let device = |kind, name: &str, tag: &str, state, progress, says: String| Device {
            kind,
            name: name.into(),
            tag: tag.into(),
            state,
            progress,
            says,
        };
        let devices = if moving {
            vec![
                device(
                    DeviceKind::Computer,
                    "Anna-Laptop",
                    "This PC",
                    DeviceState::Syncing,
                    Some(0.71),
                    format!("38 of 50 files \u{b7} 120 of 168 MB \u{b7} {speed} {unit} \u{b7} 4 s ago"),
                ),
                device(
                    DeviceKind::Phone,
                    "Anna's iPhone",
                    "iOS",
                    DeviceState::Syncing,
                    None,
                    "sending 2 files \u{b7} 410 KB/s \u{b7} 18 s ago".into(),
                ),
                device(
                    DeviceKind::Tablet,
                    "Anna's iPad",
                    "iPadOS",
                    DeviceState::Paused,
                    None,
                    "last reported 3 h ago".into(),
                ),
            ]
        } else {
            vec![
                device(
                    DeviceKind::Computer,
                    "Anna-Laptop",
                    "This PC",
                    if waiting {
                        DeviceState::Paused
                    } else {
                        DeviceState::UpToDate
                    },
                    None,
                    "1,284 files \u{b7} 3 s ago".into(),
                ),
                device(
                    DeviceKind::Phone,
                    "Anna's iPhone",
                    "iOS",
                    DeviceState::UpToDate,
                    None,
                    "18 s ago".into(),
                ),
                device(
                    DeviceKind::Tablet,
                    "Anna's iPad",
                    "iPadOS",
                    DeviceState::Paused,
                    None,
                    "last reported 3 h ago".into(),
                ),
            ]
        };
        Bandwidth {
            now: now.into(),
            upload,
            download,
            history: self.history(waiting),
            devices,
            upload_limit: self.upload_limit,
            download_limit: self.download_limit,
            sync_on_metered: self.sync_on_metered,
            waiting_metered: waiting,
            test: self.test.clone(),
        }
    }
}
