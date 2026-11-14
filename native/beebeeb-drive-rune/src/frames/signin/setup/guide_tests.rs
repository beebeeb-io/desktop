//! Tests of the first-time guide: where it starts, what Got it, Next, Skip, Done and Esc do, that
//! each ring is on the real control, and that the window under a tip stays usable.

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::guide::{self, COACH_KEY, FRAMES, NEXT_KEY, RING_KEY, SKIP_KEY, Step, TIPS, TRAY_TEXT, TRAY_TITLE};
use crate::shell::app::{Event, GateState, Screen, Section, SettingsPane};
use crate::shell::host;
use crate::{State, init, view};

/// The main window's client area on the board.
const WINDOW: (f32, f32) = (1278.0, 798.0);

fn window(state: State) -> Runtime<State> {
    window_sized(state, WINDOW)
}

fn window_sized(state: State, size: (f32, f32)) -> Runtime<State> {
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, size, env);
    rt.on_key = Some(host::on_key);
    rt.frame(0.0);
    rt
}

fn at_scene(frame: &str) -> Runtime<State> {
    at_scene_sized(frame, WINDOW)
}

fn at_scene_sized(frame: &str, size: (f32, f32)) -> Runtime<State> {
    let mut state = init();
    let scene = FRAMES
        .iter()
        .find(|(id, _)| *id == frame)
        .unwrap_or_else(|| panic!("no scene {frame}"));
    (scene.1)(&mut state);
    window_sized(state, size)
}

/// The guide started as the app starts it, on the board's Files.
fn started() -> Runtime<State> {
    let mut state = init();
    guide::start(&mut state);
    window(state)
}

fn rect(rt: &Runtime<State>, key: &str) -> Option<[f32; 4]> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .map(|e| e.rect)
}

fn text(rt: &Runtime<State>, key: &str) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .and_then(|e| e.text)
}

fn click(rt: &mut Runtime<State>, key: &str) {
    rt.settle_all();
    rt.frame(0.0);
    let r = rect(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"));
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
}

fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    rt.frame(0.0);
}

fn section(rt: &Runtime<State>) -> Option<Section> {
    match rt.state.shell.app.screen() {
        Screen::Drive(route) => Some(route.section),
        _ => None,
    }
}

/// The ring stands around its target: no side of the target is more than `slack` outside it.
fn ring_holds(rt: &Runtime<State>, target: &str, slack: f32) {
    let ring = rect(rt, RING_KEY).expect("the ring is drawn");
    let t = rect(rt, target).unwrap_or_else(|| panic!("no element keyed {target}"));
    assert!(
        ring[0] <= t[0] + 0.01
            && ring[1] <= t[1] + 0.01
            && ring[0] + ring[2] >= t[0] + t[2] - 0.01
            && ring[1] + ring[3] >= t[1] + t[3] - slack,
        "the ring {ring:?} is not around {target} {t:?}"
    );
}

#[test]
fn every_board_frame_draws_its_step_with_its_words() {
    let words = [
        ("DW-0380", TRAY_TEXT, None, "Got it"),
        ("DW-0301", TIPS[0], Some("1 of 5"), "Next"),
        ("DW-0302", TIPS[1], Some("2 of 5"), "Next"),
        ("DW-0303", TIPS[2], Some("3 of 5"), "Next"),
        ("DW-0304", TIPS[3], Some("4 of 5"), "Next"),
        ("DW-0305", TIPS[4], Some("5 of 5"), "Done"),
    ];
    assert_eq!(FRAMES.len(), 6);
    for (frame, body, counter, button) in words {
        let rt = at_scene(frame);
        assert_eq!(rt.state.guide.step.map(Step::frame), Some(frame));
        assert_eq!(text(&rt, "coach-text").as_deref(), Some(body), "{frame}");
        assert_eq!(text(&rt, "coach-counter").as_deref(), counter, "{frame}");
        assert_eq!(text(&rt, "coach-next-label").as_deref(), Some(button), "{frame}");
        // Skip is on tips 1 to 4 only; DW-0380 has its heading and no ring.
        let tip = counter.is_some();
        assert_eq!(rect(&rt, SKIP_KEY).is_some(), tip && frame != "DW-0305", "{frame}");
        assert_eq!(rect(&rt, RING_KEY).is_some(), tip, "{frame}");
        assert_eq!(
            text(&rt, "coach-title").as_deref(),
            (!tip).then_some(TRAY_TITLE),
            "{frame}"
        );
    }
}

