//! Photos on a source that answers later (the engine's), and the window's poll.
//!
//! One path for both kinds of source. A source that answers in the call (a fixture) is drawn at once, as the board
//! draws it. A source that answers later (`PhotosSource::answers_later`) is only ASKED at the press: no photo
//! leaves, none comes back and no message says a change was made until the source said, per item, what Beebeeb or
//! this PC did ([`Ui::answered`], from [`poll`]). Items are found by id then, never by the place they had.
//!
//! Try again asks for exactly the items that were left, never "what is selected now".
//!
//! Whose: the moment the gate or the account changes, everything that names a photo goes, here on the window's
//! side ([`poll`]): the selection, the open photo, the message, what Undo holds, the device shown.

use rune::{El, Tx, col};

use super::chrome::style;
use super::source::{Answered, Left, Loaded, PhotoId, Viewing};
use super::{Intent, Original, Toast, Ui, Viewer};
use crate::State;
use crate::shell::app::GateState;
use crate::theme::tokens::Theme;
use crate::ui::kit;

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

/// What was asked for, in the words a refusal uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    Trash,
    Restore,
    /// Keep on this PC (`true`) or stop keeping it.
    Keep(bool),
}

/// What the window keeps for a source that answers later. All of it is the session's.
#[derive(Default)]
pub struct Later {
    /// The gate and the account the last poll saw.
    seen: Option<(GateState, Option<String>)>,
    was_open: bool,
    /// The open photo a move to Trash was asked for, as it was at the press, and the one that comes in after it.
    open_trash: Option<(Viewing, Option<PhotoId>)>,
    /// The shell's session gate was closed at the last poll (`shell::session_gate`): a source that answers later
    /// is not asked for a change. The second line; the source's own look at the session at the press is the first.
    closed: bool,
    /// What Undo (Ctrl+Z) puts back: the photos the source said Beebeeb moved to Trash.
    pub(super) undo: Vec<PhotoId>,
}

/// How often the window looks again while something is on its way.
const POLL_EVERY: std::time::Duration = std::time::Duration::from_millis(120);

impl Ui {
    /// Asks for a move to Trash. `true`: asked (or, for a fixture, made). A refusal is said at once.
    fn ask_trash(&mut self, ids: &[PhotoId]) -> bool {
        let asked = if self.later.closed {
            Err(Left::Unavailable)
        } else {
            self.source.trash(ids)
        };
        match asked {
            Ok(()) => true,
            Err(why) => {
                self.toast = Some(Toast::Left {
                    what: What::Trash,
                    why,
                    count: ids.len(),
                    again: ids.to_vec(),
                });
                false
            }
        }
    }

    /// The open photo, as it is now, and the one that comes in when it leaves.
    fn open_and_next(&self) -> Option<(Viewing, Option<PhotoId>)> {
        let viewer = self.viewer.as_ref().filter(|viewer| viewer.gone.is_none())?;
        let was = self.source.viewing(viewer.id)?;
        let at = was.strip.iter().position(|item| item.current).unwrap_or(0);
        let next = was
            .strip
            .get(at + 1)
            .or_else(|| at.checked_sub(1).and_then(|i| was.strip.get(i)))
            .map(|item| item.id);
        Some((was, next))
    }

    /// Trash on an open photo: no question, the next one comes in, Undo is offered.
    pub fn trash_open(&mut self) {
        let Some((was, next)) = self.open_and_next() else {
            return;
        };
        self.menu = None;
        if !self.ask_trash(&[was.id]) {
            return;
        }
        if self.source.answers_later() {
            // Accepted, not done: the photo stays on the stage until the source says Beebeeb moved it.
            self.later.open_trash = Some((was, next));
            return;
        }
        self.left_the_stage(was, next);
    }

    /// The open photo was moved to Trash: the next one comes in, and Undo is offered.
    fn left_the_stage(&mut self, was: Viewing, next: Option<PhotoId>) {
        self.selected.remove(&was.id);
        self.toast = Some(Toast::Trashed(vec![was.id]));
        let next = next.filter(|next| self.source.viewing(*next).is_some());
        match (next, &mut self.viewer) {
            (Some(next), Some(viewer)) => {
                viewer.id = next;
                viewer.original = Original::Preview;
                viewer.gone = Some(was);
            }
            // The last one: back to the grid, where Undo is still offered.
            _ => {
                self.viewer = None;
            }
        }
        self.relayout();
    }

    /// Move to Trash on a selection.
    pub fn trash_selected(&mut self) {
        let ids = self.selected_ids();
        if ids.is_empty() || !self.ask_trash(&ids) {
            return;
        }
        // Accepted, not done: the tiles stay checked until the source says which Beebeeb moved.
        if self.source.answers_later() {
            return;
        }
        self.end_selecting();
        self.toast = Some(Toast::Trashed(ids));
        self.relayout();
    }

