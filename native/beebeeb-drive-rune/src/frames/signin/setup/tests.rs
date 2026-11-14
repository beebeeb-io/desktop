//! Tests of the first run: the owner's findings on the first-run window (one caption, nothing cut
//! off at 100 %, 125 % and 150 %, the compare view inside the window, a bundled sample photo), every
//! frame's state, the keyboard, names and roles.

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::source::{
    BUNDLED_SAMPLE_PNG, Explorer, FixtureSetupSource, PreviewFormat, PreviewSize, Script, SetupSource,
};
use super::{Answered, CompareMode, Intent, SHEET, Setting, Step, Ui, scene};
use crate::shell::app::Event;
use crate::{State, init, view};

/// The first-run window's client area on the board (960 x 640 less the 1 px frame the OS draws).
const WINDOW: (f32, f32) = (958.0, 638.0);

const FRAMES: [(&str, &str); 11] = [
    ("DW-0201", "Add a passkey"),
    ("DW-0202", "Lock Beebeeb"),
    ("DW-0203", "Back up your folders"),
    ("DW-0204", "Thumbnails and previews"),
    ("DW-0209", "Thumbnails and previews"),
    ("DW-0206", "Beebeeb is in File Explorer"),
    ("DW-0206b", "Adding Beebeeb to File Explorer"),
    ("DW-0280", "Couldn't add Beebeeb to File Explorer"),
    ("DW-0207", "Stay informed"),
    ("DW-0281", "Open Beebeeb when you log in"),
    ("DW-0208", "You're set up"),
];

