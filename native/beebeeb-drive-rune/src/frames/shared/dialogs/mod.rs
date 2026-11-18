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
//! **Secrets.** The account password (DW-0749, DW-0748's Delete permanently) lives in ONE secure `TextEdit`, wiped on
//! every close and when it is sent; it is drawn as dots and is in no label, intent or `Debug`.
//! A safety number is never computed here: only a scene gives one ([`SafetyNumber`]).

mod confirm;
mod item_view;
mod kit;
mod link_change;
pub mod request_menus;
#[cfg(test)]
mod safety_tests;
#[cfg(test)]
mod tests;
mod view;

pub use view::layers;

use super::Tab;
use super::source::{Access, InviteResult, Member, Notice, PeopleShare};
use crate::State;
use crate::frames::files::actions_data::Previewed;
use crate::frames::item::Origin;
use crate::frames::{self, Intent};
use crate::shell::app::{GateState, Section};
use rune::{Key, KeyEvent, TextEdit, Tx};
use zeroize::Zeroize;

/// The text inputs' keys (tests, `Tx::focus`).
pub const TITLE_INPUT: &str = "request-title-input";
pub const NOTE_INPUT: &str = "request-note-input";
pub const EMAIL_INPUT: &str = "invite-email-input";
/// Empty Trash's and Delete permanently's password field.
pub const PASSWORD_INPUT: &str = "purge-password-input";

/// What Invite someone says when the person's key is not the one they compared safety numbers against (DW-0514).
pub const KEY_CHANGED: &str = "Their key changed since you compared safety numbers. Nothing was shared.";

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
    /// Delete permanently? (DW-0748's warning, for the Trash item with this id).
    DeleteForever(String),
    /// Details (DW-0411) of the item with this id in `origin`'s list (`frames::item`).
    Details { origin: Origin, id: String },
    /// The read-only Preview (DW-0408's panel) of the item with this id in `origin`'s list (`frames::item`).
    Preview { origin: Origin, id: String },
    /// Change a live link's expiry (stub: w17-shared-links).
    LinkExpiry { share_id: String },
    /// Set or remove a link's password; its secure field is `Fields::link_password` (stub: w17-shared-links).
    LinkPassword { share_id: String, has_password: bool },
    /// Leave a share someone made with the person (stub: w17-shared-items).
    ConfirmLeave { id: String },
    /// Stop sharing the row with this id (stub: w17-shared-items).
    ConfirmStop { id: String },
    /// Close the request with this id (stub: w17-shared-items).
    ConfirmClose { id: String },
}

impl Dialog {
    /// The section the dialog belongs to: it shows only over that section.
    pub fn section(&self) -> Section {
        match self {
            Dialog::EmptyTrash | Dialog::DeleteForever(_) => Section::Trash,
            Dialog::Details { origin, .. } | Dialog::Preview { origin, .. } => origin.section(),
            _ => Section::Shared,
        }
    }
}

/// What an item's Preview holds of its file: the page, or why there is none. Decrypted content: wiped when the
/// dialog closes (`Fields::clear`) and when dropped; a `{:?}` names the state only, never a line of it.
#[derive(Clone, Default, PartialEq)]
pub struct PreviewHeld(Option<Previewed>);

impl PreviewHeld {
    pub fn of(state: Previewed) -> Self {
        PreviewHeld(Some(state))
    }

    /// What is held.
    pub fn state(&self) -> Option<&Previewed> {
        self.0.as_ref()
    }

    /// Asked, not answered yet.
    pub fn is_loading(&self) -> bool {
        self.0 == Some(Previewed::Loading)
    }

    /// Overwrite the page's text, then let it go.
    fn wipe(&mut self) {
        if let Some(Previewed::Shown(shown)) = self.0.as_mut() {
            shown.place.zeroize();
            shown.page.title.zeroize();
            shown.page.byline.zeroize();
            for (heading, text) in shown.page.sections.iter_mut() {
                heading.zeroize();
                text.zeroize();
            }
        }
        self.0 = None;
    }
}

impl Drop for PreviewHeld {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl std::fmt::Debug for PreviewHeld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = match &self.0 {
            None => "None",
            Some(Previewed::Loading) => "Loading",
            Some(Previewed::Shown(_)) => "Shown",
            Some(Previewed::NoPreview) => "NoPreview",
            Some(Previewed::NotYet) => "NotYet",
            Some(Previewed::TooLarge) => "TooLarge",
            Some(Previewed::Failed(_)) => "Failed",
        };
        write!(f, "PreviewHeld({state})")
    }
}

/// The gate and the account an item dialog was opened under (`frames::item`). A `{:?}` names the gate only.
#[derive(Clone, PartialEq, Eq)]
pub struct ItemStamp(pub GateState, pub Option<String>);

impl std::fmt::Debug for ItemStamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ItemStamp").field(&self.0).finish_non_exhaustive()
    }
}

