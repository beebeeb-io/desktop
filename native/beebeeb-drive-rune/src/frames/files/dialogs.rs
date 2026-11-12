//! The two dialogs of the Files frames: New folder (DW-0417) and Move to (DW-0409).
//!
//! A dialog sits in the middle of the window under the caption, on a scrim; it is modal: Tab
//! stays inside it, Esc and Cancel close it, Return presses its amber button. What it does is a
//! call on `FilesSource` (`new_folder`, `move_to`); a name is checked by `names::check_on` first.
//!
//! The card and the scrim are `ui::dialog`'s, the ones `ui::settings_dialog::dialog` draws;
//! `shell` adds what the Files window needs around them: the scrim under the caption bar, a
//! footer that can stack its buttons (`.wn-dlg-foot.is-stack`, DW-0409: the caller's `col()`),
//! and the modal layer with the dialog's own key.

use super::names::{self, NameError};
use super::source::{Action, ActionError};
use crate::State;
use crate::source::{Crumb, DriveItem};
use crate::theme::tokens::{Theme, radius};
use crate::theme::{icons, icons_settings};
use crate::ui::settings_dialog::{self, Weight};
use crate::ui::settings_list::VALUE_MONO;
use crate::ui::settings_shell::SUB;
use crate::ui::{button, dialog, field, kit};
use rune::{Color, El, KeyEvent, TextEdit, TextStyle, Tx, col, el, row};

/// The key of the New folder dialog's text input.
pub const NAME_INPUT: &str = "new-folder-name";

/// Which dialog is open.
#[derive(Clone, Debug)]
pub enum Dialog {
    NewFolder(NewFolder),
    Move(MoveTo),
}

/// New folder (DW-0417): where it goes. The name is `Actions::name_edit`.
#[derive(Clone, Debug)]
pub struct NewFolder {
    /// The folder it goes into (`None`: the root), and that folder's name.
    pub parent: Option<String>,
    pub parent_name: String,
    /// Asked from the Move dialog: Create goes back to it, with the new folder chosen.
    pub back_to: Option<Box<MoveTo>>,
}

/// Move to (DW-0409): what moves, the folder the dialog shows, and the one chosen in it.
#[derive(Clone, Debug)]
pub struct MoveTo {
    pub items: Vec<DriveItem>,
    /// The way to the folder shown, the root first.
    pub path: Vec<Crumb>,
    /// The folders offered: no Backups, no service folder, none of the folders being moved.
    pub choices: Vec<DriveItem>,
    /// The chosen one. `None`: the items go into the folder shown.
    pub picked: Option<usize>,
    /// Why the last Move did not happen (a name already used there).
    pub error: Option<String>,
}

impl MoveTo {
    /// The folder shown.
    pub fn here(&self) -> &Crumb {
        // `path` always holds the root.
        &self.path[self.path.len() - 1]
    }

    /// Where Move puts the items: the chosen folder, else the folder shown.
    pub fn target(&self) -> Crumb {
        match self.picked.and_then(|i| self.choices.get(i)) {
            Some(folder) => Crumb {
                id: Some(folder.id.clone()),
                name: folder.name.clone(),
            },
            None => self.here().clone(),
        }
    }

    /// The items are there already: nothing to move.
    pub fn is_where_they_are(&self) -> bool {
        let target = self.target().id;
        self.items.iter().all(|item| item.parent_id == target)
    }

    /// "Contract draft.pdf", or "3 items".
    pub fn subject(&self) -> String {
        match &self.items[..] {
            [one] => one.name.clone(),
            many => format!("{} items", many.len()),
        }
    }
}

/// "48 items", "1 item".
fn item_count(folder: &DriveItem) -> String {
    match folder.child_count {
        Some(1) => "1 item".to_string(),
        Some(n) => format!("{n} items"),
        None => String::new(),
    }
}

impl State {
    /// The folders the Move dialog offers inside `folder`: the rows of the source that are
    /// folders, without Backups and service folders (DW-0409) and without what is being moved.
    fn move_choices(&self, folder: Option<&str>, moving: &[DriveItem]) -> Vec<DriveItem> {
        let Ok(listing) = self.source.list(folder) else {
            return Vec::new();
        };
        listing
            .items
            .into_iter()
            .filter(|i| i.is_folder && !i.is_protected && !moving.iter().any(|m| m.id == i.id))
            .collect()
    }

