//! The one unsaved-changes question for every path that ends an open editor: the sign-outs (Security's, all
//! apps, the no-plan gate's) and the quit (the b's menu, a close with no b, the sync-off dialog's Quit).
//! Save (then goes on), Discard and continue (the only way the edits are dropped), Cancel (Enter presses Save).
//!
//! The sign-out of this app is asked by the engine (it refuses with `UnsavedEdits`) and drawn by Security's
//! confirm; both use [`question`] and [`save_open_editor`], so the words, the buttons and the Save are one.

use rune::{Cx, El, Tx};

use crate::State;
use crate::theme::tokens::{self, Theme};
use crate::ui::account_set::{self as set, Look, Size};

/// What goes on once the question is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Then {
    /// The app quits.
    Quit,
    /// The no-plan screen's sign-out.
    GateSignOut,
}

impl Then {
    fn title(self) -> &'static str {
        match self {
            Then::Quit => "Quit",
            Then::GateSignOut => "Sign out",
        }
    }
    fn discard(self) -> &'static str {
        match self {
            Then::Quit => "Discard and quit",
            Then::GateSignOut => "Discard and sign out",
        }
    }
    fn nothing_done(self) -> &'static str {
        match self {
            Then::Quit => "Nothing was closed.",
            Then::GateSignOut => "Nothing was signed out.",
        }
    }
}

/// The question that is up over the window.
#[derive(Clone, Debug)]
pub(crate) struct Leave {
    pub then: Then,
    pub name: String,
    /// Save was pressed: waiting for the editor's save.
    pub saving: bool,
    /// The save did not land.
    pub refused: bool,
}

/// What the answer to a question does (the buttons' presses).
pub(crate) type Press = fn(&mut State, &mut Tx);

/// The open document holds changes no save has landed.
pub(crate) fn holds(s: &State) -> bool {
    s.viewer.ui.as_ref().is_some_and(|ui| {
        ui.dirty()
            || ui
                .office
                .edit
                .as_ref()
                .is_some_and(|e| !e.stopped && e.unsaved())
    })
}

/// The open document's name for the question.
pub(crate) fn name(s: &State) -> String {
    s.viewer
        .ui
        .as_ref()
        .map(|ui| ui.file.name.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "an open document".to_string())
}

/// The editor's own save. `true`: a save is under way (an office editor answers later) or landed (the text editor
/// answers at once); `false`: nothing was asked.
pub(crate) fn save_open_editor(s: &mut State) -> bool {
    s.viewer.ui.as_mut().is_some_and(|ui| {
        if ui.dirty() {
            ui.save();
            true
        } else {
            ui.office_save();
            ui.office.edit.is_some()
        }
    })
}

/// While Save waits for the editor. `Some(true)`: it landed, go on; `Some(false)`: it did not; `None`: still going.
pub(crate) fn save_state(s: &State) -> Option<bool> {
    let office_saving = s
        .viewer
        .ui
        .as_ref()
        .and_then(|ui| ui.office.edit.as_ref())
        .is_some_and(|e| e.saving);
    if office_saving {
        return None;
    }
    Some(!holds(s))
}

/// The question: "You have unsaved changes in <name>." with Save, `discard` and Cancel.
pub(crate) fn question(
    title: &str,
    words: &str,
    saving: bool,
    discard: &str,
    on_save: Press,
    on_discard: Press,
    on_cancel: Press,
    t: &Theme,
) -> El {
    let mut save_b = set::button("btn-1", "Save", None, Look::Primary, Size::Medium, saving, t);
    let mut discard_b = set::button("btn-2", discard, None, Look::Danger, Size::Medium, saving, t)
        .label(format!("{discard}, destructive"));
    let mut cancel_b = set::button("btn-3", "Cancel", None, Look::Plain, Size::Medium, saving, t);
    if !saving {
        save_b = save_b.on_press_tx(on_save);
        discard_b = discard_b.on_press_tx(on_discard);
        cancel_b = cancel_b.on_press_tx(on_cancel);
    }
    let foot = vec![(save_b, false), (discard_b, false), (cancel_b, true)];
    set::dialog_enter(title, 440.0, title, words, Vec::new(), foot, Some(0), t).modal_tx(false, move |s: &mut State, tx: &mut Tx| {
        if !saving {
            on_cancel(s, tx);
        }
    })
}

