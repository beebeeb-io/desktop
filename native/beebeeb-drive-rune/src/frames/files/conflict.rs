//! Two versions (DW-0429): a file changed in two places at once, and nothing is overwritten
//! until the person picks.
//!
//! The route is the shell's (`shell::app::Dialog::Conflict`; the flyout's Review… opens it, and
//! so does opening the row whose date reads "Two versions"); the shell draws the scrim and the
//! window behind, this file is the card (`shell::registry::DIALOGS`).
//!
//! **Nothing is lost, whatever is pressed.** Cancel and Esc ask nothing of the source: both
//! versions stay and the file keeps waiting. Each Keep asks once (`FilesSource::keep_version`);
//! the source keeps the version that is not the file in the file's version history, or beside
//! it under the device's name (Keep both). The next waiting file follows.

use super::actions_data::{Conflict, Device, Keep, Side};
use super::source::Action;
use crate::State;
use crate::shell::app::{Dialog, Event};
use crate::shell::host;
use crate::shell::registry::{DialogFrame, FeedbackPolicy, FrameGroup};
use crate::theme::icons_settings;
use crate::theme::tokens::{self, Theme, type_};
use crate::ui::settings_dialog::{self, Weight};
use crate::ui::{button, dialog, kit};
use rune::{Color, Cx, El, Tx, col, el, row};

/// What the list says in the date column of a file that waits for a choice.
pub const ROW_WORDS: &str = "Two versions";

/// The registry's line (`shell::registry::DIALOGS`). The card names a file and its devices:
/// comment mode stays off.
pub const DIALOG: DialogFrame = DialogFrame {
    group: FrameGroup::ConflictDialog,
    view: card,
    on_key: None,
    feedback: FeedbackPolicy::Denied("it names one of your files and your devices"),
};

/// The file the open dialog is about, its place in the queue and the queue's length: "1 of 2".
/// Files answered since the dialog opened count, so the second reads "2 of 2".
pub fn shown(s: &State) -> Option<(Conflict, usize, usize)> {
    let Some(Dialog::Conflict { file }) = s.shell.app.dialog() else {
        return None;
    };
    let waiting = s.actions.source.conflicts();
    let conflict = match file {
        Some(id) => waiting.iter().find(|c| &c.file == id),
        None => waiting.first(),
    }?
    .clone();
    let done = s.actions.conflicts_done;
    Some((conflict, done + 1, done + waiting.len()))
}

/// Does this item wait for a choice?
pub fn waits(s: &State, id: &str) -> bool {
    s.actions.source.conflicts().iter().any(|c| c.file == id)
}

impl State {
    /// Opening a file that waits for a choice opens the choice instead. `false`: the selection
    /// is not one such file.
    pub fn review_selected_conflict(&mut self) -> bool {
        let id = match &self.selected_items()[..] {
            [only] if !only.is_folder && waits(self, &only.id) => only.id.clone(),
            _ => return false,
        };
        self.actions.conflicts_done = 0;
        host::dispatch(self, Event::OpenDialog(Dialog::Conflict { file: Some(id) }));
        true
    }

    /// Cancel: the dialog closes; both versions stay and the file keeps waiting.
    pub fn leave_conflict(&mut self, tx: &mut Tx) {
        self.actions.conflicts_done = 0;
        if matches!(self.shell.app.dialog(), Some(Dialog::Conflict { .. })) {
            host::dispatch(self, Event::Escape);
            self.focus_list(tx);
        }
    }

    /// One of the three answers: asked of the source once. Then the next waiting file, or the
    /// window. A refusal changes nothing and says so (DW-0109).
    pub fn keep_version(&mut self, keep: Keep, tx: &mut Tx) {
        let Some((conflict, _, _)) = shown(self) else {
            return;
        };
        if let Err(why) = self.actions.source.keep_version(&conflict.file, keep) {
            self.leave_conflict(tx);
            self.failed(Action::KeepVersion, why, tx);
            return;
        }
        self.forget_undo();
        // Keep both made a file beside this one: it stands in the rows, selected.
        self.take_changes();
        self.actions.conflicts_done += 1;
        match self.actions.source.conflicts().first() {
            Some(next) => {
                // One dialog at a time: this one closes, the next file's opens.
                let file = Some(next.file.clone());
                host::dispatch(self, Event::Escape);
                host::dispatch(self, Event::OpenDialog(Dialog::Conflict { file }));
            }
            None => self.leave_conflict(tx),
        }
    }
}

/// `.mk-page` with five lines, as the board draws a version's first page.
fn page(key: &str, t: &Theme) -> El {
    let inner = 96.0 - 2.0 * 10.0;
    let mut page = col()
        .key(format!("{key}-page"))
        .size(96.0, 124.0)
        .radius(4.0)
        .fill(t.white)
        .box_shadow(kit::drop(2.0, 10.0, t.shadow_page))
        .py(12.0)
        .px(10.0)
        .gap(5.0)
        .child(
            el().key(format!("{key}-line-0"))
                .h(6.0)
                .w(inner * 0.6)
                .radius(2.0)
                .fill(Color::BLACK.alpha(0.30))
                .mb(3.0),
        );
    for (i, w) in [1.0, 1.0, 0.7, 1.0, 1.0].iter().enumerate() {
        page = page.child(
            el().key(format!("{key}-line-{}", i + 1))
                .h(3.0)
                .w(inner * w)
                .radius(2.0)
                .fill(t.shadow_page),
        );
    }
    page
}

