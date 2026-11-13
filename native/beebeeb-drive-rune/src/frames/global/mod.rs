//! The global elements of the window (the board's "Global elements on Windows"): the one banner
//! under the toolbar, the loading strip, and what a preview says when it needs a connection.
//!
//! They sit ON another frame. The Files window asks for them at three places
//! (`frames/files/window.rs`): [`under_toolbar`], [`over_body`] and [`fit`]. What they show comes
//! from a [`GlobalSource`]; what a control asks of the rest of the app is an [`Intent`].
//!
//! The toasts of this board group (DW-0107 to DW-0109) are the Files frames' (`ui::toast`,
//! `frames::files`): an action there shows them.

pub mod grid;
pub mod samples;
pub mod source;
pub mod sync_off;
#[cfg(test)]
mod tests;
mod view;
#[cfg(test)]
mod window_tests;

use rune::Tx;

use crate::State;
use crate::ui::banner::Run;
use crate::ui::gate::Scene;
pub use grid::grid_body;
pub use source::{GlobalFixture, GlobalSource, GlobalStatus, Quiet, StorageLevel};
pub use view::{banner_under_toolbar, dims, fit, over_body, status_line, status_words, under_toolbar};

/// The banners, in the order the board gives them the window: "One banner at a time, in this
/// order: signed out, storage full, sync problem, backup problem, storage almost full, offline."
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BannerKind {
    SignedOut,
    StorageFull,
    SyncProblem,
    BackupProblem,
    StorageAlmostFull,
    Offline,
}

impl BannerKind {
    pub const ORDER: [BannerKind; 6] = [
        BannerKind::SignedOut,
        BannerKind::StorageFull,
        BannerKind::SyncProblem,
        BannerKind::BackupProblem,
        BannerKind::StorageAlmostFull,
        BannerKind::Offline,
    ];

    /// The frame its button opens ("Opens where it is solved").
    pub fn solved_at(self) -> Option<&'static str> {
        match self {
            BannerKind::SignedOut => Some("DW-1109"),
            BannerKind::StorageFull | BannerKind::StorageAlmostFull => Some("DW-0712"),
            BannerKind::SyncProblem => Some("DW-0428"),
            BannerKind::BackupProblem => Some("DW-0741"),
            BannerKind::Offline => None,
        }
    }

    /// The button's word.
    pub fn action(self) -> Option<&'static str> {
        match self {
            BannerKind::SignedOut => Some("Sign in"),
            BannerKind::StorageFull | BannerKind::StorageAlmostFull => Some("Storage"),
            BannerKind::SyncProblem | BannerKind::BackupProblem => Some("View"),
            BannerKind::Offline => None,
        }
    }

    /// May the person dismiss it? Not what stops the app (signed out, full), and not offline,
    /// which "goes away by itself when the connection returns".
    pub fn dismissible(self) -> bool {
        matches!(
            self,
            BannerKind::SyncProblem | BannerKind::BackupProblem | BannerKind::StorageAlmostFull
        )
    }
}

fn items(n: u32) -> String {
    if n == 1 {
        "1 item".to_string()
    } else {
        format!("{n} items")
    }
}

/// The sentence of a banner, in the board's words.
pub fn sentence_of(kind: BannerKind, status: &GlobalStatus) -> Vec<Run> {
    let words = |s: &str| Run::Words(s.to_string());
    match kind {
        BannerKind::SignedOut => {
            vec![words(
                "Backup and sync have stopped. Enter your password to continue. Nothing has been lost.",
            )]
        }
        BannerKind::StorageFull => {
            vec![words(
                "Your storage is full. New uploads and backups are paused. Your files are safe.",
            )]
        }
        BannerKind::SyncProblem => vec![Run::Words(format!(
            "{} can't sync. Everything else is up to date.",
            items(status.sync_problems)
        ))],
        BannerKind::BackupProblem => vec![Run::Words(format!(
            "{} couldn't back up. Everything else is backed up.",
            items(status.backup_problems)
        ))],
        BannerKind::StorageAlmostFull => {
            let storage = status.storage.unwrap_or(source::Storage {
                used_bytes: 0,
                quota_bytes: 0,
            });
            vec![
                words("Your storage is almost full. "),
                Run::Figure(source::size_words(storage.used_bytes)),
                words(" of "),
                Run::Figure(source::size_words(storage.quota_bytes)),
                words(" used."),
            ]
        }
        BannerKind::Offline => match &status.offline {
            Some(listed) => vec![
                Run::Words(format!("You're offline. This list is from {} at ", listed.day)),
                Run::Figure(listed.time.clone()),
                words(". Changes upload when you're back."),
            ],
            None => vec![words("You're offline. Changes upload when you're back.")],
        },
    }
}

/// What a control of a global element asks of the rest of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// A board frame this group does not draw ("Opens where it is solved").
    Frame(&'static str),
    /// The connection is back: open the file (by id) the preview could not show (DW-0408).
    OpenPreview(String),
}