/// What the open dialog's fields hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Fields {
    /// New request's Title and Note.
    pub title: TextEdit,
    pub note: TextEdit,
    /// New request's four menus.
    pub request: request_menus::Pick,
    /// Invite someone's Email.
    pub email: TextEdit,
    /// The account password Empty Trash and Delete permanently ask: secure, wiped whenever a dialog opens or closes
    /// and when it is sent.
    pub password: TextEdit,
    /// A link's new password (LinkPassword): secure, wiped on every close.
    pub link_password: TextEdit,
    /// What the last try said (the password was not right; an invitation was not sent), shown under the field.
    pub problem: Option<&'static str>,
    /// Invite someone: an invitation is on its way, and Send invite takes no second press.
    pub sending: bool,
    /// Invite someone for an item not shared with anyone yet (no row in By me): its name, for the title.
    pub unlisted: Option<String>,
    /// An item's Preview: its page, or why there is none.
    pub preview: PreviewHeld,
    /// An item dialog: the gate and the account it was opened under.
    pub item_stamp: Option<ItemStamp>,
}

impl Default for Fields {
    fn default() -> Self {
        Fields {
            title: TextEdit::new(""),
            note: TextEdit::new(""),
            request: Default::default(),
            email: TextEdit::new(""),
            password: TextEdit::new("").secure(),
            link_password: TextEdit::new("").secure(),
            problem: None,
            sending: false,
            unlisted: None,
            preview: PreviewHeld::default(),
            item_stamp: None,
        }
    }
}

impl Fields {
    /// Empty every field; the password's characters are overwritten, not left behind.
    fn clear(&mut self) {
        for edit in [
            &mut self.title,
            &mut self.note,
            &mut self.email,
            &mut self.password,
            &mut self.link_password,
        ] {
            edit.clear();
            edit.focused = false;
        }
        self.request = Default::default();
        self.problem = None;
        self.sending = false;
        self.unlisted = None;
        self.preview.wipe();
        self.item_stamp = None;
    }
}

impl super::SharedUi {
    /// Open `dialog` over the section, every field empty; a menu that was open closes.
    pub fn open_dialog(&mut self, dialog: Dialog) {
        self.menu = None;
        self.fields.clear();
        self.dialog = Some(dialog);
    }