/// What the question says.
pub(crate) fn words(name: &str, saving: bool) -> String {
    if saving {
        format!("Saving {name}. Nothing is closed until it is saved.")
    } else {
        format!("You have unsaved changes in {name}.")
    }
}

/// An end of the editor was asked for. `true`: there are unsaved changes, so the question is up over the window and
/// the caller stops; `false`: nothing is unsaved, the caller goes on.
pub(crate) fn ask(s: &mut State, then: Then) -> bool {
    if !holds(s) {
        return false;
    }
    if s.viewer.leave.is_none() {
        s.viewer.leave = Some(Leave {
            then,
            name: name(s),
            saving: false,
            refused: false,
        });
    }
    s.shell.open_main();
    true
}

/// The ending goes on (Save landed, or Discard).
fn proceed(s: &mut State, tx: &mut Tx, then: Then, discard: bool) {
    s.viewer.leave = None;
    match then {
        Then::Quit => tx.quit(),
        Then::GateSignOut => crate::frames::signin::before_files::sign_out_now(s, discard),
    }
}

fn on_save(s: &mut State, _tx: &mut Tx) {
    let asked = save_open_editor(s);
    if let Some(leave) = s.viewer.leave.as_mut() {
        if asked {
            leave.saving = true;
        } else {
            leave.refused = true;
        }
    }
}

fn on_discard(s: &mut State, tx: &mut Tx) {
    if let Some(then) = s.viewer.leave.as_ref().map(|l| l.then) {
        proceed(s, tx, then, true);
    }
}

fn on_cancel(s: &mut State, _tx: &mut Tx) {
    s.viewer.leave = None;
}

/// Where the keyboard goes as the question closes, when what opened it (the b's menu row, the tray, a sign-out
/// button behind a gate) is gone with it: the text editor's field, else the office document's surface. An opener
/// that stays gets focus back from the modal itself.
pub(crate) fn back_key(s: &State) -> &'static str {
    match s.viewer.ui.as_ref() {
        Some(ui) if ui.editor.is_some() => super::EDITOR_KEY,
        Some(ui) => super::office_over::doc_key(&ui.file.name),
        None => super::EDITOR_KEY,
    }
}

/// The window with the question over it, while it is up; else `behind` as it is.
pub(crate) fn over(s: &State, cx: &mut Cx<State>, behind: El) -> El {
    let Some(leave) = s.viewer.leave.as_ref() else {
        return behind;
    };
    let t = tokens::theme(cx.system_dark());
    let words = if leave.refused {
        format!("The changes were not saved. {}", leave.then.nothing_done())
    } else {
        words(&leave.name, leave.saving)
    };
    let card = question(
        leave.then.title(),
        &words,
        leave.saving,
        leave.then.discard(),
        on_save,
        on_discard,
        on_cancel,
        &t,
    )
    .focus_return(back_key(s));
    rune::el()
        .key("window-with-unsaved")
        .w_full()
        .h_full()
        .child(behind)
        .child(rune::el().key("dialog-unsaved").pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).child(card))
}

/// The poll: the question goes with its editor; while Save waits, the ending follows the save.
pub(crate) fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let Some(leave) = s.viewer.leave.clone() else {
        return false;
    };
    if !leave.saving {
        if !holds(s) {
            s.viewer.leave = None;
            return true;
        }
        return false;
    }
    match save_state(s) {
        None => {
            tx.wake_after(std::time::Duration::from_millis(100));
            false
        }
        Some(true) => {
            proceed(s, tx, leave.then, false);
            true
        }
        Some(false) => {
            if let Some(l) = s.viewer.leave.as_mut() {
                l.saving = false;
                l.refused = true;
            }
            true
        }
    }
}
