//! The account sections in the running app's window, on the real engine against the parent module's fake: what
//! is drawn, pressed and said. Every name is invented.

use rune::{Env, Runtime, Tx};

use super::*;
use crate::State;
use crate::shell::app::{Event as ShellEvent, SettingsPane};
use crate::shell::host;

fn window_on(engine: TestEngine, section: SettingsPane) -> Runtime<State> {
    let state = crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), engine);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (958.0, 638.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(|s: &mut State, _: f64, tx: &mut Tx| crate::frames::account::poll(s, tx));
    rt.frame(0.0);
    rt.update(move |s, _| {
        host::dispatch(s, ShellEvent::OpenSettings(section));
    });
    rt.frame(0.016);
    rt
}

fn shows(rt: &Runtime<State>, words: &str) -> bool {
    rt.elements().iter().any(|e| {
        !e.exiting && (e.text.as_deref() == Some(words) || e.label.as_deref().is_some_and(|l| l.contains(words)))
    })
}

fn says_anywhere(rt: &Runtime<State>, words: &str) -> bool {
    rt.elements().iter().any(|e| {
        e.text.as_deref().is_some_and(|t| t.contains(words)) || e.label.as_deref().is_some_and(|l| l.contains(words))
    })
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| !e.exiting && e.key.as_deref() == Some(key))
}

fn wait_for(rt: &mut Runtime<State>, what: &str, done: impl Fn(&Runtime<State>) -> bool) {
    let began = Instant::now();
    while !done(rt) {
        assert!(began.elapsed() < Duration::from_secs(10), "never: {what}");
        std::thread::sleep(Duration::from_millis(20));
        rt.frame(0.1);
    }
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no element keyed {key}"))
        .rect;
    let (x, y) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.016);
    rt.frame(0.4);
}

#[test]
fn the_running_apps_account_draws_loading_then_the_account_and_never_a_sample() {
    let server = server();
    server.read_delay_ms.store(0, Ordering::SeqCst);
    let mut rt = window_on(signed_in(&server), SettingsPane::Account);
    wait_for(&mut rt, "the account", |rt| shows(rt, FIRST));
    assert!(shows(&rt, FIRST_NAME) && shows(&rt, "240 GB"));
    assert!(!has(&rt, "not-built"));
    for sample in ["Anna", "northwind", "Pass", "VPN", "Chat", "Mail"] {
        assert!(!says_anywhere(&rt, sample), "a sample value shows: {sample}");
    }
}

/// Sign-in and security in the running app: the page with what Beebeeb said for THIS account (two-factor, the
/// number of devices), and for everything else "not known" or "not available yet": no score, no mark, no
/// sample value, and nothing to press that is not bound.
#[test]
fn the_running_apps_security_page_says_what_is_known_and_guesses_nothing() {
    let server = server();
    server.two_factor.lock().unwrap()[0] = Some(true);
    let mut rt = window_on(signed_in(&server), SettingsPane::Security);
    wait_for(&mut rt, "the page", |rt| shows(rt, "3 signed in"));
    assert!(!has(&rt, "not-built"));
    assert!(shows(&rt, "Security score not known yet") && shows(&rt, "On"));
    let text = |rt: &Runtime<State>, key: &str| {
        rt.elements()
            .into_iter()
            .find(|e| !e.exiting && e.key.as_deref() == Some(key))
            .and_then(|e| e.text)
            .unwrap_or_default()
    };
    assert_eq!(text(&rt, "item-0-1-sub"), "On");
    assert_eq!(text(&rt, "item-0-0-sub"), "Last change not known");
    assert_eq!(text(&rt, "item-0-2-sub"), "Not known");
    // The one mark on the page is two-factor's: it is the one state that was read as good.
    let marks: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.key)
        .filter(|k| k.ends_with("-check"))
        .collect();
    assert_eq!(marks, ["item-0-1-check"]);
    // A state that was read is not also called unavailable.
    assert!(shows(&rt, "Two-factor authentication, On, done"));
    assert!(!shows(&rt, "Two-factor authentication, On, done, Not available yet"));
    for sample in [
        "Anna",
        "northwind",
        "3 weeks",
        "backup codes",
        "Nothing to do",
        " of 7",
        "anyone using this PC",
    ] {
        assert!(!says_anywhere(&rt, sample), "a sample or a guess shows: {sample}");
    }
    // Nothing that is not bound can be pressed: no switch, and a press on such a row changes nothing.
    assert!(!has(&rt, "item-1-0-switch"));
    for row in ["item-0-0", "item-0-1", "item-3-0", "item-4-0"] {
        click(&mut rt, row);
        assert!(shows(&rt, "Security score not known yet"), "{row}: the page stays");
        assert!(!has(&rt, "dlg") && !has(&rt, "not-built"), "{row}");
    }
    // Devices and sessions is bound: its row opens it.
    click(&mut rt, "item-1-1");
    wait_for(&mut rt, "the sessions", |rt| shows(rt, PHONE));
}