    /// Close Empty Trash's or Delete permanently's dialog (a delete for good ended, or its item is gone). Any other
    /// dialog stays.
    pub fn close_purge_dialog(&mut self) {
        if matches!(self.dialog, Some(Dialog::EmptyTrash | Dialog::DeleteForever(_))) {
            self.close_dialog();
        }
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

    /// Whether the lane's open dialog is one of `section`'s.
    pub fn dialog_over(&self, section: Section) -> bool {
        self.dialog.as_ref().is_some_and(|d| d.section() == section)
    }

    /// Whether the open dialog has what it draws: its share, and its person. One about a share
    /// or a person that went is not drawn (`view::layers`).
    pub fn dialog_drawn(&self) -> bool {
        let person_of = |share: &str, person: &str| self.share(share).is_some() && self.member(share, person).is_some();
        match &self.dialog {
            None => false,
            // Its row may be gone (Trash's poll closes it then): nothing here knows Trash's list.
            Some(Dialog::NewRequest | Dialog::EmptyTrash | Dialog::DeleteForever(_)) => true,
            // Its row is in another section's list: `frames::item::poll` closes it when the row goes, and
            // `section_keys` when it is not drawn.
            Some(Dialog::Details { .. } | Dialog::Preview { .. }) => true,
            // Stubs: the row they name may be a link, a request or an incoming share; the later tasks check it.
            Some(
                Dialog::LinkExpiry { .. }
                | Dialog::LinkPassword { .. }
                | Dialog::ConfirmLeave { .. }
                | Dialog::ConfirmStop { .. }
                | Dialog::ConfirmClose { .. },
            ) => true,
            Some(Dialog::People { share, .. }) => self.share(share).is_some(),
            // An item not shared with anyone yet has no row: its name is kept with the dialog.
            Some(Dialog::Invite { share, .. }) => self.share(share).is_some() || self.fields.unlisted.is_some(),
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
            // Back to People, when the share has a list of people to go back to.
            Some(Dialog::Invite { share, .. }) if self.share(&share).is_some() => Some(Dialog::People {
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

    /// Invite someone… for an item from Files (DW-0514): its share's dialog when it has one, else the same dialog
    /// for an item not shared with anyone yet, titled with `name`.
    pub fn start_invite_named(&mut self, share: &str, name: &str) {
        self.start_invite(share);
        if self.share(share).is_none() {
            self.fields.unlisted = Some(name.to_string());
        }
    }

    /// Send invite: hands the address typed to the source, with the access Beebeeb gives a new invitation. The
    /// dialog says "Sending…" and takes no second press until the answer ([`Self::invite_answered`]); nothing is
    /// drawn as sent before it. Not an address (`is_address`), one already on its way, or the source refuses at
    /// once (they are on the share already): nothing is sent and the dialog stays.
    pub fn send_invite(&mut self) -> bool {
        let Some(Dialog::Invite { share, .. }) = self.dialog.clone() else {
            return false;
        };
        if self.fields.sending {
            return false;
        }
        let email = self.fields.email.text.trim().to_string();
        if !is_address(&email) || !self.source.start_invite(&share, &email) {
            return false;
        }
        self.fields.sending = true;
        self.fields.problem = None;
        true
    }

    /// An invitation's answer (the window's poll). The dialog that sent it closes only on an answer that reached
    /// Beebeeb (People shows the share, when it has a list); a changed key or a refusal keeps it open, with its
    /// words. An answer whose dialog was closed in the meantime: a refusal says so over the lists.
    pub fn invite_answered(&mut self, share: &str, result: InviteResult) {
        let waiting =
            self.fields.sending && matches!(&self.dialog, Some(Dialog::Invite { share: s, .. }) if s == share);
        if !waiting {
            if !result.sent() {
                self.notice = Some(Notice::ChangeRefused);
            }
            return;
        }
        self.fields.sending = false;
        match result {
            InviteResult::KeyChanged => {
                self.fields.problem = Some(KEY_CHANGED);
                // The source names the person whose key stopped it: People warns over them (DW-0515).
                // Its row may not be listed yet: the warning draws once the lists are read again.
                if let Some(person) = self.source.key_changed_person(share) {
                    self.open_dialog(Dialog::People {
                        share: share.to_string(),
                        key_changed: Some(person),
                    });
                }
            }
            InviteResult::Refused(words) => self.fields.problem = Some(words),
            _ => {
                if !self.show_people(share) {
                    self.close_dialog();
                }
            }
        }
    }
}

/// An address someone can be invited at, as far as the window can tell: one `@`, something
/// before it, a dot inside what follows it, no spaces. Whether it reaches anyone is the
/// engine's to find out.
pub fn is_address(email: &str) -> bool {
    let Some((name, domain)) = email.split_once('@') else {
        return false;
    };
    !name.is_empty()
        && !domain.contains('@')
        && domain.split('.').count() >= 2
        && domain.split('.').all(|part| !part.is_empty())
        && !email.chars().any(char::is_whitespace)
}

/// The section the window shows, when it is one of this lane's: Shared, Trash, or Files while Search's window
/// stands over it (an item's Details over the results, `frames::item`).
pub fn section_shown(s: &State) -> Option<Section> {
    crate::frames::item::origin_shown(s).map(Origin::section)
}

/// Close what is open, through its way back (the modal's Esc, a Cancel). An item's dialog closes, and the keyboard
/// goes back to its row.
pub(crate) fn dismiss(s: &mut State, tx: &mut Tx) {
    if matches!(
        s.frames.shared.dialog,
        Some(Dialog::Details { .. } | Dialog::Preview { .. })
    ) {
        item_view::close(s, tx);
    } else {
        s.frames.shared.escape_dialog();
    }
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
/// Right move a switch. Every other key stops here: the
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
    match (focused.as_deref(), &ev.key) {
        (Some(key), Key::Left | Key::Right) if key.starts_with("seg-") && plain => {
            view::step_access(s, ev.key == Key::Right, tx);
        }
        _ => {}
    }
    true
}

/// Brings the lane's dialog in line with the route: one whose section the window no longer
/// shows is closed, its fields emptied (the code is wiped), so nothing of it waits while the
/// person is elsewhere. Run on every key of Shared and Trash (`section_keys`); the shell's
/// route change is the place that catches every way out (a request to the shell lane, as
/// `settings::sync`).
pub fn sync(s: &mut State) {
    let routed = section_shown(s);
    let ui = &mut s.frames.shared;
    if ui.dialog.as_ref().is_some_and(|d| Some(d.section()) != routed) {
        ui.close_dialog();
    }
}

/// The keys `section` gets while the lane may have a dialog open (`shared::on_key`, and
/// Trash's registry line [`TRASH`]). A dialog of another section closes first (`sync`). A dialog
/// of this section that is drawn takes the key (`on_section_key`); one whose share or person
/// went closes, and the section has its keys again. `None`: the section's own keys go on.
pub fn section_keys(s: &mut State, ev: &KeyEvent, tx: &mut Tx, section: Section) -> Option<bool> {
    sync(s);
    if !s.frames.shared.dialog_over(section) {
        return None;
    }
    if s.frames.shared.dialog_drawn() && crate::frames::item::drawn(s) {
        return Some(on_section_key(s, ev, tx));
    }
    s.frames.shared.close_dialog();
    None
}

/// Trash's line in the shell's registry: `frames::TRASH`, its keys behind the lane's dialog
/// (Empty Trash? stands over Trash, and `trash::on_key` knows nothing of it).
pub const TRASH: crate::shell::registry::Frame = crate::shell::registry::Frame {
    on_key: Some(trash_keys),
    ..crate::frames::TRASH
};

fn trash_keys(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    match section_keys(s, ev, tx, Section::Trash) {
        Some(took) => took,
        None => crate::frames::trash::on_key(s, ev, tx),
    }
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
        // Space opened an item's Preview from the row; Space closes it again, wherever focus is.
        _ if frames::is_space(ev) && matches!(s.frames.shared.dialog, Some(Dialog::Preview { .. })) => {
            item_view::close(s, tx);
            true
        }
        _ => true,
    }
}
