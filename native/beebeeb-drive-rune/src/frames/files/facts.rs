//! Details (DW-0411) and Version history (DW-0413): two dialogs about one file.
//!
//! Details lists what the source knows of the file, the same rows as the iPhone's Details; its
//! last row goes to Version history. Version history lists the file's versions, the current
//! one first, and Restore makes an earlier one current: one call (`FilesSource::restore_version`)
//! per press, and the list is read again from the source, so what is drawn afterwards is what
//! the source has. Restoring deletes nothing: the version that was current stays in the list.

use super::actions_data::{Version, versions_words};
use super::dialogs::{self, Dialog};
use super::share::{big_tile, file_head};
use super::source::Action;
use crate::State;
use crate::source::DriveItem;
use crate::theme::tokens::{Theme, type_};
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_list::{self as lists, Item, Trailing};
use crate::ui::settings_notice::PILL;
use crate::ui::settings_shell::SUB;
use crate::ui::{button, kit};
use rune::{El, Tx, col, el, row};

impl State {
    /// The one selected item, when exactly one is.
    fn one_selected(&self) -> Option<DriveItem> {
        match &self.selected_items()[..] {
            [only] => Some(only.clone()),
            _ => None,
        }
    }

    /// Details (Alt+Enter, the file menu): the dialog for the one selected item.
    pub fn begin_details(&mut self) -> bool {
        let Some(item) = self.one_selected() else {
            return false;
        };
        self.close_menu();
        self.actions.dialog = Some(Dialog::Details(Box::new(item)));
        true
    }

    /// Version history… (the file menu, the details panel, Details' last row): of one file.
    pub fn begin_versions(&mut self) -> bool {
        let item = match &self.actions.dialog {
            // From Details: the file it is about.
            Some(Dialog::Details(item)) => Some((**item).clone()),
            _ => self.one_selected(),
        };
        let Some(item) = item.filter(|item| !item.is_folder) else {
            return false;
        };
        self.close_menu();
        self.actions.dialog = Some(Dialog::Versions(Box::new(item)));
        true
    }

    /// Restore: the version with this id becomes the current one. The dialog stays and shows
    /// the list as the source has it now; a failure says so (DW-0109) and changes nothing.
    pub fn restore_version(&mut self, version: &str, tx: &mut Tx) {
        let Some(Dialog::Versions(item)) = self.actions.dialog.clone() else {
            return;
        };
        self.forget_undo();
        if let Err(why) = self.actions.source.restore_version(&item, version) {
            self.failed(Action::RestoreVersion, why, tx);
        }
    }

    /// A double-click on a version: Preview (DW-0408) of the file.
    pub fn preview_version(&mut self) {
        let Some(Dialog::Versions(item)) = self.actions.dialog.clone() else {
            return;
        };
        self.open_preview(*item);
    }
}

/// "Yes", "No": is there a copy on this PC?
fn on_this_pc(item: &DriveItem) -> &'static str {
    match item.pc_status.as_deref() {
        Some("on_this_pc" | "available_offline") => "Yes",
        _ => "No",
    }
}

fn done(index: usize, t: &Theme) -> El {
    row().center_y().justify_end().child(
        button::outline_button(&format!("btn-{index}"), "Done", None, t)
            .default_button()
            .on_press_tx(|s: &mut State, tx: &mut Tx| s.close_dialog(tx)),
    )
}

pub fn details(s: &State, item: &DriveItem, t: &Theme) -> El {
    let d = s.actions.source.details(item, &s.listing.folder.name);
    let versions = s.actions.source.versions(item);
    let mut rows: Vec<El> = Vec::new();
    let add = |title: &str, piece: Trailing, rows: &mut Vec<El>| {
        let index = rows.len();
        let mut it = Item::new(None, title).trailing(piece);
        if index > 0 {
            it = it.ruled();
        }
        rows.push(lists::item(0, index, &it, t));
    };
    if !d.size.is_empty() {
        add("Size", Trailing::Mono(d.size.clone()), &mut rows);
    }
    add("Changed", Trailing::Mono(d.changed.clone()), &mut rows);
    if let Some(added) = &d.added {
        add("Added", Trailing::Mono(added.clone()), &mut rows);
    }
    add("Where", Trailing::Value(d.where_.clone()), &mut rows);
    add("On this PC", Trailing::Value(on_this_pc(item).to_string()), &mut rows);
    // Every item is: there is no other kind of file in Beebeeb.
    add("Encrypted", Trailing::Value("Yes, end to end".to_string()), &mut rows);
    // The versions, counted from the source's own list; else what its details say.
    let count = if versions.is_empty() {
        d.versions.clone()
    } else {
        Some(versions_words(versions.len()))
    };
    if let Some(count) = count.filter(|_| !item.is_folder) {
        let index = rows.len();
        let it = Item::new(None, "Version history")
            .trailing(Trailing::Value(count))
            .opens()
            .ruled();
        rows.push(lists::item(0, index, &it, t).on_press(|s: &mut State| {
            s.begin_versions();
        }));
    }
    let body = vec![
        file_head(item, &item.name, &d.kind, false, 40.0, t),
        lists::list(0, None, rows, None, t),
    ];
    dialogs::shell("Details", 420.0, "Details", None, body, done(0, t), t)
}