#[test]
fn the_security_page_drops_the_first_account_when_another_signs_in() {
    let server = server();
    *server.two_factor.lock().unwrap() = [Some(true), Some(false)];
    let engine = signed_in(&server);
    let mut rt = window_on(engine.clone(), SettingsPane::Security);
    wait_for(&mut rt, "the page", |rt| shows(rt, "3 signed in"));
    engine.sign_out_keeping_local_data().expect("signs out");
    // Before any poll of the source: a draw alone shows nothing of the account.
    rt.poll = None;
    rt.update(|_, _| {});
    rt.frame(0.016);
    rt.frame(1.0);
    assert!(shows(&rt, "Not signed in"));
    assert!(!says_anywhere(&rt, "3 signed in") && !has(&rt, "item-0-1-check"));
    sign_in(&engine, &server.people[1]);
    rt.poll = Some(|s: &mut State, _: f64, tx: &mut Tx| crate::frames::account::poll(s, tx));
    wait_for(&mut rt, "the second account's page", |rt| shows(rt, "1 signed in"));
    assert!(shows(&rt, "Off: your password alone signs you in"));
    assert!(!has(&rt, "item-0-1-check") && !says_anywhere(&rt, "3 signed in"));
}

/// The name and the email rows of the live Account pane: nothing answers for the pages behind them, so they
/// are not links, a press does nothing, and no route ends on the full-window "not built yet" frame (which has
/// no way back to Account).
#[test]
fn the_name_and_email_rows_are_not_links_while_nothing_answers_for_their_pages() {
    let server = server();
    let mut rt = window_on(signed_in(&server), SettingsPane::Account);
    wait_for(&mut rt, "the account", |rt| shows(rt, FIRST));
    let row = |rt: &Runtime<State>, label: &str| {
        rt.elements()
            .into_iter()
            .find(|e| !e.exiting && e.label.as_deref() == Some(label))
            .unwrap_or_else(|| panic!("no row labelled {label}"))
    };
    for label in [format!("Name, {FIRST_NAME}"), format!("Sign-in email, {FIRST}")] {
        let it = row(&rt, &label);
        assert_eq!(it.role, "listitem", "{label}: the value alone, not a link");
        let (x, y) = (it.rect[0] + it.rect[2] / 2.0, it.rect[1] + it.rect[3] / 2.0);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.016);
        rt.frame(0.4);
        assert!(!has(&rt, "not-built"), "{label}: a press leads nowhere");
        assert!(
            shows(&rt, FIRST) && shows(&rt, "240 GB"),
            "{label}: Account still shows"
        );
    }
}

/// Any other way to the pages nothing answers for (a link, a key) stays on Account too: only the section
/// itself, when nothing answers for it, is the shell's frame.
#[test]
fn no_route_from_a_live_pane_ends_on_the_not_built_frame() {
    let server = server();
    let mut rt = window_on(signed_in(&server), SettingsPane::Account);
    wait_for(&mut rt, "the account", |rt| shows(rt, FIRST));
    for frame in ["DW-A64", "DW-A65", "DW-A66"] {
        rt.update(move |s, _| crate::frames::account::go(s, frame));
        rt.frame(0.016);
        rt.frame(0.4);
        assert!(!has(&rt, "not-built"), "{frame}");
        assert!(
            shows(&rt, FIRST) && shows(&rt, "240 GB"),
            "{frame}: Account still shows"
        );
    }
    // The pages under Security that nothing answers for do not open either: the person stays on Account.
    for frame in ["DW-A67", "DW-A83"] {
        rt.update(move |s, _| crate::frames::account::go(s, frame));
        rt.frame(0.016);
        rt.frame(0.4);
        assert!(!has(&rt, "not-built"), "{frame}");
        assert!(
            shows(&rt, FIRST) && shows(&rt, "240 GB"),
            "{frame}: Account still shows"
        );
    }
    // Security itself answers: its page, inside settings, with the sidebar.
    rt.update(|s, _| crate::frames::account::go(s, "DW-A52"));
    wait_for(&mut rt, "the Security page", |rt| {
        shows(rt, "Security score not known yet")
    });
    assert!(!has(&rt, "not-built"));
    assert!(shows(&rt, "Back to Drive") && shows(&rt, "Devices and sessions"));
    assert!(!says_anywhere(&rt, FIRST) && !says_anywhere(&rt, "deletion"));
    rt.update(|s, _| crate::frames::account::go(s, "DW-A51"));
    wait_for(&mut rt, "back on Account", |rt| {
        shows(rt, FIRST) && !has(rt, "not-built")
    });
}

