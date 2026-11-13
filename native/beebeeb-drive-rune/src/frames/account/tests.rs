//! Tests of the account sections: structure, keyboard, roles and names, and the step-up contract.
//!
//! Input is scripted: keys and presses go through Rune's `Runtime`, as a person's would.

use super::source::{AccountSource, BoardAccount, Password, ProductState, Purpose, Refused};
use super::{Dialog, Pane};
use crate::frames::settings::Intent;
use crate::shell::app::{Screen, SettingsPane};
use crate::shell::host;
use crate::{State, init, view};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

/// A window showing board frame `frame`.
pub(super) fn runtime(frame: &str) -> Runtime<State> {
    let mut state = init();
    assert!(super::scene(frame, &mut state), "{frame} is a built frame");
    window(state)
}

pub(super) fn window(state: State) -> Runtime<State> {
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 638.0), env);
    // The app's own wiring (`shell::run`): the shell's keys first, then the frame's.
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    rt
}

pub(super) fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

pub(super) fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"))
}

pub(super) fn says(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

pub(super) fn name(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).label.unwrap_or_default()
}

/// Lets leaving elements finish leaving: Rune keeps them in the tree while they animate out.
pub(super) fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}

pub(super) fn key(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

pub(super) fn click(rt: &mut Runtime<State>, key: &str) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

/// Tab until the element keyed `target` has focus; panics when focus never gets there.
pub(super) fn tab_to(rt: &mut Runtime<State>, target: &str) {
    for _ in 0..80 {
        if find(rt, target).is_some_and(|e| e.focused) {
            return;
        }
        key(rt, Key::Tab);
    }
    panic!("Tab never reaches {target}");
}

fn tab_stops(rt: &mut Runtime<State>) -> Vec<String> {
    let mut stops: Vec<String> = Vec::new();
    for _ in 0..120 {
        key(rt, Key::Tab);
        let Some(now) = rt
            .elements()
            .into_iter()
            .find(|e| e.focused && !e.exiting)
            .and_then(|e| e.key)
        else {
            continue;
        };
        if stops.contains(&now) {
            break;
        }
        stops.push(now);
    }
    stops
}

pub(super) fn section(rt: &Runtime<State>) -> Option<SettingsPane> {
    match rt.state.shell.app.screen() {
        Screen::Settings(pane) => Some(pane),
        _ => None,
    }
}

fn source(rt: &Runtime<State>) -> &dyn AccountSource {
    rt.state.account.source.as_deref().expect("the fixture")
}

pub(super) fn asked(rt: &Runtime<State>) -> Vec<Intent> {
    rt.state.settings.intents.clone()
}

#[test]
fn every_built_frame_renders_and_names_no_city_and_no_provider() {
    assert_eq!(super::FRAMES.len(), 39, "the frames this lane built");
    for (frame, _) in super::FRAMES {
        let rt = runtime(frame);
        assert!(find(&rt, "caption-bar").is_some(), "{frame}: the same window");
        // DW-A83 is a screen of its own; behind DW-A72's confirm the board draws no heading.
        let title = match *frame {
            "DW-A83" => "title",
            "DW-A72" => "dlg-title",
            _ => "set-title",
        };
        assert_eq!(find(&rt, "set-main").is_some(), *frame != "DW-A83", "{frame}");
        assert_eq!(info(&rt, title).role, "heading", "{frame}");
        for e in rt.elements() {
            let words = format!("{} {}", e.text.unwrap_or_default(), e.label.unwrap_or_default()).to_lowercase();
            for banned in [
                "hetzner",
                "falkenstein",
                "nuremberg",
                "helsinki",
                "frankfurt",
                "amsterdam",
            ] {
                assert!(!words.contains(banned), "{frame} says {banned}: {words}");
            }
        }
    }
}

#[test]
fn the_settings_sidebar_leads_to_the_three_sections_and_back() {
    // From General (the settings lane's pane), by its sidebar: the shell routes, this lane draws.
    let mut state = init();
    assert!(crate::frames::settings::scene("DW-0701", &mut state));
    let mut rt = window(state);
    for (row, want, title) in [
        ("Account-row", SettingsPane::Account, "Account"),
        ("Security-row", SettingsPane::Security, "Sign-in and security"),
        (
            "Devices and sessions-row",
            SettingsPane::DevicesAndSessions,
            "Devices and sessions",
        ),
    ] {
        click(&mut rt, row);
        assert_eq!(section(&rt), Some(want), "{row}");
        assert_eq!(says(&rt, "set-title"), title);
        assert_eq!(info(&rt, row).role, "link");
    }
    // And out again: another lane's section, then Drive.
    click(&mut rt, "General-row");
    assert_eq!(section(&rt), Some(SettingsPane::General));
    click(&mut rt, "Account-row");
    key(&mut rt, Key::Escape);
    assert_eq!(section(&rt), None, "Esc goes back to Drive");
}

#[test]
fn account_says_who_is_signed_in_and_the_phrase_row_is_text() {
    let mut rt = runtime("DW-A51");
    assert_eq!(says(&rt, "item-0-0-title"), "Anna Visser");
    assert_eq!(says(&rt, "item-0-1-title"), "anna@northwind.studio");
    // The recovery row reads 'Recovery phrase, shown once at sign-up'; it is not a button.
    let phrase = info(&rt, "item-1-0");
    assert_eq!(phrase.role, "listitem");
    assert_eq!(phrase.label.as_deref(), Some("Recovery phrase, shown once at sign-up"));
    let stops = tab_stops(&mut rt);
    assert!(!stops.iter().any(|k| k.starts_with("item-1-")), "{stops:?}");
    // The product states are text, not only colour.
    assert_eq!(says(&rt, "item-2-0-pill-label"), "This app");
    assert_eq!(says(&rt, "item-2-2-pill-label"), "Signed in");
    assert_eq!(says(&rt, "item-2-1-value"), "Not signed in");
    assert_eq!(name(&rt, "item-2-2"), "VPN, Signed in");

    // Name and email open their pages; the plan card opens the plan.
    tab_to(&mut rt, "item-0-0");
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.pane, Pane::Name);
    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.account.pane, Pane::Account, "Esc cancels");
    click(&mut rt, "item-0-1");
    assert_eq!(rt.state.account.pane, Pane::Email);
    key(&mut rt, Key::Escape);
    click(&mut rt, "card-0");
    assert_eq!(
        rt.state.account.dialog,
        Some(Dialog::Plan),
        "the plan, read only (DW-0708)"
    );
    assert_eq!(asked(&rt), Vec::new());
}

