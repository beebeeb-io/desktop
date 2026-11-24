//! Every password field of the account's forms is wiped on every path out of it: Done, a refusal, Cancel, Esc and
//! leaving the section (W1's rule). One exception is the board's own: DW-A68 keeps the NEW password after a wrong
//! current one (only the current one is typed again).

use super::Dialog;
use super::form;
use super::source::BOARD_PASSWORD;
use super::tests::{click, key, runtime, settle};
use crate::shell::app::{Event, Route, Section};
use crate::{State, init};
use rune::{Key, Runtime};

const NEW: &str = "a new long passphrase 42";

fn typed(rt: &Runtime<State>) -> [bool; 3] {
    let f = &rt.state.account.form;
    [
        !f.current.text.is_empty(),
        !f.new.text.is_empty(),
        !f.again.text.is_empty(),
    ]
}

fn fill(rt: &mut Runtime<State>, current: &str) {
    let current = current.to_string();
    rt.update(move |s, _| {
        let f = &mut s.account.form;
        form::type_in(&mut f.current, &current);
        form::type_in(&mut f.new, NEW);
        form::type_in(&mut f.again, NEW);
    });
    settle(rt);
}

fn check(fails: &mut Vec<String>, path: &str, rt: &Runtime<State>, wanted: [bool; 3]) {
    let left = typed(rt);
    if left != wanted {
        fails.push(format!(
            "{path}: typed (current, new, again) = {left:?}, wanted {wanted:?}"
        ));
    }
}

fn to_files(rt: &mut Runtime<State>) {
    rt.update(|s, _| {
        crate::shell::host::dispatch(s, Event::Navigate(Route::section(Section::Files)));
    });
    settle(rt);
}

#[test]
fn every_password_field_is_wiped_on_every_path_out() {
    let mut fails = Vec::new();

    // Change password.
    let mut rt = runtime("DW-A67");
    fill(&mut rt, BOARD_PASSWORD);
    click(&mut rt, "btn-1");
    check(&mut fails, "Change password, Done", &rt, [false; 3]);
    let mut rt = runtime("DW-A67");
    fill(&mut rt, "not the password");
    click(&mut rt, "btn-1");
    check(
        &mut fails,
        "Change password, refused (DW-A68 keeps the new one)",
        &rt,
        [false, true, true],
    );
    let mut rt = runtime("DW-A67");
    fill(&mut rt, BOARD_PASSWORD);
    click(&mut rt, "btn-0");
    check(&mut fails, "Change password, Cancel", &rt, [false; 3]);
    let mut rt = runtime("DW-A67");
    fill(&mut rt, BOARD_PASSWORD);
    key(&mut rt, Key::Escape);
    check(&mut fails, "Change password, Esc", &rt, [false; 3]);
    let mut rt = runtime("DW-A67");
    fill(&mut rt, BOARD_PASSWORD);
    to_files(&mut rt);
    check(&mut fails, "Change password, leaving the section", &rt, [false; 3]);

    // Change email (the board answers on the next poll).
    for (path, password) in [
        ("Change email, Done", BOARD_PASSWORD),
        ("Change email, refused", "not the password"),
    ] {
        let mut rt = runtime("DW-A65");
        rt.update(move |s, _| {
            form::type_in(&mut s.account.form.email, "new.address@example.eu");
            form::type_in(&mut s.account.form.current, password);
        });
        settle(&mut rt);
        click(&mut rt, "btn-1");
        settle(&mut rt);
        check(&mut fails, path, &rt, [false; 3]);
    }
    let mut rt = runtime("DW-A65");
    rt.update(|s, _| form::type_in(&mut s.account.form.current, BOARD_PASSWORD));
    settle(&mut rt);
    click(&mut rt, "btn-0");
    check(&mut fails, "Change email, Cancel", &rt, [false; 3]);

    // Removing a passkey: the confirm's password field (a source that asks it draws it).
    for (path, close) in [("Remove a passkey, Cancel", true), ("Remove a passkey, Esc", false)] {
        let mut rt = runtime("DW-A76");
        rt.update(|s, _| form::type_in(&mut s.account.form.current, BOARD_PASSWORD));
        settle(&mut rt);
        if close {
            click(&mut rt, "btn-1");
        } else {
            key(&mut rt, Key::Escape);
        }
        if rt.state.account.dialog.is_some() {
            fails.push(format!("{path}: the confirm is still open"));
        }
        check(&mut fails, path, &rt, [false; 3]);
    }

    // Stopping the sign-in activity: the password dialog.
    for (path, how) in [
        ("Stop activity, Turn off", 0),
        ("Stop activity, Cancel", 1),
        ("Stop activity, Esc", 2),
    ] {
        let mut init_state = init();
        assert!(super::scene("DW-A78", &mut init_state));
        let mut rt = super::tests::window(init_state);
        settle(&mut rt);
        rt.update(|s, _| form::type_in(&mut s.account.form.current, BOARD_PASSWORD));
        settle(&mut rt);
        match how {
            0 => click(&mut rt, "btn-0"),
            1 => click(&mut rt, "btn-1"),
            _ => key(&mut rt, Key::Escape),
        }
        if rt.state.account.dialog == Some(Dialog::Code(super::CodeFor::StopActivity)) {
            fails.push(format!("{path}: the dialog is still open"));
        }
        check(&mut fails, path, &rt, [false; 3]);
    }
    assert!(
        fails.is_empty(),
        "{} paths kept a password:\n{}",
        fails.len(),
        fails.join("\n")
    );
}
