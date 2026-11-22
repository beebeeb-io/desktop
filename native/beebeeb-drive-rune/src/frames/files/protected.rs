//! Protected folders in the Files window (DW-0422 to DW-0425, DW-1110): Backups as a section of
//! its own, what one device backed up, the folder a service manages, and the message that says
//! why nothing in them can be moved, renamed or deleted.
//!
//! **The rule is the source's, the refusal is the window's.** [`FilesSource::protection`] says
//! whether an item (or anything in a folder) can change, and hands over the words to say it with
//! (the core's, so every Beebeeb app says the same). The window asks BEFORE it calls an action
//! on the source ([`State::protects`]), in every path: the file menu, Delete, F2, Ctrl+D, Move,
//! a selection of several that holds one such item. A source that refuses anyway
//! (`ActionError::Protected`) ends in the same message ([`State::say_protected`]).
//!
//! What still works everywhere in the tree: open, preview, download, Show in File Explorer,
//! Details ("Everything can be opened and downloaded; only the service changes it").
//!
//! A source that knows no Backups folder (`backups_root` is `None`: the default) leaves the
//! section on the shell's "not built yet" frame: no sample is ever shown in its place.

use super::source::{Action, ActionError};
use super::{Intent, Toast};
use crate::State;
use crate::shell::app::{Event, Route, Screen, Section, SettingsPane};
use crate::shell::registry::{FeedbackPolicy, Frame, FrameGroup};
use crate::source::DriveItem;
use crate::theme::icons;
use crate::theme::icons_settings;
use crate::theme::scale;
use crate::theme::tokens::{Theme, type_};
use crate::ui::backups_kit::{self, Md, TileTone};
use crate::ui::banner::{self, Banner, Run, Tone};
use crate::ui::button::{self, ButtonState, Size, Variant};
use crate::ui::file_list::{PcState, format_date, pc_glyph};
use crate::ui::file_row::{self, Column, RowLook, Width};
use crate::ui::settings_dialog::{self, Weight};
use crate::ui::view_state::{self, ViewState};
use crate::ui::{dialog, kit};
use rune::{Align, Cx, El, Icon, Key, KeyEvent, TextStyle, Tx, el, row};

/// The words of the message, as the source hands them over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Words {
    pub title: String,
    pub body: String,
}

/// Why something cannot be moved, renamed or deleted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Protection {
    /// A Beebeeb service manages it: the message says so (DW-0425, DW-1110).
    Managed(Words),
    /// It is (in) the backup of the device with this id (the id the Backups section knows the
    /// device by): only the backup changes it, and deleting
    /// it is Delete this backup, behind a code (DW-0742).
    DeviceBackup(String),
    /// The Backups folder itself: only backups add to it.
    BackupsRoot,
}

/// What leads a row of the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupKind {
    Phone,
    Tablet,
    Laptop,
    Mail,
    Contacts,
    Calendar,
    /// A plain folder that has something to say beside its name.
    Folder,
}

/// What a row of the protected tree is, beyond a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupEntry {
    pub kind: BackupKind,
    /// The Status column of Backups: "12,418 photos", "This PC", "Not signed in".
    pub status: Option<String>,
    /// A service's folder: it shows the lock.
    pub managed: bool,
    /// Words beside the name: "Chosen folder" for a folder the person added to the backup (DW-0423).
    pub note: Option<String>,
}

/// What the window says once the local file list was rebuilt (144b moved a damaged `state.db` aside).
pub const REBUILT_WORDS: &str =
    "Beebeeb rebuilt its list of your files on this PC. Nothing was lost; your files are on the server.";

fn dismiss_rebuilt(s: &mut State, _: &mut Tx) {
    s.actions.protected.rebuilt = None;
}

/// The calm banner for a rebuilt local list, with Dismiss.
pub fn rebuilt_notice(s: &State, t: &Theme) -> Option<El> {
    let words = s.actions.protected.rebuilt.as_deref()?;
    let runs = [Run::Words(words.to_string())];
    Some(banner::banner(
        &Banner {
            tone: Tone::Neutral,
            icon: &icons::CHECK_CIRCLE,
            runs: &runs,
            action: None,
            dismiss: Some(dismiss_rebuilt as banner::Handler),
        },
        t,
    ))
}

/// The note over a folder a service manages (DW-0424).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderNotice {
    pub words: String,
    /// The button's word; it opens the service in Settings.
    pub action: String,
}