#[test]
fn signing_out_is_confirmed_and_enter_is_cancel() {
    let mut rt = runtime("DW-A52");
    // Destructive rows are announced as such.
    assert_eq!(name(&rt, "item-3-0"), "Sign out of Drive, destructive");
    assert_eq!(name(&rt, "item-4-0"), "Delete your Beebeeb account, destructive");
    tab_to(&mut rt, "item-3-0");
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.dialog, Some(Dialog::SignOutApp));
    assert_eq!(info(&rt, "dlg").role, "dialog");
    assert_eq!(name(&rt, "dlg"), "Sign out of Drive?");
    assert!(says(&rt, "dlg-text-0").starts_with("This signs out Drive on this PC only."));
    // Enter is Cancel (the safe choice): nothing is signed out, nothing is asked of the shell.
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.dialog, None);
    assert!(asked(&rt).is_empty());
    assert_eq!(section(&rt), Some(SettingsPane::Security));

    // Esc cancels, and leaves settings open.
    click(&mut rt, "item-3-0");
    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.account.dialog, None);
    assert_eq!(section(&rt), Some(SettingsPane::Security));

    // The red button signs out this app only and opens Sign in.
    click(&mut rt, "item-3-0");
    assert_eq!(name(&rt, "btn-1"), "Sign out, destructive");
    tab_to(&mut rt, "btn-1");
    key(&mut rt, Key::Text(" ".into()));
    assert_eq!(asked(&rt), vec![Intent::Show("DW-A02")]);
    assert_eq!(
        source(&rt).products()[2].1,
        ProductState::SignedIn,
        "the other Beebeeb apps stay signed in"
    );
}

#[test]
fn signing_out_of_all_apps_is_its_own_confirm() {
    let mut rt = runtime("DW-A57a");
    assert_eq!(name(&rt, "dlg"), "Sign out of all Beebeeb apps on this PC?");
    assert_eq!(says(&rt, "btn-1-label"), "Sign out of all");
    click(&mut rt, "btn-2");
    assert_eq!(rt.state.account.dialog, None, "Cancel");
    assert!(asked(&rt).is_empty());
    click(&mut rt, "item-3-1");
    click(&mut rt, "btn-1");
    assert_eq!(asked(&rt), vec![Intent::Show("DW-A03")]);
    assert!(source(&rt).products().iter().all(|p| p.1 == ProductState::NotSignedIn));
}

#[test]
fn app_lock_turns_by_keyboard_and_without_it_there_is_no_time_choice() {
    let mut rt = runtime("DW-A52");
    let sw = info(&rt, "item-1-0-switch");
    assert_eq!(sw.role, "switch");
    assert_eq!(sw.label.as_deref(), Some("Lock Beebeeb"));
    assert!(find(&rt, "item-1-0-pop").is_some());
    tab_to(&mut rt, "item-1-0-switch");
    key(&mut rt, Key::Text(" ".into()));
    assert!(!source(&rt).security().app_lock);
    assert!(find(&rt, "item-1-0-pop").is_none(), "off: no time choice (DW-0716b)");
    assert_eq!(says(&rt, "item-1-0-sub"), "Off: anyone using this PC can open Beebeeb");
    key(&mut rt, Key::Enter);
    assert!(source(&rt).security().app_lock);
}

