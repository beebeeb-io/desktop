//! Tests of Sign-in and security, the one page (`security_overview.rs`): what it says for a source that knows
//! everything, part, or nothing; and that nothing unbound can be pressed. Input is scripted, as in `tests.rs`.

use super::source::{BoardAccount, SecurityFacts};
use super::tests::{asked, click, find, info, key, name, runtime, says, section, settle, tab_to, window};
use super::{Pane, SettingsPane};
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
        let mut rt = runtime("DW-A52");
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
    let mut rt = runtime("DW-A52");
    assert_eq!(says(&rt, "item-1-1-value"), "4 devices");
    click(&mut rt, "item-1-1");
    assert_eq!(section(&rt), Some(SettingsPane::DevicesAndSessions));
    assert_eq!(says(&rt, "item-0-1-title"), "Anna's iPhone");
}

#[test]
fn the_score_and_every_state_are_on_the_one_page() {
    let rt = runtime("DW-A52");
    assert_eq!(says(&rt, "set-title"), "Sign-in and security");
    assert_eq!(says(&rt, "ring-head"), "7 of 7 security steps done");
    assert_eq!(says(&rt, "ring-label"), "7");
    assert_eq!(
        says(&rt, "ring-sub"),
        "Nothing to do. Checked each time you open this page."
    );
    assert!(
        find(&rt, "todo-0").is_none() && find(&rt, "btn-0").is_none(),
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
    for frame in ["DW-A84", "DW-A84a"] {
        let rt = runtime(frame);
        assert_eq!(rt.state.account.pane, Pane::Security, "{frame}");
        assert_eq!(section(&rt), Some(SettingsPane::Security), "{frame}");
        assert_eq!(says(&rt, "set-title"), "Sign-in and security", "{frame}");
    }
}

#[test]
fn one_thing_left_is_in_the_score_card_with_the_button_that_fixes_it() {
    let mut rt = runtime("DW-A84a");
    assert_eq!(says(&rt, "ring-head"), "6 of 7 security steps done");
    assert_eq!(says(&rt, "ring-label"), "6");
    assert_eq!(says(&rt, "ring-sub"), "One thing left to do.");
    assert_eq!(says(&rt, "todo-0-title"), "Add a passkey");
    assert_eq!(says(&rt, "btn-0-label"), "Add");
    assert_eq!(name(&rt, "btn-0"), "Add, Add a passkey");
    assert!(find(&rt, "todo-1").is_none(), "one thing");
    // The row of the thing that is left has no mark; the others keep theirs.
    assert_eq!(says(&rt, "item-0-2-sub"), "None added");
    assert!(find(&rt, "item-0-2-check").is_none());
    assert!(find(&rt, "item-0-1-check").is_some());
    // The button stays inside its card.
    let (card, button) = (info(&rt, "card-0").rect, info(&rt, "btn-0").rect);
    assert!(
        button[0] + button[2] <= card[0] + card[2] - 17.9,
        "{button:?} inside {card:?}"
    );
    tab_to(&mut rt, "btn-0");
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.account.pane, Pane::Passkeys);
    assert_eq!(says(&rt, "empty-title"), "No passkeys yet");
}

/// The Security page on a source that says only what `change` leaves.
fn page_knowing(change: fn(SecurityFacts) -> SecurityFacts, sections_only: bool) -> Runtime<State> {
    let mut state = init();
    assert!(super::scene("DW-A52", &mut state));
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
        "4 devices",
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
    for row in ["item-0-0", "item-0-1", "item-0-2", "item-1-2", "item-2-0", "item-4-0"] {
        assert_eq!(info(&rt, row).role, "listitem", "{row}");
        assert!(
            name(&rt, row).ends_with("Not available yet"),
            "{row}: {}",
            name(&rt, row)
        );
        click(&mut rt, row);
        assert_eq!(rt.state.account.pane, Pane::Security, "{row}: a press leads nowhere");
    }
    // Deleting is not drawn as a danger while it leads nowhere.
    assert_eq!(name(&rt, "item-4-0"), "Delete your Beebeeb account, Not available yet");
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
    // Asked for by its frame id, the confirm still does not draw.
    rt.update(|s, _| super::go(s, "DW-A57"));
    settle(&mut rt);
    assert!(find(&rt, "dlg").is_none());
    // Devices and sessions answers, so its row is a link.
    assert_eq!(info(&rt, "item-1-1").role, "link");
    click(&mut rt, "item-1-1");
    assert_eq!(section(&rt), Some(SettingsPane::DevicesAndSessions));
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
    assert!(super::scene("DW-A52", &mut state));
    state.account.source = Some(Box::new(BoardAccount::new().without_passkeys().sections_only()));
    let rt = window(state);
    assert_eq!(says(&rt, "todo-0-title"), "Add a passkey");
    assert!(find(&rt, "btn-0").is_none());
    assert_eq!(says(&rt, "todo-0-value"), "Not available yet");
}

#[test]
fn security_logging_off_is_said_and_shows_no_event() {
    let mut state = init();
    assert!(super::scene("DW-A52", &mut state));
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
