//! `TraySource`: what the b's flyout asks of the engine, and the board's fixture.
//!
//! The flyout draws three things: the status block (which state, its two sentences, its one
//! button), Recent changes (up to 50 rows) and the three buttons at the bottom. The words of the
//! status block are the flyout's own ([`Status::title`]); the numbers in them, the rows and what
//! a button does come from here.
//!
//! [`BoardFixture`] holds exactly what the board draws for each frame (DW-1202 to DW-1222) and
//! answers each action with the frame the board sends it to. The engine lane replaces it; the
//! call each action needs is listed in `FRAMES.md`.

/// The states of the status block, in the board's order of frames.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// DW-1212: the first moment, before the state is known. A skeleton, no words.
    Checking,
    /// DW-1202.
    UpToDate { files: u32, last_change: String },
    /// DW-1203. `fraction`: bytes done of the whole batch.
    Syncing {
        files: u32,
        left: String,
        about: String,
        fraction: f32,
        speed: String,
    },
    /// DW-1204.
    Paused { waiting: u32 },
    /// DW-1222: this PC is on a metered connection and syncing on those is off.
    WaitingMetered,
    /// DW-1205.
    NeedsDecision { files: u32 },
    /// DW-1206.
    Offline,
    /// DW-1207.
    Stopped { files: u32 },
    /// DW-1208.
    StorageAlmostFull { used: String, plan: String },
    /// DW-1209.
    StorageFull { waiting: u32 },
    /// DW-1210.
    UpdateAvailable { version: String },
    /// DW-1214: this version is too old to connect.
    UpdateRequired,
    /// DW-1211.
    Locked,
    /// DW-1213. `waiting`: the plans page was opened, so the line reads "Waiting for your plan."
    NoPlan { waiting: bool },
    /// DW-1215.
    AddingToExplorer,
    /// DW-1216: `code` is the error Windows returned, for support.
    ExplorerFailed { code: String },
    /// DW-1219: signed out after 90 days without use.
    SignedOut,
}

/// The status block's one button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Pause,
    Resume,
    SyncAnyway,
    Review,
    TryAgain,
    ShowProblems,
    AddStorage,
    RestartToUpdate,
    Unlock,
    ChoosePlan,
    RetryExplorer,
    SignIn,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Pause => "Pause",
            Action::Resume => "Resume",
            Action::SyncAnyway => "Sync anyway",
            Action::Review => "Review\u{2026}",
            Action::TryAgain | Action::RetryExplorer => "Try again",
            Action::ShowProblems => "Show",
            Action::AddStorage => "Add storage",
            Action::RestartToUpdate => "Restart to update",
            Action::Unlock => "Unlock with Windows Hello",
            Action::ChoosePlan => "Choose a plan on beebeeb.io",
            Action::SignIn => "Sign in",
        }
    }
}

/// The colour of the status block's icon tile (`.mk-pst-ic.tone-*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Green,
    /// Orange: needs a decision, signed out.
    Warn,
    Red,
}

impl Status {
    pub fn title(&self, name: &str) -> String {
        match self {
            Status::Checking => "Checking".into(),
            Status::UpToDate { .. } => "Up to date".into(),
            Status::Syncing { files, .. } => format!("Syncing {} {}", count(*files), plural(*files, "file", "files")),
            Status::Paused { .. } => "Sync is paused".into(),
            Status::WaitingMetered => "Sync is waiting".into(),
            Status::NeedsDecision { files } => format!(
                "{} {} a decision",
                count(*files),
                plural(*files, "file needs", "files need")
            ),
            Status::Offline => format!("Can't reach {name}"),
            Status::Stopped { .. } => "Sync stopped".into(),
            Status::StorageAlmostFull { .. } => "Storage almost full".into(),
            Status::StorageFull { .. } => "Your storage is full".into(),
            Status::UpdateAvailable { version } => format!("Version {version} is available"),
            Status::UpdateRequired => format!("Update {name}"),
            Status::Locked => format!("{name} is locked"),
            Status::NoPlan { .. } => "Choose a plan".into(),
            Status::AddingToExplorer => format!("Adding {name} to File Explorer\u{2026}"),
            Status::ExplorerFailed { .. } => format!("Couldn't add {name} to File Explorer"),
            Status::SignedOut => "Signed out".into(),
        }
    }

