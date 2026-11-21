//! A password change answered while its page was not open (desktop#427, the lead's follow-up): said at once wherever
//! the person is, as a notice over the window whatever it shows (Files, another section of Settings, an account
//! page), one toast line above a toast's own place, and in full on the Change password page, which its button
//! opens. Said once: the page takes it.
//!
//! It can be dismissed (it then goes, and nothing of it is said later). It is the press's: kept only while the call's mark is current (the session and the account of the press, per
//! kind), dropped by `end_flows` (a lock, a sign-out, another account), never drawn while the gate is not Ready, and
//! held outside the form so that leaving the account's sections does not lose it.

use rune::{Cx, El, el, row};

use super::AccountUi;
use super::source::{Answered, CallKind, CallMark, Refused};
use crate::State;
use crate::theme::tokens;
use crate::ui::toast::{self, Lead};
use crate::ui::{global_icons, kit};

/// The answer of one password press, with that press's serial.
pub struct Late {
    serial: u64,
    answer: Result<(), Refused>,
}

/// The window kept no wait for this answer: it is kept to be said, when the press's session is still the one now.
pub(super) fn keep(ui: &mut AccountUi, mark: Option<CallMark>, answered: Answered) {
    let Some(mark) = mark.filter(|mark| mark.current) else {
        ui.late_change = None;
        return;
    };
    ui.late_change = Some(Late {
        serial: mark.serial,
        answer: match answered {
            Answered::Done => Ok(()),
            Answered::Refused(why) => Err(why),
            _ => Err(Refused::Offline),
        },
    });
}

/// The poll: an answer whose press's session or account is no longer the one now, or that a later press replaced,
/// is dropped.
pub(super) fn check(ui: &mut AccountUi) {
    let Some(late) = ui.late_change.as_ref() else {
        return;
    };
    let mark = ui.source.as_deref().and_then(|s| s.call_mark(CallKind::ChangePassword));
    if !mark.is_some_and(|mark| mark.current && !mark.out && mark.serial == late.serial) {
        ui.late_change = None;
    }
}

/// The Change password page says it, in full: taken once.
pub(super) fn take(ui: &mut AccountUi) -> Option<Result<(), Refused>> {
    check(ui);
    ui.late_change.take().map(|late| late.answer)
}

/// The notice's words: short, the page says the rest.
pub(super) fn notice(answer: &Result<(), Refused>) -> &'static str {
    match answer {
        Ok(()) => "Your password was changed",
        Err(Refused::Unconfirmed) => "Your password change could not be confirmed",
        Err(_) => "Your password was not changed",
    }
}

/// Over the window, whatever it shows: the notice of a late password answer, with "Show" (the Change password page).
/// One parent whether the notice shows or not: the frame behind keeps its identity (its keyboard focus, scroll and
/// hover) when the notice comes and goes.
pub fn over(s: &State, cx: &mut Cx<State>, behind: El) -> El {
    let window = el().key("window-with-late-change").w_full().h_full().child(behind);
    let Some(late) = s.account.late_change.as_ref() else {
        return window;
    };
    if s.shell.app.gate() != crate::shell::app::GateState::Ready {
        return window;
    }
    let t = tokens::theme(cx.system_dark());
    let lead = match late.answer {
        Ok(()) => Lead::Check,
        Err(_) => Lead::Warn,
    };
    // Show: the page the answer belongs to. Dismiss: the notice goes, and nothing of it is said later (#440 review, P3).
    let show = el()
        .key("late-change-action")
        .role("button")
        .label("Show the password change")
        .h(24.0)
        .px(8.0)
        .radius(12.0)
        .fill(t.selection)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .center()
        .on_press(|s: &mut State| super::go(s, "DW-A67"))
        .child(kit::label("late-change-action-text", "Show", toast::ACTION, t.text));
    let dismiss = el()
        .key("late-change-close")
        .role("button")
        .label("Dismiss")
        .size(24.0, 24.0)
        .radius(12.0)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .center()
        .on_press(|s: &mut State| s.account.late_change = None)
        .child(kit::glyph(
            "late-change-close-icon",
            &global_icons::CLOSE,
            12.0,
            t.text_secondary,
        ));
    let action = row()
        .key("late-change-buttons")
        .center_y()
        .gap(3.0)
        .child(show)
        .child(dismiss);
    window.child(
        // One toast line above a toast's own place (`ui::toast::layer`: 42 pt): it stands until Show, so it never
        // covers a screen's own toast (Files' Undo, an account page's notice).
        row()
            .key("late-change-layer")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .pb(84.0)
            .justify_center()
            .items_end()
            .child(toast::toast_with(
                "late-change",
                notice(&late.answer),
                lead,
                Some(action),
                &t,
            )),
    )
}
