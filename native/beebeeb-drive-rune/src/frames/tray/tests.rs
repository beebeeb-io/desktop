//! Tests of the tray group: every flyout frame's words and button, what each control asks
//! for, the keyboard, names and roles, and that a locked flyout shows no file name.

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::source::{Action, BoardFixture, Changes, FRAMES, Status, TraySource, count};
use super::{Intent, Ui, frame_for, view};
use crate::shell::flyout_window::{self, FlyoutState};
use crate::shell::tray::{EngineState, TrayInputs};
use crate::theme::tokens;
use crate::{State, init};

/// Every flyout frame of the board: its title, its line, its button and whether that button
/// is the amber one (`None`: the state has no button).
const BUILT: [(&str, &str, &str, Option<(&str, bool)>); 19] = [
    (
        "DW-1202",
        "Up to date",
        "1,284 files \u{00B7} last change 2 min ago",
        None,
    ),
    (
        "DW-1203",
        "Syncing 12 files",
        "48 MB left \u{00B7} about a minute",
        Some(("Pause", false)),
    ),
    (
        "DW-1204",
        "Sync is paused",
        "7 changes wait on this PC",
        Some(("Resume", true)),
    ),
    (
        "DW-1205",
        "2 files need a decision",
        "Changed in two places",
        Some(("Review\u{2026}", true)),
    ),
    (
        "DW-1206",
        "Can't reach Beebeeb",
        "Files on this PC are unchanged",
        Some(("Try again", false)),
    ),
    (
        "DW-1207",
        "Sync stopped",
        "3 files couldn't upload. Nothing was lost.",
        Some(("Show", true)),
    ),
    (
        "DW-1208",
        "Storage almost full",
        "912 GB of 1 TB",
        Some(("Add storage", false)),
    ),
    (
        "DW-1209",
        "Your storage is full",
        "9 changes can't upload. They stay on this PC.",
        Some(("Add storage", true)),
    ),
    (
        "DW-1210",
        "Version 0.9.2 is available",
        "Restart to apply the update.",
        Some(("Restart to update", false)),
    ),
    (
        "DW-1211",
        "Beebeeb is locked",
        "Sync keeps running while it's locked.",
        Some(("Unlock with Windows Hello", true)),
    ),
    ("DW-1212", "", "", None),
    (
        "DW-1213",
        "Choose a plan",
        "Your account is ready. Choose a plan on beebeeb.io, including the free trial. Beebeeb continues here by itself.",
        Some(("Choose a plan on beebeeb.io", true)),
    ),
    (
        "DW-1214",
        "Update Beebeeb",
        "This version of Beebeeb is too old to connect. Update to continue.",
        Some(("Restart to update", true)),
    ),
    (
        "DW-1215",
        "Adding Beebeeb to File Explorer\u{2026}",
        "Beebeeb adds itself to File Explorer after sign-in.",
        None,
    ),
    (
        "DW-1216",
        "Couldn't add Beebeeb to File Explorer",
        "Windows didn't finish adding Beebeeb to File Explorer.",
        Some(("Try again", true)),
    ),
    (
        "DW-1219",
        "Signed out",
        "Enter your password to keep syncing. Nothing has been lost.",
        Some(("Sign in", true)),
    ),
    (
        "DW-1220",
        "Up to date",
        "1,284 files \u{00B7} last change 2 min ago",
        None,
    ),
    (
        "DW-1221",
        "Couldn't add Beebeeb to File Explorer",
        "Windows didn't finish adding Beebeeb to File Explorer.",
        Some(("Try again", true)),
    ),
    (
        "DW-1222",
        "Sync is waiting",
        "This PC is on a metered connection",
        Some(("Sync anyway", false)),
    ),
];

fn runtime_of(ui: Ui) -> Runtime<State> {
    let mut state = init();
    state.tray = Some(ui);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, view::SIZE, env);
    rt.frame(0.0);
    rt
}

fn runtime(frame: &str) -> Runtime<State> {
    runtime_of(Ui::board(frame).unwrap_or_else(|| panic!("no flyout frame {frame}")))
}