#[test]
fn a_device_is_signed_out_after_its_confirm_and_this_pc_has_no_button() {
    let mut rt = runtime("DW-A55");
    assert_eq!(
        name(&rt, "item-0-1"),
        "Anna's iPhone, Drive, Pass, last active 12 min ago"
    );
    assert_eq!(says(&rt, "item-0-0-pill-label"), "This PC");
    assert_eq!(says(&rt, "item-0-0-value"), "DE");
    let stops = tab_stops(&mut rt);
    assert_eq!(
        stops.iter().filter(|k| k.starts_with("btn-")).count(),
        4,
        "three devices and Sign out everywhere else: {stops:?}"
    );
    assert_eq!(name(&rt, "btn-0"), "Sign out Anna's iPhone");
    tab_to(&mut rt, "btn-0");
    key(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.account.dialog,
        Some(Dialog::SignOutDevice("dev-iphone".into()))
    );
    assert_eq!(says(&rt, "dlg-title"), "Sign out Anna's iPhone?");
    // The red button is labelled with the device name.
    assert_eq!(name(&rt, "btn-3"), "Sign out Anna's iPhone, destructive");
    // Enter is Cancel.
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.dialog, None);
    assert_eq!(source(&rt).sessions().len(), 4, "nothing was signed out");

    click(&mut rt, "btn-0");
    click(&mut rt, "btn-3");
    assert_eq!(rt.state.account.dialog, None);
    let left: Vec<String> = source(&rt).sessions().into_iter().map(|s| s.name).collect();
    assert_eq!(
        left,
        ["Anna-Laptop", "Studio-PC (bb)", "Chrome on Windows"],
        "the row disappears"
    );
    assert_eq!(says(&rt, "item-0-1-title"), "Studio-PC (bb)");
}

#[test]
fn signing_out_everywhere_else_keeps_this_pc() {
    let mut rt = runtime("DW-A55");
    click(&mut rt, "btn-3");
    assert_eq!(rt.state.account.dialog, Some(Dialog::SignOutElsewhere));
    key(&mut rt, Key::Escape);
    assert_eq!(source(&rt).sessions().len(), 4, "Esc cancels");
    click(&mut rt, "btn-3");
    click(&mut rt, "btn-3");
    let left = source(&rt).sessions();
    assert_eq!(left.len(), 1);
    assert!(left[0].this_device);
    assert!(
        find(&rt, "btn-0").is_none(),
        "no other device: nothing left to sign out"
    );
}

#[test]
fn nothing_stays_open_across_leaving_the_section_and_coming_back() {
    let mut rt = runtime("DW-A57");
    assert!(find(&rt, "dlg").is_some());
    key(&mut rt, Key::Escape); // the confirm
    key(&mut rt, Key::Escape); // settings
    assert_eq!(section(&rt), None);
    let mut rt = runtime("DW-A67");
    click(&mut rt, "Account-row");
    click(&mut rt, "Security-row");
    assert_eq!(rt.state.account.pane, Pane::Security, "a section opens on its own pane");
    assert_eq!(says(&rt, "set-title"), "Sign-in and security");
}

#[test]
fn a_build_that_ships_never_shows_the_sample_content() {
    assert!(super::source::first(false).is_none());
    let mut state = init();
    assert!(super::scene("DW-A51", &mut state));
    state.account.source = None;
    let rt = window(state);
    assert!(find(&rt, "set-main").is_none(), "no pane of sample values");
    for e in rt.elements() {
        let words = format!("{} {}", e.text.unwrap_or_default(), e.label.unwrap_or_default());
        assert!(!words.contains("Anna") && !words.contains("northwind"), "{words}");
    }
}

#[test]
fn the_fixture_is_the_board() {
    let board = BoardAccount::new();
    let sec = board.security();
    assert_eq!(
        (
            sec.password_changed.as_str(),
            sec.two_factor,
            sec.passkeys,
            sec.sessions
        ),
        ("3 weeks ago", true, 2, 4)
    );
    assert_eq!(super::source::score(&board.factors()), (7, 7));
    assert_eq!(
        super::source::score(&BoardAccount::new().without_passkeys().factors()),
        (6, 7)
    );
    let countries: Vec<String> = board.sessions().into_iter().map(|s| s.country).collect();
    assert!(
        countries.iter().all(|c| c.len() == 2),
        "a country in two letters, never a city"
    );
}

/// The step-up contract: the password goes into `step_up` and nowhere else; what comes back
/// is a confirmation for one purpose, used once; nothing the fixture "sends" holds the password.
#[test]
fn a_step_up_takes_the_password_and_hands_back_a_confirmation_never_text() {
    const SECRET: &str = "correct horse battery staple";
    let mut board = BoardAccount::new();
    assert_eq!(
        board
            .step_up(Password::from_field("not the password"), Purpose::DeleteAccount)
            .map(|_| ())
            .unwrap_err(),
        Refused::WrongPassword
    );
    let confirmation = board
        .step_up(Password::from_field(SECRET), Purpose::ChangePassword)
        .expect("the right password");
    assert_eq!(confirmation.purpose(), Purpose::ChangePassword);
    let shown = format!("{confirmation:?}");
    assert_eq!(
        shown, "StepUp(ChangePassword)",
        "no handle and no secret in what a log would print"
    );
    assert_eq!(board.wire.len(), 2);
    for sent in &board.wire {
        assert!(!sent.contains(SECRET) && !sent.contains("not the password"), "{sent}");
        assert!(sent.starts_with("POST auth/step-up"), "{sent}");
    }
    // For another purpose it is worth nothing; for its own, once.
    let other = board
        .step_up(Password::from_field(SECRET), Purpose::TwoFactor)
        .expect("the right password");
    assert!(!board.redeem(other, Purpose::DeleteAccount));
    assert!(board.redeem(confirmation, Purpose::ChangePassword));
    assert!(Password::from_field("").is_empty());
}

