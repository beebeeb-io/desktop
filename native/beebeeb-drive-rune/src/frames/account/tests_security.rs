//! Tests of Sign-in and security, the one page (`security_overview.rs`): what it says for a source that knows
//! everything, part, or nothing; and that nothing unbound can be pressed. Input is scripted, as in `tests.rs`.

use super::source::{BOARD_SIGN_OUT_REFUSED, BoardAccount, SecurityFacts, SignOutMode};
use super::tests::{asked, click, find, info, key, name, runtime, says, section, settle, tab_to, window};
use super::{Dialog, Pane, SettingsPane};
use crate::frames::settings::Intent;
use crate::{State, init};
use rune::{Key, Runtime};

#[test]
fn every_row_of_sign_in_and_security_goes_where_it_points() {
    // The pages of this lane open here; Esc goes back to Sign-in and security.
    for (row, pane) in [
        ("item-0-0", Pane::Password),
        ("item-0-1", Pane::TwoFactor),
        ("item-0-2", Pane::Passkeys),
        ("item-1-2", Pane::ApproveCode),
        ("item-2-3", Pane::Activity),
        ("item-4-0", Pane::Delete),
    ] {
        let mut rt = runtime("DW-S01");
        assert_eq!(info(&rt, row).role, "link", "{row}");
        tab_to(&mut rt, row);
        key(&mut rt, Key::Enter);
        assert_eq!(rt.state.account.pane, pane, "{row}");
        assert_eq!(asked(&rt), Vec::new(), "{row}");
        // From a page with a field in focus Esc goes one step back. From a page without one
        // the shell takes Esc first and closes settings (the PR's request to the shell lane).
        if matches!(pane, Pane::Password | Pane::Delete) {
            key(&mut rt, Key::Escape);
            assert_eq!(rt.state.account.pane, Pane::Security, "{row}: Esc goes back");
        }
    }
    let mut rt = runtime("DW-S01");
    assert_eq!(says(&rt, "item-1-1-value"), "4 signed in");
    click(&mut rt, "item-1-1");
    assert_eq!(section(&rt), Some(SettingsPane::DevicesAndSessions));
    assert_eq!(says(&rt, "item-0-1-title"), "Anna's iPhone");
}

#[test]
fn the_score_and_every_state_are_on_the_one_page() {
    let rt = runtime("DW-S01");
    assert_eq!(says(&rt, "set-title"), "Sign-in and security");
    assert_eq!(says(&rt, "ring-head"), "7 of 7 security steps done");
    assert_eq!(says(&rt, "ring-label"), "7");
    assert_eq!(says(&rt, "ring-sub"), "Nothing to do.");
    assert!(
        find(&rt, "todo-0").is_none() && find(&rt, "todo-0-btn").is_none(),
        "nothing left: no to-do"
    );
    // Each subject says its state, and has the mark of a thing that is done.
    assert_eq!(says(&rt, "item-0-0-sub"), "Changed 3 weeks ago");
    assert_eq!(says(&rt, "item-0-1-sub"), "On · 6 of 8 backup codes left");
    assert_eq!(says(&rt, "item-0-2-sub"), "2 passkeys");
    for row in ["item-0-0", "item-0-1", "item-0-2"] {
        assert!(find(&rt, &format!("{row}-check")).is_some(), "{row}: done");
    }
    assert_eq!(
        name(&rt, "item-0-1"),
        "Two-factor authentication, On · 6 of 8 backup codes left, done"
    );
    // The last three, newest first, then the way to all of them.
    assert_eq!(says(&rt, "list-2-title"), "Recent sign-in activity");
    assert_eq!(says(&rt, "item-2-0-title"), "New sign-in on Anna-Laptop");
    assert_eq!(says(&rt, "item-2-2-title"), "Device approved: Anna's iPad");
    assert_eq!(says(&rt, "item-2-3-title"), "All sign-in activity");
    assert!(find(&rt, "item-2-4").is_none(), "three events and the link, no more");
    assert_eq!(info(&rt, "item-2-0").role, "listitem", "an event is text, not a link");
}

