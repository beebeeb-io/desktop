//! `ThisPcSource`: what Sync history and Available offline ask of the engine, and the board's
//! fixture (DW-1301 to DW-1306, DW-0428, DW-0715, DW-0120: exactly the board's sample content).
//!
//! The history is this PC's own record (the board: "kept on this PC only", 30 days). The items
//! that did not sync are a list of their own ([`ThisPcSource::problems`]): the Problems filter
//! shows that list, the other filters show the changes, and a change that failed carries the
//! id of its problem so Try again reaches the same item from both.

use chrono::NaiveDate;

/// What kind of thing changed: which glyph its tile shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Document,
    Sheet,
    Deck,
    Video,
    Audio,
    Folder,
    /// A picture with a thumbnail; the number picks the board's sample picture until the engine
    /// hands over pixels.
    Photo(u8),
}

/// Where a change came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    ThisPc,
    Phone(String),
    Tablet(String),
    SharedFolder,
}

impl Origin {
    pub fn label(&self) -> &str {
        match self {
            Origin::ThisPc => "This PC",
            Origin::Phone(name) | Origin::Tablet(name) => name,
            Origin::SharedFolder => "Shared folder",
        }
    }
}

/// Why an item did not sync, in the board's plain words. Never a raw error code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    OpenInAnotherApp,
    Unreachable { tries: u8 },
    NameInUse { place: String },
}

impl Reason {
    /// The line under the name: why, and that nothing was lost.
    pub fn line(&self) -> String {
        match self {
            Reason::OpenInAnotherApp => "Couldn't upload: the file was open in another app. Nothing was lost.".into(),
            Reason::Unreachable { tries } => {
                format!("Couldn't reach Beebeeb after {tries} tries. Nothing was lost.")
            }
            Reason::NameInUse { place } => format!("The name is already in use in {place}. Nothing was lost."),
        }
    }
}

/// What happened to the item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum What {
    Uploaded {
        place: String,
    },
    Added {
        place: String,
    },
    Changed {
        place: String,
    },
    Downloaded {
        place: String,
    },
    MovedToTrash {
        place: String,
    },
    /// Someone added files to a shared folder.
    SharedAdded {
        who: String,
        files: u32,
    },
    /// It did not sync: `problem` is the id in [`ThisPcSource::problems`].
    Failed {
        problem: String,
        reason: Reason,
    },
}

impl What {
    pub fn line(&self) -> String {
        match self {
            What::Uploaded { place } => format!("Uploaded \u{00B7} {place}"),
            What::Added { place } => format!("Added \u{00B7} {place}"),
            What::Changed { place } => format!("Changed \u{00B7} {place}"),
            What::Downloaded { place } => format!("Downloaded \u{00B7} {place}"),
            What::MovedToTrash { place } => format!("Moved to Trash \u{00B7} {place}"),
            What::SharedAdded { who, files } => format!("{who} added {files} files"),
            What::Failed { reason, .. } => reason.line(),
        }
    }

    /// The folder the search also matches ("Matches names and folders").
    pub fn place(&self) -> Option<&str> {
        match self {
            What::Uploaded { place }
            | What::Added { place }
            | What::Changed { place }
            | What::Downloaded { place }
            | What::MovedToTrash { place } => Some(place),
            What::Failed {
                reason: Reason::NameInUse { place },
                ..
            } => Some(place),
            _ => None,
        }
    }
}

/// One line of the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub what: What,
    pub origin: Origin,
    pub date: NaiveDate,
    /// "09:39", this PC's clock.
    pub time: String,
}

impl Change {
    pub fn problem(&self) -> Option<&str> {
        match &self.what {
            What::Failed { problem, .. } => Some(problem),
            _ => None,
        }
    }
}

/// How an item kept on this PC is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptStatus {
    UpToDate,
}

impl KeptStatus {
    pub fn label(self) -> &'static str {
        match self {
            KeptStatus::UpToDate => "Up to date",
        }
    }
}

/// What a kept item holds: a folder's item count, or the folder a file is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contains {
    Items(u32),
    In(String),
}

impl Contains {
    pub fn label(&self) -> String {
        match self {
            Contains::Items(1) => "1 item".into(),
            Contains::Items(n) => format!("{n} items"),
            Contains::In(place) => place.clone(),
        }
    }
}