fn ui(rt: &mut Runtime<State>) -> &mut Ui {
    rt.state.tray.as_mut().expect("the flyout is the scene")
}

fn info<S: 'static>(rt: &Runtime<S>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn has<S: 'static>(rt: &Runtime<S>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn text<S: 'static>(rt: &Runtime<S>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

fn press<S: 'static>(rt: &mut Runtime<S>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    rt.frame(0.0);
}

fn click<S: 'static>(rt: &mut Runtime<S>, key: &str) {
    rt.settle_all();
    rt.frame(0.0);
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

fn focused<S: 'static>(rt: &Runtime<S>) -> Option<String> {
    rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
}

/// Tab until `key` has focus (at most 20 stops).
fn tab_to<S: 'static>(rt: &mut Runtime<S>, key: &str) {
    for _ in 0..20 {
        if focused(rt).as_deref() == Some(key) {
            return;
        }
        press(rt, Key::Tab);
    }
    panic!("Tab never reached {key}; focus is on {:?}", focused(rt));
}

/// The fixture behind a scene's flyout, to read what was asked of the engine.
fn asked(frame: &str, act: impl FnOnce(&mut BoardFixture)) -> Vec<String> {
    let mut fixture = BoardFixture::frame(frame).expect("a flyout frame");
    act(&mut fixture);
    fixture.asked
}

#[test]
fn the_fixture_has_the_boards_19_flyout_frames() {
    assert_eq!(FRAMES.len(), 19);
    assert_eq!(BUILT.map(|(frame, ..)| frame), FRAMES);
    assert!(
        BoardFixture::frame("DW-1201").is_none(),
        "DW-1201 is the desktop around the flyout"
    );
    assert!(Ui::board("DW-0401").is_none());
}

/// Every frame: the title, the line and the one button the board draws, the amber only on a
/// state's main action, and the three buttons at the bottom, always (G2).
#[test]
fn every_frame_draws_its_words_its_button_and_the_three_buttons() {
    let t = tokens::theme(false);
    let mut checked = 0;
    for (frame, title, line, button) in BUILT {
        let rt = runtime(frame);
        if frame == "DW-1212" {
            // Checking: a skeleton, no words and no guess (DW-1212).
            assert!(!has(&rt, "st-title") && !has(&rt, "st-line") && !has(&rt, "st-icon"));
            for skeleton in ["st-skel-0", "st-skel-1", "st-skel-2"] {
                assert!(has(&rt, skeleton), "{frame}: {skeleton}");
            }
        } else {
            assert_eq!(text(&rt, "st-title"), title, "{frame}");
            assert_eq!(text(&rt, "st-line"), line, "{frame}");
        }
        match button {
            // Without one, the only button of the kit left is the list's own.
            None => assert!(
                !has(&rt, "btn-0") || text(&rt, "btn-0-label") == "View full history",
                "{frame}"
            ),
            Some((label, amber)) => {
                assert_eq!(text(&rt, "btn-0-label"), label, "{frame}");
                assert_eq!(info(&rt, "btn-0").fill == t.accent, amber, "{frame}: amber");
                assert_eq!(info(&rt, "btn-0").role, "button");
                assert!(info(&rt, "btn-0").focusable, "{frame}: a Tab stop");
            }
        }
        for (i, label) in ["Open Beebeeb", "Open in File Explorer", "Settings"].iter().enumerate() {
            assert_eq!(text(&rt, &format!("foot-{i}-label")), *label, "{frame}");
        }
        checked += 1;
    }
    assert_eq!(checked, 19);
}

/// "Fixed at 340 x 520 px in every state; it never grows with its content" (DW-1201): the
/// three buttons sit at the same place whatever the status block holds.
#[test]
fn the_flyout_never_grows_with_its_content() {
    for frame in FRAMES {
        let rt = runtime(frame);
        assert_eq!(info(&rt, "fly").rect, [0.0, 0.0, 340.0, 520.0], "{frame}");
        let foot = info(&rt, "foot").rect;
        assert_eq!((foot[1], foot[3]), (455.5, 64.5), "{frame}");
        let (st, list) = (info(&rt, "st").rect, info(&rt, "list").rect);
        assert!(
            (st[3] + 28.5 + list[3] - 455.5).abs() < 0.01,
            "{frame}: the list gives up what the status block takes"
        );
    }
}