    /// The line under the title. `None` only while checking.
    pub fn line(&self, name: &str) -> Option<String> {
        Some(match self {
            Status::Checking => return None,
            Status::UpToDate { files, last_change } => format!(
                "{} {} \u{00B7} last change {last_change} ago",
                count(*files),
                plural(*files, "file", "files")
            ),
            Status::Syncing { left, about, .. } => format!("{left} left \u{00B7} {about}"),
            Status::Paused { waiting } => format!(
                "{} {} on this PC",
                count(*waiting),
                plural(*waiting, "change waits", "changes wait")
            ),
            Status::WaitingMetered => "This PC is on a metered connection".into(),
            Status::NeedsDecision { .. } => "Changed in two places".into(),
            Status::Offline => "Files on this PC are unchanged".into(),
            Status::Stopped { files } => format!(
                "{} {} couldn't upload. Nothing was lost.",
                count(*files),
                plural(*files, "file", "files")
            ),
            Status::StorageAlmostFull { used, plan } => format!("{used} of {plan}"),
            Status::StorageFull { waiting } => format!(
                "{} {} can't upload. They stay on this PC.",
                count(*waiting),
                plural(*waiting, "change", "changes")
            ),
            Status::UpdateAvailable { .. } => "Restart to apply the update.".into(),
            Status::UpdateRequired => format!("This version of {name} is too old to connect. Update to continue."),
            Status::Locked => "Sync keeps running while it's locked.".into(),
            Status::NoPlan { waiting: false } => {
                format!(
                    "Your account is ready. Choose a plan on beebeeb.io, including the free trial. {name} continues here by itself."
                )
            }
            Status::NoPlan { waiting: true } => "Waiting for your plan.".into(),
            Status::AddingToExplorer => format!("{name} adds itself to File Explorer after sign-in."),
            Status::ExplorerFailed { .. } => format!("Windows didn't finish adding {name} to File Explorer."),
            Status::SignedOut => "Enter your password to keep syncing. Nothing has been lost.".into(),
        })
    }

    /// The reason in mono under the line (DW-1216), for support.
    pub fn code(&self) -> Option<&str> {
        match self {
            Status::ExplorerFailed { code } => Some(code),
            _ => None,
        }
    }

    pub fn tone(&self) -> Tone {
        match self {
            Status::UpToDate { .. } => Tone::Green,
            Status::NeedsDecision { .. } | Status::SignedOut => Tone::Warn,
            Status::Stopped { .. }
            | Status::StorageFull { .. }
            | Status::UpdateRequired
            | Status::ExplorerFailed { .. } => Tone::Red,
            _ => Tone::Neutral,
        }
    }

    /// The one button, and whether it is the state's main action (amber: "the only amber in
    /// the flyout when it is the state's main action", DW-1201).
    pub fn action(&self) -> Option<(Action, bool)> {
        Some(match self {
            Status::Checking | Status::UpToDate { .. } | Status::AddingToExplorer => return None,
            Status::Syncing { .. } => (Action::Pause, false),
            Status::Paused { .. } => (Action::Resume, true),
            Status::WaitingMetered => (Action::SyncAnyway, false),
            Status::NeedsDecision { .. } => (Action::Review, true),
            Status::Offline => (Action::TryAgain, false),
            Status::Stopped { .. } => (Action::ShowProblems, true),
            Status::StorageAlmostFull { .. } => (Action::AddStorage, false),
            Status::StorageFull { .. } => (Action::AddStorage, true),
            Status::UpdateAvailable { .. } => (Action::RestartToUpdate, false),
            Status::UpdateRequired => (Action::RestartToUpdate, true),
            Status::Locked => (Action::Unlock, true),
            Status::NoPlan { .. } => (Action::ChoosePlan, true),
            Status::ExplorerFailed { .. } => (Action::RetryExplorer, true),
            Status::SignedOut => (Action::SignIn, true),
        })
    }