#[test]
fn each_ring_is_on_the_real_control_and_follows_it() {
    for (frame, target) in [
        ("DW-0301", "new-btn"),
        ("DW-0302", "row-6"),
        ("DW-0303", "Shared-row"),
        ("DW-0304", "Photos-row"),
    ] {
        ring_holds(&at_scene(frame), target, 0.0);
    }
    // With no file selected when the guide starts, tip 2 points at the folder's first file.
    let mut state = init();
    state.clear_selection();
    guide::start(&mut state);
    let mut rt = window(state);
    click(&mut rt, NEXT_KEY);
    click(&mut rt, NEXT_KEY);
    assert_eq!(rt.state.guide.step, Some(Step::Tip(2)));
    ring_holds(&rt, "row-5", 0.0);
    let (ring, row) = (rect(&rt, RING_KEY).unwrap(), rect(&rt, "row-5").unwrap());
    assert!((ring[1] - row[1]).abs() < 0.01, "the ring {ring:?} is on row 5 {row:?}");
    // The board's ring ends 1.5 above the gear button's lower edge.
    ring_holds(&at_scene("DW-0305"), "settings-btn", 1.5);

    // A window of another size: the gear moved, and the ring with it.
    let before = rect(&at_scene("DW-0305"), RING_KEY).unwrap();
    let rt = at_scene_sized("DW-0305", (1100.0, 700.0));
    let after = rect(&rt, RING_KEY).unwrap();
    assert!((before[1] - after[1] - 98.0).abs() < 0.5, "{before:?} -> {after:?}");
    ring_holds(&rt, "settings-btn", 1.5);
}

#[test]
fn got_it_and_next_step_through_the_five_tips_and_done_ends_on_files() {
    let mut rt = started();
    assert_eq!(rt.state.guide.step, Some(Step::Tray));
    let mut seen = Vec::new();
    for _ in 0..6 {
        seen.push((rt.state.guide.step.unwrap(), section(&rt).unwrap()));
        assert!(
            rect(&rt, COACH_KEY).is_some(),
            "{:?} draws its card",
            rt.state.guide.step
        );
        click(&mut rt, NEXT_KEY);
    }
    assert_eq!(
        seen,
        [
            (Step::Tray, Section::Files),
            (Step::Tip(1), Section::Files),
            (Step::Tip(2), Section::Files),
            // "Switches to the real Shared view by itself."
            (Step::Tip(3), Section::Shared),
            (Step::Tip(4), Section::Photos),
            (Step::Tip(5), Section::Files),
        ]
    );
    // Done: no guide, and the window is on Files.
    assert_eq!(rt.state.guide.step, None);
    assert_eq!(section(&rt), Some(Section::Files));
    assert!(rect(&rt, COACH_KEY).is_none() && rect(&rt, RING_KEY).is_none());
}

#[test]
fn tip_2_is_left_out_when_the_folder_has_no_file() {
    let mut state = init();
    state.listing.items.retain(|item| item.is_folder);
    assert!(!state.listing.items.is_empty(), "the board's folders are still there");
    state.clear_selection();
    guide::start(&mut state);
    let mut rt = window(state);
    click(&mut rt, NEXT_KEY);
    assert_eq!(rt.state.guide.step, Some(Step::Tip(1)));
    click(&mut rt, NEXT_KEY);
    assert_eq!(rt.state.guide.step, Some(Step::Tip(3)));
    assert_eq!(section(&rt), Some(Section::Shared));
}

#[test]
fn skip_and_esc_end_the_whole_guide_where_the_window_is() {
    let mut rt = at_scene("DW-0303");
    click(&mut rt, SKIP_KEY);
    assert_eq!(rt.state.guide.step, None);
    assert_eq!(section(&rt), Some(Section::Shared));
    assert!(rect(&rt, COACH_KEY).is_none());

    let mut rt = at_scene("DW-0301");
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.guide.step, None);
    assert!(rect(&rt, COACH_KEY).is_none() && rect(&rt, RING_KEY).is_none());
    // With no guide, Esc is the window's again: it is not swallowed by a guide that is gone.
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.guide.step, None);
}

