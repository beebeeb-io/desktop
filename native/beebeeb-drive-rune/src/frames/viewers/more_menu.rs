//! The "…" (More actions) button of the open file's toolbar (task 67): one menu under the button, for every kind of
//! file the shared toolbar draws (text, notes, photo, audio, video, PDF, table, archive, the states). It holds the
//! actions of the file row's menu that apply to the file already open. Keys: `more-menu`, `more-menu-item-<i>`.

use rune::{El, Key, KeyEvent, Tx, row};

use super::{Intent, Ui};
use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::menu::{self, Entry};

pub const MENU_W: f32 = 236.0;

/// The menu while it is open: the highlighted line (the keyboard or the pointer is on it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Open {
    pub hot: Option<usize>,
}

/// The lines, as the file row's menu words them.
pub fn entries(ui: &Ui) -> Vec<Entry> {
    let mut out = vec![
        Entry::item("open-with", "Open with\u{2026}"),
        Entry::item("show", concat!("Show in ", beebeeb_engine::file_manager!())),
        Entry::Sep,
        Entry::item("share", "Share\u{2026}"),
        Entry::item("versions", "Version history\u{2026}"),
    ];
    // Only where the source read whether the file is kept on this PC: a state that was not read is not shown.
    if ui.knows_offline() {
        out.push(Entry::check("offline", "Available offline", ui.file.details.offline));
    }
    out.push(Entry::item(
        "save-copy",
        if cfg!(target_os = "macos") { "Save a copy to Mac\u{2026}" } else { "Save a copy\u{2026}" },
    ));
    out.push(Entry::Sep);
    out.push(Entry::keyed("rename", "Rename", "F2"));
    out.push(Entry::danger("trash", "Move to Trash", "Delete"));
    out
}

impl Ui {
    /// The "…" button: opens the menu, or closes it.
    pub fn toggle_more(&mut self) {
        self.more = if self.more.is_some() { None } else { Some(Open::default()) };
    }

    /// The line `index` is chosen: the menu closes and the file's action runs.
    pub fn pick_more(&mut self, index: usize) {
        let entries = entries(self);
        let Some(id) = entries.get(index).filter(|e| !e.is_off()).and_then(|e| e.id()) else {
            return;
        };
        self.more = None;
        match id {
            "open-with" => self.intent(Intent::ChooseApp),
            "show" => self.intent(Intent::ShowInFolder),
            "share" => self.intent(Intent::Share),
            "versions" => self.intent(Intent::VersionHistory),
            "offline" => self.toggle_offline(),
            "save-copy" => self.intent(Intent::SaveCopy),
            "rename" => self.intent(Intent::Rename),
            "trash" => self.intent(Intent::Trash),
            _ => {}
        }
    }
}

/// The menu's keys: the arrows, Home and End move, Enter picks, Escape closes. The menu is modal, so nothing else
/// reaches the file under it.
fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(ui) = super::ui(s) else { return false };
    let Some(open) = ui.more else { return false };
    let entries = entries(ui);
    let hot = match ev.key {
        Key::Down => menu::step(&entries, open.hot, 1),
        Key::Up => menu::step(&entries, open.hot, -1),
        Key::Home => menu::step(&entries, None, 1),
        Key::End => menu::step(&entries, None, -1),
        Key::Enter => {
            if let Some(hot) = open.hot {
                ui.pick_more(hot);
            }
            return true;
        }
        Key::Escape => {
            ui.more = None;
            tx.focus("more-btn", true);
            return true;
        }
        _ => return true,
    };
    if let Some(hot) = hot {
        ui.more = Some(Open { hot: Some(hot) });
        if let Some(key) = menu::item_key("more-menu", &entries, hot) {
            tx.focus(key, false);
        }
    }
    true
}

/// The menu, under the toolbar's "…" at the window's right.
pub fn layer(ui: &Ui, open: Open, t: &Theme) -> El {
    let surface = menu::menu(
        "more-menu",
        "More actions",
        MENU_W,
        &entries(ui),
        open.hot,
        t,
        |s: &mut State, i, _| {
            if let Some(ui) = super::ui(s) {
                ui.pick_more(i);
            }
        },
        |s: &mut State, i| {
            if let Some(ui) = super::ui(s) {
                ui.more = Some(Open { hot: Some(i) });
            }
        },
        |s: &mut State, tx: &mut Tx| {
            if let Some(ui) = super::ui(s) {
                ui.more = None;
                tx.focus("more-btn", true);
            }
        },
    )
    .on_key(on_key);
    row()
        .key("more-menu-layer")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .pt(78.0)
        .pr(96.0)
        .justify_end()
        .items_start()
        .child(surface)
}