    /// The name of the state for the b's accessibility label ("Beebeeb: syncing").
    pub fn spoken(&self) -> &'static str {
        match self {
            Status::Checking => "checking",
            Status::UpToDate { .. } => "up to date",
            Status::Syncing { .. } => "syncing",
            Status::Paused { .. } => "paused",
            Status::WaitingMetered => "waiting",
            Status::NeedsDecision { .. } => "needs a decision",
            Status::Offline => "offline",
            Status::Stopped { .. } => "stopped",
            Status::StorageAlmostFull { .. } => "storage almost full",
            Status::StorageFull { .. } => "storage full",
            Status::UpdateAvailable { .. } => "update available",
            Status::UpdateRequired => "update required",
            Status::Locked => "locked",
            Status::NoPlan { .. } => "no plan yet",
            Status::AddingToExplorer => "adding to File Explorer",
            Status::ExplorerFailed { .. } => "not in File Explorer",
            Status::SignedOut => "signed out",
        }
    }
}

fn plural(n: u32, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

/// A count as the board writes it: "1,284".
pub fn count(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// What a row's tile shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tile {
    Document,
    Sheet,
    Deck,
    Video,
    Audio,
    Archive,
    Folder,
    /// A file that was moved to Trash.
    Trashed,
    /// A photo's thumbnail. Until the engine hands over pixels: the board's sample photo of
    /// that number (`.mk-ph1` to `.mk-ph6`).
    Photo(u8),
}

/// Where a change came from: the small icon before the row's second line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    ThisPc,
    Phone,
    Tablet,
    People,
}

/// A row's second line.
#[derive(Clone, Debug, PartialEq)]
pub enum What {
    /// It happened: "Uploaded from this PC · Documents".
    Done { origin: Origin, text: String },
    /// It is moving: "Uploading · 3.4 of 8.1 MB" and its bar (amber for an upload, the
    /// encryption step; grey for a download).
    Moving { text: String, fraction: f32, upload: bool },
    /// It could not be synced: the reason, in red, on up to two lines.
    Failed { text: String },
}

/// One row of Recent changes.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    /// What the engine knows the file by; a click hands it back ([`TraySource::reveal`]).
    pub id: String,
    pub name: String,
    pub tile: Tile,
    pub what: What,
    /// A relative time: now, 2 min, 44 min, 2 h, Yesterday, a weekday within 7 days, then 26 Oct.
    pub when: String,
    /// The file is in Trash: a click opens Trash in the window instead of File Explorer.
    pub in_trash: bool,
}

/// The Recent changes list.
#[derive(Clone, Debug, PartialEq)]
pub enum Changes {
    /// Locked: no file names (DW-1211).
    Hidden,
    /// Nothing yet, and the sentence that says when there will be (DW-1213, DW-1215).
    Empty { text: String },
    /// Newest first. `complete`: these are the last 50, so the list ends with "That's the last
    /// 50 changes." (DW-1220).
    Rows { rows: Vec<Change>, complete: bool },
}

/// The most rows the flyout shows (DW-1201: "up to 50 rows, newest first").
pub const MAX_ROWS: usize = 50;

/// What the flyout shows right now.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub status: Status,
    pub changes: Changes,
    /// Beebeeb is in File Explorer: "Open in File Explorer" works (DW-1213, DW-1215, DW-1216
    /// draw it disabled).
    pub explorer_ready: bool,
}

