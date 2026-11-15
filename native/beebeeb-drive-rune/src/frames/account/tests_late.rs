//! The lead's follow-up of desktop#427 (2026-11-15), the window's side: an answer is only ever said for the press and
//! the session that asked for it, a second press while the same call is out starts nothing and says so, and a late
//! password answer is said wherever the person is, once.

use super::form;
use super::source::{Answered, BOARD_PASSWORD, BoardAccount, CallKind, HeldCalls, Refused, Secret};
use super::tests::{click, says, settle, window};
use super::{CodeFor, Dialog, Pane};
use crate::shell::app::{Event, Route, Section};
use crate::{State, init};
use rune::Runtime;
use serde_json::Value;

const FIRST: &str = "first.address@example.eu";
const SECOND: &str = "second.address@example.eu";

struct Rig {
    rt: Runtime<State>,
    held: HeldCalls,
}

fn rig(frame: &str, held: HeldCalls) -> Rig {
    let mut state = init();
    assert!(super::scene(frame, &mut state), "{frame} is a built frame");
    state.account.source = Some(Box::new(BoardAccount::new().holding_calls(held.clone())));
    state.account.form.setup = None;
    form::arrive(&mut state.account);
    let mut rt = window(state);
    settle(&mut rt);
    Rig { rt, held }
}

/// The engine's rule for a kind that is already out: the press is "taken" and nothing new starts.
fn like_the_engine() -> HeldCalls {
    let held = HeldCalls::default();
    held.repeat_out.set(true);
    held
}

impl Rig {
    fn started(&self) -> Vec<CallKind> {
        self.held.started.borrow().clone()
    }
    fn form(&mut self, f: impl FnOnce(&mut form::Form)) {
        self.rt.update(|s, _| f(&mut s.account.form));
        settle(&mut self.rt);
    }
    fn go(&mut self, frame: &'static str) {
        self.rt.update(|s, _| super::go(s, frame));
        settle(&mut self.rt);
    }
    fn answer(&mut self, answered: Answered) {
        *self.held.answer.borrow_mut() = Some(answered);
        settle(&mut self.rt);
    }
    fn pane(&self) -> Pane {
        self.rt.state.account.pane
    }
    fn waiting(&self) -> bool {
        self.rt.state.account.form.waiting.is_some()
    }
    fn text(&self) -> String {
        self.rt.accessibility_json().to_string()
    }
}

fn node(rt: &Runtime<State>, key: &str) -> Value {
    fn find(v: &Value, key: &str) -> Option<Value> {
        if v["key"] == key {
            return Some(v.clone());
        }
        v["children"].as_array()?.iter().find_map(|c| find(c, key))
    }
    find(&rt.accessibility_json(), key).unwrap_or_else(|| panic!("no accessibility node keyed {key}"))
}

fn disabled(rt: &Runtime<State>, key: &str) -> bool {
    node(rt, key)["states"]
        .as_array()
        .is_some_and(|all| all.iter().any(|s| s == "disabled"))
}

fn send_code(r: &mut Rig, address: &str) {
    let address = address.to_string();
    r.form(move |f| {
        form::type_in(&mut f.email, &address);
        form::type_in(&mut f.current, BOARD_PASSWORD);
    });
    click(&mut r.rt, "btn-1");
}

/// Review of desktop#427 (delta check, second P2): the page waits for the press it started, the fields freeze while it
/// is out, a second press after leaving and coming back starts nothing and says so, and DW-A66 names the address the
/// code went to (the source's), never what the field holds.
#[test]
fn a_second_send_code_while_the_first_is_out_starts_nothing_and_dw_a66_names_the_address_the_code_went_to() {
    let mut r = rig("DW-A65", like_the_engine());
    send_code(&mut r, FIRST);
    assert_eq!(r.started(), [CallKind::StartEmail]);
    assert!(r.waiting());
    assert!(
        r.rt.state.account.form.current.text.is_empty(),
        "the password left the field with the press"
    );
    assert!(
        disabled(&r.rt, "field-0"),
        "the address is frozen while the code is sent"
    );
    assert!(disabled(&r.rt, "field-1"), "and the password");
    // Away and back: the page shows the same wait, never a blank form that starts a second change.
    click(&mut r.rt, "btn-0");
    assert_eq!(r.pane(), Pane::Account);
    r.go("DW-A65");
    assert!(r.waiting(), "the wait is adopted on return");
    assert!(
        says(&r.rt, "field-0-msg").contains("still being sent"),
        "and said: {}",
        r.text()
    );
    send_code(&mut r, SECOND);
    assert_eq!(r.started(), [CallKind::StartEmail], "nothing new was started");
    r.answer(Answered::Done);
    assert_eq!(r.pane(), Pane::EmailCode);
    let heading = says(&r.rt, "set-sub");
    assert!(heading.contains(FIRST), "the address the code went to: {heading}");
    assert!(!heading.contains(SECOND), "never the field's text: {heading}");
}