    /// Move… (DW-0409): the dialog on the root's folders, the first one chosen. Nothing that is
    /// protected moves (DW-0425 says why, in its own frame).
    pub fn begin_move(&mut self, tx: &mut Tx) -> bool {
        let items = self.selected_items();
        if items.is_empty() {
            return false;
        }
        if self.protects(&items) {
            self.failed(Action::Move, ActionError::Protected, tx);
            return true;
        }
        self.open_move(items);
        true
    }

    /// The Move dialog for these items, on the root's folders, the first one chosen.
    pub fn open_move(&mut self, items: Vec<DriveItem>) {
        let root = self.listing.path.first().cloned().unwrap_or(Crumb {
            id: None,
            name: "Files".into(),
        });
        let choices = self.move_choices(root.id.as_deref(), &items);
        self.actions.menu = None;
        self.actions.rename = None;
        self.actions.dialog = Some(Dialog::Move(MoveTo {
            picked: if choices.is_empty() { None } else { Some(0) },
            items,
            path: vec![root],
            choices,
            error: None,
        }));
    }

    fn move_dialog(&mut self) -> Option<&mut MoveTo> {
        match self.actions.dialog.as_mut() {
            Some(Dialog::Move(m)) => Some(m),
            _ => None,
        }
    }

    /// A click on a folder of the Move dialog: choose it.
    pub fn move_pick(&mut self, index: usize) {
        if let Some(m) = self.move_dialog()
            && index < m.choices.len()
        {
            m.picked = Some(index);
            m.error = None;
        }
    }

    /// Up and Down in the Move dialog.
    pub fn move_step(&mut self, delta: isize) {
        if let Some(m) = self.move_dialog()
            && !m.choices.is_empty()
        {
            let last = m.choices.len() - 1;
            m.picked = Some(match m.picked {
                Some(i) => i.saturating_add_signed(delta).min(last),
                None if delta < 0 => last,
                None => 0,
            });
            m.error = None;
        }
    }

    /// A double-click on a folder of the Move dialog (or Right): go into it.
    pub fn move_into(&mut self, index: usize) {
        let Some(m) = self.move_dialog() else { return };
        let Some(folder) = m.choices.get(index).cloned() else {
            return;
        };
        let moving = m.items.clone();
        let choices = self.move_choices(Some(&folder.id), &moving);
        if let Some(m) = self.move_dialog() {
            m.path.push(Crumb {
                id: Some(folder.id),
                name: folder.name,
            });
            m.picked = None;
            m.choices = choices;
            m.error = None;
        }
    }

    /// A click on a folder of the Move dialog's path (or Left, Backspace: one up).
    pub fn move_up_to(&mut self, depth: usize) {
        let Some(m) = self.move_dialog() else { return };
        if depth + 1 >= m.path.len() {
            return;
        }
        let left = m.path[depth + 1].id.clone();
        let here = m.path[depth].id.clone();
        let moving = m.items.clone();
        let choices = self.move_choices(here.as_deref(), &moving);
        if let Some(m) = self.move_dialog() {
            m.path.truncate(depth + 1);
            // The folder just left is the chosen one, as in a file picker.
            m.picked = choices.iter().position(|c| Some(&c.id) == left.as_ref());
            m.choices = choices;
            m.error = None;
        }
    }