    /// Undo, Ctrl+Z: brings back what the message is about.
    pub fn undo(&mut self) -> bool {
        let ids = match &self.toast {
            Some(Toast::Trashed(ids)) => ids.clone(),
            _ if !self.later.undo.is_empty() => self.later.undo.clone(),
            _ => return false,
        };
        let asked = if self.later.closed {
            Err(Left::Unavailable)
        } else {
            self.source.restore(&ids)
        };
        if let Err(why) = asked {
            self.toast = Some(Toast::Left {
                what: What::Restore,
                why,
                count: ids.len(),
                again: ids,
            });
            return true;
        }
        self.toast = None;
        // Accepted, not done: the photos come back when the source says Beebeeb put them back.
        if self.source.answers_later() {
            self.later.undo.clear();
            return true;
        }
        if let Some(viewer) = &mut self.viewer
            && let Some(was) = viewer.gone.take()
        {
            viewer.id = was.id;
        }
        self.relayout();
        true
    }

    /// Offline: keeps the original on this PC; pressed again, asks to offer its removal.
    pub fn press_offline(&mut self) {
        let Some(id) = self.viewer.as_ref().map(|v| v.id) else {
            return;
        };
        if self.source.kept_offline(id) {
            self.ask(Intent::OfferRemoveFromPc(id));
        } else {
            self.keep(id, true);
        }
    }

    fn keep(&mut self, id: PhotoId, keep: bool) {
        let asked = if self.later.closed {
            Err(Left::Unavailable)
        } else {
            self.source.keep_offline(id, keep)
        };
        match asked {
            Err(why) => {
                self.toast = Some(Toast::Left {
                    what: What::Keep(keep),
                    why,
                    count: 1,
                    again: vec![id],
                });
            }
            // Accepted, not done: the message comes when this PC said it keeps it.
            Ok(()) if self.source.answers_later() => {}
            Ok(()) if keep => self.toast = Some(Toast::Kept),
            Ok(()) => {}
        }
    }

    /// Try again, on a message that says something was left: exactly the items it names.
    pub fn try_again(&mut self) {
        let Some(Toast::Left { what, again, .. }) = self.toast.clone() else {
            return;
        };
        if again.is_empty() {
            return;
        }
        self.toast = None;
        match what {
            What::Trash => {
                // The open photo among them: as a press on Trash over it.
                let open = self.open_and_next().filter(|(was, _)| again.contains(&was.id));
                if self.ask_trash(&again) {
                    match open {
                        Some(open) if self.source.answers_later() => self.later.open_trash = Some(open),
                        Some((was, next)) => self.left_the_stage(was, next),
                        None if self.source.answers_later() => {}
                        None => {
                            self.end_selecting();
                            self.toast = Some(Toast::Trashed(again));
                            self.relayout();
                        }
                    }
                }
            }
            What::Restore => {
                self.later.undo = again;
                self.undo();
            }
            What::Keep(keep) => self.keep(again[0], keep),
        }
    }

    /// A source that answers later said what a change did, per item.
    pub(super) fn answered(&mut self, answer: Answered) {
        let split = |items: &[(PhotoId, Option<Left>)]| {
            let done: Vec<PhotoId> = items
                .iter()
                .filter(|item| item.1.is_none())
                .map(|item| item.0)
                .collect();
            let left: Vec<(PhotoId, Left)> = items.iter().filter_map(|(id, why)| why.map(|why| (*id, why))).collect();
            (done, left)
        };
        match answer {
            Answered::Trashed(items) => {
                let (done, left) = split(&items);
                let open = self
                    .later
                    .open_trash
                    .take_if(|(was, _)| items.iter().any(|item| item.0 == was.id));
                for id in &done {
                    self.selected.remove(id);
                }
                if self.selecting && self.selected.is_empty() && !done.is_empty() {
                    self.end_selecting();
                }
                if !done.is_empty() {
                    self.later.undo = done.clone();
                    self.toast = Some(Toast::Trashed(done.clone()));
                }
                match open {
                    // Still on the stage: it leaves now, as the board draws it.
                    Some((was, next))
                        if done.contains(&was.id)
                            && self
                                .viewer
                                .as_ref()
                                .is_some_and(|viewer| viewer.id == was.id && viewer.gone.is_none()) =>
                    {
                        self.left_the_stage(was, next);
                    }
                    _ => {
                        // An open photo that is no longer there (moved from the grid's menu, or stepped to).
                        if self
                            .viewer
                            .as_ref()
                            .is_some_and(|viewer| viewer.gone.is_none() && done.contains(&viewer.id))
                        {
                            self.viewer = None;
                        }
                    }
                }
                self.say_left(What::Trash, &left);
            }
            Answered::Restored(items) => {
                let (done, left) = split(&items);
                if let Some(viewer) = &mut self.viewer
                    && let Some(was) = viewer.gone.take_if(|was| done.contains(&was.id))
                {
                    viewer.id = was.id;
                }
                // Not put back: still in Trash, and still what Undo is for.
                self.later.undo.extend(left.iter().map(|item| item.0));
                self.say_left(What::Restore, &left);
            }
            Answered::Kept { id, keep, left } => match left {
                Some(why) => {
                    self.toast = Some(Toast::Left {
                        what: What::Keep(keep),
                        why,
                        count: 1,
                        again: vec![id],
                    });
                }
                None if keep && self.viewer.as_ref().is_some_and(|viewer| viewer.id == id) => {
                    self.toast = Some(Toast::Kept);
                }
                None => {}
            },
        }
        self.relayout();
    }