#[test]
fn the_window_under_a_tip_stays_usable() {
    // A row under the dim takes its click.
    let mut rt = at_scene("DW-0301");
    assert_eq!(rt.state.selected, None);
    click(&mut rt, "row-9");
    assert_eq!(rt.state.selected, Some(9));
    assert_eq!(rt.state.guide.step, Some(Step::Tip(1)), "and the tip is still there");

    // The control inside the ring takes its click too: the gear opens settings.
    let mut rt = at_scene("DW-0305");
    click(&mut rt, "settings-btn");
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::General));
    // A tip belongs to its place: over settings it is not drawn, and Esc is settings' own.
    assert!(rect(&rt, COACH_KEY).is_none());
    press(&mut rt, Key::Escape);
    assert_eq!(section(&rt), Some(Section::Files));
    assert_eq!(rt.state.guide.step, Some(Step::Tip(5)));
    assert!(rect(&rt, COACH_KEY).is_some(), "back on Files the tip shows again");
}

/// The first run on "You're set up".
fn set_up_ui() -> super::Ui {
    use super::source::{Explorer, FixtureSetupSource, Script};
    let mut ui = super::Ui::new(Box::new(FixtureSetupSource::new(Script {
        explorer: Explorer::Added,
        ..Script::default()
    })));
    ui.skip();
    ui
}

/// The first-run window on "You're set up", for an account that has or has not done the guide on this PC.
fn set_up(guide_done: bool) -> Runtime<State> {
    let mut state = init();
    if guide_done {
        let account = state.shell.app.account().expect("the fixture is signed in").to_string();
        state.guide_marks = Box::new(super::guide_marks::FixtureMarks::done_for(&account));
    }
    host::dispatch(&mut state, Event::Gate(GateState::FirstRun));
    state.setup = Some(set_up_ui());
    window_sized(state, (958.0, 638.0))
}

/// A first run ends now ("Open my Beebeeb"), and the window's poll runs: whether the guide started by itself.
fn a_first_run_ends(rt: &mut Runtime<State>) -> bool {
    let mut ui = set_up_ui();
    ui.finish();
    rt.update(|s, tx| {
        s.setup = Some(ui);
        host::poll(s, 0.0, tx);
    });
    let started = rt.state.guide.step == Some(Step::Tray);
    rt.update(|s, _| s.setup = None);
    started
}

#[test]
fn open_my_beebeeb_at_the_end_of_the_first_run_starts_the_guide() {
    let mut rt = set_up(false);
    // A row of "You're set up" is not the guide's to take, and starts nothing.
    click(&mut rt, "item-1");
    rt.update(|s, tx| {
        guide::poll(s, tx);
    });
    assert_eq!(rt.state.guide.step, None, "nothing starts the guide before the press");

    click(&mut rt, "btn-0");
    rt.update(|s, tx| {
        host::poll(s, 0.0, tx);
    });
    assert_eq!(rt.state.guide.step, Some(Step::Tray));
    // The first run's intents are all still there for the shell: the guide read none of them.
    assert_eq!(
        rt.state.setup.as_mut().unwrap().take_intents(),
        [
            super::Intent::OpenSetting(super::Setting::AppLock),
            super::Intent::OpenDrive
        ]
    );
    // The tip shows once the window is the main window on Files (the session's gate opens it),
    // never over the first-run window. Since scene gates are set on the fixture engine and
    // survive the poll's refresh, the test opens the gate as the engine would.
    rt.update(|s, _| {
        host::dispatch(s, Event::Gate(GateState::Ready));
    });
    rt.frame(0.0);
    assert_eq!(section(&rt), Some(Section::Files));
    assert_eq!(text(&rt, "coach-title").as_deref(), Some(TRAY_TITLE));
    rt.update(|s, _| {
        host::dispatch(s, Event::Gate(GateState::FirstRun));
    });
    rt.frame(0.0);
    assert_eq!(rt.state.guide.step, Some(Step::Tray));
    assert!(rect(&rt, COACH_KEY).is_none(), "no tip over the first-run window");
}

#[test]
fn an_account_that_has_done_the_guide_does_not_get_it_again() {
    let mut rt = set_up(true);
    click(&mut rt, "btn-0");
    rt.update(|s, tx| {
        host::poll(s, 0.0, tx);
    });
    assert_eq!(rt.state.guide.step, None);
    assert_eq!(
        rt.state.setup.as_mut().unwrap().take_intents(),
        [super::Intent::OpenDrive]
    );
}