/// What the protected folders keep on `Actions`.
#[derive(Default)]
pub struct Ui {
    /// The open message (DW-0425, DW-1110).
    pub alert: Option<Words>,
    /// The section the window showed when this last looked ([`poll`]).
    seen: Option<Section>,
    /// The window's `session_no` and the folder the source was last told is shown ([`FilesSource::folder_shown`]).
    told: Option<(u64, Option<String>)>,
    /// The "local file list was rebuilt" words, taken from the source once and shown until Dismiss (the engine says
    /// it once per move, so it is gone after a restart too).
    pub rebuilt: Option<String>,
}

/// Contacts in a device's backup (DW-0423).
static CONTACTS: Icon = Icon {
    name: "contacts",
    view: 24.0,
    body: concat!(
        r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
        r#"<path d="M6.5 3h11A1.5 1.5 0 0 1 19 4.5v15a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 19.5v-15A1.5 1.5 0 0 1 6.5 3z"></path><path d="M12 8.2a2.3 2.3 0 1 1 0 4.6 2.3 2.3 0 0 1 0-4.6z"></path><path d="M8.5 17a3.6 3.6 0 0 1 7 0"></path>"#,
        "</g>"
    ),
};
/// The calendar in a device's backup (DW-0423).
static CALENDAR: Icon = Icon {
    name: "calendar",
    view: 24.0,
    body: concat!(
        r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
        r#"<path d="M5.5 5h13A1.5 1.5 0 0 1 20 6.5v12a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18.5v-12A1.5 1.5 0 0 1 5.5 5z"></path><path d="M4 10h16"></path><path d="M8 3v4"></path><path d="M16 3v4"></path>"#,
        "</g>"
    ),
};

impl BackupKind {
    fn icon(self) -> &'static Icon {
        match self {
            BackupKind::Phone => &icons_settings::PHONE,
            BackupKind::Tablet => &icons_settings::TABLET,
            BackupKind::Laptop => &icons_settings::LAPTOP,
            BackupKind::Mail => &icons_settings::MAIL,
            BackupKind::Contacts => &CONTACTS,
            BackupKind::Calendar => &CALENDAR,
            BackupKind::Folder => &icons::FILES,
        }
    }
}

/// 1820 as "1,820".
pub fn grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn count_words(n: i64) -> String {
    if n == 1 {
        "1 item".to_string()
    } else {
        format!("{} items", grouped(n))
    }
}

impl State {
    /// Is the folder shown one of the protected tree (Backups, or anything inside it)?
    pub fn in_protected_folder(&self) -> bool {
        self.list_error.is_none()
            && self
                .actions
                .source
                .protection(self.listing.folder.id.as_deref(), None)
                .is_some()
    }

    /// Why these items cannot change: the first reason found. `None`: they can.
    pub fn protection_of(&self, items: &[DriveItem]) -> Option<Protection> {
        let folder = self.listing.folder.id.as_deref();
        items
            .iter()
            .find_map(|item| self.actions.source.protection(folder, Some(item)))
    }

    /// May none of these be moved, renamed or deleted? The item's own flag (the Backups row in
    /// Files), or the source's word for it.
    pub fn protects(&self, items: &[DriveItem]) -> bool {
        items.iter().any(|item| item.is_protected) || self.protection_of(items).is_some()
    }

    /// The same, of the selection.
    pub fn protects_selection(&self) -> bool {
        self.protects(&self.selected_items())
    }

    /// In Backups one row is selected at a time: its folders "can't be selected for a bulk
    /// action" (DW-0422).
    pub fn selects_one_at_a_time(&self) -> bool {
        self.list_error.is_none()
            && self.actions.source.protection(self.listing.folder.id.as_deref(), None) == Some(Protection::BackupsRoot)
    }

    /// A change of these items was refused: say why, the way the board does for what it is.
    /// A service's folder: the message (DW-0425, DW-1110). A device's backup: moving it to Trash
    /// is Delete this backup, behind its code (DW-0742); anything else there says that it did
    /// not happen. No wake is asked for here: `Some` is how long the message that was set stays.
    pub(super) fn say_protected(&mut self, action: Action, items: &[DriveItem]) -> Option<std::time::Duration> {
        match self.protection_of(items) {
            Some(Protection::Managed(words)) => {
                self.actions.menu = None;
                self.actions.protected.alert = Some(words);
                None
            }
            Some(Protection::DeviceBackup(device)) if action == Action::Trash => {
                self.intent(Intent::Frame("DW-0742"));
                crate::shell::host::dispatch(self, Event::OpenSettings(SettingsPane::Backups));
                // The dialog is the Backups section's, for a device IT lists; this PC's own
                // backup is managed on the section itself.
                let listed = self
                    .backups
                    .source()
                    .is_some_and(|source| source.devices().iter().any(|d| d.id == device));
                if listed {
                    self.backups.ask_delete_device(&device);
                }
                None
            }
            Some(Protection::DeviceBackup(_) | Protection::BackupsRoot) => {
                Some(self.set_toast(Toast::Failed(action, ActionError::Protected)))
            }
            // The source has no words for it: nothing is shown here (the frame asked for is DW-1110's).
            None => None,
        }
    }