/// An item that stays on this PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub bytes: i64,
    pub contains: Contains,
    pub status: KeptStatus,
}

/// What the two This PC sections read and ask. Engine calls: see the PR body.
pub trait ThisPcSource {
    /// This PC's date, for "Today" and "Yesterday".
    fn today(&self) -> NaiveDate;
    /// The last 30 days, newest first.
    fn history(&self) -> Vec<Change>;
    /// The items that did not sync, newest first.
    fn problems(&self) -> Vec<Change>;
    /// Try one item again now.
    fn retry(&mut self, problem: &str);
    /// Try every item again.
    fn retry_all(&mut self);
    /// What is kept on this PC.
    fn kept(&self) -> Vec<Kept>;
    /// Stop keeping one item here; it stays in Beebeeb and becomes online only.
    fn remove_from_pc(&mut self, id: &str);
    /// Stop keeping every item here.
    fn remove_all_from_pc(&mut self);
}

/// The board's sample content. A retry in the fixture succeeds at once: the item leaves the
/// list ("it leaves the list once it syncs").
pub struct FixtureSource {
    history: Vec<Change>,
    problems: Vec<Change>,
    kept: Vec<Kept>,
}

fn day(d: u32, m: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap_or_default()
}

/// The board's "today": Sunday is 1 Nov and Yesterday holds Monday's changes.
pub const BOARD_TODAY: (u32, u32) = (3, 11);

#[allow(clippy::too_many_arguments)]
fn change(id: &str, date: NaiveDate, time: &str, name: &str, kind: Kind, what: What, origin: Origin) -> Change {
    Change {
        id: id.into(),
        name: name.into(),
        kind,
        what,
        origin,
        date,
        time: time.into(),
    }
}

impl FixtureSource {
    /// DW-1301 and its filters, DW-0428, DW-1304: 19 changes over five days, 3 items that did
    /// not sync; and DW-0715's three kept items.
    pub fn board() -> FixtureSource {
        let up = |p: &str| What::Uploaded { place: p.into() };
        let add = |p: &str| What::Added { place: p.into() };
        let chg = |p: &str| What::Changed { place: p.into() };
        let down = |p: &str| What::Downloaded { place: p.into() };
        let iphone = || Origin::Phone("Anna's iPhone".into());
        let ipad = || Origin::Tablet("Anna's iPad".into());
        let pc = || Origin::ThisPc;
        let studio = || What::Failed {
            problem: "p-studio".into(),
            reason: Reason::OpenInAnotherApp,
        };
        let (today, yesterday, sun, sat, fri) = (day(3, 11), day(2, 11), day(1, 11), day(31, 10), day(30, 10));
        use Kind::{Audio, Deck, Document, Folder, Photo, Sheet, Video};
        let history = vec![
            change(
                "c-01",
                today,
                "09:39",
                "Moodboard v3.pdf",
                Document,
                up("Documents"),
                pc(),
            ),
            change("c-02", today, "09:38", "Studio walkthrough.mov", Video, studio(), pc()),
            change("c-03", today, "09:12", "Budget 2026.xlsx", Sheet, up("Files"), pc()),
            change(
                "c-04",
                today,
                "08:57",
                "IMG_4821.heic",
                Photo(2),
                add("Photos"),
                iphone(),
            ),
            change(
                "c-05",
                today,
                "08:57",
                "IMG_4820.heic",
                Photo(3),
                add("Photos"),
                iphone(),
            ),
            change(
                "c-06",
                today,
                "07:40",
                "Invoice 0142.pdf",
                Document,
                chg("Invoices 2026"),
                ipad(),
            ),
            change(
                "c-07",
                yesterday,
                "16:42",
                "Contract draft.pdf",
                Document,
                chg("Files"),
                iphone(),
            ),
            change(
                "c-08",
                yesterday,
                "14:05",
                "Notes old.md",
                Document,
                What::MovedToTrash {
                    place: "Documents".into(),
                },
                pc(),
            ),
            change(
                "c-09",
                yesterday,
                "11:20",
                "Shoot selects",
                Folder,
                What::SharedAdded {
                    who: "Lotte de Wit".into(),
                    files: 6,
                },
                Origin::SharedFolder,
            ),
            change("c-10", yesterday, "10:02", "Q4 plan.key", Deck, down("Files"), pc()),
            change("c-11", sun, "18:05", "Harbour.heic", Photo(5), add("Files"), iphone()),
            change("c-12", sat, "17:48", "IMG_4790.heic", Photo(1), add("Photos"), iphone()),
            change(
                "c-13",
                sat,
                "12:30",
                "Invoice 0141.pdf",
                Document,
                add("Invoices 2026"),
                ipad(),
            ),
            change("c-14", sat, "11:15", "Rates 2026 v2.xlsx", Sheet, up("Documents"), pc()),
            change(
                "c-15",
                sat,
                "10:02",
                "Client call 2.m4a",
                Audio,
                down("Documents"),
                pc(),
            ),
            change(
                "c-16",
                fri,
                "16:20",
                "Proposal draft 2.pdf",
                Document,
                up("Documents"),
                pc(),
            ),
            change("c-17", fri, "13:54", "IMG_4785.heic", Photo(6), add("Photos"), iphone()),
            change(
                "c-18",
                fri,
                "09:41",
                "Invoice 0140.pdf",
                Document,
                add("Invoices 2026"),
                ipad(),
            ),
            change("c-19", fri, "08:30", "Rates 2026 v3.xlsx", Sheet, up("Documents"), pc()),
        ];
        let failed = |problem: &str, reason: Reason| What::Failed {
            problem: problem.into(),
            reason,
        };
        let problems = vec![
            change("c-02", today, "09:38", "Studio walkthrough.mov", Video, studio(), pc()),
            change(
                "c-20",
                today,
                "09:31",
                "Tram 28.mov",
                Photo(3),
                failed("p-tram", Reason::Unreachable { tries: 5 }),
                pc(),
            ),
            change(
                "c-21",
                today,
                "08:12",
                "Budget 2026 (2).xlsx",
                Sheet,
                failed("p-budget", Reason::NameInUse { place: "Files".into() }),
                pc(),
            ),
        ];
        FixtureSource {
            history,
            problems,
            kept: Self::board_kept(),
        }
    }

