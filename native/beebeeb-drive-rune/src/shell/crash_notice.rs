//! drive-complete-138b: after a crash, the next start says so. "Beebeeb closed unexpectedly last time", over the
//! window whatever it shows, once the vault is open, with "Send a report" and "Dismiss".
//!
//! - **Send a report** opens Report a problem (DW-0782) with the crash record attached: the dialog shows the
//!   record's text before anything is saved, and "Save and write email" puts it into the support bundle
//!   (`log_file::crash::attach_to_bundle`); the record file is deleted once it is in a saved bundle.
//! - **Dismiss** deletes the record file; nothing of it is said again.
//!
//! The record is the one found at start (`log_file::crash::remember_pending`), before this run could write one. Its
//! text is reduced by shape on reading: a version, `main`/`other` and `<file>.rs:<line>`, never a name, path,
//! email, key or token.

use rune::{Cx, El, el, row};

use crate::State;
use crate::log_file::crash::{self, CrashRecord};
use crate::theme::tokens;
use crate::ui::toast::{self, Lead};
use crate::ui::{global_icons, kit};

pub const WORDS: &str = "Beebeeb closed unexpectedly last time";

/// The poll: the record found at start comes into the window once.
pub fn poll(s: &mut State) -> bool {
    match crash::take_at_start() {
        Some(record) => {
            s.crash = Some(record);
            true
        }
        None => false,
    }
}

/// Send a report: Report a problem opens with the record attached (deleted once a bundle holding it is saved).
pub fn send(s: &mut State) {
    if let Some(record) = s.crash.take() {
        s.settings.crash_record = Some(record);
        crate::frames::settings::go(s, "DW-0782");
    }
}

/// Dismiss: the record is deleted, and nothing of it is said again.
pub fn dismiss(s: &mut State) {
    if let Some(record) = s.crash.take() {
        record.clear();
    }
}

/// Over the window, whatever it shows, while the vault is open. One parent whether it shows or not, so the frame
/// behind keeps its identity when the notice comes and goes.
pub fn over(s: &State, cx: &mut Cx<State>, behind: El) -> El {
    let window = el().key("window-with-crash-notice").w_full().h_full().child(behind);
    if s.crash.is_none() || s.shell.app.gate() != crate::shell::app::GateState::Ready {
        return window;
    }
    let t = tokens::theme(cx.system_dark());
    let send_button = el()
        .key("crash-notice-send")
        .role("button")
        .label("Send a report")
        .h(24.0)
        .px(8.0)
        .radius(12.0)
        .fill(t.selection)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .center()
        .on_press(send)
        .child(kit::label(
            "crash-notice-send-text",
            "Send a report",
            toast::ACTION,
            t.text,
        ));
    let dismiss_button = el()
        .key("crash-notice-dismiss")
        .role("button")
        .label("Dismiss")
        .size(24.0, 24.0)
        .radius(12.0)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .center()
        .on_press(dismiss)
        .child(kit::glyph(
            "crash-notice-dismiss-icon",
            &global_icons::CLOSE,
            12.0,
            t.text_secondary,
        ));
    let action = row()
        .key("crash-notice-buttons")
        .center_y()
        .gap(3.0)
        .child(send_button)
        .child(dismiss_button);
    window.child(
        // Two toast lines above a toast's own place: it stands until pressed, above the late-password notice's line.
        row()
            .key("crash-notice-layer")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .pb(126.0)
            .justify_center()
            .items_end()
            .child(toast::toast_with("crash-notice", WORDS, Lead::None, Some(action), &t)),
    )
}

/// What the Report a problem dialog shows of an attached record: its text, as it will be in the bundle.
pub fn attached_text(record: &CrashRecord) -> String {
    crash::reduce(&record.text)
}