/// A window on a fixture engine of the test's own: the test changes the account and the gate on it.
fn on_engine() -> (std::sync::Arc<crate::shell::actions::FixtureEngine>, Runtime<State>) {
    let engine = std::sync::Arc::new(crate::shell::actions::FixtureEngine::ready());
    let shell = host::Shell::fixture_on(
        engine.clone(),
        std::sync::Arc::new(crate::shell::platform::fake::FakePlatform::default()),
    );
    let mut rt = window(crate::shell::state::init_with(shell));
    rt.poll = Some(host::poll);
    (engine, rt)
}

/// Owner ruling 2026-11-13: done once per ACCOUNT on this PC. Finished or skipped, the account does not get the
/// guide again, also after a sign-out and a sign-in; another account on the same PC gets it once; and a guide that
/// ended because the account locked was not done.
#[test]
fn the_guide_is_done_once_per_account() {
    let first = "fixture-account";
    for ending in ["Done", "Skip", "Esc"] {
        let (engine, mut rt) = on_engine();
        assert_eq!(rt.state.shell.app.account(), Some(first));
        assert!(a_first_run_ends(&mut rt), "{ending}: a first run starts the guide");
        // Not done yet: started is not done. A lock ends it, and the next first run starts it again.
        engine.set_gate(GateState::Locked);
        tick(&mut rt);
        assert_eq!(rt.state.guide.step, None, "{ending}: the lock ended it");
        engine.set_gate(GateState::Ready);
        tick(&mut rt);
        tick(&mut rt);
        assert_eq!(
            rt.state.guide.step,
            Some(Step::Tray),
            "{ending}: a guide the lock ended was not done: it starts again when the account is back"
        );

        match ending {
            "Done" => {
                for _ in 0..6 {
                    click(&mut rt, NEXT_KEY);
                    tick(&mut rt);
                }
            }
            "Skip" => {
                click(&mut rt, NEXT_KEY);
                tick(&mut rt);
                click(&mut rt, SKIP_KEY);
            }
            _ => {
                tick(&mut rt);
                press(&mut rt, Key::Escape);
            }
        }
        assert_eq!(rt.state.guide.step, None, "{ending}: the guide ended");
        assert!(!a_first_run_ends(&mut rt), "{ending}: done for this account");

        // A sign-out and a sign-in of the same account: still done.
        engine.set_gate(GateState::SignedOut);
        tick(&mut rt);
        engine.set_gate(GateState::Ready);
        tick(&mut rt);
        assert_eq!(rt.state.shell.app.account(), Some(first));
        assert!(
            !a_first_run_ends(&mut rt),
            "{ending}: still done after a sign-out and a sign-in"
        );

        // Another account on this PC: once.
        engine.set_account("user-b");
        tick(&mut rt);
        assert_eq!(rt.state.shell.app.account(), Some("user-b"));
        assert!(a_first_run_ends(&mut rt), "{ending}: another account gets it once");
        tick(&mut rt);
        press(&mut rt, Key::Escape);
        assert_eq!(rt.state.guide.step, None);
        assert!(!a_first_run_ends(&mut rt), "{ending}: and not again");

        // And the first account's mark is still its own.
        engine.set_account(first);
        tick(&mut rt);
        assert!(!a_first_run_ends(&mut rt), "{ending}: the first account stays done");
    }
}

/// An answer is for the account it was asked for: asked under one account and taken in under another, there is
/// none (so nothing starts); asked and taken in under the same account, there is.
#[test]
fn an_answer_asked_under_one_account_is_none_under_another() {
    use super::guide_marks::{FixtureMarks, GuideMarks, Known};
    let mut marks = FixtureMarks::done_for("account-a");
    marks.ask(Some("account-b"));
    assert_eq!(marks.answer(Some("account-a")), None, "asked for b, taken in under a");
    assert_eq!(marks.answer(Some("account-b")), None, "and an answer is given once");
    marks.ask(Some("account-b"));
    assert_eq!(marks.answer(Some("account-b")), Some(Known::NotDone));
    marks.ask(Some("account-a"));
    assert_eq!(marks.answer(Some("account-a")), Some(Known::Done));
    marks.ask(None);
    assert_eq!(marks.answer(None), Some(Known::NotKnown), "nobody signed in: not known");
}