pub const INTENTS_KEPT: usize = 32;

/// What was dismissed, and at what count, so a banner "comes back when another item can't sync"
/// and "as storage gets fuller (90%, 95%, 99%)".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Dismissed {
    sync_at: u32,
    backup_at: u32,
    storage_at: u8,
}

/// What the global elements keep.
pub struct Ui {
    pub source: Box<dyn GlobalSource>,
    dismissed: std::cell::Cell<Dismissed>,
    /// The file a preview could not open without a connection (DW-0121): its id and its name.
    pub blocked_preview: Option<(String, String)>,
    /// The last things asked for, oldest first (at most [`INTENTS_KEPT`]).
    pub intents: Vec<Intent>,
    /// A board frame that draws only the upper part of the window (`.mk-cropfade.at-bottom`):
    /// the body keeps its full height and its lower edge fades. `false` in the app.
    pub board_crop_bottom: bool,
    /// A board frame whose status line (under its crop, not drawn) carries another frame's words:
    /// the words, so the frame's text is the board's. `None` in the app: the window counts.
    pub board_status_words: Option<String>,
    /// The folders shown as a grid, and what is known of their thumbnails (DW-0119).
    pub grid: grid::GridUi,
}

impl Default for Ui {
    fn default() -> Self {
        Ui::new(Box::new(Quiet))
    }
}

impl Ui {
    pub fn new(source: Box<dyn GlobalSource>) -> Self {
        Ui {
            source,
            dismissed: std::cell::Cell::new(Dismissed::default()),
            blocked_preview: None,
            intents: Vec::new(),
            board_crop_bottom: false,
            board_status_words: None,
            grid: grid::GridUi::default(),
        }
    }

    /// Does this state call for this banner (dismissed or not)?
    fn holds(kind: BannerKind, status: &GlobalStatus) -> bool {
        let level = status.storage.map_or(StorageLevel::Roomy, source::Storage::level);
        match kind {
            BannerKind::SignedOut => status.signed_out,
            BannerKind::StorageFull => level == StorageLevel::Full,
            BannerKind::SyncProblem => status.sync_problems > 0,
            BannerKind::BackupProblem => status.backup_problems > 0,
            BannerKind::StorageAlmostFull => matches!(level, StorageLevel::AlmostFull(_)),
            BannerKind::Offline => status.offline.is_some(),
        }
    }

    fn is_dismissed(&self, kind: BannerKind, status: &GlobalStatus) -> bool {
        let dismissed = self.dismissed.get();
        match kind {
            BannerKind::SyncProblem => status.sync_problems <= dismissed.sync_at,
            BannerKind::BackupProblem => status.backup_problems <= dismissed.backup_at,
            BannerKind::StorageAlmostFull => match status.storage.map(source::Storage::level) {
                Some(StorageLevel::AlmostFull(step)) => step <= dismissed.storage_at,
                _ => false,
            },
            _ => false,
        }
    }

    /// Forget a dismissal whose cause is gone, so the next time it happens it shows again.
    fn settle(&self, status: &GlobalStatus) {
        let mut dismissed = self.dismissed.get();
        if status.sync_problems == 0 {
            dismissed.sync_at = 0;
        }
        if status.backup_problems == 0 {
            dismissed.backup_at = 0;
        }
        if !matches!(
            status.storage.map(source::Storage::level),
            Some(StorageLevel::AlmostFull(_))
        ) {
            dismissed.storage_at = 0;
        }
        self.dismissed.set(dismissed);
    }

    /// The one banner to show: the first in the board's order that holds and was not dismissed.
    /// None while the app is locked (the lock screen is another frame).
    pub fn banner(&self) -> Option<(BannerKind, GlobalStatus)> {
        let status = self.source.status();
        self.settle(&status);
        if status.locked {
            return None;
        }
        let kind = BannerKind::ORDER
            .into_iter()
            .find(|&kind| Self::holds(kind, &status) && !self.is_dismissed(kind, &status))?;
        Some((kind, status))
    }

    /// Dismiss the banner that shows, "until it happens again".
    pub fn dismiss(&mut self) -> bool {
        let Some((kind, status)) = self.banner() else {
            return false;
        };
        let mut dismissed = self.dismissed.get();
        match kind {
            BannerKind::SyncProblem => dismissed.sync_at = status.sync_problems,
            BannerKind::BackupProblem => dismissed.backup_at = status.backup_problems,
            BannerKind::StorageAlmostFull => {
                if let Some(StorageLevel::AlmostFull(step)) = status.storage.map(source::Storage::level) {
                    dismissed.storage_at = step;
                }
            }
            _ => return false,
        }
        self.dismissed.set(dismissed);
        true
    }

    fn ask(&mut self, intent: Intent) {
        if self.intents.len() >= INTENTS_KEPT {
            self.intents.remove(0);
        }
        self.intents.push(intent);
    }