/// What the flyout asks of the engine. Every call returns at once; the engine reports a change
/// by making the flyout read [`TraySource::snapshot`] again (`frames::tray::refresh`).
pub trait TraySource {
    fn snapshot(&self) -> Snapshot;
    /// The status block's button was pressed.
    fn act(&mut self, action: Action);
    /// A row was chosen: show the file in File Explorer, selected in its folder.
    fn reveal(&mut self, id: &str);
    /// The shell's tray state changed. A source on the engine knows already and ignores it;
    /// the fixture shows the frame of that state.
    fn follow(&mut self, _shell: &crate::shell::tray::TrayInputs) {}
}

/// The board's frames, each with exactly what the board draws in it.
pub struct BoardFixture {
    frame: &'static str,
    /// The plans page was opened from DW-1213: the line reads "Waiting for your plan."
    plan_opened: bool,
    /// What was asked of it, for tests: the engine calls a real source would make.
    pub asked: Vec<String>,
}

/// The flyout frames of the board, in its order.
pub const FRAMES: [&str; 19] = [
    "DW-1202", "DW-1203", "DW-1204", "DW-1205", "DW-1206", "DW-1207", "DW-1208", "DW-1209", "DW-1210", "DW-1211",
    "DW-1212", "DW-1213", "DW-1214", "DW-1215", "DW-1216", "DW-1219", "DW-1220", "DW-1221", "DW-1222",
];

impl BoardFixture {
    /// `None` for an id that is not a flyout frame.
    pub fn frame(id: &str) -> Option<BoardFixture> {
        let frame = FRAMES.iter().find(|f| **f == id)?;
        Some(BoardFixture {
            frame,
            plan_opened: false,
            asked: Vec::new(),
        })
    }

    /// Shows another frame's content (the shell's tray state changed: `frames::tray::follow`).
    pub fn show(&mut self, id: &str) {
        if let Some(frame) = FRAMES.iter().find(|f| **f == id) {
            self.frame = frame;
        }
    }
}

impl TraySource for BoardFixture {
    fn snapshot(&self) -> Snapshot {
        let (mut status, changes) = frame_content(self.frame);
        if let Status::NoPlan { waiting } = &mut status {
            *waiting = self.plan_opened;
        }
        Snapshot {
            status,
            changes,
            explorer_ready: !matches!(self.frame, "DW-1213" | "DW-1215" | "DW-1216" | "DW-1221"),
        }
    }

    /// Each action leads to the frame the board's Actions send it to.
    fn act(&mut self, action: Action) {
        self.asked.push(format!("{action:?}"));
        self.plan_opened |= action == Action::ChoosePlan;
        self.frame = match (self.frame, action) {
            (_, Action::Pause) => "DW-1204",
            (_, Action::Resume | Action::SyncAnyway) => "DW-1203",
            ("DW-1206", Action::TryAgain) => "DW-1202",
            (_, Action::RetryExplorer) => "DW-1215",
            // Review, Show, Sign in and Unlock continue in the window; Add storage, Choose a
            // plan and Restart to update leave the app. The flyout stays as it is until the
            // engine says otherwise.
            (frame, _) => frame,
        };
    }

    fn reveal(&mut self, id: &str) {
        self.asked.push(format!("reveal {id}"));
    }

    fn follow(&mut self, shell: &crate::shell::tray::TrayInputs) {
        self.show(super::frame_for(shell));
    }
}

fn done(name: &str, tile: Tile, origin: Origin, text: &str, when: &str) -> Change {
    Change {
        id: name.to_string(),
        name: name.to_string(),
        tile,
        what: What::Done {
            origin,
            text: text.to_string(),
        },
        when: when.to_string(),
        in_trash: false,
    }
}