#[test]
fn the_checkups_frame_ids_lead_to_the_page() {
    // The checkup is not a screen any more: its ids show the page, so a link to one still arrives.
    for frame in ["DW-A84", "DW-A84a", "DW-A52"] {
        // Retired as frames (no scene of their own); still names `go` understands.
        assert!(!super::FRAMES.iter().any(|(id, _)| *id == frame), "{frame} is retired");
        let mut rt = runtime("DW-A51");
        rt.update(move |s, _| super::go(s, frame));
        settle(&mut rt);
        assert_eq!(rt.state.account.pane, Pane::Security, "{frame}");
        assert_eq!(section(&rt), Some(SettingsPane::Security), "{frame}");
        assert_eq!(says(&rt, "set-title"), "Sign-in and security", "{frame}");
    }
}

#[test]
fn one_thing_left_is_in_the_score_card_with_the_button_that_fixes_it() {
    let mut rt = runtime("DW-S01a");
    assert_eq!(says(&rt, "ring-head"), "6 of 7 security steps done");
    assert_eq!(says(&rt, "ring-label"), "6");
    assert_eq!(says(&rt, "ring-sub"), "One thing left to do.");
    assert_eq!(says(&rt, "todo-0-title"), "Add a passkey");
    assert_eq!(says(&rt, "todo-0-btn-label"), "Add");
    assert_eq!(name(&rt, "todo-0-btn"), "Add, Add a passkey");
    assert!(find(&rt, "todo-1").is_none(), "one thing");
    // The row of the thing that is left has no mark; the others keep theirs.
    assert_eq!(says(&rt, "item-0-2-sub"), "None added");
    assert!(find(&rt, "item-0-2-check").is_none());
    assert!(find(&rt, "item-0-1-check").is_some());
    // The button stays inside its card.
    let (card, button) = (info(&rt, "card-0").rect, info(&rt, "todo-0-btn").rect);
    assert!(
        button[0] + button[2] <= card[0] + card[2] - 17.9,
        "{button:?} inside {card:?}"
    );
    tab_to(&mut rt, "todo-0-btn");
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.pane, Pane::Passkeys);
    assert_eq!(says(&rt, "empty-title"), "No passkeys yet");
}

/// The Security page on a source that says only what `change` leaves.
fn page_knowing(change: fn(SecurityFacts) -> SecurityFacts, sections_only: bool) -> Runtime<State> {
    let mut state = init();
    assert!(super::scene("DW-S01", &mut state));
    let source = BoardAccount::new().knowing(change);
    state.account.source = Some(Box::new(if sections_only { source.sections_only() } else { source }));
    window(state)
}

fn nothing_known(_: SecurityFacts) -> SecurityFacts {
    SecurityFacts::unknown()
}