/// One version (`.kit-card`): its first page, the device, when and how large.
fn side(key: &str, side: &Side, t: &Theme) -> El {
    let mark = match side.kind {
        Device::Computer => &icons_settings::LAPTOP,
        Device::Tablet => &icons_settings::TABLET,
        Device::Phone => &icons_settings::PHONE,
    };
    col()
        .key(key.to_string())
        .role("group")
        .label(format!("{}, {}, {}", side.device, side.when, side.size))
        // Two side by side in the 512 pt body, 12 pt between them.
        .w(250.0)
        .padding(12.0)
        .gap(10.0)
        .radius(10.0)
        .fill(t.card)
        .box_shadow(kit::drop(1.0, 2.0, Color::BLACK.alpha(0.04)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            el().key(format!("{key}-well"))
                .w_full()
                .py(12.0)
                .center()
                .radius(8.0)
                .fill(t.text.alpha(0.05))
                .child(page(key, t)),
        )
        .child(
            row()
                .key(format!("{key}-device-row"))
                .center_y()
                .gap(8.0)
                .child(kit::glyph(format!("{key}-icon"), mark, 15.0, t.text))
                .child(kit::label(format!("{key}-device"), side.device.clone(), type_::HEAD, t.text).truncate()),
        )
        .child(kit::label(
            format!("{key}-when"),
            format!("{} \u{00B7} {}", side.when, side.size),
            type_::KBD,
            t.text_tertiary,
        ))
}

/// The dialog's card. The shell draws the scrim and the window behind it.
pub fn card(_dialog: &Dialog, s: &State, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());
    let Some((conflict, at, of)) = shown(s) else {
        // The file was answered somewhere else, or nothing is connected: say so, change nothing.
        let foot = row().center_y().justify_end().child(
            button::outline_button("btn-0", "Close", None, &t)
                .default_button()
                .on_press_tx(|s: &mut State, tx: &mut Tx| s.leave_conflict(tx)),
        );
        return dialog::card(
            "Two versions",
            420.0,
            "Two versions",
            Some(dialog::Sub::Wrap("No file is waiting for a choice.")),
            Vec::new(),
            foot,
            &t,
        )
        .mt(crate::TITLEBAR_H);
    };
    let sub = format!("{at} of {of} \u{00B7} {} changed in two places at once", conflict.name);
    let body = vec![
        row()
            .key("versions-pair")
            .w_full()
            .gap(12.0)
            .child(side("version-here", &conflict.here, &t))
            .child(side("version-there", &conflict.there, &t)),
        kit::label(
            "dlg-text-0",
            "Keep both saves the other one next to it with the device's name added. Either way, the other version stays in version history.",
            type_::CAPTION,
            t.text_tertiary,
        )
        .wrap(),
    ];
    // "Keep this PC's", "Keep Anna's iPad's".
    let keep_here = if conflict.here.device == "This PC" {
        "Keep this PC's".to_string()
    } else {
        format!("Keep {}'s", conflict.here.device)
    };
    let keep_there = format!("Keep {}'s", conflict.there.device);
    let foot = col()
        .child(
            settings_dialog::button(0, "Keep both", Weight::Primary, &t)
                .w_full()
                .default_button()
                .on_press_tx(|s: &mut State, tx: &mut Tx| s.keep_version(Keep::Both, tx)),
        )
        // `.flex-1` between the default and the other answers: one more gap.
        .child(el().key("dlg-foot-gap").h(0.0))
        .child(
            button::outline_button("btn-1", &keep_here, None, &t)
                .w_full()
                .on_press_tx(|s: &mut State, tx: &mut Tx| s.keep_version(Keep::Here, tx)),
        )
        .child(
            button::outline_button("btn-2", &keep_there, None, &t)
                .w_full()
                .on_press_tx(|s: &mut State, tx: &mut Tx| s.keep_version(Keep::There, tx)),
        )
        .child(
            settings_dialog::button(3, "Cancel", Weight::Quiet, &t)
                .w_full()
                .on_press_tx(|s: &mut State, tx: &mut Tx| s.leave_conflict(tx)),
        );
    // The shell centres a dialog in the whole window; the board centres this one in the area
    // under the caption (as `tray::shortcuts` places its card): the caption's height above it.
    dialog::card(
        "Two versions",
        560.0,
        "Two versions",
        Some(dialog::Sub::Truncate(&sub)),
        body,
        foot,
        &t,
    )
    .mt(crate::TITLEBAR_H)
}