/// The same rule for a code confirm: the page that comes back starts nothing while the same call is out, says so, and
/// shows that call's answer (new backup codes are never dropped unseen).
#[test]
fn a_code_press_while_the_same_call_is_out_starts_nothing_and_says_so() {
    let mut r = rig("DW-A71", like_the_engine());
    let confirm = |r: &mut Rig| {
        r.rt.update(|s, _| super::ask(s, Dialog::Code(CodeFor::NewBackupCodes)));
        settle(&mut r.rt);
        r.form(|f| form::type_in(&mut f.code, "514208"));
        click(&mut r.rt, "btn-0");
    };
    confirm(&mut r);
    assert_eq!(r.started(), [CallKind::NewBackupCodes]);
    // The dialog is closed and opened again while the call is out.
    r.rt.update(|s, _| {
        s.account.dialog = None;
        form::arrive(&mut s.account);
    });
    settle(&mut r.rt);
    confirm(&mut r);
    assert_eq!(r.started(), [CallKind::NewBackupCodes], "nothing new was started");
    assert!(r.text().contains("still being made"), "and said: {}", r.text());
    r.answer(Answered::Codes(vec![Secret::new("48271590")]));
    assert_eq!(r.pane(), Pane::BackupCodes, "that call's codes are shown");
}

fn change_password(held: HeldCalls) -> Rig {
    let mut r = rig("DW-A67", held);
    r.form(|f| {
        form::type_in(&mut f.current, BOARD_PASSWORD);
        form::type_in(&mut f.new, "a new long passphrase 42");
        form::type_in(&mut f.again, "a new long passphrase 42");
    });
    click(&mut r.rt, "btn-1");
    r
}

/// Review of desktop#427 (delta check, first P2): a password answer that lands after the person left the page is said
/// at once on the page that shows, still there when they leave Sign-in and security, and said once (the Change
/// password page says it in full and the notice goes).
#[test]
fn a_late_password_answer_is_said_wherever_the_person_is_and_once() {
    for (answer, notice, words) in [
        (Answered::Done, "Your password was changed", super::forms::CHANGED_AWAY),
        (
            Answered::Refused(Refused::Unconfirmed),
            "Your password change could not be confirmed",
            super::forms::UNCONFIRMED,
        ),
        (
            Answered::Refused(Refused::WrongPassword),
            "Your password was not changed",
            "Current password is incorrect. Nothing was changed.",
        ),
    ] {
        let mut r = change_password(HeldCalls::default());
        click(&mut r.rt, "btn-0");
        assert_eq!(r.pane(), Pane::Security, "left the page");
        r.answer(answer);
        assert_eq!(says(&r.rt, "late-change-text"), notice, "said where it landed");
        // Out of the account's sections: the notice stays with the person.
        r.rt.update(|s, _| {
            crate::shell::host::dispatch(s, Event::Navigate(Route::section(Section::Files)));
        });
        settle(&mut r.rt);
        assert_eq!(says(&r.rt, "late-change-text"), notice, "said over Files too");
        // Its button opens the Change password page, which says the answer in full; the notice goes.
        click(&mut r.rt, "late-change-action");
        assert_eq!(r.pane(), Pane::Password);
        assert_eq!(says(&r.rt, "field-0-msg"), words);
        assert!(super::tests::find(&r.rt, "late-change-text").is_none(), "said once");
        r.go("DW-A52");
        r.go("DW-A67");
        assert_eq!(r.rt.state.account.form.problem, None, "and not again");
    }
}

/// Review of desktop#427 (delta check, P1 re-rated P2): a page that adopts the wait of a change made under another
/// session (the same account signed in again while it was out) never says that change's answer as its own.
#[test]
fn an_adopted_password_wait_drops_an_answer_made_for_another_session() {
    let mut r = change_password(HeldCalls::default());
    click(&mut r.rt, "btn-0");
    // Another session of the same account, then the page again while the change is still out.
    r.held.moved.set(true);
    r.go("DW-A67");
    assert!(r.waiting(), "the wait shows");
    r.answer(Answered::Refused(Refused::Unconfirmed));
    assert!(!r.waiting(), "the page is live again");
    assert_eq!(
        r.rt.state.account.form.problem, None,
        "nothing of the other session's change is said"
    );
    assert!(super::tests::find(&r.rt, "late-change-text").is_none());
}

/// Review of desktop#427 (delta check, P3): Beebeeb refusing the new password (the same as now, too weak, or found in a
/// breach) is said in words, on the page and after the person left it.
#[test]
fn a_refused_new_password_is_said_in_words() {
    const WORDS: &str = "Beebeeb did not accept the new password. Nothing was changed. Choose a different one.";
    let mut r = change_password(HeldCalls::default());
    r.answer(Answered::Refused(Refused::Weak));
    assert_eq!(says(&r.rt, "field-1-msg"), WORDS, "on the page");
    let mut r = change_password(HeldCalls::default());
    click(&mut r.rt, "btn-0");
    r.answer(Answered::Refused(Refused::Weak));
    r.go("DW-A67");
    assert_eq!(
        says(&r.rt, "field-1-msg"),
        WORDS,
        "on return, with the new password gone"
    );
}
