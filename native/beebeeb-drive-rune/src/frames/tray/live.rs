//! The flyout of the running app: what the engine knows, in the flyout's words.
//!
//! `engine_host::EngineTraySource` gathers [`Facts`] from the engine handle, the state database and
//! the engine's last report of itself (the host's fold of the engine's events, `LiveEngine::drain`);
//! [`snapshot`] turns them into what the flyout draws. Nothing here reads the board's fixture: each
//! sentence is the board's copy for its state, or, for the states the board draws no frame for,
//! this lane's (`FRAMES.md`, "States the board does not draw").
//!
//! The order of the states is DW-1201's ("update required, locked, checking, no plan, signed out
//! after 90 days, couldn't add / adding to File Explorer, paused, storage full, offline, stopped;
//! then needs a decision and update available; then storage almost full, syncing, up to date"),
//! less what the engine cannot say yet (`FRAMES.md`, "Not wired"), with "sync isn't running" right
//! after paused (none of the quieter states holds while nothing runs), and with checking after
//! those three: the session and the two switches are known at once, and checking waits only for
//! what the engine checks.
//!
//! Must not: say "Up to date" before the engine reported a finished check (DW-1212: "never a guess
//! such as Up to date"); show a file name while the account's keys are not in memory; offer a
//! button that would do nothing.

use chrono::{DateTime, Datelike, Local as LocalTime, TimeZone, Weekday};

use super::source::{Action, Change, Changes, Hidden, MAX_ROWS, Origin, Snapshot, Status, Tile, What};
use crate::frames::global::source::size_words;

/// The session gate, as the engine says it (`signin::gate::GateState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    SignedOut,
    Locked,
    NeedsRecoveryPhrase,
    NoPlan,
    KeyStoreUnavailable,
    Ready,
}

/// The engine's last report of itself (`engine-status`), as the host folded it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Report {
    /// No report since this app started, or since sync was resumed: not known.
    #[default]
    Unknown,
    /// It started, and its first check is not finished.
    Starting,
    /// Its last check reached Beebeeb, and nothing was left to do.
    Idle,
    /// Files are moving.
    Syncing,
    /// Its last check did not reach Beebeeb.
    Offline,
    /// Beebeeb did not answer its last check.
    ServerError,
    /// Its last check failed on this PC, not on the way to Beebeeb (`engine_error`).
    Failed,
    /// Paused or stopped: what it says when it is not working.
    Quiet,
}

/// The numbers of the engine's last report while it syncs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub files_remaining: Option<u32>,
    pub bytes_total: Option<u64>,
    pub bytes_done: Option<u64>,
}

/// What the state database on this PC says. Read only while the account's keys are in memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Local {
    /// The files Beebeeb keeps on this PC, online-only ones included (DW-1202); `None` when they
    /// were not counted.
    pub files: Option<u32>,
    /// Files changed in two places.
    pub conflicts: u32,
    /// Changes Beebeeb stopped retrying (out of attempts).
    pub gave_up: u32,
    /// Changes on this PC that are not sent yet.
    pub unsent: u32,
    /// Changes held back because the account is full.
    pub held_for_quota: u32,
    /// Files moving now.
    pub moving: u32,
    /// The history: what is moving, then what failed, then what happened, newest first.
    pub rows: Vec<Row>,
}

/// One row of the history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// What the engine knows the file by (a click shows it); `None` when it is not known.
    pub id: Option<String>,
    /// Its path in Beebeeb.
    pub path: String,
    /// Unix seconds: when it happened; for a file that moves or failed, its last change.
    pub at: i64,
    pub kind: RowKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// Moving now; `bytes` is the file's size (0 when not known).
    Uploading {
        bytes: u64,
    },
    Downloading {
        bytes: u64,
    },
    Failed,
    Uploaded,
    Downloaded,
    MovedToTrash,
}

/// Everything the flyout of the running app is drawn from.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    pub gate: Gate,
    /// Beebeeb refused this session again and again: it ended (DW-1219).
    pub session_ended: bool,
    /// The engine's task runs on this PC.
    pub running: bool,
    /// The pause switch.
    pub paused: bool,
    pub report: Report,
    pub progress: Progress,
    /// Beebeeb is in File Explorer (the shell's tray state).
    pub explorer_ready: bool,
    /// `None`: no state database on this PC yet, or the keys are not in memory.
    pub local: Option<Local>,
    /// The account's used and total bytes as last read from Beebeeb; `None` until read.
    pub storage: Option<(u64, u64)>,
}