/// `.kit-pill.tone-neutral` without a dot. Keys: `<key>`, `<key>-label`.
fn pill(key: &str, words: &str, t: &Theme) -> El {
    row()
        .key(key.to_string())
        .h(20.0)
        .px(8.0)
        .center_y()
        .radius(10.0)
        .fill(t.text.alpha(0.07))
        .child(kit::label(format!("{key}-label"), words, PILL, t.text_secondary))
}

/// One version (`.kit-item` with a 28 pt tile): when, how large, and Restore, or "Current".
fn version_row(index: usize, button: usize, item: &DriveItem, v: &Version, t: &Theme) -> El {
    let key = format!("item-0-{index}");
    let sub = if v.current {
        format!("{} \u{00B7} Current version", v.size)
    } else {
        v.size.clone()
    };
    let mut name = format!("{}, {sub}", v.when);
    if let Some(device) = &v.changed_on {
        name = format!("{name}. Changed on {device}");
    }
    let mut r = row()
        .key(key.clone())
        .role("listitem")
        .label(name)
        .w_full()
        .min_h(40.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .on_double_press(|s: &mut State| s.preview_version());
    if let Some(device) = &v.changed_on {
        // "Each row says which device made the change on hover."
        r = r.help(format!("Changed on {device}"));
    }
    if index > 0 {
        r = r.child(
            el().key(format!("{key}-rule"))
                .pin(Some(10.0), Some(0.0), Some(0.0), None)
                .h(kit::HAIR)
                .fill(t.separator),
        );
    }
    r = r.child(big_tile(&key, item, 28.0, t)).child(
        col()
            .key(format!("{key}-text"))
            .grow(1.0)
            .min_w(0.0)
            .gap(1.0)
            .child(kit::label(format!("{key}-title"), v.when.clone(), type_::ITEM_TITLE, t.text).truncate())
            .child(kit::label(format!("{key}-sub"), sub, SUB, t.text_tertiary).truncate()),
    );
    if v.current {
        r.child(pill(&format!("{key}-pill"), "Current", t))
    } else {
        let id = v.id.clone();
        r.child(
            controls::button(button, "Restore", None, Emphasis::Plain, t)
                .description(format!(
                    "Makes the version of {} the current one. No version is deleted.",
                    v.when
                ))
                .on_press_tx(move |s: &mut State, tx: &mut Tx| s.restore_version(&id, tx)),
        )
    }
}

pub fn versions(s: &State, item: &DriveItem, t: &Theme) -> El {
    let all = s.actions.source.versions(item);
    let sub = if all.is_empty() {
        item.name.clone()
    } else {
        format!("{} \u{00B7} {}", item.name, versions_words(all.len()))
    };
    let mut body = Vec::new();
    let mut buttons = 0;
    if all.is_empty() {
        // A source that knows no versions (nothing is connected): say so, list nothing.
        body.push(kit::label(
            "dlg-text-0",
            "The versions of this file can't be shown right now.",
            SUB,
            t.text_tertiary,
        ));
    } else {
        let rows = all
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let row = version_row(i, buttons, item, v, t);
                if !v.current {
                    buttons += 1;
                }
                row
            })
            .collect();
        body.push(lists::list(0, None, rows, None, t));
        body.push(
            kit::label(
                "dlg-text-0",
                "Earlier versions are kept for 30 days and count toward your storage.",
                type_::CAPTION,
                t.text_tertiary,
            )
            .wrap(),
        );
    }
    dialogs::shell(
        "Version history",
        440.0,
        "Version history",
        Some(&sub),
        body,
        done(buttons, t),
        t,
    )
}
