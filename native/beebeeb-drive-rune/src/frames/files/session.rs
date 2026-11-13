//! The session ended (DW-1109): after 90 days without use the server no longer takes this PC's
//! session. The list and the key are still on this PC, so the window stays as it was and asks
//! for one thing, the password.
//!
//! - **Whose state it is.** The engine's: `GlobalSource::status().signed_out`. It is NOT a
//!   sign-out: after a sign-out the gate closes, the sign-in screen shows and nothing of the
//!   account is drawn (`shell::host::on_gate_change`). Here the gate stays open and the banner
//!   says what stopped. No shell value overrides the gate.
//! - **What still shows.** Only what the window already holds: the rows as they were. Nothing is
//!   read again, no thumbnail is loaded, nothing is decrypted: every action that would need the
//!   session is refused before the source is asked ([`State::needs_session`]), the keyboard goes
//!   to the banner's Sign in, and the refusal is counted ([`Ui::asked`]).
//! - **Sign in** asks the engine for the password screen (`ShellActions::sign_in_again`) and
//!   records the frame it leads to (DW-A03).
//! - **The other answer is one switch.** Whether file names may stay visible while the session
//!   is not valid waits for the owner's ruling. [`KEEP_THE_LIST`] `= false` blanks the rows in
//!   this state.

use super::Intent;
use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;
use rune::{El, Tx, el, row};

/// The board's answer (DW-1109: "The list as it was", "Nothing on this PC is removed"). `false`:
/// the rows are not drawn while the session is not valid.
pub const KEEP_THE_LIST: bool = true;

/// What the window keeps of this state.
#[derive(Default)]
pub struct Ui {
    /// How often something that needs the session was asked for, and refused.
    pub asked: u32,
    /// A test's answer to [`KEEP_THE_LIST`].
    pub keep_the_list: Option<bool>,
    /// The rows taken out of the window while they are not to be shown; they come back when the
    /// session is valid again (they never left this PC).
    held: Vec<crate::source::DriveItem>,
}

/// The frames that need no session: the sign-in itself, the b's flyout, Settings, and the two
/// views of what is on this PC.
fn needs_none(frame: &str) -> bool {
    matches!(frame, "DW-A03" | "DW-1201" | "DW-0701" | "DW-1301" | "DW-0715")
}

impl State {
    /// Did the session end (the engine says so)? The key and the list are still on this PC.
    pub fn session_ended(&self) -> bool {
        self.global.source.status().signed_out
    }

    /// Something was asked for that needs the session. `true`: it is refused: the menu closes,
    /// the keyboard goes to the banner's Sign in, and the source is not asked.
    pub(super) fn needs_session(&mut self, tx: &mut Tx) -> bool {
        if !self.refused_without_session() {
            return false;
        }
        self.actions.menu = None;
        tx.focus("banner-action", true);
        true
    }

    /// [`State::needs_session`], where no keyboard focus can be moved.
    pub(super) fn refused_without_session(&mut self) -> bool {
        if !self.session_ended() {
            return false;
        }
        self.actions.session.asked += 1;
        true
    }

    /// May this frame be asked for now? Not while the session ended, unless it needs none.
    pub(super) fn refuses_frame(&mut self, frame: &str) -> bool {
        !needs_none(frame) && self.refused_without_session()
    }

    /// The banner's Sign in: the engine is asked for the password screen (the shared sign-in, the
    /// email filled in: DW-A03). What the window shows next is the gate's to say.
    pub fn sign_in_again(&mut self) {
        self.intent(Intent::Frame("DW-A03"));
        if let Some(signin) = self.signin.as_mut() {
            signin.open_sign_in();
        }
        self.shell.actions.sign_in_again();
    }
}

/// Are the rows left out now (the other answer to DW-1109)?
pub fn blanks(s: &State) -> bool {
    !s.actions.session.keep_the_list.unwrap_or(KEEP_THE_LIST) && s.session_ended()
}

/// Every frame (`protected::poll`, from `shell::host::poll`): with the rows not to be shown, they
/// are taken out of the window when the session ends (no row, no name and no count of them is in
/// the element tree) and put back when it is valid again. `true`: something changed.
pub fn poll(s: &mut State) -> bool {
    if blanks(s) {
        if s.listing.items.is_empty() {
            return false;
        }
        s.clear_selection();
        s.actions.session.held = std::mem::take(&mut s.listing.items);
        return true;
    }
    if s.actions.session.held.is_empty() {
        return false;
    }
    s.listing.items = std::mem::take(&mut s.actions.session.held);
    true
}

/// The status line while the session ended: a red dot, "Signed out", and that nothing syncs.
/// `None`: the session is valid.
pub fn status_line(s: &State, t: &Theme) -> Option<El> {
    if !s.session_ended() {
        return None;
    }
    Some(
        row()
            .key("status-bar")
            .role("status")
            .h(28.0)
            .w_full()
            .fill(t.status_bar_bg)
            .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
            .border_layout()
            .px(16.0)
            .gap(7.0)
            .center_y()
            .child(
                row()
                    .key("status-left")
                    .center_y()
                    .gap(7.0)
                    .child(el().key("status-dot").size(7.0, 7.0).radius(3.5).fill(t.red))
                    .child(kit::label(
                        "status-text",
                        "Signed out",
                        type_::STATUS_STRONG,
                        t.text_secondary,
                    )),
            )
            .child(el().key("status-spacer").grow(1.0))
            .child(kit::label(
                "status-details",
                "Nothing syncs until you sign in",
                type_::STATUS,
                t.text_tertiary,
            )),
    )
}

/// The board frame: the window as it was, the session ended.
pub const SCENES: &[crate::ui::gate::Scene<State>] = &[("DW-1109", |s| {
    use crate::frames::global::source::board;
    s.clear_selection();
    s.show_detail_pane = false;
    s.global = crate::frames::global::Ui::new(Box::new(crate::frames::global::GlobalFixture::of(board::signed_out())));
})];
