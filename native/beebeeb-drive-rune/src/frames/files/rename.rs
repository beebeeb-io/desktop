//! Rename where the name is, as File Explorer does (DW-0410), and the name that is already used
//! (DW-0410b).
//!
//! F2, or Rename in the file menu, turns the selected row's name into a field: the name without
//! its extension selected, so typing keeps `.pdf`. Enter or a click elsewhere saves (a call on
//! `FilesSource::rename`, after `names::check_on`), Esc cancels. A name that cannot be given keeps
//! the field open in its error style and says why in a tip under it; the old name stays until
//! another one is typed, or Esc.

use super::names::{self, NameError};
use super::source::{Action, ActionError};
use crate::State;
use crate::theme::icons_files;
use crate::theme::tokens::Theme;
use crate::ui::{field, kit};
use rune::{Align, El, Key, KeyEvent, Side, TextEdit, Tx, el, row, text_input};

/// The key of the rename field's text input.
pub const RENAME_INPUT: &str = "rename-input";

/// Which row is being renamed. The typed name is `Actions::name_edit`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rename {
    pub row: usize,
    pub id: String,
}

impl State {
    /// Rename (F2): the selected row's name becomes a field. One row only; Backups, a service
    /// folder and what is inside them cannot be renamed (DW-0425 says why).
    pub fn begin_rename(&mut self, tx: &mut Tx) -> bool {
        if self.needs_session(tx) {
            return true;
        }
        let rows = self.selected_rows();
        let [row] = rows[..] else { return false };
        if self.protects(std::slice::from_ref(&self.listing.items[row])) {
            self.failed(Action::Rename, ActionError::Protected, tx);
            return true;
        }
        self.open_rename(row);
        tx.focus(RENAME_INPUT, false);
        true
    }

    /// The name of this row as a field, the name without its extension selected.
    pub fn open_rename(&mut self, row: usize) {
        let Some(item) = self.listing.items.get(row).cloned() else {
            return;
        };
        let mut edit = TextEdit::new(item.name.clone());
        edit.set_selection(0, names::stem_len(&item.name, item.is_folder));
        self.actions.name_edit = edit;
        self.actions.name_error = None;
        self.actions.menu = None;
        self.actions.rename = Some(Rename { row, id: item.id });
    }

    /// The typed name against the folder's other names: the name as it would be stored, or why
    /// it cannot be given (which the field then shows).
    pub fn check_rename(&mut self) -> Option<String> {
        let rename = self.actions.rename.clone()?;
        let typed = self.actions.name_edit.text.clone();
        let others = self
            .listing
            .items
            .iter()
            .filter(|other| other.id != rename.id)
            .map(|other| (other.name.as_str(), other.is_folder));
        match names::check_on(self.actions.host, &typed, others) {
            Ok(name) => Some(name),
            Err(why) => {
                self.actions.name_error = Some((typed, why));
                None
            }
        }
    }

    /// Esc: the old name stays.
    pub fn cancel_rename(&mut self, tx: &mut Tx) {
        if self.actions.rename.take().is_some() {
            self.actions.name_error = None;
            self.focus_list(tx);
        }
    }

    /// Why the typed name cannot be given, while it is still the name that was refused.
    pub fn rename_error(&self) -> Option<&NameError> {
        self.actions
            .name_error
            .as_ref()
            .filter(|(typed, _)| typed == &self.actions.name_edit.text)
            .map(|(_, why)| why)
    }

    /// Enter, or a click elsewhere: save the new name. Returns whether the field closed.
    pub fn commit_rename(&mut self, tx: &mut Tx) -> bool {
        let Some(rename) = self.actions.rename.clone() else {
            return true;
        };
        let Some(row) = self.listing.items.iter().position(|i| i.id == rename.id) else {
            // The row went away under the field (a change from another device).
            self.actions.rename = None;
            return true;
        };
        let item = self.listing.items[row].clone();
        let typed = self.actions.name_edit.text.clone();
        if typed.trim() == item.name {
            self.cancel_rename(tx);
            return true;
        }
        let Some(name) = self.check_rename() else {
            return false;
        };
        match self.actions.source.rename(&item, &name) {
            Ok(()) => {
                self.listing.items[row].name = name;
                self.actions.rename = None;
                self.actions.name_error = None;
                self.forget_undo();
                // The row keeps its place in the folder's order, and stays the selected one.
                self.select_only(row);
                self.apply_sort();
                self.focus_list(tx);
                true
            }
            Err(ActionError::NameTaken) => {
                let taken = NameError::Taken {
                    name: name.clone(),
                    folder: item.is_folder,
                };
                self.actions.name_error = Some((typed, taken));
                false
            }
            Err(why) => {
                self.actions.rename = None;
                self.failed(Action::Rename, why, tx);
                true
            }
        }
    }
}