/// DW-1221: a title wraps to two lines at most, its line to three, names never wrap, and a
/// label over 12 characters goes under the text.
#[test]
fn long_copy_wraps_and_long_names_are_cut() {
    let rt = runtime("DW-1221");
    assert_eq!(info(&rt, "st-title").rect[3], 34.0, "two lines of 17");
    assert_eq!(info(&rt, "st-line").rect[3], 32.0, "two lines of 16");
    let name = info(&rt, "row-0-name").rect;
    assert_eq!(name[3], 15.0, "one line");
    assert!(
        name[0] + name[2] <= info(&rt, "row-0-time").rect[0],
        "cut before the time"
    );
    assert!(text(&rt, "row-0-name").starts_with("Northwind Studio"));

    for (frame, _, _, button) in BUILT {
        let Some((label, _)) = button else { continue };
        let rt = runtime(frame);
        let (button, line) = (info(&rt, "btn-0").rect, info(&rt, "st-line").rect);
        let under = button[1] >= line[1] + line[3];
        assert_eq!(under, label.chars().count() > 12, "{frame}: {label}");
        if under {
            assert_eq!(button[0], line[0], "{frame}: under the text, at its left edge");
        }
    }
}

/// "Open in File Explorer is disabled only while Beebeeb is not in File Explorer" (DW-1201).
#[test]
fn open_in_file_explorer_is_off_only_while_beebeeb_is_not_in_file_explorer() {
    let t = tokens::theme(false);
    let mut off = Vec::new();
    for frame in FRAMES {
        let mut rt = runtime(frame);
        let button = info(&rt, "foot-1");
        if button.focusable {
            assert_eq!(info(&rt, "foot-1-label").text_color, Some(t.text_secondary), "{frame}");
            click(&mut rt, "foot-1");
            assert_eq!(
                ui(&mut rt).take_intents(),
                [Intent::OpenInFileExplorer, Intent::Close],
                "{frame}"
            );
        } else {
            assert_eq!(info(&rt, "foot-1-label").text_color, Some(t.text_quaternary), "{frame}");
            click(&mut rt, "foot-1");
            assert_eq!(
                ui(&mut rt).take_intents(),
                [],
                "{frame}: a disabled button asks for nothing"
            );
            off.push(frame);
        }
    }
    assert_eq!(off, ["DW-1213", "DW-1215", "DW-1216", "DW-1221"]);
}

/// The status block's button, frame by frame: what it asks the window or the shell for, what
/// it asks the engine for, and the state the board sends it to.
#[test]
fn each_status_button_does_what_the_board_says() {
    use Intent::{Close, OpenSyncHistory, OpenWindow, PauseSync, ResumeSync, ReviewConflict};
    let cases: [(&str, Action, &[Intent], &str); 14] = [
        ("DW-1203", Action::Pause, &[PauseSync], "Sync is paused"),
        ("DW-1204", Action::Resume, &[ResumeSync], "Syncing 12 files"),
        ("DW-1222", Action::SyncAnyway, &[], "Syncing 12 files"),
        (
            "DW-1205",
            Action::Review,
            &[ReviewConflict, Close],
            "2 files need a decision",
        ),
        ("DW-1206", Action::TryAgain, &[], "Up to date"),
        (
            "DW-1207",
            Action::ShowProblems,
            &[OpenSyncHistory { problems: true }, Close],
            "Sync stopped",
        ),
        ("DW-1208", Action::AddStorage, &[Close], "Storage almost full"),
        ("DW-1209", Action::AddStorage, &[Close], "Your storage is full"),
        (
            "DW-1210",
            Action::RestartToUpdate,
            &[Close],
            "Version 0.9.2 is available",
        ),
        ("DW-1214", Action::RestartToUpdate, &[Close], "Update Beebeeb"),
        ("DW-1211", Action::Unlock, &[OpenWindow, Close], "Beebeeb is locked"),
        ("DW-1213", Action::ChoosePlan, &[Close], "Choose a plan"),
        (
            "DW-1216",
            Action::RetryExplorer,
            &[],
            "Adding Beebeeb to File Explorer\u{2026}",
        ),
        ("DW-1219", Action::SignIn, &[OpenWindow, Close], "Signed out"),
    ];
    for (frame, action, intents, then) in cases {
        let mut rt = runtime(frame);
        assert_eq!(
            ui(&mut rt).snapshot().status.action().map(|(a, _)| a),
            Some(action),
            "{frame}"
        );
        click(&mut rt, "btn-0");
        assert_eq!(ui(&mut rt).take_intents(), intents, "{frame}");
        assert_eq!(text(&rt, "st-title"), then, "{frame}");
        // The engine hears every one of them.
        assert_eq!(asked(frame, |f| f.act(action)), [format!("{action:?}")], "{frame}");
    }
}