/// A secret that passed through the source is in no element's text and in no label, on any
/// built frame (the trees a screen reader, the frame gate and the feedback mode read).
#[test]
fn a_seeded_secret_is_in_no_element_tree_and_no_label() {
    const SECRET: &str = "correct horse battery staple";
    for (frame, _) in super::FRAMES {
        let mut state = init();
        assert!(super::scene(frame, &mut state));
        let granted = state
            .account
            .source
            .as_mut()
            .expect("the fixture")
            .step_up(Password::from_field(SECRET), Purpose::ChangePassword);
        assert!(granted.is_ok());
        let rt = window(state);
        let mut seen = 0;
        for e in rt.elements() {
            seen += 1;
            let words = format!(
                "{} {} {}",
                e.key.unwrap_or_default(),
                e.text.unwrap_or_default(),
                e.label.unwrap_or_default()
            );
            assert!(
                !words.contains(SECRET) && !words.contains("battery"),
                "{frame}: {words}"
            );
        }
        assert!(seen > 40, "{frame}: the tree was read ({seen} elements)");
    }
}

// ---------------------------------------------------------------------------
// The pages with a field, a code or a secret (DW-A58 to DW-A83)
// ---------------------------------------------------------------------------

use super::form;
use super::source::{BOARD_CODE, BOARD_EMAIL_CODE, BOARD_PASSWORD, CodeRefused, Deletion, Strength};

/// Everything a screen reader, the frame gate or the feedback mode could read of the window.
fn every_word(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .map(|e| {
            format!(
                "{} {} {}\n",
                e.key.unwrap_or_default(),
                e.text.unwrap_or_default(),
                e.label.unwrap_or_default()
            )
        })
        .collect()
}

/// The typed passwords, the two-factor key, the backup codes and a typed code are in no
/// element's key, text or label: each is drawn by a secure node. (Seen red: `codes()` without
/// `.secure()` puts 48271590 in the tree and fails this.)
#[test]
fn typed_and_shown_secrets_are_in_no_element_tree_and_no_label() {
    const TYPED: &str = "seeded-secret-hunter2";
    let mut state = init();
    assert!(super::scene("DW-A67", &mut state));
    for edit in [
        &mut state.account.form.current,
        &mut state.account.form.new,
        &mut state.account.form.again,
    ] {
        form::type_in(edit, TYPED);
    }
    let words = every_word(&window(state));
    assert!(words.contains("Change password"), "the page was read");
    assert!(!words.contains(TYPED) && !words.contains("hunter2"), "{words}");

    let words = every_word(&runtime("DW-A69"));
    assert!(words.contains("Or enter this key"), "the page was read");
    for secret in ["JBSW", "3PXP", "EHPK"] {
        assert!(!words.contains(secret), "the set-up key is in the tree: {words}");
    }
    let words = every_word(&runtime("DW-A70"));
    assert!(words.contains("Save your backup codes"), "the page was read");
    for code in ["48271590", "30617724", "04198873"] {
        assert!(!words.contains(code), "a backup code is in the tree: {words}");
    }
    let mut state = init();
    assert!(super::scene("DW-A80", &mut state));
    form::type_in(&mut state.account.form.code, "913577");
    let words = every_word(&window(state));
    assert!(words.contains("6-digit code"), "the input is named");
    assert!(!words.contains("913577"), "a typed code is in the tree: {words}");
}

/// Changing the password: a wrong current password changes nothing and keeps the new one
/// (DW-A68); the right one goes through the step-up, and no request holds either password.
#[test]
fn the_password_changes_through_the_step_up_and_is_in_no_request() {
    const NEW: &str = "a new long passphrase 42";
    let mut rt = runtime("DW-A67");
    rt.update(|s, _| {
        let f = &mut s.account.form;
        form::type_in(&mut f.current, "not the password");
        form::type_in(&mut f.new, NEW);
        form::type_in(&mut f.again, NEW);
    });
    settle(&mut rt);
    click(&mut rt, "btn-1");
    assert_eq!(rt.state.account.pane, Pane::Password);
    assert_eq!(
        says(&rt, "field-0-msg"),
        "Current password is incorrect. Nothing was changed."
    );
    assert_eq!(rt.state.account.form.new.text, NEW, "the new password is kept");
    assert!(
        rt.state.account.form.current.text.is_empty(),
        "the current one is retyped"
    );
    assert_eq!(source(&rt).security().password_changed, "3 weeks ago");

    rt.update(|s, _| form::type_in(&mut s.account.form.current, BOARD_PASSWORD));
    settle(&mut rt);
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.pane, Pane::Security, "back to Sign-in and security");
    assert_eq!(source(&rt).security().password_changed, "Just now");
    assert!(
        rt.state.account.form.new.text.is_empty(),
        "nothing typed outlives the page"
    );

    // The same on the source, where the requests can be read.
    let mut board = BoardAccount::new();
    let confirmation = board
        .step_up(Password::from_field(BOARD_PASSWORD), Purpose::ChangePassword)
        .expect("the right password");
    assert!(board.change_password(confirmation, Password::from_field(NEW)).is_ok());
    let wrong = board
        .step_up(Password::from_field(BOARD_PASSWORD), Purpose::ChangeEmail)
        .expect("the right password");
    assert_eq!(
        board.change_password(wrong, Password::from_field(NEW)),
        Err(Refused::NotConfirmed),
        "a confirmation for something else changes nothing"
    );
    assert_eq!(
        board.password_strength(&Password::from_field("short")),
        Strength::TooShort
    );
    for sent in &board.wire {
        assert!(!sent.contains(NEW) && !sent.contains(BOARD_PASSWORD), "{sent}");
    }
}