fn runtime_at(ui: Ui, scale: f32, dark: bool) -> Runtime<State> {
    let mut state = init();
    crate::shell::host::dispatch(&mut state, Event::Gate(crate::shell::app::GateState::FirstRun));
    state.setup = Some(ui);
    let env = Env {
        system_dark: dark,
        scale,
    };
    let mut rt = Runtime::new(state, view, WINDOW, env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.frame(0.0);
    rt
}

fn runtime(frame: &str) -> Runtime<State> {
    runtime_at(scene(frame).unwrap_or_else(|| panic!("no scene {frame}")), 2.0, false)
}

fn ui(rt: &mut Runtime<State>) -> &mut Ui {
    rt.state.setup.as_mut().expect("setup is open")
}

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn count(rt: &Runtime<State>, key: &str) -> usize {
    rt.elements()
        .iter()
        .filter(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .count()
}

fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    rt.frame(0.0);
}

fn click(rt: &mut Runtime<State>, key: &str) {
    rt.settle_all();
    rt.frame(0.0);
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
}

fn tab_to(rt: &mut Runtime<State>, key: &str) {
    for _ in 0..30 {
        if focused(rt).as_deref() == Some(key) {
            return;
        }
        press(rt, Key::Tab);
    }
    panic!("Tab never reached {key}; focus is on {:?}", focused(rt));
}

fn fixture(script: Script) -> (Ui, super::source::CallLog) {
    let source = FixtureSetupSource::new(script);
    let log = source.log();
    (Ui::new(Box::new(source)), log)
}

// ---------------------------------------------------------------------------
// The owner's findings (task 2445, 2026-11-11 12:46 and 12:50)
// ---------------------------------------------------------------------------

/// "The window has TWO title bars ... with a menu bar": every frame draws exactly one caption
/// bar, one window, and nothing with the role of a menu bar or a menu.
#[test]
fn every_step_has_one_caption_bar_and_no_menu_bar() {
    for (frame, _) in FRAMES {
        let rt = runtime(frame);
        assert_eq!(count(&rt, "caption-bar"), 1, "{frame}: one caption bar");
        assert_eq!(count(&rt, "cap-title"), 1, "{frame}: one window title");
        assert_eq!(count(&rt, "window"), 1, "{frame}: one window");
        let menus = rt
            .elements()
            .iter()
            .filter(|e| !e.exiting && matches!(&*e.role, "menubar" | "menu" | "menuitem"))
            .count();
        assert_eq!(menus, 0, "{frame}: no menu bar");
        // The caption is the first thing in the window and as wide as it.
        let caption = info(&rt, "caption-bar").rect;
        assert_eq!((caption[0], caption[1], caption[2]), (0.0, 0.0, WINDOW.0), "{frame}");
    }
}

/// "Every step fits the board's 960 x 640 window without scrolling", at 100 %, 125 % and 150 %:
/// every keyed element of every frame lies inside the window, in both themes.
#[test]
fn every_step_fits_the_window_at_100_125_and_150_percent() {
    let mut checked = 0;
    for (frame, _) in FRAMES {
        for scale in [1.0, 1.25, 1.5] {
            let rt = runtime_at(scene(frame).expect("a scene"), scale, false);
            for e in rt.elements() {
                let Some(key) = e.key.as_deref() else { continue };
                if e.exiting {
                    continue;
                }
                let [x, y, w, h] = e.rect;
                assert!(
                    x >= -0.01 && y >= -0.01 && x + w <= WINDOW.0 + 0.01 && y + h <= WINDOW.1 + 0.01,
                    "{frame} at {scale}: {key} is cut off ({x}, {y}, {w} x {h}) in {} x {}",
                    WINDOW.0,
                    WINDOW.1
                );
                checked += 1;
            }
            // The buttons at the foot of the column are whole and inside the main part.
            let main = info(&rt, "au-main").rect;
            let column = info(&rt, "au-col").rect;
            assert!(
                column[1] >= main[1] && column[1] + column[3] <= main[1] + main[3],
                "{frame} at {scale}: the column ({} high) does not fit the main part ({} high)",
                column[3],
                main[3]
            );
        }
    }
    assert!(checked > 11 * 3 * 40, "the check looked at elements ({checked})");
}

/// "The comparison step is a bit too big for the screen": the compare view is the board's
/// 700 x 520 sheet, whole inside the window under the caption, and so is everything in it.
#[test]
fn the_compare_view_fits_inside_the_window() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let rt = runtime_at(scene("DW-0209").expect("a scene"), scale, false);
        let caption = info(&rt, "caption-bar").rect;
        let sheet = info(&rt, "sheet").rect;
        assert_eq!((sheet[2], sheet[3]), SHEET, "at {scale}");
        assert!(
            sheet[1] >= caption[1] + caption[3] - 0.01,
            "at {scale}: under the caption"
        );
        assert!(
            sheet[0] >= 0.0 && sheet[0] + sheet[2] <= WINDOW.0,
            "at {scale}: inside, sideways"
        );
        assert!(sheet[1] + sheet[3] <= WINDOW.1, "at {scale}: its foot is in the window");
        for key in [
            "cmp-done",
            "cmp-seg",
            "cmp-photo",
            "cmp-wipe",
            "cmp-opt-0",
            "cmp-opt-5",
            "cmp-choose",
        ] {
            let r = info(&rt, key).rect;
            assert!(
                r[0] >= sheet[0]
                    && r[1] >= sheet[1]
                    && r[0] + r[2] <= sheet[0] + sheet[2]
                    && r[1] + r[3] <= sheet[1] + sheet[3],
                "at {scale}: {key} is outside the sheet"
            );
        }
    }
}

/// "When somebody just started out the comparison tool should load a photo": the sample ships
/// inside the app, decodes, and is what the step and the compare view draw.
#[test]
fn with_no_photo_of_their_own_the_bundled_sample_is_loaded() {
    let (ui, _) = fixture(Script::default());
    assert!(ui.sample().bundled);
    assert_eq!(ui.sample().original_png, BUNDLED_SAMPLE_PNG);
    assert_eq!(
        &BUNDLED_SAMPLE_PNG[..8],
        b"\x89PNG\r\n\x1a\n",
        "the bundled sample is a PNG"
    );
    let (original, preview) = ui.pictures();
    assert_eq!(original.size(), (1120, 660), "the sample decoded");
    assert_eq!(preview.size(), (1120, 660));
    // A file that is not a picture draws nothing and breaks nothing.
    assert_eq!(super::decode(b"not a png").size(), (0, 0));
    let rt = runtime("DW-0209");
    assert_eq!(info(&rt, "cmp-photo").rect[2..], [560.0, 330.0]);
    assert_eq!(
        info(&runtime("DW-0204"), "prev-thumb").label.as_deref(),
        Some("Sample photo")
    );
}

// ---------------------------------------------------------------------------
// The steps
// ---------------------------------------------------------------------------

