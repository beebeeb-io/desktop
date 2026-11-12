//! The lane's dialogs (`.wn-dlg` on `.mk-scrim`): Shared's New request (DW-0504), People with
//! access and what opens from it (DW-0511 to DW-0515), and Trash's Empty Trash (DW-0749).
//!
//! **State.** One dialog is open at a time ([`Dialog`]; DW-0515's alert over People is one
//! state). What its fields hold is kept beside it ([`Fields`]), so a field's `on_edit` can
//! reach it. Both live on `SharedUi`; `parts::chrome::window` draws them over the section they
//! belong to, Shared or Trash. (Trash's own state is another file area's: `TrashUi` has no
//! field for its dialog, so the lane's one dialog layer holds it.)
//!
//! **Where each opens.** People with access opens from By me (Enter on a share with people,
//! its People item, a click on its people); a person's access, the confirmation and Invite
//! someone open from People. New request (DW-0504), the safety number (DW-0513, DW-0513b),
//! a changed key (DW-0515) and Empty Trash (DW-0749) open from their scenes only: New request's
//! four menus have no choices on the board, the next three carry crypto material the engine
//! has to give (the security-reviewed wiring), and Trash's Empty Trash… button is Trash's
//! (it asks `Intent::Frame("DW-0749")`). A button whose action needs the engine records an
//! [`Intent`] and leaves the dialog as it is: nothing claims to have happened.
//!
//! **Secrets.** The authenticator code (DW-0749) lives in ONE secure `TextEdit`, wiped on
//! every close; it is drawn as secure text inside `secure-code` and is in no label or intent.
//! A safety number is never computed here: only a scene gives one ([`SafetyNumber`]).

mod kit;
#[cfg(test)]
mod tests;
mod view;

pub use kit::CODE_LEN;
pub use view::layers;

use super::Tab;
use super::source::{Access, Member, PeopleShare};
use crate::State;
use crate::frames::{self, Intent};
use crate::shell::app::Section;
use rune::{Key, KeyEvent, TextEdit, Tx};

/// The text inputs' keys (tests, `Tx::focus`).
pub const TITLE_INPUT: &str = "request-title-input";
pub const NOTE_INPUT: &str = "request-note-input";
pub const EMAIL_INPUT: &str = "invite-email-input";

/// A safety number as the board draws it: twelve groups of five digits. Crypto material: in
/// this build only a scene makes one (the board's sample); nothing here computes, keeps beyond
/// its dialog, or compares one. The engine gives the real one in the security-reviewed wiring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafetyNumber(pub Vec<String>);

/// The dialog that shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Dialog {
    /// New request (DW-0504).
    NewRequest,
    /// People with access to a share (DW-0511). `key_changed`: the person whose remembered key
    /// changed, alerted over it (DW-0515).
    People { share: String, key_changed: Option<String> },
    /// One person's access and identity (DW-0512).
    Person { share: String, person: String },
    /// "Remove <name>'s access?" (DW-0512b).
    RemoveAccess { share: String, person: String },
    /// The safety number to compare (DW-0513).
    SafetyNumber {
        share: String,
        person: String,
        number: SafetyNumber,
    },
    /// The numbers did not match: sharing with them is stopped (DW-0513b).
    Stopped {
        share: String,
        person: String,
        number: SafetyNumber,
    },
    /// Invite someone to a share, with this access (DW-0514).
    Invite { share: String, access: Access },
    /// Empty Trash? (DW-0749).
    EmptyTrash,
}

impl Dialog {
    /// The section the dialog belongs to: it shows only over that section.
    pub fn section(&self) -> Section {
        match self {
            Dialog::EmptyTrash => Section::Trash,
            _ => Section::Shared,
        }
    }
}

/// What the open dialog's fields hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Fields {
    /// New request's Title and Note.
    pub title: TextEdit,
    pub note: TextEdit,
    /// Invite someone's Email.
    pub email: TextEdit,
    /// Empty Trash's authenticator code: secure, wiped whenever a dialog opens or closes.
    pub code: TextEdit,
}

impl Default for Fields {
    fn default() -> Self {
        Fields {
            title: TextEdit::new(""),
            note: TextEdit::new(""),
            email: TextEdit::new(""),
            code: TextEdit::new("").secure(),
        }
    }
}