/// Name and email: an empty name is refused in the board's words; the email changes only once
/// the code from the new address is entered.
#[test]
fn a_name_is_saved_by_enter_and_an_email_changes_after_its_code() {
    let mut rt = runtime("DW-A64");
    assert!(info(&rt, form::NAME_KEY).focused, "the name field opens in focus");
    rt.update(|s, _| form::type_in(&mut s.account.form.name, "  "));
    settle(&mut rt);
    key(&mut rt, Key::Enter);
    assert_eq!(says(&rt, "field-0-msg"), "A name can't be empty.");
    rt.update(|s, _| form::type_in(&mut s.account.form.name, "Anna de Visser"));
    settle(&mut rt);
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.pane, Pane::Account);
    assert_eq!(source(&rt).identity().name, "Anna de Visser");

    let mut rt = runtime("DW-A65");
    rt.update(|s, _| form::type_in(&mut s.account.form.current, BOARD_PASSWORD));
    settle(&mut rt);
    click(&mut rt, "btn-1");
    assert_eq!(rt.state.account.pane, Pane::EmailCode);
    assert!(!info(&rt, "btn-1").focusable, "Confirm waits for eight digits");
    assert_eq!(source(&rt).identity().email, "anna@northwind.studio", "not changed yet");
    rt.update(|s, _| form::type_in(&mut s.account.form.code, BOARD_EMAIL_CODE));
    settle(&mut rt);
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.pane, Pane::Account);
    assert_eq!(source(&rt).identity().email, "anna.visser@northwind.studio");
}

/// Two-factor: a wrong code turns nothing off; the right one does; Esc cancels the confirm.
#[test]
fn two_factor_turns_off_only_with_a_current_code() {
    let mut rt = runtime("DW-A71");
    assert_eq!(name(&rt, "item-0-0"), "Two-factor on");
    click(&mut rt, "item-2-1");
    assert_eq!(
        rt.state.account.dialog,
        Some(Dialog::Code(super::CodeFor::TurnOffTwoFactor))
    );
    assert_eq!(name(&rt, "btn-0"), "Turn off, destructive");
    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.account.dialog, None, "Esc cancels");
    click(&mut rt, "item-2-1");
    rt.update(|s, _| form::type_in(&mut s.account.form.code, "000000"));
    settle(&mut rt);
    click(&mut rt, "btn-0");
    assert_eq!(says(&rt, "code-msg"), "Incorrect code. Try again.");
    assert!(source(&rt).two_factor().on, "nothing changed");
    rt.update(|s, _| form::type_in(&mut s.account.form.code, BOARD_CODE));
    settle(&mut rt);
    key(&mut rt, Key::Enter);
    assert!(!source(&rt).two_factor().on);
    assert_eq!(rt.state.account.pane, Pane::Security);
}

/// Passkeys: the menu's Remove asks first, Enter in the confirm is Cancel, and the last
/// passkey that can unlock the data gets the bold warning.
#[test]
fn a_passkey_is_removed_only_after_its_confirm() {
    let mut rt = runtime("DW-A73");
    assert_eq!(name(&rt, "item-0-0"), "iCloud Keychain, added 28 Oct, Can unlock data");
    click(&mut rt, "item-0-0-more");
    assert_eq!(name(&rt, "pop-0-item-1"), "Remove, destructive");
    click(&mut rt, "pop-0-item-1");
    assert_eq!(says(&rt, "dlg-title"), "Remove iCloud Keychain?");
    assert!(find(&rt, "dlg-bold-box").is_some(), "the warning in bold");
    key(&mut rt, Key::Enter);
    assert_eq!(source(&rt).passkeys().len(), 2, "Enter is Cancel");
    click(&mut rt, "item-0-0-more");
    click(&mut rt, "pop-0-item-1");
    click(&mut rt, "btn-0");
    assert_eq!(source(&rt).passkeys().len(), 1);
    click(&mut rt, "btn-0");
    assert_eq!(
        source(&rt).passkeys().len(),
        2,
        "Add a passkey adds one (Windows Hello)"
    );
}