/// The field's own keys, before the editor: Esc cancels. (Enter is the input's `on_submit`.)
fn input_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    match ev.key {
        Key::Escape => {
            s.cancel_rename(tx);
            true
        }
        // F2 again does nothing, also while the name is taken (DW-0410b).
        Key::Function(2) => true,
        _ => false,
    }
}

/// The name of a row as a field (the board's inline field of `.mk-name`): paper, an Amber ring
/// and a 1 pt Amber-deep line inside; red ones when the name cannot be given.
/// Keys: `rename-field`, `rename-text`, `rename-caret`; with an error also `tip`, `tip-icon`, `tip-text`.
pub fn name_field(s: &State, t: &Theme) -> El {
    let edit = &s.actions.name_edit;
    let error = s.rename_error();
    let (from, to) = edit.range();
    // The caret the board draws takes 3.5 pt of the field's width (1.5 and a point each side).
    let width = field::advance(&edit.text, field::VALUE) + 3.5 + 8.0;
    kit::note_text("rename-text", "rename-field", &edit.text, field::VALUE, t.text);
    let (ring, line) = match error {
        Some(_) => (kit::ring(3.0, t.red.alpha(0.22)), t.red),
        None => (kit::ring(2.0, t.accent.alpha(0.40)), t.accent_deep),
    };
    let mut input = text_input(edit, field::VALUE)
        .key(RENAME_INPUT)
        .label("Name")
        .single_line()
        .color(t.text)
        .caret(t.accent_deep, field::selection(t))
        .selection_style(rune::SelectionStyle::new(field::selection(t)))
        .invalid(error.is_some())
        .on_edit(|s: &mut State| &mut s.actions.name_edit)
        .on_submit(|s: &mut State, tx: &mut Tx| {
            s.commit_rename(tx);
        })
        .on_input_key(input_key)
        .grow(1.0)
        .min_w(0.0);
    if let Some(why) = error {
        input = input.description(why.message());
    }
    let mut well = row()
        .key("rename-field")
        .w(width.max(40.0))
        .h(22.0)
        // `margin: 0 -4px`: the text stays where the name was.
        .mx(-4.0)
        .px(4.0)
        .center_y()
        .radius(4.0)
        .fill(t.card)
        .box_shadow(ring)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: line,
            inset: true,
        })
        // A click elsewhere saves; Esc is the input's own key (`input_key`), so what the modal
        // hears as a dismissal is that click.
        .modal_tx(true, |s: &mut State, tx: &mut Tx| {
            s.commit_rename(tx);
        })
        .child(input);
    if edit.focused && from != to {
        // Windows draws the caret at the end of a selection.
        let at = field::advance(&edit.text[..to.max(from)], field::VALUE);
        well = well.child(
            // `.kit-caret` in a row: 14 pt, its foot 3 pt under the text line's (`margin: 0 1px -3px`).
            el().key("rename-caret-at")
                .a11y_hidden()
                // Between the two letters, where a text field's caret stands: it covers neither.
                .absolute(field::snap(4.0 + at - 0.5), 5.5)
                .child(field::caret_mark("rename-caret".to_string(), 14.0, t)),
        );
    }
    // Where the tip of an error hangs from (`tip_layer`): 4 pt left of the field.
    well = well.child(el().key("tip-anchor").a11y_hidden().absolute(-4.0, 0.0).w(1.0).h_full());
    well
}

/// The reason the typed name cannot be given (`.mk-tip`, DW-0410b): under the row, from the
/// field's left edge. A layer of the window's root, so no list clips it.
pub fn tip_layer(s: &State, t: &Theme) -> Option<El> {
    s.actions.rename.as_ref()?;
    let why = s.rename_error()?;
    Some(field::tip("tip", &icons_files::ALERT, &why.message(), t).anchored(
        "tip-anchor",
        Side::Bottom,
        Align::Start,
        6.0,
    ))
}
