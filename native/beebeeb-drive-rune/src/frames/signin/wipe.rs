//! What leaves memory when the window closes during sign-in (census row, lane signin-wipe): nothing typed waits in a
//! hidden window, and neither does a proof the engine holds for a half-done step.
//!
//! Closing the window keeps Beebeeb running in the notification area (DW-1231), so the sign-in group's state would
//! otherwise stay in memory, unseen, until the window comes back. See [`Ui::window_closed`] for what goes.

use super::Ui;
use crate::State;

/// The shell's close hook (`shell/host.rs`, `on_close`): [`Ui::window_closed`] on the group's state.
pub fn on_window_closed(s: &mut State) {
    if let Some(ui) = super::ui(s) {
        ui.window_closed();
    }
}

impl Ui {
    /// The window went to the notification area.
    pub fn window_closed(&mut self) {}
}
