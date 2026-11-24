//! Details (DW-0411) and a read-only Preview (DW-0408's panel) of an item that is not in Files' list: a row of
//! Trash, a file shared with the person (Shared, With me), a result of Search. What the shell's poll does with the
//! frames' two intents `About("DW-0411", id)` and `Do("preview", id)` (`shell::intents::drain_frame_intents`).
//!
//! - **Where the id is looked up (R1).** In the section the window shows, in ITS current list, when the shell takes
//!   the intent: Trash's listing (the server's rows, for the session in memory), Shared's With me, Search's results.
//!   An id that is not there opens nothing.
//! - **What is shown.** Details: the row's own facts, nothing read from this PC for Trash. Preview: the panel in its
//!   read-only mode (no Share, no Open, no Make available offline). A file in Trash is read from Beebeeb only, in
//!   memory, never written to disk (`ReadFrom::Beebeeb`); nothing in Trash reaches the viewer.
//! - **How long (R2, R7).** The dialog closes, and the read stops with what it decrypted wiped, when the section is
//!   left, when the session ends or another account comes (`end_session`, from the shell's session-end block), and
//!   when a list that is ready no longer has the row ([`poll`]). The page is held in the dialog's fields only, and
//!   wiped with them.
//!
//! Must not: put an id, a name or typed words in a log line ([`tag`] is built from the intent's fixed words).

pub mod scenes;
pub mod source;
#[cfg(test)]
pub(crate) mod tests;

use std::time::Duration;

use rune::Tx;

use crate::State;
use crate::frames::Intent;
use crate::frames::files::actions_data::Previewed;
use crate::frames::shared::dialogs::{Dialog, ItemStamp, PreviewHeld};
use crate::frames::shared::parts::table::{Extra, Kind, Row};
use crate::shell::app::Section;
use crate::shell::registry::{FrameGroup, group_for};
use crate::source::DriveItem;
use crate::ui::file_list::PcState;
use source::{PreviewAsk, ReadFrom};

/// How often the poll asks again while a preview is on its way.
const ASK_EVERY: Duration = Duration::from_millis(250);

/// The section an item dialog was opened over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Trash,
    Shared,
    /// Search's results, drawn over the Files window.
    Search,
}

impl Origin {
    /// The section whose window the dialog is drawn over.
    pub fn section(self) -> Section {
        match self {
            Origin::Trash => Section::Trash,
            Origin::Shared => Section::Shared,
            Origin::Search => Section::Files,
        }
    }

    /// What the read-only panel says after why it shows no page.
    pub fn tail(self) -> &'static str {
        match self {
            Origin::Trash => " Restore it to open it.",
            // Shared's Open is not taken up yet: no words point at it.
            Origin::Shared | Origin::Search => "",
        }
    }
}

/// The section the window shows, when an item dialog can stand over it.
pub fn origin_shown(s: &State) -> Option<Origin> {
    match group_for(&s.shell.app) {
        FrameGroup::Section(Section::Trash) => Some(Origin::Trash),
        FrameGroup::Section(Section::Shared) => Some(Origin::Shared),
        FrameGroup::Section(Section::Files) if crate::frames::search::shows_over_files(s) => Some(Origin::Search),
        _ => None,
    }
}

/// One line of Details: a figure (JetBrains Mono) or words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fact {
    Figure(String),
    Words(String),
}