    /// "Move to Documents": move the items and close the dialog. A name already used in the
    /// target keeps the dialog open and says so (DW-0410b's rule).
    pub fn commit_move(&mut self, tx: &mut Tx) {
        if !matches!(self.actions.dialog, Some(Dialog::Move(_))) {
            return;
        }
        // The dialog is taken out while the source is asked (no copy of its folders and items),
        // and put back wherever it stays open.
        let Some(Dialog::Move(mut m)) = self.actions.dialog.take() else {
            return;
        };
        if m.is_where_they_are() {
            self.actions.dialog = Some(Dialog::Move(m));
            return;
        }
        let target = m.target();
        // What the target already holds, where it can be read.
        if let Ok(there) = self.source.list(target.id.as_deref()) {
            for item in &m.items {
                let others = there.items.iter().map(|o| (o.name.as_str(), o.is_folder));
                if let Some(taken) = names::taken(&item.name, others) {
                    m.error = Some(taken.message());
                    self.actions.dialog = Some(Dialog::Move(m));
                    return;
                }
            }
        }
        match self.actions.source.move_to(&m.items, target.id.as_deref()) {
            Ok(()) => {
                self.forget_undo();
                if self.listing.folder.id != target.id {
                    self.listing.items.retain(|row| !m.items.iter().any(|i| i.id == row.id));
                    self.clear_selection();
                }
                self.focus_list(tx);
            }
            Err(ActionError::NameTaken) => {
                let taken = NameError::Taken {
                    name: m.items[0].name.clone(),
                    folder: m.items[0].is_folder,
                };
                m.error = Some(taken.message());
                self.actions.dialog = Some(Dialog::Move(m));
            }
            Err(why) => self.failed(Action::Move, why, tx),
        }
    }

    /// New folder (DW-0417), in the folder shown: the dialog, "Untitled folder" selected.
    pub fn begin_new_folder(&mut self) -> bool {
        if !self.can_add() {
            return false;
        }
        let parent = self.listing.folder.clone();
        let taken: Vec<&str> = self.listing.items.iter().map(|i| i.name.as_str()).collect();
        let name = names::untitled_folder(taken.iter().copied());
        self.open_new_folder(parent, name, None);
        true
    }

    /// New folder, asked from the Move dialog: in the folder that dialog shows.
    pub fn begin_new_folder_in_move(&mut self, tx: &mut Tx) {
        tx.focus(NAME_INPUT, false);
        let Some(Dialog::Move(m)) = self.actions.dialog.clone() else {
            return;
        };
        let parent = m.here().clone();
        let name = match self.source.list(parent.id.as_deref()) {
            Ok(there) => names::untitled_folder(there.items.iter().map(|i| i.name.as_str())),
            Err(_) => "Untitled folder".to_string(),
        };
        self.open_new_folder(parent, name, Some(Box::new(m)));
    }

    fn open_new_folder(&mut self, parent: Crumb, name: String, back_to: Option<Box<MoveTo>>) {
        let mut edit = TextEdit::new(name);
        edit.set_selection(0, edit.text.len());
        self.actions.name_edit = edit;
        self.actions.name_error = None;
        self.actions.menu = None;
        self.actions.rename = None;
        self.actions.dialog = Some(Dialog::NewFolder(NewFolder {
            parent: parent.id,
            parent_name: parent.name,
            back_to,
        }));
    }

    /// Cancel, Esc: close the dialog. New folder asked from Move goes back to Move.
    pub fn close_dialog(&mut self, tx: &mut Tx) {
        match self.actions.dialog.take() {
            Some(Dialog::NewFolder(NewFolder {
                back_to: Some(back), ..
            })) => self.actions.dialog = Some(Dialog::Move(*back)),
            Some(_) => self.focus_list(tx),
            None => {}
        }
        self.actions.name_error = None;
    }