    /// OK, Esc, or a press beside the message: it closes and nothing changed.
    pub fn close_protected(&mut self, tx: &mut Tx) -> bool {
        let open = self.actions.protected.alert.take().is_some();
        if open {
            self.focus_list(tx);
        }
        open
    }

    /// Open Settings, on the message and on the note over a service's folder: where the service
    /// is turned on or off. The service's own section (DW-0740) is asked for; the window goes to
    /// Settings' Backups, where every backup is managed.
    pub fn open_protected_settings(&mut self) {
        self.actions.protected.alert = None;
        self.intent(Intent::Frame("DW-0740"));
        crate::shell::host::dispatch(self, Event::OpenSettings(SettingsPane::Backups));
    }
}

/// The message's keys: Esc. Nothing reaches the window under it. `None`: it is not open.
pub fn key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> Option<bool> {
    s.actions.protected.alert.as_ref()?;
    // Esc closes it. Every other key is the dialog's own (Enter and Space press the focused
    // button, Tab goes to the next): unhandled here, and never the window's under it.
    if ev.key == Key::Escape {
        s.close_protected(tx);
        return Some(true);
    }
    Some(false)
}

/// Is the listing a folder of the tree under `root`?
///
/// Anywhere in the path: the fixture's path starts at Backups, the real engine's starts at the Files root.
fn inside(s: &State, root: &str) -> bool {
    s.listing.path.iter().any(|crumb| crumb.id.as_deref() == Some(root))
}

/// Show Backups: the section, on its own folder. What a scene and the section's entrances call.
pub fn show_backups(s: &mut State) -> bool {
    let Some(root) = s.actions.source.backups_root() else {
        return false;
    };
    crate::shell::host::dispatch(s, Event::Navigate(Route::section(Section::Backups)));
    // The panel follows the folder; Backups starts without it.
    s.show_detail_pane = false;
    if !inside(s, &root) {
        s.open_folder(Some(root));
    }
    s.actions.protected.seen = Some(Section::Backups);
    true
}

/// Keeps the section and the folder shown together, every frame (`shell::host::poll`): the
/// sidebar's Backups (Ctrl+4) shows the Backups folder, opening the Backups row in Files moves
/// the sidebar to Backups, and Back out of it moves it to Files again.
pub fn poll(s: &mut State) -> bool {
    // The engine says a rebuilt local list once; kept here until the user dismisses it.
    if s.actions.protected.rebuilt.is_none() && s.actions.source.take_rebuilt_notice().is_some() {
        s.actions.protected.rebuilt = Some(REBUILT_WORDS.to_string());
        return true;
    }
    // The source is told which folder is shown, once per folder and per session, before anything else: every way a
    // folder comes to be shown (a click, Back, a breadcrumb, the sidebar, a reset) ends here.
    let shown = (s.actions.session_no, s.listing.folder.id.clone());
    if s.list_error.is_none() && s.actions.protected.told.as_ref() != Some(&shown) {
        s.actions.source.folder_shown(shown.1.as_deref());
        s.actions.protected.told = Some(shown);
    }
    // What a source that answers later did since the last frame (`State::late_changes`).
    let late = s.late_changes();
    // A preview that is still being read (a source that answers it later).
    let previewing = super::preview::poll(s);
    if super::session::poll(s) || late || previewing {
        return true;
    }
    let Some(root) = s.actions.source.backups_root() else {
        // On the sidebar's Backups with no folder known: the source looks again (it may throttle itself).
        if matches!(s.shell.app.screen(), Screen::Drive(r) if r.section == Section::Backups) {
            s.actions.source.look_for_backups();
        }
        return false;
    };
    // No folder is read while the session ended (`session.rs`): the sidebar's Backups is refused
    // like every other ask, and the window stays on the list it has.
    if s.session_ended() {
        let asks_backups = matches!(s.shell.app.screen(), Screen::Drive(r) if r.section == Section::Backups);
        if asks_backups && !inside(s, &root) && s.refused_without_session() {
            crate::shell::host::dispatch(s, Event::Navigate(Route::section(Section::Files)));
            return true;
        }
        return false;
    }
    let Screen::Drive(route) = s.shell.app.screen() else {
        return false;
    };
    let section = route.section;
    let seen = s.actions.protected.seen.replace(section);
    let within = inside(s, &root);
    // A folder that could not be read shows its reason where it was asked for: the section does
    // not follow it. A change of section still shows that section's folder.
    let unread = s.list_error.is_some();
    if unread && seen == Some(section) {
        return false;
    }
    let (target, load) = match (section, within, seen == Some(section)) {
        // Away from a folder that could not be read: the section's own folder, read again.
        // (Not on the first poll: nothing was left yet, and the folder was just asked for.)
        (Section::Backups, _, false) if unread && seen.is_some() => (Section::Backups, Some(Some(root))),
        (Section::Files, _, false) if unread && seen.is_some() => (Section::Files, Some(None)),
        // The section changed (the sidebar, a shortcut, the shell's back): the folder follows.
        (Section::Backups, false, false) => (Section::Backups, Some(Some(root))),
        (Section::Files, true, false) => (Section::Files, Some(None)),
        // The folder changed (the Backups row was opened, Back left the tree): the section follows.
        (Section::Files, true, true) => (Section::Backups, None),
        (Section::Backups, false, true) => (Section::Files, None),
        _ => return false,
    };
    match load {
        Some(folder) => s.open_folder(folder),
        None => {
            crate::shell::host::dispatch(s, Event::Navigate(Route::section(target)));
        }
    }
    s.actions.protected.seen = Some(target);
    true
}