/// What the flyout draws for `facts`. `name` is the app's (the build's identity), `now` the clock.
pub fn snapshot(facts: &Facts, name: &str, now: DateTime<LocalTime>) -> Snapshot {
    let (status, changes) = match facts.gate {
        Gate::SignedOut => (Status::SignedOut, Changes::Hidden(Hidden::SignedOut)),
        // The board's app lock keeps sync running (DW-1211); this engine stops it while the keys
        // are not in memory, and the flyout says which one holds.
        Gate::Locked if facts.running => (Status::Locked, Changes::Hidden(Hidden::Locked)),
        Gate::Locked => (Status::LockedSyncOff, Changes::Hidden(Hidden::Locked)),
        Gate::NeedsRecoveryPhrase => (Status::NeedsRecoveryPhrase, Changes::Hidden(Hidden::NoKeys)),
        Gate::KeyStoreUnavailable => (Status::KeysUnreadable, Changes::Hidden(Hidden::NoKeys)),
        Gate::NoPlan => (Status::NoPlan { waiting: false }, nothing_yet(name)),
        Gate::Ready => (ready(facts, now), recent(facts.local.as_ref(), name, now)),
    };
    let button = status.action().filter(|&(action, _)| offered(action, facts.gate));
    Snapshot {
        status,
        changes,
        explorer_ready: facts.explorer_ready,
        button,
    }
}

/// The status of a signed-in, unlocked account. What the switches and the session say is known at
/// once (session ended, paused, not running); what the engine checks waits for its first report.
fn ready(facts: &Facts, now: DateTime<LocalTime>) -> Status {
    let none = Local::default();
    let local = facts.local.as_ref().unwrap_or(&none);
    if facts.session_ended {
        return Status::SignedOut;
    }
    if facts.paused {
        return Status::Paused { waiting: local.unsent };
    }
    if !facts.running {
        return Status::NotSyncing;
    }
    if matches!(facts.report, Report::Unknown | Report::Starting) {
        return Status::Checking;
    }
    if local.held_for_quota > 0 {
        return Status::StorageFull {
            waiting: local.held_for_quota,
        };
    }
    if matches!(facts.report, Report::Offline | Report::ServerError) {
        return Status::Offline;
    }
    if local.gave_up > 0 || facts.report == Report::Failed {
        return Status::Stopped { files: local.gave_up };
    }
    if local.conflicts > 0 {
        return Status::NeedsDecision { files: local.conflicts };
    }
    // DW-1208: "At 90% of the plan, warn once, plainly".
    if let Some((used, quota)) = facts.storage
        && quota > 0
        && used.saturating_mul(10) >= quota.saturating_mul(9)
    {
        return Status::StorageAlmostFull {
            used: size_words(used),
            plan: size_words(quota),
        };
    }
    // A file moving now is syncing, even when the last report (up to a second old) said idle.
    let reported = if facts.report == Report::Syncing {
        facts.progress
    } else {
        Progress::default()
    };
    if facts.report == Report::Syncing || local.moving > 0 || reported.files_remaining.unwrap_or(0) > 0 {
        let known = reported.bytes_total.filter(|total| *total > 0).zip(reported.bytes_done);
        return Status::Syncing {
            files: reported.files_remaining.filter(|n| *n > 0).unwrap_or(local.moving),
            left: known.map(|(total, done)| size_words(total.saturating_sub(done))),
            // Not wired: the engine reports no transfer rate.
            about: None,
            fraction: known.map(|(total, done)| (done.min(total) as f64 / total as f64) as f32),
            speed: None,
        };
    }
    if facts.report == Report::Idle {
        return Status::UpToDate {
            files: local.files,
            last_change: newest(Some(local)).map(|at| last_change(at, now)),
        };
    }
    // A report that does not fit the switches (stopped or paused while running and not paused):
    // not known.
    Status::Checking
}

/// The buttons the running app carries out: Pause and Resume (the engine's pause switch, through
/// the shell), Review… and Show (they move the window), and Sign in while signed out (the window
/// is on sign-in then). Every other one waits for its engine call (`FRAMES.md`, "Not wired").
fn offered(action: Action, gate: Gate) -> bool {
    match action {
        Action::Pause | Action::Resume | Action::Review | Action::ShowProblems => true,
        Action::SignIn => gate == Gate::SignedOut,
        Action::SyncAnyway
        | Action::TryAgain
        | Action::AddStorage
        | Action::RestartToUpdate
        | Action::Unlock
        | Action::ChoosePlan
        | Action::RetryExplorer => false,
    }
}

/// DW-1213's empty list: nothing has synced on this PC yet.
fn nothing_yet(name: &str) -> Changes {
    Changes::Empty {
        text: format!("Changes show here once {name} syncs this PC."),
    }
}

fn recent(local: Option<&Local>, name: &str, now: DateTime<LocalTime>) -> Changes {
    let Some(local) = local.filter(|local| !local.rows.is_empty()) else {
        return nothing_yet(name);
    };
    let rows: Vec<Change> = local.rows.iter().take(MAX_ROWS).map(|row| change(row, now)).collect();
    // "That's the last 50 changes." only when the list holds 50 (DW-1220).
    Changes::Rows {
        complete: rows.len() == MAX_ROWS,
        rows,
    }
}