    /// Create (DW-0417): make the folder, then show it (DW-1106), or go back to the Move dialog
    /// with it chosen. A name that cannot be given keeps the dialog open and says why.
    pub fn create_folder(&mut self, tx: &mut Tx) {
        let Some(Dialog::NewFolder(new)) = self.actions.dialog.clone() else {
            return;
        };
        let typed = self.actions.name_edit.text.clone();
        let there = self.source.list(new.parent.as_deref()).ok();
        let others = there
            .iter()
            .flat_map(|l| l.items.iter())
            .map(|i| (i.name.as_str(), i.is_folder));
        let name = match names::check_on(self.actions.host, &typed, others) {
            Ok(name) => name,
            Err(why) => {
                self.actions.name_error = Some((typed, why));
                return;
            }
        };
        match self.actions.source.new_folder(new.parent.as_deref(), &name) {
            Ok(()) => {}
            Err(ActionError::NameTaken) => {
                let taken = NameError::Taken {
                    name: name.clone(),
                    folder: true,
                };
                self.actions.name_error = Some((typed, taken));
                return;
            }
            Err(why) => {
                self.actions.dialog = None;
                self.failed(Action::NewFolder, why, tx);
                return;
            }
        }
        self.actions.dialog = None;
        self.actions.name_error = None;
        self.forget_undo();
        let (_, failures) = self.take_changes();
        let made = self.actions.made.pop();
        self.actions.made.clear();
        if let Some(&(action, why)) = failures.last() {
            self.failed(action, why, tx);
        }
        match (new.back_to, made) {
            (Some(mut back), made) => {
                back.choices = self.move_choices(back.here().id.as_deref(), &back.items);
                if let Some(made) = made {
                    back.picked = back.choices.iter().position(|c| c.id == made.id);
                }
                back.error = None;
                self.actions.dialog = Some(Dialog::Move(*back));
            }
            (None, Some(made)) => {
                self.open_folder(Some(made.id));
                self.focus_list(tx);
            }
            (None, None) => self.focus_list(tx),
        }
    }

    /// The keys of an open dialog: Esc closes it; in Move to, with focus on the list, the arrows
    /// choose, Right goes in, Left and Backspace go up, Enter moves. Any other key is left to the
    /// focused control (Tab, a button's Space and Enter, the field's typing); none reaches the
    /// list under the dialog (`keys::on_key` asks nobody else while a dialog is open).
    pub fn dialog_key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        use rune::Key;
        if ev.key == Key::Escape {
            self.close_dialog(tx);
            return true;
        }
        let Some(Dialog::Move(m)) = &self.actions.dialog else {
            return false;
        };
        let (picked, depth, empty) = (m.picked, m.path.len(), m.choices.is_empty());
        if empty {
            // A folder without folders has no list to hold the keyboard (it is on the path or a
            // button): Left and Backspace still go back up, or the arrows would be a dead end.
            return match ev.key {
                Key::Left | Key::Backspace if depth > 1 => {
                    self.move_up_to(depth - 2);
                    self.focus_move_list(tx);
                    true
                }
                _ => false,
            };
        }
        if tx.focused() != Some(MOVE_LIST) {
            return false;
        }
        match ev.key {
            Key::Down => self.move_step(1),
            Key::Up => self.move_step(-1),
            Key::Home => self.move_pick(0),
            Key::End => {
                let last = m.choices.len().saturating_sub(1);
                self.move_pick(last);
            }
            Key::Right => {
                if let Some(i) = picked {
                    self.move_into(i);
                }
            }
            Key::Left | Key::Backspace if depth > 1 => self.move_up_to(depth - 2),
            Key::Enter => self.commit_move(tx),
            _ => return false,
        }
        true
    }

    /// The list of folders takes the keyboard when the folder shown has one: after a way up was
    /// pressed (the link may be gone from the path) and after Left out of a folder without folders.
    pub fn focus_move_list(&mut self, tx: &mut Tx) {
        if let Some(Dialog::Move(m)) = &self.actions.dialog
            && !m.choices.is_empty()
        {
            tx.focus(MOVE_LIST, false);
        }
    }
}

/// The key of the Move dialog's list of folders: its one Tab stop.
pub const MOVE_LIST: &str = "list-0-body";

/// `.kit-item-title`: 13px/500, tracked -0.005em.
const ITEM_TITLE: TextStyle = TextStyle::new(13.0, 500.0).lh(17.0).tracking(-0.065);
/// The path line of the Move dialog (`.t-sub`, the folder shown in ink at 500).
const PATH_HERE: TextStyle = TextStyle::new(12.0, 500.0).lh(15.0);