fn section_view(s: &State, cx: &mut Cx<State>) -> El {
    if s.actions.source.backups_root().is_some() {
        crate::shell::files_frame::view(s, cx)
    } else {
        // The source knows no Backups folder (this account has none yet, or it has not been read): an honest empty
        // state in the shell's own window, never the dev card and never a sample.
        let t = crate::theme::tokens::theme(cx.system_dark());
        crate::shell::placeholder::around(s, cx, no_backups(&t))
    }
}

/// No Backups folder: an honest empty state (design note: the board has no frame for an account without a Backups
/// folder; ruled by the lead 2026-11-15), with the way to a backup.
fn no_backups(t: &Theme) -> El {
    let action = button::button(
        button::keys("backups-none-link"),
        "Open Settings \u{203A} Backups",
        None,
        Variant::Secondary,
        Size::Md,
        ButtonState::Enabled,
        t,
    )
    .on_press(|s: &mut State| s.open_protected_settings());
    view_state::view_state(
        ViewState::Empty {
            key: "backups-none",
            icon: &icons::BACKUPS,
            title: "No backups yet",
            body: "Turn on backups in Settings to see your devices here.",
            action: Some(action),
        },
        t,
    )
}

fn section_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    s.actions.source.backups_root().is_some() && crate::shell::files_frame::on_key(s, ev, tx)
}

/// The registry's line for the sidebar's Backups (`shell/registry.rs`): the Files window on the
/// Backups folder.
pub const SECTION: Frame = Frame {
    group: FrameGroup::Section(Section::Backups),
    view: section_view,
    on_key: Some(section_key),
    feedback: FeedbackPolicy::Denied("it lists your devices and their backups"),
};

// ---- what is drawn ----

const COL_DATE_W: f32 = 132.0;
const COL_SIZE_W: f32 = 84.0;
const COL_SHARED_W: f32 = 92.0;
/// `.mk-cols` of Backups: Status is wider than Shared.
const COL_STATUS_W: f32 = 150.0;
const COL_PC_W: f32 = 34.0;
/// The Status column's words (`.mk-row .t-cap`).
const STATUS: TextStyle = crate::theme::scale::text::NOTE;
/// The words beside a name (`.mk-row .t-cap` in the name cell).
const NOTE: TextStyle = TextStyle::new(11.0, 400.0).lh(13.0);

fn is_root(s: &State) -> bool {
    s.selects_one_at_a_time()
}

