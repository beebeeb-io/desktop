//! Pinned folders in the Files window: "Pin to sidebar" / "Unpin from sidebar" in the file menu and the Details
//! pane, and a pinned folder opened from the sidebar. The pins are the account's `pinned_folders` preference, the
//! one the web app keeps, so they follow the person between web and desktop. Folders only, as on the web.

use super::source::{Action, ActionError};
use crate::State;
use crate::shell::app::{Intent, Section};
use crate::shell::host;
use crate::source::DriveItem;
use rune::Tx;

/// The most folders the sidebar pins (the web app's limit).
pub use beebeeb_engine::api_client::MAX_PINS;

impl State {
    /// The one selected row when Pin / Unpin applies to it: a folder of the account's, not a protected one (Backups).
    pub fn pin_target(&self) -> Option<DriveItem> {
        let items = self.selected_items();
        match items.as_slice() {
            [item] if item.is_folder && !self.protects(&items) => Some(item.clone()),
            _ => None,
        }
    }

    /// `Some(pinned)` when the selection can be pinned or unpinned: for the menu's and the pane's words.
    pub fn pin_state(&self) -> Option<bool> {
        let target = self.pin_target()?;
        Some(self.actions.source.pinned_ids().contains(&target.id))
    }

    /// Pin the selected folder, or unpin it when it is pinned.
    pub fn toggle_pin(&mut self, tx: &mut Tx) {
        let Some(folder) = self.pin_target() else { return };
        let pinned = self.actions.source.pinned_ids().contains(&folder.id);
        let action = if pinned { Action::Unpin } else { Action::Pin };
        let asked = if !pinned && self.actions.source.pinned_ids().len() >= MAX_PINS {
            Err(ActionError::PinLimit)
        } else {
            self.actions.source.set_pinned(&folder, !pinned)
        };
        if let Err(why) = asked {
            let stay = self.set_toast(super::Toast::Failed(action, why));
            tx.wake_after(stay);
        }
    }
}

/// A pinned folder pressed in the sidebar: the Files window shows it.
pub fn open(s: &mut State, id: &str) {
    host::request(s, Intent::Go(crate::shell::app::Route::section(Section::Files)));
    if !matches!(s.shell.app.screen(), crate::shell::app::Screen::Drive(route) if route.section == Section::Files) {
        return;
    }
    super::reset::close_dialogs(s);
    s.open_folder(Some(id.to_string()));
}

/// The window got focus: the account's pins are read again.
pub fn on_focus(s: &mut State, focused: bool) {
    if focused {
        s.actions.source.refresh_pins();
    }
}
