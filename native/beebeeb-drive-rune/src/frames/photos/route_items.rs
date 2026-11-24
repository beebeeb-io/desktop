//! Photos routes: Rename and Duplicate of an open photo, by Files' own source (`FilesSource`).
//!
//! Duplicate is one call: the engine makes a local copy in the sync folder that uploads as a new file (no server
//! copy). A place that refuses (protected backups, a read-only share, sync off) says so in a message. Rename opens
//! a small name dialog (`rename`) that holds the name to Files' rules before the source is asked.

use rune::Tx;

use super::{Intent, Toast};
use crate::State;
use crate::frames::files::source::{Action, ActionError};

#[path = "rename.rs"]
pub(super) mod rename;
#[cfg(test)]
#[path = "route_items_tests.rs"]
mod tests;

/// Takes the intent it can; `Err` hands it back and Photos says "Not available in this version yet".
pub fn handle(s: &mut State, tx: &mut Tx, intent: Intent) -> Result<(), Intent> {
    match intent {
        Intent::Duplicate(id) => duplicate(s, tx, id),
        Intent::Rename(id) => rename::open(s, tx, id),
        Intent::ShowInFolder(id) => show_in_folder(s, id),
        Intent::OpenInOsApp(id) => open_in_os_app(s, id),
        other => return Err(other),
    }
    Ok(())
}

fn duplicate(s: &mut State, tx: &mut Tx, id: super::PhotoId) {
    let items = s.photos.source.file_items(&[id]).filter(|items| !items.is_empty());
    let Some(items) = items else {
        s.photos.toast = Some(Toast::Words(NOT_HERE));
        return;
    };
    // A protected place is refused before the source is asked, as Files does.
    if s.protects(&items) {
        s.photos.toast = Some(Toast::Words(words(Action::Duplicate, ActionError::Protected)));
        return;
    }
    match s.actions.source.duplicate(&items, &[]) {
        Ok(()) => {
            s.apply_changes(tx);
            s.photos.source.refresh();
            s.photos.toast = Some(Toast::Words("Duplicated \u{00B7} the copy uploads as a new file"));
        }
        Err(why) => s.photos.toast = Some(Toast::Words(words(Action::Duplicate, why))),
    }
}

/// Show in File Explorer / Finder: the photo's placeholder in its folder, selected; never opened or hydrated.
fn show_in_folder(s: &mut State, id: super::PhotoId) {
    let items = s.photos.source.file_items(&[id]).filter(|items| !items.is_empty());
    let Some(items) = items else {
        s.photos.toast = Some(Toast::Words(NOT_HERE));
        return;
    };
    if let Err(words) = crate::frames::show_in_folder::show(s, &items[0].id, &items[0].name) {
        s.photos.toast = Some(Toast::Words(words));
    }
}

/// w31: the shared open chain's outside steps (the system's app, else the folder with the file selected). Never the
/// app's own viewer: this file is one it cannot show.
fn open_in_os_app(s: &mut State, id: super::PhotoId) {
    let items = s.photos.source.file_items(&[id]).filter(|items| !items.is_empty());
    let Some(items) = items else {
        s.photos.toast = Some(Toast::Words(NOT_HERE));
        return;
    };
    if let crate::frames::open_file::Opened::Failed(_) = crate::frames::open_file::open_outside_app(s, &items[0]) {
        s.photos.toast = Some(Toast::Words("Couldn\u{2019}t open it \u{00B7} not available right now"));
    }
}

const NOT_HERE: &str = concat!("Couldn\u{2019}t find that file on ", beebeeb_engine::device_word!());

/// What did not happen and why, in a line.
pub(super) fn words(action: Action, why: ActionError) -> &'static str {
    let rename = action == Action::Rename;
    match why {
        ActionError::Protected if rename => "Couldn\u{2019}t rename \u{00B7} this is kept safe and can\u{2019}t change",
        ActionError::Protected => "Couldn\u{2019}t duplicate \u{00B7} this is kept safe and can\u{2019}t change",
        ActionError::Offline if rename => "Couldn\u{2019}t rename \u{00B7} needs a connection",
        ActionError::Offline => "Couldn\u{2019}t duplicate \u{00B7} needs a connection",
        ActionError::Unavailable if rename => "Couldn\u{2019}t rename \u{00B7} not available right now",
        ActionError::Unavailable => "Couldn\u{2019}t duplicate \u{00B7} sync is off or not available right now",
        ActionError::InFlight if rename => "Couldn\u{2019}t rename. The last change is still on its way.",
        ActionError::InFlight => "Couldn\u{2019}t duplicate. The last change is still on its way.",
        _ if rename => "Couldn\u{2019}t rename",
        _ => "Couldn\u{2019}t duplicate",
    }
}