/// The words of the status line in the tree: what is on this PC and what is not (DW-0422).
/// `None`: not a protected folder; the Files window counts its own way.
pub fn status_words(s: &State) -> Option<String> {
    if !s.in_protected_folder() || s.listing.items.is_empty() {
        return None;
    }
    let online = s
        .listing
        .items
        .iter()
        .filter(|i| i.pc_status.as_deref() == Some("online_only"))
        .count();
    Some(format!(
        "{} on this PC \u{00B7} {} online only",
        s.listing.items.len() - online,
        online
    ))
}

/// The column headers of a protected folder: Backups says Last backup and Status; the folders
/// in it keep Files' words. The list stays in the order its owner gives it, so no header sorts.
/// `None`: not a protected folder.
pub fn column_headers(s: &State, t: &Theme) -> Option<El> {
    if !s.in_protected_folder() {
        return None;
    }
    let (date, shared, shared_w) = if is_root(s) {
        ("Last backup", "Status", COL_STATUS_W)
    } else {
        ("Date changed", "Shared", COL_SHARED_W)
    };
    let columns = [
        Column::new("name", "Name", Width::Flex(1.0)),
        Column::new("date", date, Width::Fixed(COL_DATE_W)),
        Column::new("size", "Size", Width::Fixed(COL_SIZE_W)).align(Align::End),
        Column::new("shared", shared, Width::Fixed(shared_w)),
        Column::new("pc", "PC", Width::Fixed(COL_PC_W))
            .align(Align::Center)
            .spoken("On this PC"),
    ];
    Some(file_row::header(&columns, Some((0, true)), t))
}

/// One row of a protected folder (`.mk-row`): a device, a service's folder, or what is in one.
/// `None`: not a protected folder; the list draws its own row.
pub fn row_of(s: &State, i: usize, item: &DriveItem, t: &Theme) -> Option<El> {
    if !s.in_protected_folder() {
        return None;
    }
    let root = is_root(s);
    let entry = s.actions.source.backup_entry(item);
    let selected = s.is_selected(i);
    let k = format!("row-{i}");
    let date = format_date(&item.modified_at);
    let meta = if selected { t.text_secondary } else { t.text_tertiary };

    let tile = file_row::tile_frame(&k)
        .fill(t.tile)
        .child(match entry.as_ref().map(|e| e.kind) {
            None | Some(BackupKind::Folder) => crate::ui::file_icon::file_icon(
                format!("{k}-icon"),
                crate::theme::file_icons::icon_for_item(item),
                14.0,
                t,
            ),
            Some(kind) => kit::glyph(format!("{k}-icon"), kind.icon(), 14.0, t.text_secondary),
        });
    let mut marks = Vec::new();
    let managed = entry.as_ref().is_some_and(|e| e.managed);
    if managed {
        marks.push(
            kit::glyph(format!("{k}-lock"), &icons::LOCK, 11.0, t.text_tertiary)
                .ml(2.0)
                .role("image")
                .label("Managed by Beebeeb"),
        );
    }

    // A backup says how much it holds; a folder in one, how many items.
    let size = match (item.size, item.child_count) {
        (Some(bytes), _) if item.is_folder => crate::frames::global::source::size_words(bytes.max(0) as u64),
        (_, Some(count)) => count_words(count),
        (Some(bytes), None) => crate::source::format_size(bytes),
        (None, None) => String::new(),
    };
    let note = entry.as_ref().and_then(|e| e.note.clone());
    if let Some(note) = &note {
        marks.push(kit::label(format!("{k}-note"), note.clone(), NOTE, t.text_tertiary));
    }
    let status = entry.as_ref().and_then(|e| e.status.clone());
    let pc_state = PcState::of(item.pc_status.as_deref());

    let mut said = vec![item.name.clone()];
    said.push(format!("{} {date}", if root { "last backup" } else { "changed" }));
    if !size.is_empty() {
        said.push(size.clone());
    }
    said.extend(note);
    said.extend(status.clone());
    said.extend(pc_state.map(|state| {
        match state {
            PcState::OnlineOnly => "online only",
            PcState::Downloaded => "on this PC",
            PcState::AvailableOffline => "available offline",
        }
        .to_string()
    }));

    let mut pc = el().key("pc-cell").w(COL_PC_W).center();
    if let Some(state) = pc_state {
        pc = pc.child(pc_glyph(format!("{k}-pc"), state, t.bg, t));
    }
    let last = if root {
        let cell = row().key("status-cell").w(COL_STATUS_W).min_w(0.0).center_y();
        match status {
            Some(words) => cell.child(kit::label(format!("{k}-dash"), words, STATUS, meta).truncate()),
            None => cell,
        }
    } else {
        row()
            .key("shared-cell")
            .w(COL_SHARED_W)
            .min_w(0.0)
            .center_y()
            .child(kit::label(format!("{k}-dash"), "\u{2014}", type_::META_MONO, meta))
    };
    let cells = vec![
        kit::label(format!("{k}-date"), date, type_::META_MONO, meta)
            .w(COL_DATE_W)
            .truncate(),
        kit::label(format!("{k}-size"), size, type_::META_MONO, meta)
            .w(COL_SIZE_W)
            .align(Align::End),
        last,
        pc,
    ];
    let look = RowLook {
        selected,
        ..RowLook::default()
    };
    Some(file_row::row_with(
        &k,
        &said.join(", "),
        tile,
        &item.name,
        marks,
        cells,
        look,
        t,
    ))
}