    /// One message per answer: where some were left, that is the one (Ctrl+Z still undoes the rest). It counts and
    /// gives the reason of what asking again can change; where every one left is protected, of those.
    fn say_left(&mut self, what: What, left: &[(PhotoId, Left)]) {
        let again: Vec<(PhotoId, Left)> = left.iter().copied().filter(|item| item.1 != Left::Protected).collect();
        let Some(first) = again.first().or(left.first()) else {
            return;
        };
        self.toast = Some(Toast::Left {
            what,
            why: first.1,
            count: if again.is_empty() { left.len() } else { again.len() },
            again: again.iter().map(|item| item.0).collect(),
        });
    }

    /// What the group asked of the shell, on a source that answers later: what this source can do is done, and a
    /// control whose work is not connected in this build says so instead of doing nothing.
    fn take_in_intents(&mut self) -> bool {
        let intents = self.take_intents();
        let any = !intents.is_empty();
        for intent in intents {
            match intent {
                // "On this PC", pressed again: it stops being kept (asked; said once this PC answered).
                Intent::OfferRemoveFromPc(id) => self.keep(id, false),
                Intent::KeepSeveral(ids) => {
                    for id in ids {
                        self.keep(id, true);
                    }
                }
                // Kept in this window for as long as it is open.
                Intent::RememberAcross(_) | Intent::StopOriginal(_) => {}
                Intent::DownloadOriginal(_) => {
                    // No download was started: the tab does not say one is running.
                    if let Some(viewer) = &mut self.viewer {
                        viewer.original = Original::Preview;
                    }
                    self.toast = Some(Toast::NotYet);
                }
                _ => self.toast = Some(Toast::NotYet),
            }
        }
        any
    }

    /// Nothing that names a photo of the account that was here stays.
    fn end_session(&mut self) {
        self.source.forget();
        self.selecting = false;
        self.selected.clear();
        self.cursor = None;
        self.menu = None;
        self.dialog = None;
        self.viewer = None;
        self.toast = None;
        self.filter = None;
        self.later.open_trash = None;
        self.later.undo.clear();
        self.relayout();
    }
}

impl Toast {
    /// The words of a message that says something was left.
    pub(super) fn left_words(what: What, why: Left, count: usize) -> String {
        if why == Left::InFlight {
            return "Still on its way \u{00B7} nothing was sent twice".into();
        }
        let several = count > 1;
        let base = match what {
            What::Trash if several => format!("Couldn\u{2019}t move {count} to Trash"),
            What::Trash => "Couldn\u{2019}t move to Trash".into(),
            What::Restore if several => format!("Couldn\u{2019}t put {count} back"),
            What::Restore => "Couldn\u{2019}t put it back".into(),
            What::Keep(true) => "Couldn\u{2019}t keep it on this PC".into(),
            What::Keep(false) => "Couldn\u{2019}t remove it from this PC".into(),
        };
        let reason = match why {
            Left::Offline => " \u{00B7} you\u{2019}re offline",
            Left::Protected => " \u{00B7} backups can\u{2019}t be changed here",
            Left::Unavailable => " \u{00B7} not available right now",
            Left::Failed | Left::InFlight => "",
        };
        base + reason
    }

    /// Whether the message offers Try again.
    pub(super) fn can_retry(&self) -> bool {
        matches!(self, Toast::Left { why, again, .. } if !again.is_empty() && *why != Left::InFlight)
    }
}

/// The status line's words for an open item. A video on a source that answers later: playing it here is not
/// connected in this build, and the line says so instead of "Streaming".
pub(super) fn status(ui: &Ui, viewing: &Viewing) -> String {
    if ui.source.answers_later() && matches!(viewing.kind, super::source::Kind::Video(_)) {
        return "Video \u{00B7} playing it here is not available in this version yet".into();
    }
    super::viewer::status(viewing)
}