/// DW-1213: after the plans page was opened the line reads "Waiting for your plan."
#[test]
fn choosing_a_plan_leaves_the_flyout_waiting_for_it() {
    let mut rt = runtime("DW-1213");
    click(&mut rt, "btn-0");
    assert_eq!(text(&rt, "st-title"), "Choose a plan");
    assert_eq!(text(&rt, "st-line"), "Waiting for your plan.");
}

#[test]
fn the_three_buttons_ask_for_the_window_file_explorer_and_settings() {
    for (key, intents) in [
        ("foot-0", [Intent::OpenBeebeeb, Intent::Close]),
        ("foot-1", [Intent::OpenInFileExplorer, Intent::Close]),
        ("foot-2", [Intent::OpenSettings, Intent::Close]),
    ] {
        let mut rt = runtime("DW-1202");
        click(&mut rt, key);
        assert_eq!(ui(&mut rt).take_intents(), intents, "{key} by a click");
        let mut rt = runtime("DW-1202");
        tab_to(&mut rt, key);
        press(&mut rt, Key::Text(" ".into()));
        assert_eq!(ui(&mut rt).take_intents(), intents, "{key} by Tab and Space");
    }
}

/// A row shows its file in File Explorer; a trashed file's row opens Trash in the window
/// (DW-1201, "Click a row").
#[test]
fn a_row_shows_its_file_and_a_trashed_one_opens_trash() {
    let mut rt = runtime("DW-1202");
    click(&mut rt, "row-0");
    assert_eq!(ui(&mut rt).take_intents(), [Intent::Close]);
    click(&mut rt, "row-6");
    assert_eq!(text(&rt, "row-6-name"), "Notes old.md");
    assert_eq!(ui(&mut rt).take_intents(), [Intent::OpenTrash, Intent::Close]);

    let mut fixture = Ui::new(Box::new(BoardFixture::frame("DW-1202").expect("a frame")));
    fixture.open_row(0);
    fixture.open_row(6);
    assert_eq!(
        asked("DW-1202", |f| f.reveal("Moodboard v3.pdf")),
        ["reveal Moodboard v3.pdf"]
    );
    assert_eq!(fixture.selected(), Some(6));
}