/// A dialog on the Files window's scrim. `foot`: the buttons, the amber one first; `stack`:
/// one under the other, each as wide as the dialog (`.wn-dlg-foot.is-stack`).
/// Keys: `scrim`, `dlg`, `dlg-title`, `dlg-sub`, `dlg-body`, `dlg-foot`.
fn shell(name: &str, width: f32, title: &str, sub: Option<&str>, body: Vec<El>, foot: El, t: &Theme) -> El {
    // The card is `ui::dialog`'s, the one `settings_dialog::dialog` draws; a file's name under
    // the title stays on one line. Under the caption: the window can still be moved and closed
    // (`.mk-scrim`).
    let card = dialog::card(name, width, title, sub.map(dialog::Sub::Truncate), body, foot, t);
    let scrim = dialog::scrim(32.0, card, t);
    // The modal layer has the dialog's own key: one dialog opened from another (New folder from
    // Move to) is a new modal, so the keyboard goes to ITS first field. A modal's keys are heard
    // on the modal itself, not on the window under it.
    el().key(format!("dialog-{}", name.to_lowercase().replace(' ', "-")))
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .child(scrim)
        .on_key(super::keys::on_key)
        .modal_tx(false, |s: &mut State, tx: &mut Tx| s.close_dialog(tx))
}

fn new_folder(s: &State, new: &NewFolder, t: &Theme) -> El {
    let edit = &s.actions.name_edit;
    // The error is of the name as it was when Create was pressed: typing takes it away.
    let error = s
        .actions
        .name_error
        .as_ref()
        .filter(|(typed, _)| typed == &edit.text)
        .map(|(_, why)| why.message());
    let note = format!("Goes into {}. The name is encrypted too.", new.parent_name);
    let body = vec![field::bare_field(
        field::Bare {
            key: "field-0",
            name: "Folder name",
            message: Some(error.as_deref().unwrap_or(&note)),
            error: error.is_some(),
        },
        edit,
        NAME_INPUT,
        |s: &mut State| &mut s.actions.name_edit,
        |input| input.on_submit(|s: &mut State, tx: &mut Tx| s.create_folder(tx)),
        t,
    )];
    let foot = row()
        .center_y()
        .justify_end()
        .child(
            settings_dialog::button(0, "Create", Weight::Primary, t)
                .default_button()
                .on_press_tx(|s: &mut State, tx: &mut Tx| s.create_folder(tx)),
        )
        .child(
            settings_dialog::button(1, "Cancel", Weight::Quiet, t)
                .on_press_tx(|s: &mut State, tx: &mut Tx| s.close_dialog(tx)),
        );
    shell("New folder", 380.0, "New folder", None, body, foot, t)
}

