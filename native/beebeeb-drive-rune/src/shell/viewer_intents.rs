//! What the open viewer asked of the shell (`frames::viewers::Intent`), taken by the poll. SaveCopy and DownloadTo go
//! to `viewer_save`. Every press here acts on the OPEN file only; the Files window's own selection is not read.
//! Office(..) is taken while an editor runs on the open document (a refusal, said as one) and NewDocument opens the
//! New document dialog in the folder shown; EditInSheets, OpenInAnotherApp and CreateFile are not this file's (`false`).

use rune::Tx;

use crate::State;
use crate::frames::files::source::Action;
use crate::frames::files::{MenuKind, Toast};
use crate::frames::viewers::source::Origin;
use crate::frames::viewers::{self, Intent};
use crate::source::DriveItem;

#[cfg(test)]
#[path = "viewer_intents_tests.rs"]
mod tests;

/// Takes `intent`; `false`: nothing takes it yet (the router then shows "Not built yet: <what>" where it has a name).
pub fn take(s: &mut State, tx: &mut Tx, intent: Intent) -> bool {
    match intent {
        Intent::SaveCopy | Intent::DownloadTo => return super::viewer_save::take(s, tx, intent),
        Intent::Office(_) => return office_command(s),
        Intent::NewDocument(kind) => {
            let folder = s.listing.folder.id.clone();
            crate::frames::office::edit::new_document::open(s, tx, Some(kind), folder);
            return true;
        }
        Intent::EditInSheets => return false,
        // Not this file's: a dialog of the viewer's own, or a thing the viewer's source already did.
        Intent::OpenInAnotherApp | Intent::CreateFile { .. } => return false,
        _ => {}
    }
    let Some(item) = open_item(s) else {
        return false;
    };
    match intent {
        Intent::Share => share(s, item),
        Intent::VersionHistory => {
            if !s.begin_versions_for(item) {
                say(s, "Version history is for files only".into());
            }
        }
        Intent::Trash => trash(s, tx, item),
        Intent::Edit => {
            let why = why_not_edit(s, &item);
            say(s, why.into());
        }
        Intent::ReportProblem => crate::frames::settings::go(s, "DW-0782"),
        Intent::FreeUpSpace => free_up_space(s, item),
        Intent::KeepOffline => keep_offline(s, item),
        Intent::OpenInDrive(path) => open_in_drive(s, tx, &path),
        Intent::OpenInBrowser(address) => open_in_browser(s, &address),
        Intent::NewMenu => new_menu(s),
        _ => return false,
    }
    true
}

/// What a command says when the running editor did not take it.
pub const NOT_TAKEN: &str = "That change was not taken. Try again.";

/// A command the office editor's controls sent here because its source did not take it. With an editor running on
/// the open document that is a refusal, said as one; a stopped editor says that it stopped. `false`: no editor runs
/// (the board's sample document): the router's "Not built yet".
fn office_command(s: &mut State) -> bool {
    let Some(ui) = viewers::ui(s) else { return false };
    let Some(edit) = ui.office.edit.as_ref() else { return false };
    let words = if edit.stopped {
        crate::frames::office::edit::recovery::read_only_words()
    } else {
        NOT_TAKEN
    };
    ui.toast = Some(words.to_string());
    true
}

/// The viewer's own toast.
fn say(s: &mut State, words: String) {
    if let Some(ui) = viewers::ui(s) {
        ui.toast = Some(words);
    }
}

/// The open file as Files knows it: its row when the folder shown holds it, else what the viewer knows of it.
fn open_item(s: &mut State) -> Option<DriveItem> {
    let ui = viewers::ui(s)?;
    let (id, name, offline) = (ui.file.id.clone(), ui.file.name.clone(), ui.file.details.offline);
    if let Some(row) = s.listing.items.iter().find(|row| row.id == id) {
        return Some(row.clone());
    }
    Some(DriveItem {
        id,
        parent_id: s.listing.folder.id.clone(),
        name,
        is_folder: false,
        size: None,
        child_count: None,
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: offline.then(|| "available_offline".to_string()),
        is_protected: false,
    })
}

fn row_of(s: &State, item: &DriveItem) -> Option<usize> {
    s.listing.items.iter().position(|row| row.id == item.id)
}

fn share(s: &mut State, item: DriveItem) {
    if !s.begin_share_for(vec![item]) {
        say(s, "This file is protected and can't be shared".into());
    }
}