/// Deleting the account: every step in order, nothing scheduled until the link is used, a
/// wrong code keeps the proof of the password, and the undo ends on its own screen.
#[test]
fn deleting_the_account_takes_four_steps_and_can_be_undone() {
    let mut rt = runtime("DW-A58");
    assert_eq!(
        says(&rt, "set-sub"),
        "After you confirm by email, you have 72 hours to undo it. After that, it can't be undone."
    );
    assert_eq!(name(&rt, "btn-1"), "Continue, destructive");
    rt.update(|s, _| form::type_in(&mut s.account.form.current, "wrong"));
    settle(&mut rt);
    click(&mut rt, "btn-1");
    assert_eq!(says(&rt, "field-0-msg"), "Incorrect password. Nothing was deleted.");
    rt.update(|s, _| form::type_in(&mut s.account.form.current, BOARD_PASSWORD));
    settle(&mut rt);
    click(&mut rt, "btn-1");
    assert_eq!(rt.state.account.pane, Pane::DeleteFactor);
    assert!(!info(&rt, "btn-1").focusable, "Continue waits for six digits");
    rt.update(|s, _| form::type_in(&mut s.account.form.code, "000000"));
    settle(&mut rt);
    click(&mut rt, "btn-1");
    assert_eq!(
        rt.state.account.pane,
        Pane::DeleteFactor,
        "a wrong code stays on step 2"
    );
    assert_eq!(source(&rt).deletion(), Deletion::None);
    rt.update(|s, _| form::type_in(&mut s.account.form.code, BOARD_CODE));
    settle(&mut rt);
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.pane, Pane::DeleteLink);
    assert_eq!(source(&rt).deletion(), Deletion::LinkSent, "nothing is scheduled yet");
    click(&mut rt, "btn-0");
    assert_eq!(rt.state.account.pane, Pane::DeleteScheduled);
    assert_eq!(
        says(&rt, "set-sub"),
        "Your account will be deleted in 72 hours, on 8 November at 19:42."
    );
    click(&mut rt, "btn-0");
    assert_eq!(rt.state.account.pane, Pane::DeleteUndone);
    assert_eq!(says(&rt, "title"), "Deletion cancelled");
    assert_eq!(source(&rt).deletion(), Deletion::None);

    let mut board = BoardAccount::new();
    assert_eq!(board.stop_activity("000000"), Err(CodeRefused::Wrong));
    assert!(board.activity().is_some(), "a wrong code stops nothing");
}

/// The board's sample devices, word for word (DW-A55's rows, DW-A56's sentence): the confirm
/// says what the source gives for THAT device, and a device with one app gets the plain form.
#[test]
fn the_confirm_names_the_apps_the_source_gives_for_that_device() {
    let board = BoardAccount::new();
    let rows: Vec<(String, String)> = board.sessions().into_iter().map(|s| (s.name, s.apps)).collect();
    let rows: Vec<(&str, &str)> = rows.iter().map(|(n, a)| (n.as_str(), a.as_str())).collect();
    assert_eq!(
        rows,
        vec![
            ("Anna-Laptop", "Drive, Drive"),
            ("Anna's iPhone", "Drive, Pass"),
            ("Studio-PC (bb)", "Command line"),
            ("Chrome on Windows", "Drive web"),
        ],
        "DW-A55, as the board draws it"
    );
    let mut rt = runtime("DW-A56");
    assert_eq!(says(&rt, "dlg-title"), "Sign out Anna's iPhone?");
    assert!(
        says(&rt, "dlg-text-0").starts_with("Drive and Drive on that device are signed out the next time"),
        "DW-A56, as the board draws it"
    );
    // Another source, another device: its own apps, never the sample's.
    rt.update(|s, _| {
        s.account.dialog = None;
        let source = s.account.source.as_deref_mut().expect("the fixture");
        source.sign_out_device("dev-iphone");
    });
    settle(&mut rt);
    assert!(source(&rt).sessions().iter().all(|s| s.name != "Anna's iPhone"));
    click(&mut rt, "btn-0");
    assert_eq!(says(&rt, "dlg-title"), "Sign out Studio-PC (bb)?");
    assert!(
        says(&rt, "dlg-text-0").starts_with("The Beebeeb apps on that device are signed out"),
        "one app there: no pair of names"
    );
}

/// Sign-in activity (DW-A77 to DW-A79): turning it off asks for a current code first (DW-A78),
/// Esc and a wrong code stop nothing, the right code stops it, and turning it on asks nothing.
#[test]
fn sign_in_activity_stops_only_after_a_current_code() {
    let mut rt = runtime("DW-A77");
    assert_eq!(info(&rt, "item-0-0-switch").role, "switch");
    assert!(source(&rt).activity().is_some());
    tab_to(&mut rt, "item-0-0-switch");
    key(&mut rt, Key::Text(" ".into()));
    assert_eq!(
        rt.state.account.dialog,
        Some(Dialog::Code(super::CodeFor::StopActivity)),
        "the switch opens the confirm (DW-A78)"
    );
    assert!(source(&rt).activity().is_some(), "nothing stops before the code");
    assert_eq!(info(&rt, "dlg").role, "dialog");
    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.account.dialog, None, "Esc cancels");
    assert!(source(&rt).activity().is_some());
    assert_eq!(section(&rt), Some(SettingsPane::Security), "and settings stays open");

    click(&mut rt, "item-0-0-switch");
    rt.update(|s, _| form::type_in(&mut s.account.form.code, "000000"));
    settle(&mut rt);
    click(&mut rt, "btn-0");
    assert_eq!(says(&rt, "code-msg"), "Incorrect code. Try again.");
    assert!(source(&rt).activity().is_some(), "a wrong code stops nothing");
    rt.update(|s, _| form::type_in(&mut s.account.form.code, BOARD_CODE));
    settle(&mut rt);
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.dialog, None);
    assert!(source(&rt).activity().is_none(), "the log is off (DW-A79)");

    // On again: no confirm, a new log starts at once.
    click(&mut rt, "item-0-0-switch");
    assert_eq!(rt.state.account.dialog, None);
    assert!(source(&rt).activity().is_some());
}