#[test]
fn every_frame_draws_its_heading_and_its_place_in_the_steps_in_both_themes() {
    for (i, (frame, title)) in FRAMES.iter().enumerate() {
        for dark in [false, true] {
            let rt = runtime_at(scene(frame).expect("a scene"), 2.0, dark);
            let heading = info(&rt, "title");
            assert_eq!(heading.text.as_deref(), Some(*title), "{frame} (dark: {dark})");
            assert_eq!(heading.role, "heading", "{frame}");
            assert_eq!(
                count(&rt, "skip"),
                usize::from(*frame != "DW-0208"),
                "{frame}: Skip setup"
            );
        }
        let _ = i;
    }
    // Step 4 of 7: three done, one current, three ahead.
    let rt = runtime("DW-0204");
    assert_eq!(
        (0..7).filter(|i| count(&rt, &format!("step-{i}-check")) == 1).count(),
        3
    );
    assert_eq!(
        info(&rt, "step-3-row").label.as_deref(),
        Some("Step 4 of 7: Thumbnails and previews, current step")
    );
}

#[test]
fn the_amber_buttons_walk_the_seven_steps_and_call_the_engine() {
    let (ui_, log) = fixture(Script {
        explorer: Explorer::Added,
        ..Script::default()
    });
    let mut rt = runtime_at(ui_, 2.0, false);
    // Enter presses the step's amber button, seven times.
    for step in [
        Step::Lock,
        Step::Backup,
        Step::Previews,
        Step::Explorer,
        Step::Notifications,
        Step::Login,
        Step::Done,
    ] {
        press(&mut rt, Key::Enter);
        assert_eq!(ui_step(&mut rt), step);
    }
    assert_eq!(
        *log.borrow(),
        [
            "add_passkey",
            "set_app_lock true 5 min",
            "start_backup Desktop, Documents, Pictures",
            "set_previews 4K HEIC",
            "set_notifications true",
            "set_open_at_login true",
        ]
    );
    // "You're set up" says what was chosen, as the board's DW-0208 does.
    let values: Vec<String> = (0..7)
        .map(|i| info(&rt, &format!("item-{i}-value")).text.unwrap_or_default())
        .collect();
    assert_eq!(
        values,
        [
            "Added",
            "On",
            "Desktop, Documents, Pictures",
            "4K HEIC",
            "On",
            "On",
            "On"
        ]
    );
    click(&mut rt, "item-1");
    click(&mut rt, "btn-0");
    assert_eq!(
        ui(&mut rt).take_intents(),
        [Intent::OpenSetting(Setting::AppLock), Intent::OpenDrive]
    );
    assert_eq!(log.borrow().last().map(String::as_str), Some("finish"));
}

fn ui_step(rt: &mut Runtime<State>) -> Step {
    ui(rt).step
}

#[test]
fn not_now_and_skip_turn_nothing_on() {
    let (ui_, log) = fixture(Script::default());
    let mut rt = runtime_at(ui_, 2.0, false);
    click(&mut rt, "btn-1"); // Not now: no passkey
    assert_eq!(ui_step(&mut rt), Step::Lock);
    click(&mut rt, "skip");
    assert_eq!(ui_step(&mut rt), Step::Done);
    assert!(
        log.borrow().is_empty(),
        "nothing was asked of the engine: {:?}",
        log.borrow()
    );
    let values: Vec<String> = (0..3)
        .map(|i| info(&rt, &format!("item-{i}-value")).text.unwrap_or_default())
        .collect();
    assert_eq!(values, ["Not added", "Off", "Off"]);
}

#[test]
fn a_cancelled_passkey_dialog_stays_on_step_one() {
    let (ui_, _) = fixture(Script {
        passkey_added: false,
        ..Script::default()
    });
    let mut rt = runtime_at(ui_, 2.0, false);
    click(&mut rt, "btn-0");
    assert_eq!(ui_step(&mut rt), Step::Passkey);
    assert!(!ui(&mut rt).results.passkey);
}