impl Fields {
    /// Empty every field; the code's characters are overwritten, not left behind.
    fn clear(&mut self) {
        for edit in [&mut self.title, &mut self.note, &mut self.email, &mut self.code] {
            edit.clear();
            edit.focused = false;
        }
    }
}

impl super::SharedUi {
    /// Open `dialog` over the section, every field empty; a menu that was open closes.
    pub fn open_dialog(&mut self, dialog: Dialog) {
        self.menu = None;
        self.fields.clear();
        self.dialog = Some(dialog);
    }

    /// Close the dialog, and empty its fields (the code is wiped).
    pub fn close_dialog(&mut self) {
        self.dialog = None;
        self.fields.clear();
    }

    /// The share with people of this id.
    pub fn share(&self, id: &str) -> Option<PeopleShare> {
        self.source.people_shares().into_iter().find(|p| p.id == id)
    }

    /// One person of a share.
    pub fn member(&self, share: &str, person: &str) -> Option<Member> {
        self.source.members(share).into_iter().find(|m| m.id == person)
    }

    /// Whether the open dialog has what it draws: its share, and its person. One about a share
    /// or a person that went is not drawn (`view::layers`).
    pub fn dialog_drawn(&self) -> bool {
        let person_of = |share: &str, person: &str| self.share(share).is_some() && self.member(share, person).is_some();
        match &self.dialog {
            None => false,
            Some(Dialog::NewRequest | Dialog::EmptyTrash) => true,
            Some(Dialog::People { share, .. } | Dialog::Invite { share, .. }) => self.share(share).is_some(),
            Some(
                Dialog::Person { share, person }
                | Dialog::RemoveAccess { share, person }
                | Dialog::SafetyNumber { share, person, .. }
                | Dialog::Stopped { share, person, .. },
            ) => person_of(share, person),
        }
    }

    /// The first `list-<n>` a dialog's lists take: after the lists of the tab behind it, as the
    /// board numbers every `.kit-list` of a frame.
    pub fn lists_behind(&self) -> usize {
        match self.tab {
            Tab::WithMe => 0,
            Tab::ByMe if self.count() > 0 => 2,
            Tab::Requests if self.count() > 0 => 1,
            _ => 0,
        }
    }

    /// Escape: back the way the dialog came, as its Cancel does; out of the first dialog.
    pub fn escape_dialog(&mut self) {
        let back = match self.dialog.take() {
            Some(Dialog::People {
                share,
                key_changed: Some(_),
            }) => Some(Dialog::People {
                share,
                key_changed: None,
            }),
            Some(Dialog::RemoveAccess { share, person })
            | Some(Dialog::SafetyNumber { share, person, .. })
            | Some(Dialog::Stopped { share, person, .. }) => Some(Dialog::Person { share, person }),
            Some(Dialog::Invite { share, .. }) => Some(Dialog::People {
                share,
                key_changed: None,
            }),
            _ => None,
        };
        match back {
            Some(dialog) => self.open_dialog(dialog),
            None => self.close_dialog(),
        }
    }

    /// People with access to a share with people (DW-0511). `false`: no such share (a link has
    /// no list of people: whoever has it can open it).
    pub fn show_people(&mut self, share: &str) -> bool {
        if self.share(share).is_none() {
            return false;
        }
        self.open_dialog(Dialog::People {
            share: share.to_string(),
            key_changed: None,
        });
        true
    }

    /// One person's access (DW-0512); the owner has none to change.
    pub fn show_person(&mut self, share: &str, person: &str) {
        if self.member(share, person).is_some_and(|m| m.access.is_some()) {
            self.open_dialog(Dialog::Person {
                share: share.to_string(),
                person: person.to_string(),
            });
        }
    }

    /// Can view or Can edit, changed at once (DW-0512).
    pub fn set_access(&mut self, share: &str, person: &str, access: Access) -> bool {
        self.source.set_access(share, person, access)
    }

    /// Remove access (DW-0512b's answer): People shows who is left.
    pub fn remove_access(&mut self, share: &str, person: &str) -> bool {
        let done = self.source.remove_access(share, person);
        if done {
            self.show_people(share);
        }
        done
    }

