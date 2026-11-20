//! Shared's Open, Show in File Explorer, a link's People, Leave, and the answers of the Stop sharing / Close request
//! confirmations (w17-shared-items).
//!
//! Open: a folder opens in Files, a file as Files opens it (the viewer for a type it has one for, else the file's own
//! app), like This PC's kept items. A link row opens its item (never the share id); View files on a request opens its
//! target folder, or says it has none. Show in File Explorer asks the Files source (a shared item that is not kept on
//! this PC says so). A link's People is the item's People dialog when it also has a share with people, else Invite
//! someone… for that item. Leave, Stop sharing and Close request were asked first (`dialogs::confirm`); only the
//! answer of the confirmation that is open for that id makes the engine call.

use rune::Tx;

use super::dialogs::Dialog;
use super::source::{Notice, RowTarget};
use crate::State;
use crate::frames::Intent;
use crate::frames::shared::Tab;
use crate::shell::app::{Event, Route, Section};
use crate::source::DriveItem;

/// Takes `intent` when it is one of these; `false`: not taken.
pub fn take(s: &mut State, tx: &mut Tx, intent: &Intent) -> bool {
    match intent {
        Intent::OpenItem(id) => open(s, id),
        Intent::About("DW-1180", id) => show_in_file_explorer(s, id),
        Intent::About("DW-0511", id) => people_of_link(s, id),
        Intent::Do(tag @ ("confirm-leave" | "confirm-stop" | "confirm-close"), id) => answer(s, tx, tag, id),
        _ => return false,
    }
    true
}

/// The name a row of Shared carries, for the viewer's type check.
fn name_of(s: &State, id: &str) -> String {
    let source = &s.frames.shared.source;
    source
        .with_me()
        .into_iter()
        .find(|r| r.id == id)
        .map(|r| r.name)
        .or_else(|| source.links().into_iter().find(|l| l.id == id).map(|l| l.name))
        .or_else(|| source.people_shares().into_iter().find(|p| p.id == id).map(|p| p.name))
        .unwrap_or_default()
}

fn say(s: &mut State, notice: Notice) {
    s.frames.shared.notice = Some(notice);
}

fn show_in_files(s: &mut State, folder: String) {
    let route = Route {
        section: Section::Files,
        folder: Some(folder),
        item: None,
    };
    crate::shell::host::dispatch(s, Event::Navigate(route));
}

fn open(s: &mut State, id: &str) {
    let name = name_of(s, id);
    match s.frames.shared.source.target_of(id) {
        Some(RowTarget::Folder(folder)) | Some(RowTarget::Item { id: folder, is_folder: true }) => {
            show_in_files(s, folder);
        }
        Some(RowTarget::Item { id: item, .. }) => {
            let item = drive_item(&item, &name, false);
            if !crate::frames::viewers::open_item(s, &item) {
                say(s, Notice::NotOnThisPc);
            }
        }
        None if s.frames.shared.tab == Tab::Requests => say(s, Notice::NoFolder),
        None => say(s, Notice::NotAvailable),
    }
}

fn show_in_file_explorer(s: &mut State, id: &str) {
    let name = name_of(s, id);
    let item = match s.frames.shared.source.target_of(id) {
        Some(RowTarget::Item { id, is_folder }) => drive_item(&id, &name, is_folder),
        Some(RowTarget::Folder(id)) => drive_item(&id, "", true),
        None => return say(s, Notice::NotAvailable),
    };
    if s.actions.source.show_in_file_explorer(&[item]).is_err() {
        say(s, Notice::NotOnThisPc);
    }
}

/// A link row's People: the item's own People when it has a share with people, else Invite someone… for that item.
fn people_of_link(s: &mut State, id: &str) {
    let Some(RowTarget::Item { id: item, .. }) = s.frames.shared.source.target_of(id) else {
        return say(s, Notice::NotAvailable);
    };
    if !s.frames.shared.show_people(&item) {
        let name = name_of(s, id);
        s.frames.shared.start_invite_named(&item, &name);
    }
}

/// The answer of a confirmation: only the one open for this id acts, once.
fn answer(s: &mut State, tx: &mut Tx, tag: &str, id: &str) {
    let ui = &mut s.frames.shared;
    let asked = match (&ui.dialog, tag) {
        (Some(Dialog::ConfirmLeave { id: open }), "confirm-leave")
        | (Some(Dialog::ConfirmStop { id: open }), "confirm-stop")
        | (Some(Dialog::ConfirmClose { id: open }), "confirm-close") => open == id,
        _ => false,
    };
    if !asked {
        return;
    }
    ui.close_dialog();
    let gone = match tag {
        "confirm-leave" => ui.source.leave(id),
        "confirm-stop" => ui.source.stop_sharing(id),
        _ => {
            ui.source.close_request(id);
            false
        }
    };
    if gone {
        let left = ui.count();
        ui.selected = ui.selected.filter(|_| left > 0).map(|i| i.min(left - 1));
    }
    if let Some(key) = ui.selected_key() {
        tx.return_focus(key, tx.focus_visible());
    }
}

/// A row as Files' actions take it: they find the item by its id.
fn drive_item(id: &str, name: &str, is_folder: bool) -> DriveItem {
    DriveItem {
        id: id.to_string(),
        parent_id: None,
        name: name.to_string(),
        is_folder,
        size: None,
        child_count: None,
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    }
}

#[cfg(test)]
#[path = "actions_items_tests.rs"]
mod tests;