/// "Arrow keys move through the rows; Enter shows the selected row in File Explorer" (DW-1202).
#[test]
fn the_arrow_keys_move_through_the_rows_and_enter_shows_the_row() {
    let mut rt = runtime("DW-1202");
    tab_to(&mut rt, "list-rows");
    assert_eq!(ui(&mut rt).selected(), None);
    press(&mut rt, Key::Enter);
    assert_eq!(
        ui(&mut rt).take_intents(),
        [],
        "Enter with no row selected does nothing"
    );
    press(&mut rt, Key::Down);
    assert_eq!(ui(&mut rt).selected(), Some(0));
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    assert_eq!(ui(&mut rt).selected(), Some(2));
    rt.settle_all();
    rt.frame(0.0);
    assert_eq!(
        info(&rt, "row-2").fill,
        tokens::theme(false).selection,
        "the selected row is marked, neutral"
    );
    press(&mut rt, Key::Up);
    assert_eq!(ui(&mut rt).selected(), Some(1));
    press(&mut rt, Key::End);
    assert_eq!(ui(&mut rt).selected(), Some(9));
    press(&mut rt, Key::Down);
    assert_eq!(ui(&mut rt).selected(), Some(9), "the list does not wrap round");
    press(&mut rt, Key::Home);
    assert_eq!(ui(&mut rt).selected(), Some(0));
    press(&mut rt, Key::Up);
    assert_eq!(ui(&mut rt).selected(), Some(0));
    press(&mut rt, Key::Enter);
    assert_eq!(
        ui(&mut rt).take_intents(),
        [Intent::Close],
        "Enter shows the row and closes"
    );

    // No rows, nothing to select (DW-1211, DW-1213).
    for frame in ["DW-1211", "DW-1213"] {
        let mut ui = Ui::board(frame).expect("a frame");
        assert_eq!(ui.move_selection(1), None, "{frame}");
    }
}

/// Tab goes through the status button, the list, its end and the three buttons, in the order
/// the board announces them (DW-1202: "The status block is announced first, then the list
/// […], then the three buttons").
#[test]
fn tab_reaches_every_control_in_the_boards_order() {
    let mut rt = runtime("DW-1203");
    let mut stops = Vec::new();
    for _ in 0..6 {
        press(&mut rt, Key::Tab);
        stops.push(focused(&rt).unwrap_or_default());
    }
    assert_eq!(stops, ["btn-0", "list-rows", "btn-1", "foot-0", "foot-1", "foot-2"]);

    // The status button by the keyboard: Space on Pause pauses.
    let mut rt = runtime("DW-1203");
    tab_to(&mut rt, "btn-0");
    press(&mut rt, Key::Text(" ".into()));
    assert_eq!(ui(&mut rt).take_intents(), [Intent::PauseSync]);
    assert_eq!(text(&rt, "btn-0-label"), "Resume");
}

/// DW-1220: the list ends with where the rest is, and the sentence only when these are the
/// last 50.
#[test]
fn the_list_ends_with_the_way_to_the_full_history() {
    // The kit numbers its buttons: with no status button, "View full history" is the first.
    let mut rt = runtime("DW-1220");
    assert_eq!(text(&rt, "end-text"), "That's the last 50 changes.");
    assert_eq!(text(&rt, "btn-0-label"), "View full history");
    click(&mut rt, "btn-0");
    assert_eq!(
        ui(&mut rt).take_intents(),
        [Intent::OpenSyncHistory { problems: false }, Intent::Close]
    );
    // Ten rows of a longer history: the button, without a sentence that would be wrong.
    let rt = runtime("DW-1202");
    assert!(has(&rt, "btn-0") && !has(&rt, "end-text"));
}

/// Names and roles (the board's Accessibility notes): a dialog named Beebeeb (the test build's
/// says "Beebeeb Test": two apps side by side must be tellable apart by a screen reader); the status
/// block says its state; the list says how many items it has; a row says its name, what
/// happened and when.
#[test]
fn names_and_roles() {
    let rt = runtime("DW-1203");
    let fly = info(&rt, "fly");
    // The scene renders with the GOLDEN identity (the real app's name), so the flyout's
    // dialog label is the real product name, matching the design board.
    let name = crate::shell::identity::AppIdentity::GOLDEN.product_name();
    assert_eq!((fly.role, fly.label.as_deref()), ("dialog", Some(name)));
    let st = info(&rt, "st");
    assert_eq!(
        (st.role, st.label.as_deref()),
        ("status", Some("Syncing 12 files. 48 MB left \u{00B7} about a minute"))
    );
    let list = info(&rt, "list-rows");
    assert_eq!(
        (list.role, list.label.as_deref()),
        ("list", Some("Recent changes, 10 items"))
    );
    let row = info(&rt, "row-0");
    assert_eq!(
        (row.role, row.label.as_deref()),
        (
            "listitem",
            Some("Lisbon trip 041.heic, Uploading \u{00B7} 3.4 of 8.1 MB, now")
        )
    );
    assert_eq!(info(&rt, "st-bar").role, "progressbar");
    assert_eq!(
        info(&rt, "st-speed-row").label.as_deref(),
        Some("Upload speed 3.2 MB/s")
    );
    for (key, name) in [
        ("foot-0", "Open Beebeeb"),
        ("foot-1", "Open in File Explorer"),
        ("foot-2", "Settings"),
    ] {
        let button = info(&rt, key);
        assert_eq!((button.role, button.label.as_deref()), ("button", Some(name)));
    }
    assert_eq!(
        Status::Syncing {
            files: 1,
            left: None,
            about: None,
            fraction: None,
            speed: None
        }
        .title("Beebeeb"),
        "Syncing 1 file"
    );
    assert_eq!(
        Status::NeedsDecision { files: 1 }.title("Beebeeb"),
        "1 file needs a decision"
    );
    assert_eq!(count(1284), "1,284");
    assert_eq!(count(1_000_000), "1,000,000");
    assert_eq!(count(7), "7");
}