#[test]
fn the_guide_ends_when_the_account_locks_signs_out_or_changes() {
    for gate in [GateState::Locked, GateState::SignedOut] {
        let mut rt = at_scene("DW-0303");
        rt.update(|s, tx| {
            host::dispatch(s, Event::Gate(gate));
            guide::poll(s, tx);
        });
        assert_eq!(rt.state.guide.step, None, "{gate:?}");
        // Back in: no tip resumes.
        rt.update(|s, _| {
            host::dispatch(s, Event::Gate(GateState::Ready));
        });
        rt.frame(0.0);
        assert!(rect(&rt, COACH_KEY).is_none(), "{gate:?}");
    }
    // The same account: the step stays. Another account's step is not this account's.
    let mut rt = at_scene("DW-0301");
    assert!(rt.state.shell.app.account().is_some(), "the fixture is signed in");
    rt.update(|s, tx| {
        guide::poll(s, tx);
    });
    assert_eq!(rt.state.guide.step, Some(Step::Tip(1)));
    rt.update(|s, tx| {
        s.guide.account = Some("another-account".into());
        guide::poll(s, tx);
    });
    assert_eq!(rt.state.guide.step, None);
}

/// What `shell::host::poll` does at the start of a frame, then the frame.
fn tick(rt: &mut Runtime<State>) {
    rt.update(|s, tx| {
        host::poll(s, 0.0, tx);
    });
    rt.frame(0.0);
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
}

#[test]
fn the_keyboard_is_the_cards_tab_stays_in_it_and_enter_steps() {
    let mut rt = started();
    tick(&mut rt);
    assert_eq!(focused(&rt).as_deref(), Some(NEXT_KEY), "focus starts on Got it");
    // DW-0380 has one button: Tab stays on it.
    press(&mut rt, Key::Tab);
    assert_eq!(focused(&rt).as_deref(), Some(NEXT_KEY));
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.guide.step, Some(Step::Tip(1)));
    assert_eq!(
        focused(&rt).as_deref(),
        Some(NEXT_KEY),
        "focus is on Next after the step"
    );
    // Tab goes to Skip and back to Next: it never leaves the card.
    press(&mut rt, Key::Tab);
    assert_eq!(focused(&rt).as_deref(), Some(SKIP_KEY));
    press(&mut rt, Key::Tab);
    assert_eq!(focused(&rt).as_deref(), Some(NEXT_KEY));
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.guide.step, Some(Step::Tip(2)));
    // Enter on Skip ends the guide.
    press(&mut rt, Key::Tab);
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.guide.step, None);

    // From Settings, by the keyboard's route: focus lands on the card too.
    let mut state = init();
    host::dispatch(&mut state, Event::OpenSettings(SettingsPane::General));
    let mut rt = window(state);
    click(&mut rt, "item-1-0");
    tick(&mut rt);
    assert_eq!(focused(&rt).as_deref(), Some(NEXT_KEY));
}