/// The ten rows the board draws in most frames ("the first rows of 50").
fn recent() -> Vec<Change> {
    use Origin::{People, Phone, Tablet, ThisPc};
    let dot = "\u{00B7}";
    vec![
        done(
            "Moodboard v3.pdf",
            Tile::Document,
            ThisPc,
            &format!("Uploaded from this PC {dot} Documents"),
            "2 min",
        ),
        done(
            "Budget 2026.xlsx",
            Tile::Sheet,
            ThisPc,
            &format!("Uploaded from this PC {dot} Files"),
            "29 min",
        ),
        done(
            "IMG_4821.heic",
            Tile::Photo(2),
            Phone,
            &format!("From Anna's iPhone {dot} Photos"),
            "44 min",
        ),
        done(
            "IMG_4820.heic",
            Tile::Photo(3),
            Phone,
            &format!("From Anna's iPhone {dot} Photos"),
            "44 min",
        ),
        done(
            "Invoice 0142.pdf",
            Tile::Document,
            Tablet,
            &format!("From Anna's iPad {dot} Invoices 2026"),
            "2 h",
        ),
        done(
            "Contract draft.pdf",
            Tile::Document,
            Phone,
            &format!("From Anna's iPhone {dot} Files"),
            "Yesterday",
        ),
        Change {
            in_trash: true,
            ..done(
                "Notes old.md",
                Tile::Trashed,
                ThisPc,
                "Moved to Trash on this PC",
                "Yesterday",
            )
        },
        done(
            "Shoot selects",
            Tile::Folder,
            People,
            "Lotte de Wit added 6 files",
            "Yesterday",
        ),
        done(
            "Harbour.heic",
            Tile::Photo(5),
            Phone,
            &format!("From Anna's iPhone {dot} Files"),
            "Sunday",
        ),
        done(
            "IMG_4790.heic",
            Tile::Photo(1),
            Phone,
            &format!("From Anna's iPhone {dot} Photos"),
            "Sat",
        ),
    ]
}

fn moving(name: &str, tile: Tile, text: &str, fraction: f32, upload: bool) -> Change {
    Change {
        id: name.to_string(),
        name: name.to_string(),
        tile,
        what: What::Moving {
            text: text.to_string(),
            fraction,
            upload,
        },
        when: "now".to_string(),
        in_trash: false,
    }
}