/// DW-1211: "No file names in the flyout […] while locked; with the rows hidden there is
/// nothing to click in the list."
#[test]
fn a_locked_flyout_shows_no_file_name_anywhere_in_its_tree() {
    let names: Vec<String> = match BoardFixture::frame("DW-1202").expect("a frame").snapshot().changes {
        Changes::Rows { rows, .. } => rows.into_iter().map(|row| row.name).collect(),
        _ => panic!("DW-1202 has rows"),
    };
    assert_eq!(names.len(), 10);
    let found = |rt: &Runtime<State>| {
        let tree = rt.accessibility_json().to_string();
        let in_elements = rt.elements().iter().any(|e| {
            names.iter().any(|name| {
                e.text.as_deref().is_some_and(|t| t.contains(name.as_str()))
                    || e.label.as_deref().is_some_and(|l| l.contains(name.as_str()))
            })
        });
        in_elements || names.iter().any(|name| tree.contains(name.as_str()))
    };
    // The check can fail: an unlocked flyout does hold the names.
    assert!(found(&runtime("DW-1202")));
    let rt = runtime("DW-1211");
    assert!(!found(&rt), "a locked flyout holds no file name");
    assert!(!has(&rt, "row-0") && !has(&rt, "list-rows") && !has(&rt, "btn-1"));
    assert_eq!(text(&rt, "empty-title"), "Recent changes are hidden");
}