#[test]
fn what_a_section_has_in_front_hides_the_tip_and_gets_esc_first() {
    // Tip 1 rings New; New, then "New folder": the dialog is in front.
    let mut rt = started();
    click(&mut rt, NEXT_KEY);
    assert_eq!(rt.state.guide.step, Some(Step::Tip(1)));
    rt.update(|s, _| {
        s.begin_new_folder();
    });
    rt.frame(0.0);
    assert!(rt.state.actions.dialog.is_some());
    assert!(rect(&rt, COACH_KEY).is_none() && rect(&rt, RING_KEY).is_none());
    press(&mut rt, Key::Escape);
    assert!(rt.state.actions.dialog.is_none(), "Esc closed the dialog");
    assert_eq!(
        rt.state.guide.step,
        Some(Step::Tip(1)),
        "and the guide is still on tip 1"
    );
    assert!(rect(&rt, COACH_KEY).is_some(), "which shows again");

    // Tip 2 says "Right-click a file": the file's menu (DW-0402) is in front.
    let mut rt = at_scene("DW-0302");
    let menu = crate::frames::files::SCENES
        .iter()
        .find(|(id, _)| *id == "DW-0402")
        .unwrap();
    rt.update(|s, _| (menu.1)(s));
    rt.frame(0.0);
    assert!(rt.state.actions.menu.is_some());
    assert!(rect(&rt, COACH_KEY).is_none() && rect(&rt, RING_KEY).is_none());
    press(&mut rt, Key::Escape);
    assert!(rt.state.actions.menu.is_none());
    assert_eq!(rt.state.guide.step, Some(Step::Tip(2)));

    // Tip 3 over Shared: a dialog of Shared.
    let mut rt = at_scene("DW-0303");
    rt.update(|s, _| {
        s.frames
            .shared
            .open_dialog(crate::frames::shared::dialogs::Dialog::NewRequest)
    });
    rt.frame(0.0);
    assert!(rect(&rt, COACH_KEY).is_none());
    press(&mut rt, Key::Escape);
    assert!(rt.state.frames.shared.dialog.is_none(), "Esc closed the dialog");
    assert_eq!(rt.state.guide.step, Some(Step::Tip(3)));
    assert!(rect(&rt, COACH_KEY).is_some());

    // Tip 4 over Photos: an opened photo.
    let mut rt = at_scene("DW-0304");
    rt.update(|s, _| s.photos.open_tile(0, 0));
    rt.frame(0.0);
    assert!(rt.state.photos.viewer.is_some());
    assert!(rect(&rt, COACH_KEY).is_none());
    press(&mut rt, Key::Escape);
    assert!(rt.state.photos.viewer.is_none(), "Esc closed the photo");
    assert_eq!(rt.state.guide.step, Some(Step::Tip(4)));
    assert!(rect(&rt, COACH_KEY).is_some());
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent {
        key: Key::Command(c),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    }
}

/// Tip 1 on screen with the keyboard on the card's Next.
fn on_tip_1() -> Runtime<State> {
    let mut rt = started();
    click(&mut rt, NEXT_KEY);
    tick(&mut rt);
    assert_eq!(rt.state.guide.step, Some(Step::Tip(1)));
    assert_eq!(focused(&rt).as_deref(), Some(NEXT_KEY), "the card has the keyboard");
    rt
}

#[test]
fn the_new_file_dialog_over_tip_1_hides_the_tip_and_has_the_keyboard() {
    // Tip 1 rings New. The two ways a person opens New file (DW-1521) under it: Ctrl+N with the keyboard on the
    // card (the card lets the key through to Files), and New, then "New file…".
    for by_keyboard in [true, false] {
        let mut rt = on_tip_1();
        if by_keyboard {
            rt.key(ctrl('n'));
            rt.frame(0.0);
        } else {
            click(&mut rt, "new-btn");
            assert!(rt.state.actions.menu.is_some(), "New's menu is open");
            assert_eq!(guide::showing(&rt.state), None, "the menu is in front of the tip");
            let item = rt
                .elements()
                .into_iter()
                .find(|e| {
                    !e.exiting && e.role == "menuitem" && e.label.as_deref().is_some_and(|l| l.starts_with("New file"))
                })
                .expect("New's menu has New file");
            let (x, y) = (item.rect[0] + item.rect[2] / 2.0, item.rect[1] + item.rect[3] / 2.0);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
        }
        tick(&mut rt);
        tick(&mut rt);
        assert!(
            rt.state.viewer.new_file.is_some(),
            "keyboard {by_keyboard}: the dialog is open"
        );
        assert_eq!(guide::showing(&rt.state), None, "the dialog is in front of the tip");
        assert!(
            rect(&rt, COACH_KEY).is_none() && rect(&rt, RING_KEY).is_none(),
            "no card and no ring over the dialog"
        );
        assert_eq!(focused(&rt).as_deref(), Some("type-0"), "the keyboard is the dialog's");
        // Tab is the dialog's: it never jumps to the card's buttons.
        press(&mut rt, Key::Tab);
        let at = focused(&rt);
        assert!(
            at.as_deref().is_some_and(|k| !k.starts_with("coach")),
            "Tab stays in the dialog: {at:?}"
        );
        // Enter is the dialog's too: it never presses the card's Next.
        press(&mut rt, Key::Enter);
        tick(&mut rt);
        assert!(rt.state.viewer.new_file.is_some(), "Enter left the dialog open");
        assert_eq!(
            rt.state.guide.step,
            Some(Step::Tip(1)),
            "Enter did not move the guide on"
        );
        assert!(rect(&rt, COACH_KEY).is_none(), "still no card over the dialog");
        // Esc closes the dialog, not the guide; the tip is there again, and the keyboard is the card's again.
        press(&mut rt, Key::Escape);
        tick(&mut rt);
        tick(&mut rt);
        assert!(rt.state.viewer.new_file.is_none(), "Esc closed the dialog");
        assert_eq!(rt.state.guide.step, Some(Step::Tip(1)), "the guide is still on tip 1");
        assert!(rect(&rt, COACH_KEY).is_some() && rect(&rt, RING_KEY).is_some());
        assert_eq!(
            focused(&rt).as_deref(),
            Some(NEXT_KEY),
            "keyboard {by_keyboard}: the tip that came back has the keyboard"
        );
        // So Enter is Next, never the selected row's.
        press(&mut rt, Key::Enter);
        tick(&mut rt);
        assert_eq!(rt.state.guide.step, Some(Step::Tip(2)), "Enter pressed Next");
        assert!(rt.state.viewer.ui.is_none(), "and opened no file");
    }
}