#[test]
fn switches_and_choices_work_from_the_keyboard() {
    let mut rt = runtime("DW-0202");
    tab_to(&mut rt, "item-0-switch");
    assert_eq!(info(&rt, "item-0-switch").role, "switch");
    press(&mut rt, Key::Text(" ".into()));
    assert!(!ui(&mut rt).lock_on, "Space turns the switch off");
    click(&mut rt, "seg-0-3-tab");
    assert_eq!(ui(&mut rt).lock_after, super::source::LockAfter::Min60);
    assert_eq!(info(&rt, "seg-0-on").label.as_deref(), Some("60 min"));

    // With every folder off there is nothing to back up: the amber button takes no press.
    let mut rt = runtime("DW-0203");
    for i in 0..3 {
        click(&mut rt, &format!("item-{i}-switch"));
    }
    click(&mut rt, "btn-0");
    assert_eq!(ui_step(&mut rt), Step::Backup);
    click(&mut rt, "item-3-switch");
    click(&mut rt, "btn-0");
    assert_eq!(ui(&mut rt).results.backup, ["Music"]);
}

#[test]
fn file_explorer_failing_offers_try_again_and_a_way_on() {
    let mut rt = runtime("DW-0280");
    assert_eq!(info(&rt, "banner-0").role, "alert");
    assert_eq!(
        info(&rt, "side-status").text.as_deref(),
        Some("Couldn't add Beebeeb to File Explorer")
    );
    // Try again: the fixture's second registration works, and setup goes on at this step (DW-0206).
    click(&mut rt, "btn-0");
    assert_eq!(ui_step(&mut rt), Step::Explorer);
    assert_eq!(info(&rt, "title").text.as_deref(), Some("Beebeeb is in File Explorer"));
    let mut rt = runtime("DW-0280");
    click(&mut rt, "btn-1");
    assert_eq!(ui_step(&mut rt), Step::Notifications);
}

// ---------------------------------------------------------------------------
// The compare view
// ---------------------------------------------------------------------------

#[test]
fn compare_opens_from_step_four_and_done_or_esc_returns_with_the_choice() {
    let mut rt = runtime("DW-0204");
    click(&mut rt, "btn-0");
    assert_eq!(count(&rt, "sheet"), 1);
    assert_eq!(info(&rt, "sheet").role, "dialog");
    assert_eq!(focused(&rt).as_deref(), Some("cmp-done"), "focus moves into the view");
    // Behind the view nothing takes a press: Skip setup and the amber button do nothing.
    ui(&mut rt).skip();
    ui(&mut rt).go_on();
    assert_eq!(ui_step(&mut rt), Step::Previews);
    // A size and a format are chosen in the view.
    click(&mut rt, "cmp-opt-1-tab");
    click(&mut rt, "cmp-opt-5-tab");
    press(&mut rt, Key::Escape);
    assert_eq!(count(&rt, "sheet"), 0);
    let (size, format) = (ui(&mut rt).size, ui(&mut rt).format);
    assert_eq!((size, format), (PreviewSize::P1080, PreviewFormat::Jpeg));
    assert_eq!(focused(&rt).as_deref(), Some("btn-0"), "focus returns to Compare");
    // The board states no weight for 1080p JPEG, so none is shown (never an invented one).
    assert_eq!(count(&rt, "prev-size-bytes"), 0);
    click(&mut rt, "btn-0");
    click(&mut rt, "cmp-done");
    assert_eq!(count(&rt, "sheet"), 0);
}

#[test]
fn the_wipe_line_follows_the_arrow_keys_and_the_modes_switch() {
    let mut rt = runtime("DW-0209");
    let at = |rt: &mut Runtime<State>| {
        rt.settle_all();
        rt.frame(0.0);
        info(rt, "cmp-wipe").rect[0]
    };
    let middle = at(&mut rt);
    // The photo is a slider: with focus on it the arrow keys move the line.
    tab_to(&mut rt, "cmp-photo");
    assert_eq!(info(&rt, "cmp-photo").role, "slider");
    press(&mut rt, Key::Right);
    assert_eq!(
        ui(&mut rt).compare.map(|c| c.wipe),
        Some(0.55),
        "Right moves the line a step"
    );
    assert!(at(&mut rt) > middle, "Right moves the line right");
    for _ in 0..30 {
        press(&mut rt, Key::Left);
    }
    let photo = info(&rt, "cmp-photo").rect;
    assert!(at(&mut rt) >= photo[0] - 1.0, "the line stays on the photo");
    assert_eq!(ui(&mut rt).compare.map(|c| c.wipe), Some(0.0));
    click(&mut rt, "cmp-seg-0-tab");
    assert_eq!(ui(&mut rt).compare.map(|c| c.mode), Some(CompareMode::Stacked));
    assert_eq!(count(&rt, "cmp-wipe"), 0);
    assert_eq!(
        count(&rt, "cmp-tag-0") + count(&rt, "cmp-tag-1"),
        2,
        "both pictures, named"
    );
    click(&mut rt, "cmp-seg-2-tab");
    assert_eq!(count(&rt, "cmp-tag-1"), 1, "flip starts on the preview");
    click(&mut rt, "cmp-photo");
    assert_eq!(count(&rt, "cmp-tag-0"), 1, "a press shows the original");
}