/// DW-A82's second button: "Sign out" asks the app's own sign-out question over Sign-in and
/// security (DW-A57); cancelling it signs nothing out and the deletion stays scheduled.
#[test]
fn sign_out_on_the_scheduled_page_asks_the_sign_out_question() {
    let mut rt = runtime("DW-A82");
    assert_eq!(rt.state.account.pane, Pane::DeleteScheduled);
    let scheduled = source(&rt).deletion();
    assert_eq!(says(&rt, "btn-1-label"), "Sign out");
    tab_to(&mut rt, "btn-1");
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.pane, Pane::Security);
    assert_eq!(rt.state.account.dialog, Some(Dialog::SignOutApp));
    assert_eq!(name(&rt, "dlg"), "Sign out of Drive?");
    // Enter is Cancel (the safe choice).
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.dialog, None);
    assert!(asked(&rt).is_empty(), "nobody was signed out");
    assert_eq!(source(&rt).deletion(), scheduled, "the deletion stays scheduled");
}

// ---- The plan (DW-0708) and App lock's delay (DW-0716, DW-0716b) ----

#[test]
fn the_plan_card_opens_the_plan_read_only_and_done_closes_it() {
    let mut rt = runtime("DW-A51");
    tab_to(&mut rt, "card-0");
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.dialog, Some(Dialog::Plan));
    assert_eq!(
        section(&rt),
        Some(SettingsPane::Account),
        "over Account, not a page of its own"
    );
    let dlg = info(&rt, "dlg");
    assert_eq!(dlg.role, "dialog");
    assert_eq!(says(&rt, "dlg-title"), "Plan");
    assert_eq!(says(&rt, "plan-size"), "1 TB");
    assert_eq!(says(&rt, "item-3-0-title"), "Renews");
    assert_eq!(says(&rt, "item-3-0-value"), "14 Mar 2027");
    assert_eq!(says(&rt, "item-3-1-value"), "Yearly");
    // The meter is the share that is used: 24 % of the dialog's 352 pt.
    let part = info(&rt, "meter-0-part-0").rect;
    assert!((part[2] - 84.48).abs() < 0.01, "{part:?}");
    // Read only: two buttons, and neither buys anything. The first leaves for the website.
    click(&mut rt, "btn-0");
    assert_eq!(asked(&rt), vec![Intent::Web("plan")]);
    assert_eq!(rt.state.account.dialog, Some(Dialog::Plan), "the dialog stays");
    click(&mut rt, "btn-1");
    assert_eq!(rt.state.account.dialog, None, "Done closes");
    assert_eq!(rt.state.account.pane, Pane::Account);
    // Esc closes it too, and leaves settings open.
    click(&mut rt, "card-0");
    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.account.dialog, None);
    assert_eq!(section(&rt), Some(SettingsPane::Account));
}

#[test]
fn the_plan_scene_is_the_dialog_over_account_and_holds_no_buy_button() {
    let rt = runtime("DW-0708");
    assert_eq!(rt.state.account.dialog, Some(Dialog::Plan));
    assert_eq!(rt.state.account.pane, Pane::Account);
    let buttons: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| e.role == "button" && e.key.as_deref().is_some_and(|k| k.starts_with("btn-")))
        .filter_map(|e| e.label)
        .collect();
    assert_eq!(buttons, vec!["Manage on beebeeb.io, opens in the browser", "Done"]);
}

#[test]
fn a_plan_nobody_can_describe_opens_the_website_instead_of_an_empty_dialog() {
    let mut rt = runtime("DW-A51");
    rt.update(|s, _| s.account.source = Some(Box::new(BoardAccount::new().without_plan_details())));
    settle(&mut rt);
    click(&mut rt, "card-0");
    assert_eq!(rt.state.account.dialog, None);
    assert!(find(&rt, "dlg").is_none());
    assert_eq!(asked(&rt), vec![Intent::Web("plan")]);
}

