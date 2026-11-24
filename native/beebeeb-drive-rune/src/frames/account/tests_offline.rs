//! Row (b) of the 11-15 fixture-only wave (W1): with no open session the source starts nothing
//! (`AccountSource::start_call` is `false`), and each form of the calls Beebeeb answers later shows the Offline line
//! and asks NOTHING: no other method stands in, no step-up, nothing on the wire, and the password leaves the field.

use super::form;
use super::source::{BOARD_CODE, BOARD_PASSWORD, BoardAccount, HeldCalls, Secret, Setup};
use super::tests::{click, find, settle, window};
use super::{CodeFor, Dialog};
use crate::{State, init};
use rune::Runtime;

/// A page of `frame` on a source with no open session.
fn signed_out(frame: &str) -> Runtime<State> {
    let held = HeldCalls::default();
    held.refuse.set(true);
    let mut state = init();
    assert!(super::scene(frame, &mut state), "{frame} is a built frame");
    state.account.source = Some(Box::new(BoardAccount::new().holding_calls(held)));
    state.account.form.setup = None;
    form::arrive(&mut state.account);
    let mut rt = window(state);
    settle(&mut rt);
    rt
}

fn sent(rt: &Runtime<State>) -> Vec<String> {
    rt.state.account.source.as_deref().expect("the fixture").sent()
}

/// The words a key shows, or none.
fn words(rt: &Runtime<State>, key: &str) -> String {
    find(rt, key).and_then(|info| info.text).unwrap_or_default()
}

/// One control's result: what it said, what it sent, and whether the password stayed in its field.
fn verdict(rt: &Runtime<State>, control: &str, key: &str, wanted: &str, fails: &mut Vec<String>) {
    let said = words(rt, key);
    if !said.contains(wanted) {
        fails.push(format!("{control}: says {said:?} under {key}, not {wanted:?}"));
    }
    let sent = sent(rt);
    if !sent.is_empty() {
        fails.push(format!("{control}: asked {sent:?}"));
    }
    if !rt.state.account.form.current.text.is_empty() {
        fails.push(format!("{control}: the password stayed in the field"));
    }
}

#[test]
fn with_no_open_session_every_call_form_says_offline_and_asks_nothing() {
    let mut fails = Vec::new();
    let mut controls = 0;

    // Change email, step 1: the password is never proved by another call, and leaves the field.
    let mut rt = signed_out("DW-A65");
    rt.update(|s, _| {
        form::type_in(&mut s.account.form.email, "new.address@example.eu");
        form::type_in(&mut s.account.form.current, BOARD_PASSWORD);
    });
    settle(&mut rt);
    click(&mut rt, "btn-1");
    verdict(&rt, "Send code", "field-0-msg", "can't be reached", &mut fails);
    controls += 1;

    // Change email, step 2: the code.
    let mut rt = signed_out("DW-A66");
    rt.update(|s, _| form::type_in(&mut s.account.form.code, "48271590"));
    settle(&mut rt);
    click(&mut rt, "btn-1");
    verdict(
        &rt,
        "Confirm the email's code",
        "code-msg",
        "can't be reached",
        &mut fails,
    );
    controls += 1;

    // Send again: no other way to send it stands in.
    let mut rt = signed_out("DW-A66");
    click(&mut rt, "link-0-box");
    verdict(&rt, "Send again", "code-msg", "can't be reached", &mut fails);
    controls += 1;

    // The set-up page asks for a key on arrival: none is drawn.
    let rt = signed_out("DW-A69");
    verdict(
        &rt,
        "The set-up's key",
        "code-msg",
        "Could not start the set-up",
        &mut fails,
    );
    if rt.state.account.form.setup.is_some() {
        fails.push("The set-up's key: a key is drawn".into());
    }
    controls += 1;

    // The first code (a key on the page, then the session ended).
    let mut rt = signed_out("DW-A69");
    rt.update(|s, _| {
        s.account.form.setup = Some(Setup {
            modules: vec![vec![true; 21]; 21],
            key: Secret::new("JBSW Y3DP EHPK 3PXP"),
        });
        s.account.form.problem = None;
        form::type_in(&mut s.account.form.code, BOARD_CODE);
    });
    settle(&mut rt);
    let before = sent(&rt).len();
    click(&mut rt, "btn-1");
    // (What the arrival asked is counted in the control above.)
    if sent(&rt).len() > before {
        fails.push(format!("Turn on: asked {:?}", &sent(&rt)[before..]));
    }
    let said = words(&rt, "code-msg");
    if !said.contains("could not be reached") {
        fails.push(format!("Turn on: says {said:?}"));
    }
    controls += 1;

    // The three code confirms.
    for for_ in [
        CodeFor::TurnOffTwoFactor,
        CodeFor::MoveAuthenticator,
        CodeFor::NewBackupCodes,
    ] {
        let mut rt = signed_out("DW-A71");
        rt.update(|s, _| super::ask(s, Dialog::Code(for_)));
        settle(&mut rt);
        rt.update(|s, _| form::type_in(&mut s.account.form.code, BOARD_CODE));
        settle(&mut rt);
        click(&mut rt, "btn-0");
        verdict(
            &rt,
            &format!("{for_:?}"),
            "code-msg",
            "could not be reached",
            &mut fails,
        );
        controls += 1;
    }
    assert_eq!(controls, 8, "every control of a call answered later");
    assert!(
        fails.is_empty(),
        "{} findings over the 8 controls:\n{}",
        fails.len(),
        fails.join("\n")
    );
}