/// A plan that was cancelled: the card and the dialog say when it ends, and nowhere "renews".
#[test]
fn a_cancelled_plan_is_drawn_as_ending_not_as_renewing() {
    let server = server();
    *server.plan_status.lock().unwrap() = "cancelling";
    let mut rt = window_on(signed_in(&server), SettingsPane::Account);
    wait_for(&mut rt, "the account", |rt| shows(rt, FIRST));
    assert!(says_anywhere(&rt, "cancels on 14 Mar 2027"));
    rt.update(|s, _| crate::frames::account::go(s, "DW-0708"));
    rt.frame(0.016);
    rt.frame(0.4);
    assert!(shows(&rt, "Cancels on"), "the dialog's row");
    for words in ["renews", "Renews"] {
        assert!(!says_anywhere(&rt, words), "{words}");
    }
}

/// Leaving the account panes tells the source: a new session is then not read for a pane nobody looks at.
#[test]
fn the_window_tells_the_source_when_no_account_pane_shows() {
    let server = server();
    let engine = signed_in(&server);
    let mut rt = window_on(engine.clone(), SettingsPane::Account);
    wait_for(&mut rt, "the account", |rt| shows(rt, FIRST));
    assert_eq!(server.asked("GET /api/v1/me/profile"), 1);
    rt.update(|s, _| {
        host::dispatch(s, ShellEvent::Escape);
    });
    rt.frame(0.016);
    rt.frame(0.4);
    rt.update(|s, _| assert!(matches!(s.shell.app.screen(), crate::shell::app::Screen::Drive(_))));
    engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(server.people[0].phrase.clone()))
        .expect("the phrase unlocks");
    for _ in 0..10 {
        rt.frame(0.1);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        server.asked("GET /api/v1/me/profile"),
        1,
        "back in Drive: nothing is asked"
    );
}

#[test]
fn the_window_drops_the_first_account_when_another_signs_in() {
    let server = server();
    let engine = signed_in(&server);
    let mut rt = window_on(engine.clone(), SettingsPane::DevicesAndSessions);
    wait_for(&mut rt, "the sessions", |rt| shows(rt, PHONE));
    engine.sign_out_keeping_local_data().expect("signs out");
    // Before any poll of the source: a draw alone shows nothing of the account.
    rt.poll = None;
    rt.update(|_, _| {});
    rt.frame(0.016);
    rt.frame(1.0);
    for words in [PHONE, BROWSER, THIS_PC, FIRST, FIRST_NAME] {
        assert!(!says_anywhere(&rt, words), "signed out, still shows {words}");
    }
    assert!(shows(&rt, "Not signed in"));
    sign_in(&engine, &server.people[1]);
    rt.poll = Some(|s: &mut State, _: f64, tx: &mut Tx| crate::frames::account::poll(s, tx));
    wait_for(&mut rt, "the second account's list", |rt| {
        shows(rt, "Another invented PC")
    });
    for words in [PHONE, BROWSER, THIS_PC, FIRST, FIRST_NAME] {
        assert!(!says_anywhere(&rt, words), "the second account sees {words}");
    }
}