fn trash(s: &mut State, tx: &mut Tx, item: DriveItem) {
    let before = s.actions.toast;
    // In the folder shown: Files' own path, with its Undo toast and its answer from a source that answers later.
    let moved = if let Some(row) = row_of(s, &item) {
        s.select_only(row);
        s.trash_selection(tx);
        match s.actions.toast {
            Some(Toast::Failed(action, why)) if before != s.actions.toast => Err((action, why)),
            _ => Ok(()),
        }
    } else {
        // A file Files' list does not show: ask the source, and hold it for Undo.
        match s.actions.source.trash(std::slice::from_ref(&item)) {
            Ok(()) => {
                s.actions.undo = vec![(0, item)];
                s.actions.toast = Some(Toast::Trashed);
                s.actions.toast_until = Some(std::time::Instant::now() + Toast::Trashed.stays());
                tx.wake_after(Toast::Trashed.stays());
                Ok(())
            }
            Err(why) => Err((Action::Trash, why)),
        }
    };
    match moved {
        Ok(()) => viewers::close(s),
        // The file stays open and says why in its own toast.
        Err((action, why)) => {
            s.actions.toast = before;
            say(s, Toast::failure(action, why));
        }
    }
}

fn why_not_edit(s: &State, item: &DriveItem) -> &'static str {
    if s.protects(std::slice::from_ref(item)) {
        return "This file is in a protected backup and can't be edited";
    }
    match s.viewer.ui.as_ref().map(|ui| ui.origin()) {
        Some(Origin::Beebeeb | Origin::BeebeebNotThisPcs | Origin::BeebeebUnconfirmedHere) => {
            "Not on this PC: keep it available offline to edit it here"
        }
        _ => "This file is read-only here",
    }
}

fn free_up_space(s: &mut State, item: DriveItem) {
    let words = match s.actions.source.free_up_space(std::slice::from_ref(&item)) {
        Ok(()) => "Freed up space: this file stays in Beebeeb".to_string(),
        Err(why) => Toast::failure(Action::FreeUpSpace, why),
    };
    say(s, words);
}

fn keep_offline(s: &mut State, item: DriveItem) {
    let words = match s.actions.source.keep_offline(std::slice::from_ref(&item), true) {
        Ok(()) => "Making this file available offline".to_string(),
        Err(why) => Toast::failure(Action::KeepOffline, why),
    };
    say(s, words);
}

/// A file in Drive to open: a name from "Files" (the office editor's command search), or a link as a note wrote it.
/// A name is one of the folder shown, taken as it is (whatever characters it has); a link resolves in the folder the
/// open file is in (a name, `./name`, or the full path from Files), and anything that is not in Drive (a drive
/// letter, `..`, a scheme, backslashes) is refused. Changes no version holds are asked about first, as Back asks:
/// the open file stays until that is answered.
fn open_in_drive(s: &mut State, tx: &mut Tx, path: &str) {
    let Some(ui) = viewers::ui(s) else { return };
    if ui.hold_for_changes() {
        return;
    }
    let here: Vec<String> = ui.file.path.clone();
    let named = s.listing.items.iter().find(|row| row.name == path).cloned();
    let row = match named {
        Some(row) => row,
        None => {
            let Some(name) = resolve_in_folder(path, &here) else {
                return say(s, "That link isn't in your Beebeeb Drive".into());
            };
            let Some(row) = s.listing.items.iter().find(|row| row.name == name).cloned() else {
                return say(s, "Couldn't find that in this folder".into());
            };
            row
        }
    };
    if row.is_folder {
        viewers::close(s);
        s.open_folder(Some(row.id));
    } else if !viewers::open_item(s, &row) {
        // Not a file this group shows: Files' own Open.
        viewers::close(s);
        if let Some(at) = row_of(s, &row) {
            s.select_only(at);
            s.open_selection(tx);
        }
    }
}

/// The name a note's `path` points at inside the folder `here` ("Files" and its folders, outermost first), or `None`
/// when it is not a path in Drive or not in that folder.
fn resolve_in_folder(path: &str, here: &[String]) -> Option<String> {
    let path = path.trim();
    if path.is_empty() || path.contains('\\') || path.contains(':') || path.contains('\0') {
        return None;
    }
    let (rooted, parts): (bool, Vec<&str>) = (
        path.starts_with('/'),
        path.split('/').filter(|part| !part.is_empty() && *part != ".").collect(),
    );
    if parts.contains(&"..") {
        return None;
    }
    let (name, folders) = parts.split_last()?;
    if folders.is_empty() && !rooted {
        return Some((*name).to_string());
    }
    // A path from the top: its folders are the open file's own.
    (folders == here.iter().map(String::as_str).collect::<Vec<_>>().as_slice()
        || folders == here.iter().skip(1).map(String::as_str).collect::<Vec<_>>().as_slice())
    .then(|| (*name).to_string())
}

/// The browser opens http(s) only; the window's page opener refuses everything it does not know.
fn open_in_browser(s: &mut State, address: &str) {
    let lower = address.trim().to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return say(s, "Only web links open in the browser".into());
    }
    if crate::engine_host::signin::open_web_page(address.trim()).is_err() {
        say(s, "Couldn't open that link".into());
    }
}

/// New's menu is Files': the viewer goes back to Files and the menu opens there (DW-0414).
fn new_menu(s: &mut State) {
    viewers::close(s);
    // A file that holds changes stays open (its own question); nothing opens behind it.
    if viewers::ui(s).is_none() && s.can_add() {
        s.toggle_menu(MenuKind::New);
    }
}