    /// Invite someone… (DW-0514), the email field in focus.
    pub fn start_invite(&mut self, share: &str) {
        self.open_dialog(Dialog::Invite {
            share: share.to_string(),
            access: Access::View,
        });
        self.fields.email.focused = true;
    }

    /// Send invite: shares with the address typed; People then shows them as invited. Nothing
    /// typed, or the source refuses (they are on the share already): the dialog stays.
    pub fn send_invite(&mut self) -> bool {
        let Some(Dialog::Invite { share, access }) = self.dialog.clone() else {
            return false;
        };
        let email = self.fields.email.text.trim().to_string();
        if email.is_empty() || !self.source.invite(&share, &email, access) {
            return false;
        }
        self.show_people(&share);
        true
    }

    /// How many digits of the code are typed.
    pub fn code_len(&self) -> usize {
        self.fields.code.text.chars().count()
    }

    /// Type into the code: digits only, six at most. The next code is built in a buffer that
    /// never grows (no copy left behind by a reallocation); the edit wipes the one it replaces.
    pub fn type_code(&mut self, typed: &str) {
        for c in typed.chars().filter(char::is_ascii_digit) {
            if self.code_len() < CODE_LEN {
                let mut next = String::with_capacity(CODE_LEN);
                next.push_str(&self.fields.code.text);
                next.push(c);
                self.fields.code.replace_text(next);
            }
        }
    }

    /// Backspace in the code: the last digit goes.
    pub fn erase_code_digit(&mut self) {
        let mut next = String::with_capacity(CODE_LEN);
        next.push_str(&self.fields.code.text);
        if next.pop().is_some() {
            self.fields.code.replace_text(next);
        }
    }
}

/// The section the window shows, when it is one of this lane's.
pub fn section_shown(s: &State) -> Option<Section> {
    match crate::shell::registry::group_for(&s.shell.app) {
        crate::shell::registry::FrameGroup::Section(section @ (Section::Shared | Section::Trash)) => Some(section),
        _ => None,
    }
}

/// Close what is open, through its way back (the modal's Esc, a Cancel).
pub(crate) fn dismiss(s: &mut State, _tx: &mut Tx) {
    s.frames.shared.escape_dialog();
}

/// Close every layer (Done, Cancel of the first dialog).
pub(crate) fn close(s: &mut State) {
    s.frames.shared.close_dialog();
}

/// Record what needs the engine: the dialog stays as it is.
pub(crate) fn ask(s: &mut State, intent: Intent) {
    s.frames.intents.push(intent);
}

/// The dialog's keys, heard on its layer while focus is inside it (a text field's are the
/// field's own). Tab, Esc, Enter and Space are Rune's (move focus, dismiss, press). Left and
/// Right move a switch; the code takes digits and Backspace. Every other key stops here: the
/// list behind a dialog never moves under it.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let plain = frames::plain(ev);
    match &ev.key {
        Key::Tab | Key::Escape => return false,
        Key::Enter if plain => return false,
        _ if frames::is_space(ev) => return false,
        _ => {}
    }
    let focused = tx.focused().map(str::to_owned);
    let ui = &mut s.frames.shared;
    match (focused.as_deref(), &ev.key) {
        (Some("secure-code"), Key::Text(typed)) if plain || ev.mods.shift => {
            ui.type_code(typed);
        }
        (Some("secure-code"), Key::Backspace) => ui.erase_code_digit(),
        (Some(key), Key::Left | Key::Right) if key.starts_with("seg-") && plain => {
            view::step_access(s, ev.key == Key::Right, tx);
        }
        _ => {}
    }
    true
}

/// The section's keys while a dialog of it shows: what the dialog and Rune left (a key a text
/// field does not use, Enter that pressed nothing, any key while focus is nowhere). Esc goes
/// back the dialog's way; Tab is Rune's, to move focus into the dialog; every other key stops
/// here: nothing reaches the list behind.
pub fn on_section_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    match ev.key {
        Key::Escape => {
            dismiss(s, tx);
            true
        }
        Key::Tab => false,
        _ => true,
    }
}