#[test]
fn a_sign_out_in_the_window_is_confirmed_sent_once_and_said() {
    let server = server();
    let mut rt = window_on(signed_in(&server), SettingsPane::DevicesAndSessions);
    wait_for(&mut rt, "the sessions", |rt| shows(rt, PHONE));
    // The first row's button: nothing is sent before the confirm.
    click(&mut rt, "btn-0");
    assert!(shows(&rt, &format!("Sign out {PHONE}?")), "the confirm (DW-A56)");
    assert!(server.deleted.lock().unwrap().is_empty());
    // The confirm's own button, numbered after the rows' two.
    click(&mut rt, "btn-2");
    wait_for(&mut rt, "the toast", |rt| shows(rt, SIGNED_OUT));
    assert!(
        has(&rt, "toast-0") && !has(&rt, "toast"),
        "something that worked: the settings toast"
    );
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone"]);
    wait_for(&mut rt, "the row goes", |rt| !says_anywhere(rt, PHONE));
    assert!(shows(&rt, BROWSER));

    // A sign-out Beebeeb did not take is drawn with a way to send it again, and the row stands.
    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    click(&mut rt, "btn-0");
    click(&mut rt, "btn-1");
    wait_for(&mut rt, "the refusal", |rt| shows(rt, NOT_SIGNED_OUT));
    assert!(shows(&rt, BROWSER));
    assert!(shows(&rt, TRY_AGAIN) && has(&rt, "toast-action"));
    assert!(
        rt.elements()
            .iter()
            .any(|e| !e.exiting && e.key.as_deref() == Some("toast") && e.role == "alert"),
        "something that was not done is drawn as a failure"
    );
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone", "s-web"]);
    // "Try again" sends it once more; it still fails, and the toast still offers it (the list read after the
    // answer shows the answer was taken in).
    let lists = server.asked("GET /api/v1/account/sessions");
    click(&mut rt, "toast-action");
    wait_for(&mut rt, "sent again, and the list read after it", |_| {
        server.deleted.lock().unwrap().len() == 3 && server.asked("GET /api/v1/account/sessions") > lists
    });
    wait_for(&mut rt, "the refusal again", |rt| {
        shows(rt, NOT_SIGNED_OUT) && has(rt, "toast-action")
    });
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone", "s-web", "s-web"]);
    // The toast goes when the pane is left.
    rt.update(|s, _| crate::frames::account::go(s, "DW-A51"));
    wait_for(&mut rt, "the account", |rt| shows(rt, FIRST));
    // (It leaves with the toast's own fade: gone once that has run.)
    wait_for(&mut rt, "the toast is gone", |rt| !says_anywhere(rt, NOT_SIGNED_OUT));
    assert!(!has(&rt, "toast-action"));
}

#[test]
fn a_failed_read_is_drawn_with_a_way_to_try_again() {
    let server = server();
    let engine = signed_in(&server);
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let mut rt = window_on(engine, SettingsPane::DevicesAndSessions);
    wait_for(&mut rt, "the failure", |rt| shows(rt, "Couldn't load this"));
    assert!(!says_anywhere(&rt, PHONE));
    server.read_mode.store(ANSWER, Ordering::SeqCst);
    click(&mut rt, "btn-0");
    wait_for(&mut rt, "the sessions", |rt| shows(rt, PHONE));
    assert!(!says_anywhere(&rt, "Couldn't load this"));
}

/// Something that was not done is drawn as a failure also when there is nothing to try again: a second press
/// while the first sign-out is on its way.
#[test]
fn a_refusal_with_nothing_to_try_again_is_still_drawn_as_a_failure() {
    let server = server();
    let mut rt = window_on(signed_in(&server), SettingsPane::DevicesAndSessions);
    wait_for(&mut rt, "the sessions", |rt| shows(rt, PHONE));
    server.change_delay_ms.store(1_500, Ordering::SeqCst);
    for _ in 0..2 {
        click(&mut rt, "btn-0");
        click(&mut rt, "btn-2");
    }
    assert!(shows(&rt, STILL_SIGNING_OUT));
    assert!(
        rt.elements()
            .iter()
            .any(|e| !e.exiting && e.key.as_deref() == Some("toast") && e.role == "alert"),
        "drawn as a failure"
    );
    assert!(!has(&rt, "toast-action") && !has(&rt, "toast-0"));
    wait_for(&mut rt, "the sign-out lands", |rt| shows(rt, SIGNED_OUT));
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone"], "sent once");
}
