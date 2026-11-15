//! Row (b) of the 11-15 fixture-only wave (W1): with no open session the source starts nothing
//! (`AccountSource::start_call` is `false`), and each form of the calls Beebeeb answers later shows the Offline line
//! and asks NOTHING: no other method stands in, no step-up, nothing on the wire, and the password leaves the field.

use super::form;
use super::source::{BOARD_CODE, BOARD_PASSWORD, BoardAccount, HeldCalls, Secret, Setup};
use super::tests::{click, find, says, settle, window};
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

fn code_msg(rt: &Runtime<State>) -> String {
    find(rt, "code-msg")
        .and_then(|info| info.text)
        .unwrap_or_else(|| panic!("no words under the code: {}", rt.accessibility_json()))
}

#[test]
fn with_no_open_session_every_call_form_says_offline_and_asks_nothing() {
    // (the control, what it says)
    let mut said = Vec::new();

    // Change email, step 1: the password is never proved by another call, and leaves the field.
    let mut rt = signed_out("DW-A65");
    rt.update(|s, _| {
        form::type_in(&mut s.account.form.email, "new.address@example.eu");
        form::type_in(&mut s.account.form.current, BOARD_PASSWORD);
    });
    settle(&mut rt);
    click(&mut rt, "btn-1");
    assert_eq!(
        says(&rt, "field-0-msg"),
        "Beebeeb can't be reached. Nothing was changed."
    );
    assert!(
        rt.state.account.form.current.text.is_empty(),
        "the password left the field"
    );
    assert_eq!(sent(&rt), Vec::<String>::new(), "Send code asked nothing");
    said.push("StartEmail");

    // Change email, step 2: the code.
    let mut rt = signed_out("DW-A66");
    rt.update(|s, _| form::type_in(&mut s.account.form.code, "48271590"));
    settle(&mut rt);
    click(&mut rt, "btn-1");
    assert!(code_msg(&rt).contains("can't be reached"), "{}", code_msg(&rt));
    assert_eq!(sent(&rt), Vec::<String>::new(), "Confirm asked nothing");
    said.push("ConfirmEmail");

    // Send again: no other way to send it stands in.
    let mut rt = signed_out("DW-A66");
    click(&mut rt, "link-0-box");
    assert!(code_msg(&rt).contains("can't be reached"), "{}", code_msg(&rt));
    assert_eq!(sent(&rt), Vec::<String>::new(), "Send again asked nothing");
    said.push("ResendEmail");

    // The set-up page asks for a key on arrival.
    let rt = signed_out("DW-A69");
    assert!(rt.state.account.form.setup.is_none(), "no key is drawn");
    assert!(
        code_msg(&rt).contains("Could not start the set-up"),
        "{}",
        code_msg(&rt)
    );
    assert_eq!(sent(&rt), Vec::<String>::new(), "the set-up asked nothing");
    said.push("BeginTwoFactor");

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
    click(&mut rt, "btn-1");
    assert!(code_msg(&rt).contains("could not be reached"), "{}", code_msg(&rt));
    assert_eq!(sent(&rt), Vec::<String>::new(), "Turn on asked nothing");
    said.push("TurnOnTwoFactor");

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
        assert!(
            code_msg(&rt).contains("could not be reached"),
            "{for_:?}: {}",
            code_msg(&rt)
        );
        assert_eq!(sent(&rt), Vec::<String>::new(), "{for_:?} asked nothing");
        said.push("code confirm");
    }
    assert_eq!(said.len(), 8, "every control of a call answered later");
}