#[test]
fn choose_photo_asks_the_engine_for_another_photo() {
    let (mut ui_, log) = fixture(Script::default());
    while ui_.step < Step::Previews {
        if ui_.step == Step::Passkey {
            ui_.add_passkey()
        } else {
            ui_.go_on()
        }
    }
    ui_.open_compare();
    let mut rt = runtime_at(ui_, 2.0, false);
    click(&mut rt, "cmp-choose");
    assert_eq!(log.borrow().last().map(String::as_str), Some("choose_photo"));
}

#[test]
fn the_fixture_holds_only_what_the_board_states() {
    let source = FixtureSetupSource::new(Script::default());
    assert_eq!(source.folders().len(), 6);
    assert_eq!(source.folders().iter().filter(|f| f.on).count(), 3);
    let known = PreviewSize::ALL
        .iter()
        .flat_map(|s| PreviewFormat::ALL.iter().map(move |f| (*s, *f)))
        .filter(|(s, f)| source.weight(*s, *f).is_some())
        .count();
    assert_eq!(known, 5, "four sizes as HEIC and JPEG at 4K: the board's five weights");
}

/// With no setup state the registered frame shows the shell's "not built yet" frame, never the
/// fixture's sample values.
#[test]
fn without_a_setup_state_no_sample_value_is_shown() {
    let mut state = init();
    crate::shell::host::dispatch(&mut state, Event::Gate(crate::shell::app::GateState::FirstRun));
    assert!(state.setup.is_none());
    let mut rt = Runtime::new(
        state,
        view,
        WINDOW,
        Env {
            system_dark: false,
            scale: 2.0,
        },
    );
    rt.frame(0.0);
    assert_eq!(count(&rt, "au-col"), 0);
    assert!(rt.elements().iter().all(|e| e.text.as_deref() != Some("Add a passkey")));
}

/// A source that answers a switch after the press: the value drawn is the one the answer names, never the one
/// that was asked for; a change that did not happen is kept as that until the next answer.
#[test]
fn an_answered_switch_draws_what_the_answer_says_not_what_was_asked() {
    use super::source::{Outcome, Switch};
    let mut ui = Ui::new(Box::new(FixtureSetupSource::new(Script::default())));
    ui.results.open_at_login = true;
    ui.results.notifications = true;
    // Asked on, and the entry is off: the change did not happen, and the row says what is set.
    ui.take_in(Outcome {
        switch: Switch::OpenAtLogin,
        done: false,
        now: Some(false),
    });
    assert!(!ui.results.open_at_login, "what the answer names");
    assert_eq!(ui.answered(Switch::OpenAtLogin), Some(Answered::NotChanged));
    // Not done, and what is set is not known: the asked value is not claimed either way.
    ui.take_in(Outcome {
        switch: Switch::Notifications,
        done: false,
        now: None,
    });
    assert_eq!(ui.answered(Switch::Notifications), Some(Answered::NotKnown));
    // The next answer for the same switch replaces the last: made, so nothing is left to say.
    ui.take_in(Outcome {
        switch: Switch::OpenAtLogin,
        done: true,
        now: Some(true),
    });
    assert!(ui.results.open_at_login);
    assert_eq!(ui.answered(Switch::OpenAtLogin), None);
    assert_eq!(
        ui.answered(Switch::Notifications),
        Some(Answered::NotKnown),
        "the other one stands"
    );
    // Made, and what is set could not be read back: it was made (the value that was asked for stands).
    ui.results.notifications = true;
    ui.take_in(Outcome {
        switch: Switch::Notifications,
        done: true,
        now: None,
    });
    assert!(ui.results.notifications);
    assert_eq!(ui.answered(Switch::Notifications), None);
    // The fixture's changes are made at once: nothing waits, and every step and row is there (the board).
    assert_eq!(ui.steps(), Step::STEPS);
    assert!(ui.opens_settings() && ui.has(Setting::Passkey));
}