/// The newest row's time: DW-1202's "last change".
fn newest(local: Option<&Local>) -> Option<i64> {
    local?.rows.iter().map(|row| row.at).max()
}

fn change(row: &Row, now: DateTime<LocalTime>) -> Change {
    let name = leaf(&row.path);
    let dot = "\u{00B7}";
    let moving = |verb: &str, bytes: u64| {
        if bytes > 0 {
            format!("{verb} {dot} {}", size_words(bytes))
        } else {
            verb.to_string()
        }
    };
    let (what, tile) = match row.kind {
        // Not wired: the flyout has no per-file progress, so a moving row has no bar.
        RowKind::Uploading { bytes } => (
            What::Moving {
                text: moving("Uploading", bytes),
                fraction: None,
                upload: true,
            },
            tile_for(&name),
        ),
        RowKind::Downloading { bytes } => (
            What::Moving {
                text: moving("Downloading", bytes),
                fraction: None,
                upload: false,
            },
            tile_for(&name),
        ),
        RowKind::Failed => (
            What::Failed {
                text: "Couldn't sync this file".into(),
            },
            tile_for(&name),
        ),
        RowKind::Uploaded => (
            What::Done {
                origin: Origin::ThisPc,
                text: format!("Uploaded from this PC {dot} {}", folder_of(&row.path)),
            },
            tile_for(&name),
        ),
        RowKind::Downloaded => (
            What::Done {
                origin: Origin::ThisPc,
                text: format!("Downloaded to this PC {dot} {}", folder_of(&row.path)),
            },
            tile_for(&name),
        ),
        RowKind::MovedToTrash => (
            What::Done {
                origin: Origin::ThisPc,
                text: "Moved to Trash on this PC".into(),
            },
            Tile::Trashed,
        ),
    };
    Change {
        id: row.id.clone().unwrap_or_default(),
        name,
        tile,
        what,
        when: when(row.at, now),
        in_trash: row.kind == RowKind::MovedToTrash,
    }
}

/// The last part of a path.
fn leaf(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// The folder a path is in, by its own name; "Files" at the top (the window's name for it).
fn folder_of(path: &str) -> String {
    let path = path.trim_matches(['/', '\\']);
    match path.rfind(['/', '\\']) {
        Some(at) => leaf(&path[..at]),
        None => "Files".into(),
    }
}

/// The tile a file's name asks for. A photo is a glyph: the flyout has no thumbnail pixels.
pub fn tile_for(name: &str) -> Tile {
    let extension = name
        .rsplit_once('.')
        .filter(|(stem, ext)| !stem.is_empty() && !ext.is_empty())
        .map(|(_, ext)| ext.to_ascii_lowercase());
    match extension.as_deref() {
        Some(
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "avif" | "bmp" | "tif" | "tiff" | "dng" | "svg",
        ) => Tile::Image,
        Some("mp4" | "m4v" | "mov" | "webm" | "mkv" | "avi" | "3gp") => Tile::Video,
        Some("mp3" | "m4a" | "wav" | "flac" | "aac" | "ogg" | "opus") => Tile::Audio,
        Some("xlsx" | "xls" | "ods" | "csv" | "numbers") => Tile::Sheet,
        Some("pptx" | "ppt" | "odp" | "key") => Tile::Deck,
        Some("zip" | "7z" | "rar" | "tar" | "gz" | "tgz") => Tile::Archive,
        _ => Tile::Document,
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn weekday(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// A row's time as the board writes it (DW-1201): now, 2 min, 44 min, 2 h, Yesterday, a weekday
/// within 7 days, then 26 Oct (and the year, when it is another year's). Days are this PC's.
pub fn when(at: i64, now: DateTime<LocalTime>) -> String {
    let Some(then) = LocalTime.timestamp_opt(at, 0).single() else {
        return String::new();
    };
    let ago = now.signed_duration_since(then);
    let days = (now.date_naive() - then.date_naive()).num_days();
    if ago.num_seconds() < 60 {
        "now".into()
    } else if ago.num_minutes() < 60 {
        format!("{} min", ago.num_minutes())
    } else if days == 0 {
        format!("{} h", ago.num_hours())
    } else if days == 1 {
        "Yesterday".into()
    } else if days < 7 {
        weekday(then.weekday()).into()
    } else {
        let month = MONTHS[then.month0() as usize];
        if then.year() == now.year() {
            format!("{} {month}", then.day())
        } else {
            format!("{} {month} {}", then.day(), then.year())
        }
    }
}

/// The words after "last change" in DW-1202's line ("2 min ago").
pub fn last_change(at: i64, now: DateTime<LocalTime>) -> String {
    let word = when(at, now);
    if word == "now" {
        "just now".into()
    } else if word.ends_with(" min") || word.ends_with(" h") {
        format!("{word} ago")
    } else if word == "Yesterday" {
        "yesterday".into()
    } else {
        format!("on {word}")
    }
}