#[test]
fn the_delay_menu_opens_on_the_current_choice_and_the_keys_choose() {
    let mut rt = runtime("DW-A52");
    tab_to(&mut rt, "item-1-0-pop-button");
    key(&mut rt, Key::Enter);
    assert_eq!(
        rt.state.account.dialog,
        Some(Dialog::LockDelay(Some(1))),
        "on 5 min, the current choice"
    );
    assert_eq!(asked(&rt), Vec::new(), "nothing is asked of the shell");
    assert_eq!(info(&rt, "menu-0").role, "menu");
    let labels: Vec<String> = (0..4).map(|i| says(&rt, &format!("menu-0-item-{i}-label"))).collect();
    assert_eq!(labels, vec!["Immediately", "5 min", "30 min", "60 min"]);
    assert!(
        find(&rt, "menu-0-item-1-check").is_some(),
        "the current choice is checked"
    );
    assert!(find(&rt, "menu-0-item-2-check").is_none());
    // The menu hangs under its pop-up button, inside the card.
    let (menu, pop) = (info(&rt, "menu-0").rect, info(&rt, "item-1-0-pop").rect);
    assert!(
        (menu[1] - (pop[1] + pop[3] + 3.0)).abs() < 0.01,
        "{menu:?} under {pop:?}"
    );
    key(&mut rt, Key::Down);
    assert_eq!(rt.state.account.dialog, Some(Dialog::LockDelay(Some(2))));
    key(&mut rt, Key::End);
    assert_eq!(rt.state.account.dialog, Some(Dialog::LockDelay(Some(3))));
    key(&mut rt, Key::Down);
    assert_eq!(
        rt.state.account.dialog,
        Some(Dialog::LockDelay(Some(3))),
        "the last stays the last"
    );
    key(&mut rt, Key::Up);
    key(&mut rt, Key::Enter);
    assert_eq!(source(&rt).security().lock_after, "30 min");
    assert_eq!(rt.state.account.dialog, None, "the pop-up closes");
    assert_eq!(says(&rt, "item-1-0-pop-label"), "30 min");
    assert_eq!(rt.state.account.pane, Pane::Security);
}

#[test]
fn the_delay_menu_takes_a_press_and_esc_changes_nothing() {
    let mut rt = runtime("DW-0716");
    assert_eq!(rt.state.account.dialog, Some(Dialog::LockDelay(Some(1))));
    key(&mut rt, Key::Home);
    assert_eq!(
        rt.state.account.dialog,
        Some(Dialog::LockDelay(Some(0))),
        "Home is the first"
    );
    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.account.dialog, None);
    assert_eq!(source(&rt).security().lock_after, "5 min", "Esc chooses nothing");
    assert_eq!(section(&rt), Some(SettingsPane::Security), "and settings stays open");
    click(&mut rt, "item-1-0-pop-button");
    click(&mut rt, "menu-0-item-3");
    assert_eq!(source(&rt).security().lock_after, "60 min");
    assert!(find(&rt, "menu-0").is_none());
}

#[test]
fn with_the_lock_off_there_is_no_delay_and_turning_it_on_brings_it_back() {
    let mut rt = runtime("DW-0716b");
    assert!(!source(&rt).security().app_lock);
    assert!(find(&rt, "item-1-0-pop").is_none(), "no time choice");
    assert!(find(&rt, "menu-0").is_none());
    assert_eq!(says(&rt, "item-1-0-sub"), "Off: anyone using this PC can open Beebeeb");
    // Asking for the menu while the lock is off opens nothing.
    rt.update(|s, _| super::go(s, "DW-0716"));
    settle(&mut rt);
    assert_eq!(rt.state.account.dialog, None);
    click(&mut rt, "item-1-0-switch");
    assert!(source(&rt).security().app_lock, "DW-0716");
    assert_eq!(says(&rt, "item-1-0-sub"), "Windows Hello · locks again after");
    assert!(find(&rt, "item-1-0-pop").is_some());
}

#[test]
fn neither_the_plan_nor_the_delay_menu_is_a_secret_screen() {
    for frame in ["DW-0708", "DW-0716", "DW-0716b"] {
        let rt = runtime(frame);
        assert!(!rt.state.account.shows_secret(), "{frame}");
        // No field of any kind: nothing is typed on these three.
        assert!(
            rt.elements().into_iter().all(|e| e.role != "textbox"),
            "{frame} has a field"
        );
    }
}

#[test]
fn a_stored_delay_the_menu_does_not_have_checks_no_row() {
    // Another build's value: the pop-up button shows it, and the menu must not contradict it
    // by checking a delay of its own (least of all "Immediately", the strictest).
    let mut rt = runtime("DW-A52");
    rt.update(|s, _| {
        if let Some(source) = s.account.source.as_mut() {
            source.set_lock_after("15 min");
        }
    });
    settle(&mut rt);
    assert_eq!(says(&rt, "item-1-0-pop-label"), "15 min");
    click(&mut rt, "item-1-0-pop-button");
    assert_eq!(rt.state.account.dialog, Some(Dialog::LockDelay(None)));
    for i in 0..4 {
        assert!(
            find(&rt, &format!("menu-0-item-{i}-check")).is_none(),
            "row {i} is checked"
        );
        assert_eq!(info(&rt, &format!("menu-0-item-{i}")).fill.a, 0.0, "row {i} is hot");
    }
    // Enter with no row under the keys takes nothing; the stored delay stays.
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.dialog, None);
    assert_eq!(source(&rt).security().lock_after, "15 min");
    // The keys still reach every row: Down starts at the first.
    click(&mut rt, "item-1-0-pop-button");
    key(&mut rt, Key::Down);
    assert_eq!(rt.state.account.dialog, Some(Dialog::LockDelay(Some(0))));
    key(&mut rt, Key::Enter);
    assert_eq!(source(&rt).security().lock_after, "Immediately", "chosen, not assumed");
}