fn open_settings(s: &mut State, _: &mut Tx) {
    s.open_protected_settings();
}

/// The note over a folder a service manages (DW-0424), where the window's one banner goes.
pub fn notice(s: &State, t: &Theme) -> Option<El> {
    if s.list_error.is_some() {
        return None;
    }
    let notice = s.actions.source.folder_notice(s.listing.folder.id.as_deref())?;
    let runs = [Run::Words(notice.words.clone())];
    Some(banner::banner(
        &Banner {
            tone: Tone::Neutral,
            icon: &icons::LOCK,
            runs: &runs,
            action: Some((notice.action.as_str(), open_settings as banner::Handler)),
            dismiss: None,
        },
        t,
    ))
}

/// The message, over the window under its caption (`.wn-dlg-alert` on `.mk-scrim`): a lock, the
/// source's title and sentence, OK and Open Settings side by side.
pub fn layer(s: &State, t: &Theme) -> Option<El> {
    let words = s.actions.protected.alert.as_ref()?;
    let tile = backups_kit::tile(0, TileTone::Neutral, &icons::LOCK, 36.0, t);
    let ok = settings_dialog::button(0, "OK", Weight::Primary, t).on_press_tx(|s: &mut State, tx: &mut Tx| {
        s.close_protected(tx);
    });
    let open_settings = backups_kit::button_md(1, "Open Settings", None, Md::Plain, t)
        .on_press(|s: &mut State| s.open_protected_settings());
    let card = dialog::dialog(
        dialog::Dialog {
            alert: true,
            head: dialog::Head::Alert {
                tile,
                title: &words.title,
                sub: &words.body,
            },
            foot: dialog::Foot::Row(vec![ok.w(161.0), open_settings.w(163.0)]),
            enter: Some(0),
            ..dialog::Dialog::new("dlg", &words.title, 380.0)
        },
        t,
    )
    .description(words.body.clone());
    Some(
        el().key("dialog-protected")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .child(dialog::scrim(32.0, card, t))
            .on_key(super::keys::on_key)
            .modal_tx(false, |s: &mut State, tx: &mut Tx| {
                s.close_protected(tx);
            }),
    )
}

// ---- the board frames ----

fn open_child(s: &mut State, id: &str) {
    show_backups(s);
    s.open_folder(Some(id.to_string()));
}

/// The board frames of the protected folders, by frame id.
pub const SCENES: &[crate::ui::gate::Scene<State>] = &[
    // DW-0422: every device's backup and every service, the sidebar on Backups.
    ("DW-0422", |s| {
        show_backups(s);
    }),
    // DW-0423: what this PC backed up.
    ("DW-0423", |s| open_child(s, super::protected_fixture::LAPTOP_ID)),
    // DW-0424: the folder the mail service manages, with its note.
    ("DW-0424", |s| open_child(s, super::protected_fixture::MAIL_ID)),
    // DW-0425: 2025 was to be deleted there: the message.
    ("DW-0425", |s| {
        open_child(s, super::protected_fixture::MAIL_ID);
        s.select_only(1);
        if let Some(Protection::Managed(words)) = s.protection_of(&s.selected_items()) {
            s.actions.protected.alert = Some(words);
        }
    }),
    // DW-1110: the service's folder itself was to be deleted in Backups: the message.
    ("DW-1110", |s| {
        show_backups(s);
        s.select_only(4);
        if let Some(Protection::Managed(words)) = s.protection_of(&s.selected_items()) {
            s.actions.protected.alert = Some(words);
        }
    }),
];

#[cfg(test)]
#[path = "folder_told_tests.rs"]
mod folder_told_tests;
#[cfg(test)]
#[path = "preview_panel_tests.rs"]
mod preview_panel_tests;