/// Ctrl+N and Esc with no poll in between: the dialog is asked for and not open yet. Esc is not the tip's Skip.
#[test]
fn esc_right_after_ctrl_n_does_not_end_the_guide() {
    let mut rt = on_tip_1();
    // Both keys before the window's next poll (the test's window polls only in `tick`).
    rt.key(ctrl('n'));
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    rt.frame(0.0);
    assert!(
        rt.state.viewer.new_file.is_none(),
        "no poll ran: the dialog is not open yet"
    );
    assert_eq!(
        rt.state.guide.step,
        Some(Step::Tip(1)),
        "Esc before the poll opened the dialog is not Skip"
    );
    tick(&mut rt);
    tick(&mut rt);
    assert_eq!(rt.state.guide.step, Some(Step::Tip(1)), "the guide is still there");
}

/// What the Files window has over its list is in front of a tip, each of them: no card and no ring over it, and
/// Esc is not Skip.
#[test]
fn every_layer_of_the_files_window_is_in_front_of_a_tip() {
    type Put = fn(&mut State);
    let layers: [(&str, Put); 4] = [
        ("Uploads", |s| {
            s.actions.uploads_open = true;
            assert!(s.actions.source.uploads().is_some(), "the board's window has uploads");
        }),
        ("the panel that says a file needs a connection", |s| {
            s.global.blocked_preview = Some(("f-1".into(), "An invented name.txt".into()));
        }),
        ("a drop hovering", |s| {
            s.actions.drop_hover = Some(crate::frames::files::uploads::DropHover {
                row: Some(0),
                count: None,
            });
        }),
        ("the protected-folder message", |s| {
            s.actions.protected.alert = Some(crate::frames::files::protected::Words {
                title: "An invented title".into(),
                body: "An invented reason.".into(),
            });
        }),
    ];
    for (name, put) in layers {
        let mut rt = on_tip_1();
        assert!(guide::showing(&rt.state).is_some());
        rt.update(|s, _| put(s));
        rt.frame(0.0);
        assert_eq!(guide::showing(&rt.state), None, "{name} is in front of the tip");
        assert!(
            rect(&rt, COACH_KEY).is_none() && rect(&rt, RING_KEY).is_none(),
            "{name}: no card and no ring over it"
        );
        rt.update(|s, tx| {
            let esc = KeyEvent {
                key: Key::Escape,
                mods: Mods::default(),
            };
            assert!(!guide::on_key(s, &esc, tx), "{name}: Esc is not the tip's");
        });
        assert_eq!(
            rt.state.guide.step,
            Some(Step::Tip(1)),
            "{name}: the guide is still there"
        );
    }
}

/// Search open over Files: Esc ends search, not the guide.
#[test]
fn esc_in_search_ends_search_and_not_the_guide() {
    let mut rt = on_tip_1();
    rt.key(ctrl('f'));
    tick(&mut rt);
    assert!(crate::frames::search::is_open(&rt.state), "Ctrl+F opened search");
    assert_eq!(guide::showing(&rt.state), None, "search is in front of the tip");
    press(&mut rt, Key::Escape);
    tick(&mut rt);
    assert!(!crate::frames::search::is_open(&rt.state), "Esc ended search");
    assert_eq!(rt.state.guide.step, Some(Step::Tip(1)), "and not the guide");
    tick(&mut rt);
    assert!(rect(&rt, COACH_KEY).is_some(), "the tip is there again");
}