/// Which frame says what the shell's tray state says, in DW-1201's order.
#[test]
fn the_fixture_follows_the_shells_tray_state_in_the_boards_order() {
    let ready = TrayInputs {
        signed_in: true,
        explorer_ready: true,
        ..TrayInputs::default()
    };
    let with = |change: fn(&mut TrayInputs)| {
        let mut inputs = ready.clone();
        change(&mut inputs);
        inputs
    };
    let cases: [(TrayInputs, &str); 12] = [
        (ready.clone(), "DW-1202"),
        (with(|i| i.engine = EngineState::Syncing), "DW-1203"),
        (with(|i| i.files_remaining = Some(3)), "DW-1203"),
        (with(|i| i.paused = true), "DW-1204"),
        (with(|i| i.conflicts = 2), "DW-1205"),
        (with(|i| i.engine = EngineState::Offline), "DW-1206"),
        (with(|i| i.engine = EngineState::Error), "DW-1207"),
        (with(|i| i.storage_full = true), "DW-1209"),
        (with(|i| i.update_available = true), "DW-1210"),
        (with(|i| i.locked = true), "DW-1211"),
        (with(|i| i.explorer_ready = false), "DW-1215"),
        (
            TrayInputs {
                session_expired: true,
                explorer_ready: true,
                ..TrayInputs::default()
            },
            "DW-1219",
        ),
    ];
    for (inputs, frame) in &cases {
        assert_eq!(frame_for(inputs), *frame, "{inputs:?}");
    }
    // The order: locked over paused, paused over storage full, storage full over offline,
    // a decision over an update, an update over syncing.
    let all = with(|i| {
        i.locked = true;
        i.paused = true;
        i.storage_full = true;
        i.engine = EngineState::Offline;
        i.conflicts = 1;
        i.update_available = true;
    });
    let mut inputs = all.clone();
    let mut seen = Vec::new();
    for clear in [
        (|i: &mut TrayInputs| i.locked = false) as fn(&mut TrayInputs),
        |i| i.paused = false,
        |i| i.storage_full = false,
        |i| i.engine = EngineState::Syncing,
        |i| i.conflicts = 0,
        |i| i.update_available = false,
    ] {
        seen.push(frame_for(&inputs));
        clear(&mut inputs);
    }
    seen.push(frame_for(&inputs));
    assert_eq!(
        seen,
        [
            "DW-1211", "DW-1204", "DW-1209", "DW-1206", "DW-1205", "DW-1210", "DW-1203"
        ]
    );

    // The flyout hears a change of the shell's state once, and keeps its own state between.
    let mut ui = Ui::following(&with(|i| i.engine = EngineState::Offline));
    assert_eq!(ui.snapshot().status, Status::Offline);
    ui.press_action();
    assert_eq!(
        ui.snapshot().status.title("Beebeeb"),
        "Up to date",
        "Try again reached Beebeeb"
    );
    ui.follow(&with(|i| i.engine = EngineState::Offline));
    assert_eq!(
        ui.snapshot().status.title("Beebeeb"),
        "Up to date",
        "the same shell state is not news"
    );
    ui.follow(&with(|i| i.paused = true));
    assert_eq!(ui.snapshot().status.title("Beebeeb"), "Sync is paused");
}

// ---------------------------------------------------------------------------
// The flyout window itself (`FlyoutState`), on the shell's fixture engine
// ---------------------------------------------------------------------------

fn flyout() -> Runtime<FlyoutState> {
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(flyout_window::fixture_state(), flyout_window::view, view::SIZE, env);
    rt.on_key = Some(flyout_window::on_key);
    rt.frame(0.0);
    rt
}

/// The flyout window draws this group's view, not the placeholder, and says what the b says.
#[test]
fn the_flyout_window_draws_the_flyout_in_the_shells_state() {
    let rt = flyout();
    assert_eq!(info(&rt, "fly").rect, [0.0, 0.0, 340.0, 520.0]);
    assert_eq!(rt.state.model.label, "Up to date");
    assert_eq!(text(&rt, "st-title"), "Up to date");
}

/// Paused from the b's menu, the flyout says so; Resume in the flyout resumes through the
/// shell's actions, and both the b's model and the flyout follow.
#[test]
fn resume_in_the_flyout_window_resumes_sync_through_the_shell() {
    let mut rt = flyout();
    rt.update(|s, _| {
        s.actions.pause_sync().expect("the fixture engine pauses");
        s.refresh();
    });
    rt.frame(0.0);
    assert_eq!(rt.state.model.label, "Paused");
    assert_eq!(text(&rt, "st-title"), "Sync is paused");
    click(&mut rt, "btn-0");
    assert!(!rt.state.inputs.paused, "the engine was asked to resume");
    assert_eq!(rt.state.model.label, "Up to date");
    assert_eq!(text(&rt, "st-title"), "Up to date");
    assert_eq!(
        rt.state.tray.take_intents(),
        [],
        "the flyout window carried the intents out"
    );
}

/// The arrow keys work in the flyout window too, beside the shell's Esc.
#[test]
fn the_flyout_windows_list_takes_the_arrow_keys() {
    // Straight after the flyout opens, with nothing focused yet.
    let mut rt = flyout();
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Down);
    assert_eq!(rt.state.tray.selected(), Some(1));
    // And with the list focused: one step a key, not two (the window's handler and the list's).
    tab_to(&mut rt, "list-rows");
    press(&mut rt, Key::Down);
    assert_eq!(rt.state.tray.selected(), Some(2));
    press(&mut rt, Key::Up);
    assert_eq!(rt.state.tray.selected(), Some(1));
}