    /// The shell takes what was asked for (a lane that draws DW-1109 or DW-0428 routes them).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }
}

impl State {
    /// The banner's button: "Opens where it is solved". The shell's route machine takes the
    /// places it has (Settings' Storage and Backups, Sync history); every one is kept as an intent.
    pub fn banner_action(&mut self) -> bool {
        use crate::shell::app::{Event, Route, Section, SettingsPane};
        let Some(frame) = self.global.banner().and_then(|(kind, _)| kind.solved_at()) else {
            return false;
        };
        self.global.ask(Intent::Frame(frame));
        let event = match frame {
            "DW-0712" => Some(Event::OpenSettings(SettingsPane::Storage)),
            "DW-0741" => Some(Event::OpenSettings(SettingsPane::Backups)),
            "DW-0428" => Some(Event::Navigate(Route::section(Section::SyncHistory))),
            _ => None,
        };
        if let Some(event) = event {
            crate::shell::host::dispatch(self, event);
        }
        true
    }

    /// A file that is not on this PC was opened without a connection (DW-0121).
    pub fn block_preview(&mut self, id: impl Into<String>, name: impl Into<String>) {
        self.global.blocked_preview = Some((id.into(), name.into()));
    }

    /// Esc, or the panel's cross: back to the list (DW-1108).
    pub fn close_blocked_preview(&mut self) -> bool {
        self.global.blocked_preview.take().is_some()
    }

    /// Try again: "Checks the connection; opens the file if it's back".
    pub fn retry_blocked_preview(&mut self) -> bool {
        let Some((id, _)) = self.global.blocked_preview.clone() else {
            return false;
        };
        if self.global.source.check_connection() {
            self.global.blocked_preview = None;
            self.global.ask(Intent::OpenPreview(id));
            return true;
        }
        false
    }
}

/// A preview of the selected file was asked for. Without a connection, a file that is not on this
/// PC cannot open: the panel says so (DW-0121) and this answers `true`. Otherwise `false`: the
/// viewer opens it (DW-0408).
pub fn preview_needs_connection(s: &mut State) -> bool {
    if s.global.source.status().offline.is_none() {
        return false;
    }
    let Some(item) = s.selected.and_then(|i| s.listing.items.get(i)) else {
        return false;
    };
    if item.is_folder || item.pc_status.as_deref() != Some("online_only") {
        return false;
    }
    let (id, name) = (item.id.clone(), item.name.clone());
    s.block_preview(id, name);
    true
}

fn act_banner(s: &mut State, _: &mut Tx) {
    s.banner_action();
}

fn dismiss_banner(s: &mut State, tx: &mut Tx) {
    if s.global.dismiss() && s.sidebar_section == crate::ui::sidebar::SidebarSection::Files {
        // The cross is gone with the banner: the list takes the keyboard back.
        tx.focus("rows", false);
    }
}

fn retry_preview(s: &mut State, _: &mut Tx) {
    s.retry_blocked_preview();
}

fn close_preview(s: &mut State, tx: &mut Tx) {
    if s.close_blocked_preview() {
        tx.focus("rows", false);
    }
}

fn scene(s: &mut State, status: GlobalStatus) {
    s.show_detail_pane = false;
    s.clear_selection();
    s.global = Ui::new(Box::new(GlobalFixture::of(status)));
    s.global.board_crop_bottom = true;
}

/// The board frames of this group, by frame id.
pub const FRAMES: &[Scene<State>] = &[
    ("DW-0101", |s| scene(s, source::board::offline())),
    ("DW-0102", |s| scene(s, source::board::sync_problem())),
    ("DW-0103", |s| scene(s, source::board::storage_almost_full())),
    ("DW-0104", |s| scene(s, source::board::storage_full())),
    ("DW-0105", |s| scene(s, source::board::signed_out())),
    ("DW-0106", |s| scene(s, source::board::backup_problem())),
    ("DW-0118", |s| scene(s, source::board::loading())),
    // DW-0119: Lisbon trip as a grid, its thumbnails still to arrive.
    ("DW-0119", grid::scene),
    // DW-0121: offline, and Contract draft.pdf (online only) was opened.
    ("DW-0121", |s| {
        s.actions = crate::frames::files::Actions::new(Box::new(samples::SampleFiles::offline()));
        s.select_only(6);
        s.global = Ui::new(Box::new(GlobalFixture::of(source::board::offline())));
        let item = &s.listing.items[6];
        s.global.blocked_preview = Some((item.id.clone(), item.name.clone()));
    }),
    // DW-0114: a new account, nothing in it yet.
    ("DW-0114", |s| {
        s.source = Box::new(samples::EmptyDrive);
        (s.listing, s.list_error) = crate::load(s.source.as_ref(), None);
        (s.storage, s.storage_known) = (samples::STORAGE, true);
        s.actions = crate::frames::files::Actions::new(Box::new(samples::SampleFiles::new_account()));
        s.clear_selection();
    }),
];