/// A session the server ended draws its own state over the window: no tip over it, and no key steps the guide.
#[test]
fn a_session_the_server_ended_shows_no_tip_and_takes_no_step() {
    use crate::frames::global::source::{GlobalFixture, board};
    let mut rt = on_tip_1();
    rt.update(|s, _| {
        s.global = crate::frames::global::Ui::new(Box::new(GlobalFixture::of(board::signed_out())));
    });
    rt.frame(0.0);
    assert!(rt.state.session_ended());
    assert_eq!(guide::showing(&rt.state), None);
    assert!(rect(&rt, COACH_KEY).is_none() && rect(&rt, RING_KEY).is_none());
    rt.update(|s, tx| {
        for key in [Key::Enter, Key::Escape] {
            let ev = KeyEvent {
                key,
                mods: Mods::default(),
            };
            assert!(!guide::on_key(s, &ev, tx), "no key is the tip's");
        }
    });
    assert_eq!(
        rt.state.guide.step,
        Some(Step::Tip(1)),
        "no step was taken, and nothing was marked done"
    );
}

#[test]
fn tip_1s_ring_stands_where_the_board_places_it() {
    // The board's ring: left 964, top 37, 88 x 36 (right 1052, bottom 73). Its four edges, as numbers: nothing
    // here is derived from where New stands.
    let rt = at_scene("DW-0301");
    let ring = rect(&rt, RING_KEY).expect("the ring is drawn");
    assert!((ring[0] - 964.0).abs() < 0.01, "left {ring:?}");
    assert!((ring[1] - 37.0).abs() < 0.01, "top {ring:?}");
    assert!((ring[0] + ring[2] - 1052.0).abs() < 0.01, "right {ring:?}");
    assert!((ring[1] + ring[3] - 73.0).abs() < 0.01, "bottom {ring:?}");
}

#[test]
fn at_the_windows_smallest_size_every_card_and_ring_is_inside_it() {
    // 960 x 640 less the frame the OS draws.
    let size = (958.0, 638.0);
    for (frame, target) in [
        ("DW-0301", Some("new-btn")),
        ("DW-0302", Some("row-6")),
        ("DW-0303", Some("Shared-row")),
        ("DW-0304", Some("Photos-row")),
        ("DW-0305", Some("settings-btn")),
        ("DW-0380", None),
    ] {
        let rt = at_scene_sized(frame, size);
        let card = rect(&rt, COACH_KEY).unwrap_or_else(|| panic!("{frame} draws its card"));
        assert!(
            card[0] >= 0.0 && card[1] >= 0.0 && card[0] + card[2] <= size.0 && card[1] + card[3] <= size.1,
            "{frame}: the card {card:?} is inside {size:?}"
        );
        if let Some(target) = target {
            ring_holds(&rt, target, 1.5);
        }
    }
}

#[test]
fn show_the_guide_again_in_settings_runs_it_on_files() {
    let mut state = init();
    host::dispatch(&mut state, Event::OpenSettings(SettingsPane::General));
    let mut rt = window(state);
    assert_eq!(text(&rt, "item-1-0-title").as_deref(), Some("Show the guide again"));
    click(&mut rt, "item-1-0");
    assert_eq!(section(&rt), Some(Section::Files), "settings closed");
    assert_eq!(rt.state.guide.step, Some(Step::Tray));
    assert_eq!(text(&rt, "coach-title").as_deref(), Some(TRAY_TITLE));
    assert!(
        rt.state.settings.take_intents().is_empty(),
        "nothing is left asked of the shell"
    );
}

#[test]
fn the_cards_have_names_and_their_buttons_take_the_keyboard() {
    let rt = at_scene("DW-0302");
    let card = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(COACH_KEY))
        .unwrap();
    assert_eq!(card.label.as_deref(), Some("Guide, tip 2 of 5"));
    for key in [SKIP_KEY, NEXT_KEY] {
        let button = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap();
        assert!(button.focusable, "{key} takes Tab");
    }
}