    fn board_kept() -> Vec<Kept> {
        let kept = |id: &str, name: &str, kind, bytes, contains| Kept {
            id: id.into(),
            name: name.into(),
            kind,
            bytes,
            contains,
            status: KeptStatus::UpToDate,
        };
        vec![
            kept(
                "k-documents",
                "Documents",
                Kind::Folder,
                1_503_238_553,
                Contains::Items(48),
            ),
            kept(
                "k-invoices",
                "Invoices 2026",
                Kind::Folder,
                29_150_412,
                Contains::Items(11),
            ),
            kept(
                "k-budget",
                "Budget 2026.xlsx",
                Kind::Sheet,
                86_016,
                Contains::In("Files".into()),
            ),
        ]
    }

    /// DW-0715 and DW-1306: everything synced, three items kept.
    pub fn synced() -> FixtureSource {
        FixtureSource {
            problems: Vec::new(),
            history: Vec::new(),
            ..Self::board()
        }
    }

    /// DW-1305: a PC that just signed in. No history, nothing that failed; the three kept items
    /// are the account's (the sidebar still counts them).
    pub fn just_signed_in() -> FixtureSource {
        Self::synced()
    }

    /// DW-0120: nothing kept on this PC.
    pub fn nothing_kept() -> FixtureSource {
        FixtureSource {
            kept: Vec::new(),
            ..Self::synced()
        }
    }
}

impl ThisPcSource for FixtureSource {
    fn today(&self) -> NaiveDate {
        day(BOARD_TODAY.0, BOARD_TODAY.1)
    }

    fn history(&self) -> Vec<Change> {
        self.history.clone()
    }

    fn problems(&self) -> Vec<Change> {
        self.problems.clone()
    }

    fn retry(&mut self, problem: &str) {
        self.problems.retain(|c| c.problem() != Some(problem));
        self.history.retain(|c| c.problem() != Some(problem));
    }

    fn retry_all(&mut self) {
        self.problems.clear();
        self.history.retain(|c| c.problem().is_none());
    }

    fn kept(&self) -> Vec<Kept> {
        self.kept.clone()
    }

    fn remove_from_pc(&mut self, id: &str) {
        self.kept.retain(|k| k.id != id);
    }

    fn remove_all_from_pc(&mut self) {
        self.kept.clear();
    }
}