fn every_text(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .map(|e| format!("{} {}", e.text.unwrap_or_default(), e.label.unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_source_that_knows_nothing_gets_no_score_no_mark_and_no_sample_value() {
    let rt = page_knowing(nothing_known, true);
    assert_eq!(says(&rt, "ring-head"), "Security score not known yet");
    assert_eq!(says(&rt, "ring-label"), "?");
    assert!(find(&rt, "todo-0").is_none());
    let text = every_text(&rt);
    for never in [
        "Nothing to do",
        " of 7",
        "of 0",
        "3 weeks",
        "backup codes",
        "passkeys",
        "Anna-Laptop",
        "4 signed in",
        "5 min",
    ] {
        assert!(!text.contains(never), "says {never:?}");
    }
    let checks: Vec<String> = rt
        .elements()
        .into_iter()
        .filter_map(|e| e.key)
        .filter(|k| k.ends_with("-check"))
        .collect();
    assert_eq!(checks, Vec::<String>::new(), "a mark for a state nobody read");
    assert_eq!(says(&rt, "item-0-0-sub"), "Last change not known");
    assert_eq!(says(&rt, "item-0-1-sub"), "Not known");
    assert_eq!(says(&rt, "item-0-2-sub"), "Not known");
    assert_eq!(says(&rt, "item-1-1-value"), "Not known");
}

#[test]
fn factors_that_came_back_empty_are_no_score_either() {
    let rt = page_knowing(
        |all| SecurityFacts {
            factors: Some(Vec::new()),
            ..all
        },
        false,
    );
    assert_eq!(says(&rt, "ring-head"), "Security score not known yet");
    assert!(!every_text(&rt).contains("Nothing to do"));
    // With no factors the password row has no mark: "strong" was not read.
    assert!(find(&rt, "item-0-0-check").is_none());
    assert!(find(&rt, "item-0-1-check").is_some(), "two-factor is known on its own");
}

#[test]
fn nothing_that_is_not_bound_can_be_pressed() {
    let mut rt = page_knowing(nothing_known, true);
    // Pages nothing answers for: their rows are text that says so.
    for row in ["item-0-0", "item-0-1", "item-0-2", "item-2-0"] {
        assert_eq!(info(&rt, row).role, "listitem", "{row}");
        assert!(
            name(&rt, row).ends_with("Not available yet"),
            "{row}: {}",
            name(&rt, row)
        );
        click(&mut rt, row);
        assert_eq!(rt.state.account.pane, Pane::Security, "{row}: a press leads nowhere");
    }
    // Deleting and approving have no flow here: each says so, with the web as the way (never a row that leads
    // nowhere, never drawn as a danger).
    assert!(
        name(&rt, "item-4-0").starts_with("Delete your account on the web"),
        "{}",
        name(&rt, "item-4-0")
    );
    assert_eq!(info(&rt, "item-4-0").role, "link");
    assert!(
        name(&rt, "item-1-2").ends_with("Not available in this version yet. Approve on the web."),
        "{}",
        name(&rt, "item-1-2")
    );
    assert_eq!(info(&rt, "item-1-2").role, "listitem");
    click(&mut rt, "item-1-2");
    assert_eq!(rt.state.account.pane, Pane::Security, "a press leads nowhere");
    assert_eq!(asked(&rt), Vec::new());
    // Below the window's edge: the keys reach it.
    tab_to(&mut rt, "item-4-0");
    key(&mut rt, Key::Enter);
    assert_eq!(
        asked(&rt),
        vec![crate::frames::settings::Intent::Web("delete-account")],
        "the web's page, by name"
    );
    // App lock that cannot be set: no switch, no delay.
    assert!(find(&rt, "item-1-0-switch").is_none() && find(&rt, "item-1-0-pop").is_none());
    assert_eq!(says(&rt, "item-1-0-value"), "Not available yet");
    // Signing out that this source cannot do: no confirm opens.
    for row in ["item-3-0", "item-3-1"] {
        assert_eq!(info(&rt, row).role, "listitem", "{row}");
        click(&mut rt, row);
        assert_eq!(rt.state.account.dialog, None, "{row}");
        assert!(find(&rt, "dlg").is_none(), "{row}");
    }
    // Asked for by its frame id, the confirm does not open: nothing undrawn holds the keyboard.
    for frame in ["DW-A57", "DW-A57a", "DW-0716"] {
        rt.update(move |s, _| super::go(s, frame));
        settle(&mut rt);
        assert_eq!(rt.state.account.dialog, None, "{frame}");
        assert!(find(&rt, "dlg").is_none() && find(&rt, "menu-0").is_none(), "{frame}");
    }
    // Devices and sessions answers, so its row is a link.
    assert_eq!(info(&rt, "item-1-1").role, "link");
    click(&mut rt, "item-1-1");
    assert_eq!(section(&rt), Some(SettingsPane::DevicesAndSessions));
}

/// The page of a source that knows two-factor and how many are signed in, and answers for no page under it (the
/// running app today): a row keeps the state it has, and "Not available yet" is only on a row with none.
#[test]
fn a_row_with_a_state_keeps_it_and_does_not_also_say_not_available() {
    let mut rt = runtime("DW-S01u");
    assert_eq!(name(&rt, "item-0-1"), "Two-factor authentication, On, done");
    assert_eq!(info(&rt, "item-0-1").role, "listitem");
    assert_eq!(
        name(&rt, "item-0-0"),
        "Password, Last change not known, Not available yet"
    );
    assert_eq!(says(&rt, "item-1-1-value"), "4 signed in");
    assert_eq!(info(&rt, "item-1-1").role, "link");
    // The board's source has App lock on; this page has no row for it, so its menu does not open either.
    rt.update(|s, _| super::go(s, "DW-0716"));
    settle(&mut rt);
    assert_eq!(rt.state.account.dialog, None);
}

/// A state that was read and is NOT good is a state too: said alone, never with "Not available yet" beside it.
#[test]
fn a_read_state_that_is_not_good_is_not_called_unavailable_either() {
    let rt = page_knowing(
        |all| SecurityFacts {
            two_factor: Some(false),
            backup_codes: None,
            passkeys: Some(0),
            ..all
        },
        true,
    );
    assert_eq!(
        name(&rt, "item-0-1"),
        "Two-factor authentication, Off: your password alone signs you in"
    );
    assert_eq!(name(&rt, "item-0-2"), "Passkeys, None added");
    assert_eq!(info(&rt, "item-0-1").role, "listitem");
    assert!(find(&rt, "item-0-1-check").is_none() && find(&rt, "item-0-2-check").is_none());
}

/// A sign-in activity nobody read keeps its way to the page when the page answers.
#[test]
fn activity_that_was_not_read_still_leads_to_its_page() {
    let mut rt = page_knowing(|all| SecurityFacts { activity: None, ..all }, false);
    assert_eq!(name(&rt, "item-2-0"), "All sign-in activity, Not known");
    assert_eq!(info(&rt, "item-2-0").role, "link");
    tab_to(&mut rt, "item-2-0");
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.pane, Pane::Activity);
}

/// The page is taller than the window. A page opened from a row low on it starts at its own top, also when
/// that page is itself taller than the window (a short window here, so the activity page is).
#[test]
fn a_page_opened_from_low_on_the_page_starts_at_its_top() {
    let mut state = init();
    assert!(super::scene("DW-S01", &mut state));
    let env = rune::Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (958.0, 360.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    let top = info(&rt, "set-title").rect[1];
    tab_to(&mut rt, "item-2-3");
    assert!(
        info(&rt, "set-title").rect[1] < top - 100.0,
        "the page scrolled to reach the row"
    );
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.pane, Pane::Activity);
    // The activity page is taller than this window: an offset it inherited would stay.
    let last = info(&rt, "list-3-body").rect;
    assert!(last[1] + last[3] > 360.0, "the activity page does not fit: {last:?}");
    assert_eq!(
        info(&rt, "set-title").rect[1],
        top,
        "its heading is where a pane's heading is"
    );
}

fn sign_out_page(mode: SignOutMode) -> Runtime<State> {
    let mut state = init();
    assert!(super::scene("DW-S01", &mut state));
    state.account.source = Some(Box::new(BoardAccount::new().answering_sign_out(mode)));
    let mut rt = window(state);
    tab_to(&mut rt, "item-3-0");
    key(&mut rt, Key::Enter);
    assert_eq!(says(&rt, "dlg-title"), "Sign out of Drive?");
    rt
}

/// The confirm draws nothing of a sign-out before its answer: refused stays, says why, and nothing left.
#[test]
fn a_refused_sign_out_stays_on_the_confirm_and_says_why() {
    let mut rt = sign_out_page(SignOutMode::Refused);
    click(&mut rt, "btn-1");
    assert_eq!(rt.state.account.dialog, Some(Dialog::SignOutApp), "the confirm stays");
    assert_eq!(says(&rt, "dlg-text-0"), BOARD_SIGN_OUT_REFUSED);
    assert_eq!(asked(&rt), Vec::new(), "the sign-in screen was not asked for");
    assert_eq!(section(&rt), Some(SettingsPane::Security));
    // Cancel works; the next confirm starts with its own words again.
    click(&mut rt, "btn-2");
    assert_eq!(rt.state.account.dialog, None);
    tab_to(&mut rt, "item-3-0");
    key(&mut rt, Key::Enter);
    assert!(says(&rt, "dlg-text-0").starts_with("This signs out Drive on this PC only."));
}

/// A sign-out that is out: the confirm says so, takes no press and no Esc, and nothing is shown as signed out.
#[test]
fn a_sign_out_on_its_way_waits_on_the_confirm() {
    let mut rt = sign_out_page(SignOutMode::After(u32::MAX));
    click(&mut rt, "btn-1");
    assert_eq!(rt.state.account.signing_out.waiting, Some(false));
    assert_eq!(says(&rt, "dlg-text-0"), super::security_overview::SIGNING_OUT);
    for press in ["btn-1", "btn-2"] {
        click(&mut rt, press);
        assert_eq!(
            rt.state.account.dialog,
            Some(Dialog::SignOutApp),
            "{press}: the confirm stays"
        );
    }
    key(&mut rt, Key::Escape);
    assert_eq!(
        rt.state.account.dialog,
        Some(Dialog::SignOutApp),
        "Esc does not call it back"
    );
    // The lane's own key handler too (the confirm's own Esc is another layer).
    let escape = rune::KeyEvent {
        key: Key::Escape,
        mods: rune::Mods::default(),
    };
    rt.update(move |s, tx| assert!(super::on_key(s, &escape, tx)));
    settle(&mut rt);
    assert_eq!(
        rt.state.account.dialog,
        Some(Dialog::SignOutApp),
        "nor through the lane's keys"
    );
    assert_eq!(asked(&rt), Vec::new(), "nothing is shown as signed out");
    assert_eq!(section(&rt), Some(SettingsPane::Security));
    // The session ends under it (a lock, another account): the wait ends with the flow.
    rt.update(|s, _| super::end_flows(s));
    settle(&mut rt);
    assert_eq!(rt.state.account.signing_out.waiting, None);
    assert!(find(&rt, "dlg").is_none());
    assert_eq!(asked(&rt), Vec::new());
}

/// The answer arrives later: only then does the window leave for the sign-in screen.
#[test]
fn a_sign_out_leaves_for_sign_in_only_when_its_answer_came() {
    let mut rt = sign_out_page(SignOutMode::After(60));
    click(&mut rt, "btn-1");
    assert_eq!(asked(&rt), Vec::new(), "after the press: nothing yet");
    assert_eq!(rt.state.account.signing_out.waiting, Some(false));
    assert_eq!(says(&rt, "dlg-text-0"), super::security_overview::SIGNING_OUT);
    for _ in 0..400 {
        if !asked(&rt).is_empty() {
            break;
        }
        rt.frame(0.3);
    }
    assert_eq!(asked(&rt), vec![Intent::Show("DW-A02")]);
    assert_eq!(rt.state.account.signing_out.waiting, None);
    assert_eq!(rt.state.account.dialog, None);
}

#[test]
fn two_factor_says_off_only_when_it_was_read_as_off() {
    let off = page_knowing(
        |all| SecurityFacts {
            two_factor: Some(false),
            backup_codes: None,
            ..all
        },
        false,
    );
    assert_eq!(says(&off, "item-0-1-sub"), "Off: your password alone signs you in");
    assert!(find(&off, "item-0-1-check").is_none());
    let on = page_knowing(
        |all| SecurityFacts {
            backup_codes: None,
            ..all
        },
        false,
    );
    assert_eq!(says(&on, "item-0-1-sub"), "On");
    assert!(find(&on, "item-0-1-check").is_some());
    let unknown = page_knowing(
        |all| SecurityFacts {
            two_factor: None,
            ..all
        },
        false,
    );
    assert_eq!(says(&unknown, "item-0-1-sub"), "Not known");
    assert!(find(&unknown, "item-0-1-check").is_none());
}

#[test]
fn a_to_do_whose_page_is_not_there_has_no_button() {
    // One thing is left and nothing answers for the page that fixes it: the to-do is said, with no button.
    let mut state = init();
    assert!(super::scene("DW-S01", &mut state));
    state.account.source = Some(Box::new(BoardAccount::new().without_passkeys().sections_only()));
    let rt = window(state);
    assert_eq!(says(&rt, "todo-0-title"), "Add a passkey");
    assert!(find(&rt, "todo-0-btn").is_none());
    assert_eq!(says(&rt, "todo-0-value"), "Not available yet");
}

#[test]
fn security_logging_off_is_said_and_shows_no_event() {
    let mut state = init();
    assert!(super::scene("DW-S01", &mut state));
    state.account.source = Some(Box::new(BoardAccount::new().without_activity()));
    let mut rt = window(state);
    assert_eq!(says(&rt, "item-2-0-title"), "All sign-in activity");
    assert_eq!(
        says(&rt, "item-2-0-sub"),
        "Security logging is off: nothing is recorded"
    );
    assert!(find(&rt, "item-2-1").is_none());
    click(&mut rt, "item-2-0");
    assert_eq!(rt.state.account.pane, Pane::Activity);
}