fn frame_content(frame: &str) -> (Status, Changes) {
    use Origin::{People, Phone, Tablet, ThisPc};
    let dot = "\u{00B7}";
    let rows = |rows: Vec<Change>| Changes::Rows { rows, complete: false };
    let up_to_date = Status::UpToDate {
        files: 1284,
        last_change: "2 min".into(),
    };
    let explorer_failed = Status::ExplorerFailed {
        code: "0x8007017C".into(),
    };
    match frame {
        "DW-1203" => {
            let mut list = vec![
                moving(
                    "Lisbon trip 041.heic",
                    Tile::Photo(1),
                    &format!("Uploading {dot} 3.4 of 8.1 MB"),
                    0.42,
                    true,
                ),
                moving(
                    "Q4 plan.key",
                    Tile::Deck,
                    &format!("Downloading {dot} 9 of 22.6 MB"),
                    0.40,
                    false,
                ),
            ];
            list.extend(recent().into_iter().take(8));
            (
                Status::Syncing {
                    files: 12,
                    left: "48 MB".into(),
                    about: "about a minute".into(),
                    fraction: 0.38,
                    speed: "3.2 MB/s".into(),
                },
                rows(list),
            )
        }
        "DW-1204" => (Status::Paused { waiting: 7 }, rows(recent())),
        "DW-1205" => (Status::NeedsDecision { files: 2 }, rows(recent())),
        "DW-1206" => (Status::Offline, rows(recent())),
        "DW-1207" => {
            let mut list = vec![Change {
                id: "Studio walkthrough.mov".into(),
                name: "Studio walkthrough.mov".into(),
                tile: Tile::Video,
                what: What::Failed {
                    text: "Couldn't upload: the file was open in another app".into(),
                },
                when: "1 min".into(),
                in_trash: false,
            }];
            list.extend(recent().into_iter().take(9));
            (Status::Stopped { files: 3 }, rows(list))
        }
        "DW-1208" => (
            Status::StorageAlmostFull {
                used: "912 GB".into(),
                plan: "1 TB".into(),
            },
            rows(recent()),
        ),
        "DW-1209" => (Status::StorageFull { waiting: 9 }, rows(recent())),
        "DW-1210" => (
            Status::UpdateAvailable {
                version: "0.9.2".into(),
            },
            rows(recent()),
        ),
        "DW-1211" => (Status::Locked, Changes::Hidden),
        "DW-1212" => (Status::Checking, rows(recent())),
        "DW-1213" => (
            Status::NoPlan { waiting: false },
            Changes::Empty {
                text: "Changes show here once Beebeeb syncs this PC.".into(),
            },
        ),
        "DW-1214" => (Status::UpdateRequired, rows(recent())),
        "DW-1215" => (
            Status::AddingToExplorer,
            Changes::Empty {
                text: "Changes show here once Beebeeb is in File Explorer.".into(),
            },
        ),
        "DW-1216" => (explorer_failed, rows(recent())),
        "DW-1219" => (Status::SignedOut, rows(recent())),
        // The end of the list: the board draws the last seven of the 50.
        "DW-1220" => (
            up_to_date,
            Changes::Rows {
                rows: vec![
                    done(
                        "Proposal draft 8.pdf",
                        Tile::Document,
                        ThisPc,
                        &format!("Uploaded from this PC {dot} Documents"),
                        "23 Oct",
                    ),
                    done(
                        "IMG_4755.heic",
                        Tile::Photo(6),
                        Phone,
                        &format!("From Anna's iPhone {dot} Photos"),
                        "23 Oct",
                    ),
                    done(
                        "Invoice 0134.pdf",
                        Tile::Document,
                        Tablet,
                        &format!("From Anna's iPad {dot} Invoices 2026"),
                        "22 Oct",
                    ),
                    done(
                        "Rates 2026 v9.xlsx",
                        Tile::Sheet,
                        ThisPc,
                        &format!("Uploaded from this PC {dot} Documents"),
                        "22 Oct",
                    ),
                    done(
                        "Client call 9.m4a",
                        Tile::Audio,
                        ThisPc,
                        &format!("Downloaded to this PC {dot} Documents"),
                        "22 Oct",
                    ),
                    done(
                        "Proposal draft 9.pdf",
                        Tile::Document,
                        ThisPc,
                        &format!("Uploaded from this PC {dot} Documents"),
                        "22 Oct",
                    ),
                    done(
                        "IMG_4750.heic",
                        Tile::Photo(5),
                        Phone,
                        &format!("From Anna's iPhone {dot} Photos"),
                        "21 Oct",
                    ),
                ],
                complete: true,
            },
        ),
        // The longest copy and names.
        "DW-1221" => {
            let mut list = vec![
                done(
                    "Northwind Studio \u{2014} brand guidelines, final approved version for print and web (v14).pdf",
                    Tile::Document,
                    ThisPc,
                    &format!(
                        "Uploaded from this PC {dot} Clients \u{203A} Northwind Studio \u{203A} Brand \u{203A} 2026 \u{203A} Final"
                    ),
                    "2 min",
                ),
                done(
                    "Harbourlight shoot \u{2014} selects for the autumn campaign, colour graded, full resolution.zip",
                    Tile::Archive,
                    People,
                    "Lotte de Wit added 6 files to Harbourlight shoot",
                    "44 min",
                ),
            ];
            list.extend(recent().into_iter().skip(1).take(8));
            (explorer_failed, rows(list))
        }
        "DW-1222" => (Status::WaitingMetered, rows(recent())),
        _ => (up_to_date, rows(recent())),
    }
}
