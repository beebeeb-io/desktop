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
    ///
    /// - Every secret field is wiped: the password, its confirmation, both codes and the twelve words. The email stays.
    /// - The code after a proven password, and recovery's new password (the words were verified: the engine holds the
    ///   key they derived and the server's recovery token), are left as Back leaves them, so the engine forgets them.
    ///   Under a lock that is Back's own path too: from the code to the lock's password.
    /// - Left as they are: a step on its way (its own answer ends it), a passkey dialog, a request to another device
    ///   (nothing secret is here before that device answers), the twelve words of a session that waits for its key
    ///   (the session is the account's, not the step's: only the typed words go), and sign-up (its own screens).
    pub fn window_closed(&mut self) {
        if self.step_running() || self.passkey_waiting || matches!(self.screen, super::Screen::Create(_)) {
            return;
        }
        if matches!(
            self.screen,
            super::Screen::TwoFactor { .. } | super::Screen::NewPassword
        ) {
            self.open_sign_in();
        }
        self.password.clear();
        self.confirm.clear();
        self.code.clear();
        self.backup_code.clear();
        for word in &mut self.words {
            word.clear();
        }
    }
}