/// A sync check finished (`shell::host::poll`): what this PC knows of the account may have changed.
pub fn after_a_check(s: &mut State) {
    if s.shell.app.gate() == GateState::Ready {
        s.photos.source.refresh();
    }
}

/// The words in the library's place while there is none (`None`: there is one, draw it).
pub(super) fn not_here(loaded: Loaded) -> Option<(&'static str, &'static str)> {
    match loaded {
        Loaded::Ready => None,
        Loaded::Loading => Some((
            "Loading your photos\u{2026}",
            "Reading what this PC knows of your Beebeeb.",
        )),
        Loaded::SignedOut => Some(("Signed out", "Sign in to see your photos.")),
        Loaded::SessionEnded => Some(("Your session ended", "Sign in again to see your photos.")),
        Loaded::NothingSynced => Some((
            "Nothing has synced to this PC yet",
            "Photos shows your photos and videos once this PC has synced with Beebeeb.",
        )),
        Loaded::NotThisAccounts => Some((
            "This PC holds another account\u{2019}s files",
            "Photos shows your photos once this PC has synced with your account.",
        )),
        Loaded::Failed => Some((
            "Couldn\u{2019}t load your photos",
            "Open Photos again in a moment to try again.",
        )),
    }
}

/// The library's place while there is none: what is going on, in words. `None`: there is a library; draw it.
pub(super) fn not_here_body(ui: &Ui, t: &Theme) -> Option<El> {
    let (title, words) = not_here(ui.source.loaded())?;
    let mut body = col().key("ph-body").grow(1.0).center().child(
        col()
            .key("ph-not-here")
            .role("status")
            .items_center()
            .gap(8.0)
            .px(24.0)
            .py(28.0)
            .child(kit::label("ph-not-here-title", title, style::TITLE, t.text).role("heading"))
            .child(
                kit::label("ph-not-here-words", words, style::SUB, t.text_tertiary)
                    .w(260.0)
                    .wrap()
                    .align(rune::Align::Center),
            ),
    );
    if let Some(toast) = &ui.toast {
        body = body.child(super::overlay::toast_layer(toast, 24.0, true, t));
    }
    Some(body)
}

/// The window's poll (`shell::host::poll`): a gate or account change drops everything that names a photo at once,
/// on the window's side; the Photos section coming on screen asks for the library (the source decides whether it
/// is old); what arrived is taken in. `true`: something drawn changed.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let gate = (s.shell.app.gate(), s.shell.app.account().map(str::to_owned));
    let open = s.sidebar_section == crate::ui::sidebar::SidebarSection::Photos && gate.0 == GateState::Ready;
    // Asked of the shell's gate with the engine as it is now; kept for the presses until the next poll.
    let closed = !crate::shell::session_gate::open(s);
    let ui = &mut s.photos;
    ui.later.closed = closed && ui.source.answers_later();
    let mut changed = false;
    if ui.later.seen.as_ref() != Some(&gate) {
        if ui.later.seen.is_some() {
            ui.end_session();
            // The section must open again on the next poll so a new read starts.
            ui.later.was_open = false;
            changed = true;
        }
        ui.later.seen = Some(gate);
    }
    if open && !ui.later.was_open {
        ui.source.opened();
        changed = true;
    }
    ui.later.was_open = open;
    let later = ui.source.answers_later();
    if later {
        changed |= ui.take_in_intents();
    }
    let pumped = ui.source.pump();
    if pumped.busy {
        tx.wake_after(POLL_EVERY);
    }
    changed |= pumped.changed || !pumped.answered.is_empty();
    if pumped.changed {
        ui.relayout();
    }
    // What the changes did, first: an open photo that Beebeeb moved to Trash leaves the stage its own way.
    for answer in pumped.answered {
        ui.answered(answer);
    }
    if pumped.changed && later {
        // An open photo that is no longer in the library (it left some other way) closes, and a checked one is
        // no longer checked: a change asked for it would be refused for all of them.
        if ui
            .viewer
            .as_ref()
            .is_some_and(|viewer: &Viewer| viewer.gone.is_none() && ui.source.viewing(viewer.id).is_none())
        {
            ui.viewer = None;
        }
        let Ui { selected, source, .. } = ui;
        selected.retain(|id, _| source.viewing(*id).is_some());
    }
    // On screen with nothing read and nothing on its way (the session moved on under an unchanged gate): read.
    if open && later && !pumped.busy && ui.source.loaded() == Loaded::Loading {
        ui.source.opened();
        changed = true;
    }
    changed
}