/// What an item dialog draws of its row, read from the origin's current list.
#[derive(Clone, Debug)]
pub struct Found {
    /// For the tile and the panel only: built from the row, never read from this PC.
    pub item: DriveItem,
    /// "PDF document"
    pub kind: String,
    /// The row's size in words ("1.1 MB"; empty for a folder whose size is not known).
    pub size: String,
    /// Details' lines, in order. A line with nothing to say is left out.
    pub facts: Vec<(&'static str, Fact)>,
}

/// Every item is encrypted end to end: there is no other kind of file in Beebeeb.
const ENCRYPTED: &str = "Yes, end to end";

/// A display item for a row: its name, whether it is a folder, the kind its name says. Nothing else is known here.
pub(crate) fn display_item(id: &str, name: &str, is_folder: bool, pc: Option<PcState>) -> DriveItem {
    DriveItem {
        id: id.to_string(),
        parent_id: None,
        name: name.to_string(),
        is_folder,
        size: None,
        child_count: None,
        modified_at: String::new(),
        mime: if is_folder {
            None
        } else {
            beebeeb_engine::drive_v2::model::mime_for_name(name).map(str::to_string)
        },
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: pc.map(|pc| {
            match pc {
                PcState::OnlineOnly => "online_only",
                PcState::Downloaded => "on_this_pc",
                PcState::AvailableOffline => "available_offline",
            }
            .to_string()
        }),
        is_protected: false,
    }
}

/// "Yes" or "No": is the item on this PC. `None`: not known, and the line is left out.
fn on_this_pc(pc: Option<PcState>) -> Option<Fact> {
    pc.map(|pc| {
        Fact::Words(match pc {
            PcState::Downloaded | PcState::AvailableOffline => "Yes".into(),
            PcState::OnlineOnly => "No".into(),
        })
    })
}

/// Push `(title, fact)` unless the fact is empty.
fn add(facts: &mut Vec<(&'static str, Fact)>, title: &'static str, fact: Fact) {
    let empty = match &fact {
        Fact::Figure(words) | Fact::Words(words) => words.trim().is_empty(),
    };
    if !empty {
        facts.push((title, fact));
    }
}

fn found_in_row(row: &Row, origin: Origin) -> Found {
    let is_folder = row.kind == Kind::Folder;
    let pc = if origin == Origin::Shared { row.pc } else { None };
    let item = display_item(&row.id, &row.name, is_folder, pc);
    let kind = crate::frames::files::source::kind_of(&item).to_string();
    let mut facts = Vec::new();
    add(&mut facts, "Size", Fact::Figure(row.size.clone()));
    match origin {
        // The listing's only time is when the item went to Trash (its column says "Deleted").
        Origin::Trash => add(&mut facts, "Deleted", Fact::Figure(row.date.clone())),
        _ => {
            add(&mut facts, "Shared on", Fact::Figure(row.date.clone()));
            let from = match &row.extra {
                Extra::Person { name, .. } => name.clone(),
                Extra::Words(words) => words.clone(),
            };
            add(&mut facts, "From", Fact::Words(from));
            if let Some(fact) = on_this_pc(row.pc) {
                facts.push(("On this PC", fact));
            }
        }
    }
    facts.push(("Encrypted", Fact::Words(ENCRYPTED.into())));
    Found {
        item,
        kind,
        size: row.size.clone(),
        facts,
    }
}

/// The row of `id` in `origin`'s CURRENT list (R1). `None`: not there (or Trash cannot act on its list now).
pub fn found(s: &State, origin: Origin, id: &str) -> Option<Found> {
    match origin {
        Origin::Trash => {
            let trash = &s.frames.trash.source;
            if !trash.actions_available() {
                return None;
            }
            trash
                .items()
                .iter()
                .find(|row| row.id == id)
                .map(|row| found_in_row(row, origin))
        }
        Origin::Shared => s
            .frames
            .shared
            .source
            .with_me()
            .iter()
            .find(|row| row.id == id)
            .map(|row| found_in_row(row, origin)),
        Origin::Search => {
            let hit = crate::frames::search::hit(s, id)?;
            let is_folder = hit.kind == Kind::Folder;
            let item = display_item(&hit.id, &hit.name, is_folder, hit.pc);
            let kind = s
                .frames
                .find
                .source
                .details(id)
                .map(|details| details.kind)
                .filter(|kind| !kind.is_empty())
                .unwrap_or_else(|| crate::frames::files::source::kind_of(&item).to_string());
            let mut facts = Vec::new();
            add(&mut facts, "Size", Fact::Figure(hit.size.clone()));
            add(&mut facts, "Changed", Fact::Figure(hit.changed.clone()));
            add(&mut facts, "Where", Fact::Words(hit.place.clone()));
            if let Some(fact) = on_this_pc(hit.pc) {
                facts.push(("On this PC", fact));
            }
            facts.push(("Encrypted", Fact::Words(ENCRYPTED.into())));
            Some(Found {
                item,
                kind,
                size: hit.size,
                facts,
            })
        }
    }
}

/// The row of `id` left a list that is READY (R2(c)): a list still on its way never closes a dialog.
fn gone(s: &State, origin: Origin, id: &str) -> bool {
    match origin {
        Origin::Trash => {
            let trash = &s.frames.trash.source;
            trash.loaded() == crate::frames::trash::source::Loaded::Ready
                && !trash.items().iter().any(|row| row.id == id)
        }
        Origin::Shared => {
            let shared = &s.frames.shared.source;
            shared.loaded() == crate::frames::shared::source::Loaded::Ready
                && !shared.with_me().iter().any(|row| row.id == id)
        }
        Origin::Search => crate::frames::search::hit_gone(s, id),
    }
}

/// The gate and the account the window is under now.
fn stamp(s: &State) -> ItemStamp {
    ItemStamp(s.shell.app.gate(), s.shell.app.account().map(str::to_owned))
}

/// The intent's fixed words, for a log line: never its id, a name or typed words.
pub fn tag(intent: &Intent) -> String {
    match intent {
        Intent::Frame(frame) => format!("Frame {frame}"),
        Intent::About(frame, _) => format!("About {frame}"),
        Intent::OpenItem(_) => "OpenItem".to_string(),
        Intent::Do(what, _) => format!("Do {what}"),
    }
}

/// Takes the intents that open over the section shown: Details (`About("DW-0411", id)`) and Preview
/// (`Do("preview", id)`). `false`: not this module's, or the id has no row in the section shown (the router logs
/// it as `NotWired`).
pub fn take(s: &mut State, tx: &mut Tx, intent: &Intent) -> bool {
    match intent {
        Intent::About("DW-0411", id) => open_details(s, id, tx),
        Intent::Do("preview", id) => open_preview(s, id, tx),
        _ => false,
    }
}

/// Details of the item `id` over the section shown (`About("DW-0411", id)`). `false`: its row is not in the
/// section's current list, and nothing opened.
pub fn open_details(s: &mut State, id: &str, tx: &mut Tx) -> bool {
    let Some(origin) = origin_shown(s) else {
        return false;
    };
    if found(s, origin, id).is_none() {
        return false;
    }
    s.frames.item_preview.cancel();
    s.frames.shared.open_dialog(Dialog::Details {
        origin,
        id: id.to_string(),
    });
    s.frames.shared.fields.item_stamp = Some(stamp(s));
    // Enter presses Done; Esc closes.
    tx.focus("btn-0", true);
    true
}

/// The read-only Preview of the item `id` over Trash or Shared (`Do("preview", id)`). A folder's shows that there
/// is no preview of it, and nothing is read. `false`: its row is not in the section's current list (or the section
/// has no preview), and nothing opened.
pub fn open_preview(s: &mut State, id: &str, tx: &mut Tx) -> bool {
    let Some(origin) = origin_shown(s).filter(|origin| *origin != Origin::Search) else {
        return false;
    };
    let Some(found) = found(s, origin, id) else {
        return false;
    };
    let held = if found.item.is_folder {
        s.frames.item_preview.cancel();
        Previewed::NoPreview
    } else {
        let from = match origin {
            Origin::Trash => ReadFrom::Beebeeb,
            _ => ReadFrom::ThisPcFirst,
        };
        s.frames.item_preview.start(&PreviewAsk {
            id: id.to_string(),
            name: found.item.name.clone(),
            from,
        });
        s.frames.item_preview.later(id)
    };
    s.frames.shared.open_dialog(Dialog::Preview {
        origin,
        id: id.to_string(),
    });
    s.frames.shared.fields.preview = PreviewHeld::of(held);
    s.frames.shared.fields.item_stamp = Some(stamp(s));
    // The next Space lands in the panel and closes it.
    tx.focus("pv-close", true);
    true
}

/// The item dialog open now: its origin, its id, and whether it is the Preview.
fn open_item(s: &State) -> Option<(Origin, String, bool)> {
    match &s.frames.shared.dialog {
        Some(Dialog::Details { origin, id }) => Some((*origin, id.clone(), false)),
        Some(Dialog::Preview { origin, id }) => Some((*origin, id.clone(), true)),
        _ => None,
    }
}

/// Close the item dialog (its fields, the page among them, are wiped) and stop its read.
fn close(s: &mut State) {
    if open_item(s).is_some() {
        s.frames.shared.close_dialog();
    }
    s.frames.item_preview.cancel();
}

/// A lock, a sign-out or another account (the shell's session-end block): the item dialog closes, the page goes,
/// the read stops and what it decrypted is wiped.
pub fn end_session(s: &mut State) {
    close(s);
}

/// Every frame (`shell::host::poll`). For an open item dialog: closes it, and stops its read, when its section is
/// no longer shown, when the gate or the account moved, or when a ready list no longer has its row (R2); while a
/// Preview waits, asks where it stands (R3). With no Preview open, the read is stopped (R4): Esc, Done, Space and
/// every other way a dialog closes come by here. `true`: something drawn changed.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let Some((origin, id, is_preview)) = open_item(s) else {
        s.frames.item_preview.cancel();
        return false;
    };
    let left = origin_shown(s) != Some(origin);
    let moved = s.frames.shared.fields.item_stamp.as_ref() != Some(&stamp(s));
    if left || moved || gone(s, origin, &id) {
        close(s);
        return true;
    }
    if !is_preview {
        s.frames.item_preview.cancel();
        return false;
    }
    if !s.frames.shared.fields.preview.is_loading() {
        return false;
    }
    let now = s.frames.item_preview.later(&id);
    if now == Previewed::Loading {
        tx.wake_after(ASK_EVERY);
        return false;
    }
    s.frames.shared.fields.preview = PreviewHeld::of(now);
    true
}

/// Whether the open dialog has what it draws: an item dialog's row must still be in its origin's list (a row that
/// went while the list re-reads is not drawn, and the dialog must not keep the keys then).
pub fn drawn(s: &State) -> bool {
    match open_item(s) {
        Some((origin, id, _)) => found(s, origin, &id).is_some(),
        None => true,
    }
}

/// Focus back on the row the dialog was about, when the section has one selected.
pub fn return_focus(s: &State, origin: Origin, tx: &mut Tx) {
    let key = match origin {
        Origin::Trash => s.frames.trash.selected.map(|i| format!("row-{i}")),
        Origin::Shared => s.frames.shared.selected_key(),
        Origin::Search => crate::frames::search::selected_row_key(s),
    };
    if let Some(key) = key {
        tx.return_focus(key, tx.focus_visible());
    }
}
