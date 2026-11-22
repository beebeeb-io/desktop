//! The name dialog of Photos' Rename. The name is held to Files' rules (`names::check_on`) before the source is
//! asked: a name another file of the folder has is refused with Files' words, the dialog stays open, and the source
//! is never called. Enter or the Rename button saves, Esc or Cancel closes.

use rune::{TextEdit, Tx};

use super::{NOT_HERE, words};
use crate::State;
use crate::frames::files::names::{self, NameError};
use crate::frames::files::source::{Action, ActionError};
use crate::frames::photos::source::PhotoId;
use crate::frames::photos::{Dialog, Toast};

/// The key of the dialog's text input.
pub const INPUT: &str = "ph-rename-input";

/// A text edit that `Dialog` can compare and print.
#[derive(Clone, Default)]
pub struct Name(pub TextEdit);

impl PartialEq for Name {
    fn eq(&self, other: &Name) -> bool {
        self.0 == other.0
    }
}
impl Eq for Name {}
impl std::fmt::Debug for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Name").field(&self.0.text).finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub id: PhotoId,
    pub edit: Name,
    /// The name it has now.
    pub original: String,
    /// The other items of its folder (name, is a folder), as far as this PC knows them.
    pub others: Vec<(String, bool)>,
    /// What was refused, for the name that was typed then.
    pub error: Option<(String, NameError)>,
}

impl Draft {
    /// Why the typed name cannot be given, while it is still the name that was refused.
    pub fn error_now(&self) -> Option<&NameError> {
        self.error
            .as_ref()
            .filter(|(typed, _)| *typed == self.edit.0.text)
            .map(|(_, why)| why)
    }
}

/// More photos than this and the siblings are left to the source (it refuses a taken name itself).
const SIBLINGS_UP_TO: usize = 5000;

/// The names that lie beside the photo's file: every photo held in the same folder.
fn siblings(s: &State, id: PhotoId, parent: &Option<String>, own: &str) -> Vec<(String, bool)> {
    let src = &s.photos.source;
    let mut ids = Vec::new();
    for (day, d) in src.days(None).iter().enumerate() {
        for index in 0..d.len {
            if let Some(photo) = src.photo(None, day, index)
                && photo.id != id
            {
                ids.push(photo.id);
            }
        }
        if ids.len() > SIBLINGS_UP_TO {
            return Vec::new();
        }
    }
    // One by one: a photo that is not held makes `file_items` answer `None` for all it was asked about.
    ids.iter()
        .filter_map(|id| src.file_items(&[*id]))
        .flatten()
        .filter(|item| item.id != own && item.parent_id == *parent)
        .map(|item| (item.name, item.is_folder))
        .collect()
}

pub fn open(s: &mut State, tx: &mut Tx, id: PhotoId) {
    let Some(item) = s
        .photos
        .source
        .file_items(&[id])
        .and_then(|items| items.into_iter().next())
    else {
        s.photos.toast = Some(Toast::Words(NOT_HERE));
        return;
    };
    if s.protects(std::slice::from_ref(&item)) {
        s.photos.toast = Some(Toast::Words(words(Action::Rename, ActionError::Protected)));
        return;
    }
    let others = siblings(s, id, &item.parent_id, &item.id);
    let mut edit = TextEdit::new(item.name.clone());
    edit.set_selection(0, names::stem_len(&item.name, false));
    s.photos.menu = None;
    s.photos.dialog = Some(Dialog::Rename(Box::new(Draft {
        id,
        edit: Name(edit),
        original: item.name,
        others,
        error: None,
    })));
    tx.focus(INPUT, false);
}

/// Enter or Rename: check the name, then ask the source. A refusal keeps the dialog open.
pub fn commit(s: &mut State, _tx: &mut Tx) {
    let Some(Dialog::Rename(draft)) = s.photos.dialog.clone() else {
        return;
    };
    let typed = draft.edit.0.text.clone();
    if typed.trim() == draft.original {
        s.photos.dialog = None;
        return;
    }
    let others = draft.others.iter().map(|(name, folder)| (name.as_str(), *folder));
    let name = match names::check_on(s.actions.host, &typed, others) {
        Ok(name) => name,
        Err(why) => return refuse(s, typed, why),
    };
    let Some(item) = s
        .photos
        .source
        .file_items(&[draft.id])
        .and_then(|items| items.into_iter().next())
    else {
        s.photos.dialog = None;
        s.photos.toast = Some(Toast::Words(NOT_HERE));
        return;
    };
    match s.actions.source.rename(&item, &name) {
        Ok(()) => {
            s.photos.dialog = None;
            s.photos.source.refresh();
            s.photos.toast = Some(Toast::Words("Renamed"));
        }
        Err(ActionError::NameTaken) => refuse(s, typed, NameError::Taken { name, folder: false }),
        Err(why) => {
            s.photos.dialog = None;
            s.photos.toast = Some(Toast::Words(words(Action::Rename, why)));
        }
    }
}

fn refuse(s: &mut State, typed: String, why: NameError) {
    if let Some(Dialog::Rename(draft)) = &mut s.photos.dialog {
        draft.error = Some((typed, why));
    }
}