/// One folder of the Move dialog (`.kit-item`): its icon, name, how much it holds, and the
/// chevron that says a double-click goes into it.
fn folder_row(index: usize, folder: &DriveItem, picked: bool, t: &Theme) -> El {
    let key = format!("item-0-{index}");
    let count = item_count(folder);
    // `.kit-item + .kit-item::before`: a hairline above every row but the first, from 10 pt in.
    let rule = (index > 0).then(|| {
        el().key(format!("{key}-rule"))
            .pin(Some(10.0), Some(0.0), Some(0.0), None)
            .h(kit::HAIR)
            .fill(t.separator)
    });
    let mut r = row()
        .key(key.clone())
        .role("option")
        .label(format!("{}, {count}", folder.name))
        .checked(picked)
        .w_full()
        .h(28.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        // `.kit-item.is-selected`: ink at 6 % (9 % in the dark).
        .fill(if picked {
            t.text.alpha(if t.dark { 0.09 } else { 0.06 })
        } else {
            Color::TRANSPARENT
        })
        .hover(t.hover)
        .press(t.press)
        .on_press_tx(move |s: &mut State, tx: &mut Tx| {
            tx.focus(MOVE_LIST, false);
            s.move_pick(index);
        })
        .on_double_press(move |s: &mut State| s.move_into(index));
    if let Some(rule) = rule {
        r = r.child(rule);
    }
    r.child(el().key(format!("{key}-icon-box")).w(30.0).center().child(kit::glyph(
        format!("{key}-icon"),
        &icons_settings::FOLDER,
        17.0,
        t.text_secondary,
    )))
    .child(
        el().key(format!("{key}-text")).grow(1.0).min_w(0.0).child(
            kit::label(format!("{key}-title"), folder.name.clone(), ITEM_TITLE, t.text)
                .truncate()
                .w_full(),
        ),
    )
    .child(kit::label(
        format!("{key}-value"),
        count,
        VALUE_MONO,
        if picked { t.text_secondary } else { t.text_tertiary },
    ))
    .child(kit::glyph(
        format!("{key}-chev"),
        &icons_settings::CHEV_RIGHT,
        13.0,
        t.text_quaternary,
    ))
}

fn move_to(m: &MoveTo, t: &Theme) -> El {
    // The path: a folder mark, every folder above as a way up, the folder shown in ink.
    let mut path = row()
        .key("dlg-path")
        .role("navigation")
        .label("Folder")
        .center_y()
        .gap(4.0)
        .child(kit::glyph(
            "dlg-path-icon",
            &icons_settings::FOLDER,
            14.0,
            t.text_tertiary,
        ));
    let last = m.path.len() - 1;
    for (i, crumb) in m.path.iter().enumerate() {
        if i < last {
            path = path
                .child(
                    kit::label(format!("dlg-path-{i}"), crumb.name.clone(), SUB, t.text_tertiary)
                        .role("link")
                        .focusable(true)
                        .on_press_tx(move |s: &mut State, tx: &mut Tx| {
                            s.move_up_to(i);
                            s.focus_move_list(tx);
                        }),
                )
                .child(kit::glyph(
                    format!("dlg-path-sep-{i}"),
                    &icons::CHEVRON_RIGHT,
                    12.0,
                    t.text_tertiary,
                ));
        } else {
            path = path.child(kit::label(
                format!("dlg-path-{i}"),
                crumb.name.clone(),
                PATH_HERE,
                t.text,
            ));
        }
    }
    let mut body = vec![path];
    if m.choices.is_empty() {
        body.push(kit::label("dlg-text-0", "No folders in here.", SUB, t.text_tertiary));
    } else {
        let mut rows = col().key("list-0-rows").w_full().clip_rounded(radius::LG);
        for (i, folder) in m.choices.iter().enumerate() {
            rows = rows.child(folder_row(i, folder, m.picked == Some(i), t));
        }
        let mut list = col()
            .key(MOVE_LIST)
            .role("listbox")
            .label("Folders")
            .focusable(true)
            .no_focus_ring()
            .w_full()
            .radius(radius::LG)
            .fill(t.card)
            .box_shadow(kit::ring(kit::HAIR, t.separator));
        if let Some(i) = m.picked {
            list = list.active_descendant(format!("item-0-{i}"));
        }
        // More folders than fit: the list scrolls inside the dialog.
        body.push(el().key("list-0").w_full().max_h(280.0).child(list.child(rows)));
    }
    if let Some(error) = &m.error {
        body.push(
            kit::label("dlg-error", error.clone(), field::MSG, crate::ui::au::red_text(t))
                .role("alert")
                .wrap(),
        );
    }

    let target = m.target();
    let stay = m.is_where_they_are();
    let label = format!("Move to {}", target.name);
    let mut primary = settings_dialog::button(0, &label, Weight::Primary, t).w_full();
    primary = if stay {
        // The items are in that folder already.
        primary.opacity(0.4).disabled(true)
    } else {
        primary
            .default_button()
            .on_press_tx(|s: &mut State, tx: &mut Tx| s.commit_move(tx))
    };
    let foot = col()
        .child(primary)
        .child(
            button::outline_button("btn-1", "New folder", Some(&icons::PLUS), t)
                .w_full()
                .on_press_tx(|s: &mut State, tx: &mut Tx| s.begin_new_folder_in_move(tx)),
        )
        // `.flex-1` between the actions and the way out: one more gap.
        .child(el().key("dlg-foot-gap").h(0.0))
        .child(
            settings_dialog::button(2, "Cancel", Weight::Quiet, t)
                .w_full()
                .on_press_tx(|s: &mut State, tx: &mut Tx| s.close_dialog(tx)),
        );
    shell("Move to", 460.0, "Move to", Some(&m.subject()), body, foot, t)
}

/// The open dialog, as the window's top layer.
pub fn layer(s: &State, t: &Theme) -> Option<El> {
    Some(match s.actions.dialog.as_ref()? {
        Dialog::NewFolder(new) => new_folder(s, new, t),
        Dialog::Move(m) => move_to(m, t),
    })
}
