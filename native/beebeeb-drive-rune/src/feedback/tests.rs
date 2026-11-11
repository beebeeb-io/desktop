//! Scripted, headless: the real `State`, the real hooked `view` / `on_key` / `poll`, the same
//! renderer the window draws with.

use std::path::{Path, PathBuf};

use rune::{AppSpec, Cx, El, Env, Key, KeyEvent, Mods, Runtime, TextEdit, col, text, text_input};
use serde_json::Value;

use super::model::{self, BLANK_RGBA, NOTE_SCHEMA, REDACTED, Rgba, Status, StatusState};
use super::{Feedback, Mode, capture, paths};
use crate::State;
use crate::source::{DriveSource, LargeFixtureSource};

const SIZE: (f32, f32) = (1280.0, 800.0);
const SCALE: f32 = 2.0;

fn temp_inbox(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bb-feedback-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("inbox")
}

fn text_system() -> rune::text::TextSystem {
    rune::text::TextSystem::new(crate::FONTS, ("Inter", "JetBrains Mono"))
}

fn runtime_with(inbox: &Path, dark: bool, change: impl FnOnce(&mut State)) -> Runtime<State> {
    let spec = super::hook_spec(AppSpec {
        on_key: Some(crate::on_key),
        ..AppSpec::new("test", SIZE, crate::init, crate::view)
    });
    let mut state = crate::init();
    state.feedback = Feedback::with_inbox(Some(inbox.to_path_buf()));
    change(&mut state);
    let mut rt = Runtime::with_text(
        state,
        spec.view,
        SIZE,
        Env {
            system_dark: dark,
            scale: SCALE,
        },
        text_system(),
    );
    rt.on_key = spec.on_key;
    rt.poll = spec.poll;
    step(&mut rt);
    rt
}

fn runtime(inbox: &Path) -> Runtime<State> {
    runtime_with(inbox, false, |_| {})
}

/// One frame as the window's compositor runs it: frame, pick, capture when due, redraw.
fn step_by(rt: &mut Runtime<State>, dt: f64) {
    rt.frame(dt);
    let mut again = super::sync(rt);
    if super::capture_due(&rt.state) {
        let img = capture::render_headless(rt, SCALE);
        super::finish_capture(rt, img);
        again = true;
    }
    if again {
        rt.invalidate();
        rt.frame(0.0);
        if super::sync(rt) {
            rt.invalidate();
            rt.frame(0.0);
        }
    }
}

fn step(rt: &mut Runtime<State>) {
    step_by(rt, 1.0 / 60.0);
}

fn key(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    step(rt);
}

fn plain(rt: &mut Runtime<State>, k: Key) {
    key(rt, k, Mods::default());
}

fn hover(rt: &mut Runtime<State>, target: &str) -> (f32, f32) {
    let at = rt.find(target).unwrap_or_else(|| panic!("no element keyed {target}"));
    rt.pointer_move(at.0, at.1);
    step(rt);
    at
}

fn click(rt: &mut Runtime<State>, at: (f32, f32)) {
    rt.pointer_down(at.0, at.1);
    rt.pointer_up(at.0, at.1);
    step(rt);
}

fn has_key(rt: &Runtime<State>, k: &str) -> bool {
    rt.elements().iter().any(|e| e.key.as_deref() == Some(k) && !e.exiting)
}

fn shown_text(rt: &Runtime<State>) -> String {
    rt.elements()
        .iter()
        .filter_map(|e| e.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.51)
}

/// F8 → hover a row's name → Alt+Up to the row → click → type → Enter. Returns the note's folder.
fn save_note_on_row(rt: &mut Runtime<State>, row: &str, comment: &str) -> PathBuf {
    plain(rt, Key::Function(8));
    let at = hover(rt, &format!("{row}-name"));
    while rt.state.feedback.target().and_then(|t| t.key.as_deref()) != Some(row) {
        let before = rt.state.feedback.level;
        key(
            rt,
            Key::Up,
            Mods {
                alt: true,
                ..Mods::default()
            },
        );
        assert_ne!(rt.state.feedback.level, before, "{row} is an ancestor of {row}-name");
    }
    click(rt, at);
    assert_eq!(rt.state.feedback.mode, Mode::Writing);
    plain(rt, Key::Text(comment.to_string()));
    plain(rt, Key::Enter);
    assert_eq!(
        rt.state.feedback.mode,
        Mode::Picking,
        "saved, still in comment mode: {:?}",
        rt.state.feedback.message
    );
    let newest = rt.state.feedback.notes.first().expect("a note is listed").dir.clone();
    plain(rt, Key::Escape);
    newest
}

#[test]
fn hotkey_enters_and_leaves_comment_mode() {
    let inbox = temp_inbox("hotkey");
    let mut rt = runtime(&inbox);
    assert_eq!(rt.state.feedback.mode, Mode::Off);
    assert!(!has_key(&rt, "fb-overlay"));
    assert!(
        has_key(&rt, "fb-entry"),
        "the keyboard-reachable Feedback entry is there"
    );

    plain(&mut rt, Key::Function(8));
    assert_eq!(rt.state.feedback.mode, Mode::Picking);
    assert!(has_key(&rt, "fb-overlay") && has_key(&rt, "fb-hint"));
    plain(&mut rt, Key::Escape);
    assert_eq!(rt.state.feedback.mode, Mode::Off);
    assert!(!has_key(&rt, "fb-overlay"));

    // Ctrl+Shift+/ arrives as Command('?') on a US layout and Command('/') elsewhere.
    for c in ['?', '/'] {
        let chord = Mods {
            cmd: true,
            ctrl: true,
            shift: true,
            ..Mods::default()
        };
        key(&mut rt, Key::Command(c), chord);
        assert_eq!(rt.state.feedback.mode, Mode::Picking, "Ctrl+Shift+{c}");
        key(&mut rt, Key::Command(c), chord);
        assert_eq!(rt.state.feedback.mode, Mode::Off);
    }
    // Ctrl+/ without Shift is the board's own shortcut: not ours.
    key(
        &mut rt,
        Key::Command('/'),
        Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    );
    assert_eq!(rt.state.feedback.mode, Mode::Off);

    // Comment mode owns the keyboard: Down does not move the app's selection under it.
    plain(&mut rt, Key::Function(8));
    let selected = rt.state.selected;
    plain(&mut rt, Key::Down);
    assert_eq!(rt.state.selected, selected);
}

#[test]
fn hover_outlines_the_element_under_the_pointer_and_alt_up_steps_to_its_row() {
    let inbox = temp_inbox("hover");
    let mut rt = runtime(&inbox);
    plain(&mut rt, Key::Function(8));
    let at = hover(&mut rt, "row-3-name");

    let fb = &rt.state.feedback;
    let leaf = fb.target().expect("something is outlined").clone();
    let [x, y, w, h] = leaf.visible;
    assert!(
        x <= at.0 && at.0 <= x + w && y <= at.1 && at.1 <= y + h,
        "the outlined box holds the pointer"
    );
    assert!(
        fb.chain.iter().any(|e| e.key.as_deref() == Some("row-3-name")),
        "row-3-name is the leaf or an ancestor"
    );
    assert!(
        fb.chain.iter().all(|e| !e.path.contains("fb-overlay")),
        "only the app's own elements are picked"
    );
    let (drawn, _) = rt.presented("fb-highlight").expect("the outline is drawn");
    assert!(
        close(drawn, leaf.visible),
        "outline {drawn:?} sits on the element {:?}",
        leaf.visible
    );
    let last = super::app_path(&leaf.path).rsplit('/').next().unwrap().to_string();
    let chip = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("fb-path-text"))
        .and_then(|e| e.text)
        .expect("the key path chip");
    assert!(
        chip.ends_with(&last),
        "the chip {chip:?} ends with the element's own key {last:?}"
    );

    // Alt+Up: parent by parent until the row itself.
    let mut steps = 0;
    while rt.state.feedback.target().unwrap().key.as_deref() != Some("row-3") {
        key(
            &mut rt,
            Key::Up,
            Mods {
                alt: true,
                ..Mods::default()
            },
        );
        steps += 1;
        assert!(steps < 10, "row-3 is an ancestor");
    }
    assert!(steps >= 1);
    let (row, _) = rt.presented("row-3").unwrap();
    let (drawn, _) = rt.presented("fb-highlight").unwrap();
    assert!(close(drawn, row), "outline {drawn:?} is the row {row:?}");
    // Alt+Down: back toward the leaf.
    key(
        &mut rt,
        Key::Down,
        Mods {
            alt: true,
            ..Mods::default()
        },
    );
    assert_eq!(rt.state.feedback.level, steps - 1);

    // Another element: the outline follows and the stepping starts over.
    hover(&mut rt, "new-btn");
    assert_eq!(rt.state.feedback.level, 0);
    assert!(
        rt.state
            .feedback
            .chain
            .iter()
            .any(|e| e.key.as_deref() == Some("new-btn"))
    );
}

#[test]
fn saving_a_note_writes_four_files_that_match_the_window() {
    let inbox = temp_inbox("save");
    let mut rt = runtime(&inbox);
    plain(&mut rt, Key::Function(8));
    let at = hover(&mut rt, "row-6-name");
    while rt.state.feedback.target().unwrap().key.as_deref() != Some("row-6") {
        key(
            &mut rt,
            Key::Up,
            Mods {
                alt: true,
                ..Mods::default()
            },
        );
    }
    click(&mut rt, at);
    assert_eq!(rt.state.feedback.mode, Mode::Writing);
    assert!(has_key(&rt, "fb-popover"));

    // Enter with nothing typed saves nothing.
    plain(&mut rt, Key::Enter);
    assert_eq!(rt.state.feedback.mode, Mode::Writing);
    assert!(model::scan(&inbox).is_empty());

    plain(&mut rt, Key::Text("Row is too tall".into()));
    key(
        &mut rt,
        Key::Enter,
        Mods {
            shift: true,
            ..Mods::default()
        },
    );
    plain(&mut rt, Key::Text("second line".into()));
    assert_eq!(
        rt.state.feedback.mode,
        Mode::Writing,
        "Shift+Enter is a new line, not a save"
    );
    let tag = rt.find("fb-tag-0").unwrap();
    click(&mut rt, tag);
    let field = rt.find(super::ui::NOTE_INPUT).unwrap();
    click(&mut rt, field);
    plain(&mut rt, Key::Enter);
    assert_eq!(rt.state.feedback.mode, Mode::Picking, "{:?}", rt.state.feedback.message);

    let notes = model::scan(&inbox);
    assert_eq!(notes.len(), 1);
    let dir = &notes[0].dir;
    let leftovers: Vec<_> = std::fs::read_dir(&inbox)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
        .collect();
    assert!(leftovers.is_empty(), "no temp folder is left: {leftovers:?}");

    let note: Value = serde_json::from_slice(&std::fs::read(dir.join("note.json")).unwrap()).unwrap();
    assert_eq!(note["schema"], NOTE_SCHEMA);
    assert_eq!(note["id"].as_str().unwrap(), dir.file_name().unwrap().to_str().unwrap());
    assert_eq!(note["comment"], "Row is too tall\nsecond line");
    assert_eq!(note["tags"], serde_json::json!(["Looks wrong"]));
    assert_eq!(note["element"]["key"], "row-6");
    assert!(note["element"]["key_path"].as_str().unwrap().ends_with("row-6"));
    assert!(
        !note["element"]["key_path"].as_str().unwrap().contains("fb-root"),
        "paths start at the app's root"
    );
    assert_eq!(note["app"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(note["app"]["commit"].is_string() && note["app"]["build_kind"].is_string());
    assert_eq!(note["platform"]["scale"], 2.0);
    assert_eq!(note["platform"]["theme"], "light");
    assert_eq!(
        note["platform"]["window_size"],
        serde_json::json!({"w": 1280.0, "h": 800.0})
    );
    assert_eq!(note["frame"]["id"], "DW-0401");
    assert_eq!(note["frame"]["state"]["selected_name"], "Contract draft.pdf");
    assert_eq!(note["frame"]["state"]["item_count"], 11);
    assert!(chrono::DateTime::parse_from_rfc3339(note["created_at"].as_str().unwrap()).is_ok());

    let window = Rgba::read_png(&dir.join(note["files"]["window_png"].as_str().unwrap())).unwrap();
    let element = Rgba::read_png(&dir.join(note["files"]["element_png"].as_str().unwrap())).unwrap();
    assert_eq!((window.w, window.h), (2560, 1600), "the window at 2x");
    let b = &note["element"]["box"];
    let (bw, bh) = (b["w"].as_f64().unwrap(), b["h"].as_f64().unwrap());
    assert_eq!(
        (element.w, element.h),
        ((bw * 2.0).round() as u32, (bh * 2.0).round() as u32),
        "the crop is the box times the scale"
    );
    let distinct = |img: &Rgba| img.px.chunks_exact(4).collect::<std::collections::HashSet<_>>().len();
    assert!(
        distinct(&window) > 50 && distinct(&element) > 5,
        "neither picture is blank"
    );
    let px_box = window.px_box(
        [
            b["x"].as_f64().unwrap() as f32,
            b["y"].as_f64().unwrap() as f32,
            bw as f32,
            bh as f32,
        ],
        2.0,
    );
    assert_eq!(window.crop(px_box), element, "element.png is that box of window.png");

    // The picture is the app alone: where the hint bar and the popover were drawn, it is
    // pixel for pixel the app rendered without the feedback wrapper.
    let mut bare = Runtime::with_text(
        crate::init(),
        crate::view,
        SIZE,
        Env {
            system_dark: false,
            scale: SCALE,
        },
        text_system(),
    );
    bare.frame(0.0);
    let bare = capture::render_headless(&mut bare, SCALE);
    let diff: Vec<(u32, u32)> = window
        .px
        .chunks_exact(4)
        .zip(bare.px.chunks_exact(4))
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| (i as u32 % window.w, i as u32 / window.w))
        .collect();
    let bbox = (
        diff.iter().map(|p| p.0).min(),
        diff.iter().map(|p| p.1).min(),
        diff.iter().map(|p| p.0).max(),
        diff.iter().map(|p| p.1).max(),
    );
    assert!(
        diff.is_empty(),
        "the capture is the bare app, pixel for pixel: {} px differ inside {bbox:?}",
        diff.len()
    );

    let tree: Value = serde_json::from_slice(&std::fs::read(dir.join("tree.json")).unwrap()).unwrap();
    assert_eq!(tree["element"]["key"], "row-6");
    let up: Vec<&str> = tree["ancestors"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["key"].as_str())
        .collect();
    assert!(
        up.contains(&"file-list") && up.contains(&"window"),
        "ancestors up to the app's root: {up:?}"
    );
    let sub = tree["subtree"].as_array().unwrap();
    assert!(
        sub.iter().any(|e| e["key"] == "row-6-name"),
        "the subtree has the row's name cell"
    );
    assert!(sub.iter().any(|e| e["text"] == "Contract draft.pdf"), "and its text");
    assert!(
        sub.iter()
            .all(|e| e["box"]["w"].is_number() && e["fill"].is_array() && e["role"].is_string())
    );
    assert!(
        !serde_json::to_string(&tree).unwrap().contains("fb-"),
        "no feedback chrome in the tree"
    );
}

#[test]
fn the_mode_does_not_care_which_source_feeds_the_list() {
    let inbox = temp_inbox("large");
    let mut rt = runtime_with(&inbox, true, |s| {
        let source = Box::new(LargeFixtureSource::new(5000));
        s.listing = source.list(None);
        s.source = source;
        s.selected = None;
        s.show_detail_pane = false;
    });
    let dir = save_note_on_row(&mut rt, "row-2", "from the 5,000-item source");
    let note: Value = serde_json::from_slice(&std::fs::read(dir.join("note.json")).unwrap()).unwrap();
    assert_eq!(note["frame"]["state"]["item_count"], 5000);
    assert_eq!(note["platform"]["theme"], "dark");
    assert_eq!(note["element"]["key"], "row-2");
}

struct Secret {
    password: TextEdit,
    turned: bool,
}

fn secret_view(s: &Secret, _: &mut Cx<Secret>) -> El {
    let field = text_input(&s.password, rune::TextStyle::new(14.0, 400.0))
        .key("password")
        .label("Password")
        .secure()
        .revealed(true)
        .color(rune::Color::BLACK)
        .w(300.0);
    col()
        .key("window")
        .w_full()
        .h_full()
        .fill(rune::Color::WHITE)
        .padding(20.0)
        .gap(12.0)
        .child(text("Unlock your vault").key("title"))
        .child(if s.turned { field.scale(1.5) } else { field })
}

fn secret_runtime(turned: bool) -> Runtime<Secret> {
    let state = Secret {
        password: TextEdit::new("hunter2-recovery-phrase"),
        turned,
    };
    let mut rt = Runtime::with_text(
        state,
        secret_view,
        (400.0, 200.0),
        Env {
            system_dark: false,
            scale: SCALE,
        },
        text_system(),
    );
    rt.frame(0.0);
    rt
}

#[test]
fn a_secure_field_is_redacted_in_the_tree_and_blanked_in_both_pngs() {
    let mut rt = secret_runtime(false);
    let (tree, rigid) = capture::snapshot(&rt);
    assert!(rigid);
    let field = model::index_of(
        &tree,
        &tree.iter().find(|e| e.key.as_deref() == Some("password")).unwrap().path,
    )
    .unwrap();
    assert!(tree[field].secure && tree[field].text.is_none());
    assert_eq!(
        tree.iter().filter(|e| e.secure).count(),
        1,
        "only the password field is secure"
    );

    let mut img = capture::render_headless(&mut rt, SCALE);
    let area = img.px_box(tree[field].visible, SCALE);
    assert!(area[2] > 0 && area[3] > 0);
    // The check can fail: before redaction the revealed characters are in the picture.
    assert!(!img.is_filled(area, BLANK_RGBA));
    let ink_before = img.crop(area).px.chunks_exact(4).filter(|p| p[0] < 128).count();
    assert!(
        ink_before > 20,
        "the revealed password is drawn: {ink_before} dark pixels"
    );
    // A tree that does not mark the field leaves it readable: the marking is what redacts.
    let unmarked: Vec<_> = tree
        .iter()
        .cloned()
        .map(|mut e| {
            e.secure = false;
            e
        })
        .collect();
    let mut untouched = img.clone();
    assert_eq!(model::redact_pixels(&mut untouched, &unmarked, SCALE, true), Ok(0));
    assert_eq!(untouched, img);

    assert_eq!(model::redact_pixels(&mut img, &tree, SCALE, rigid), Ok(1));
    assert!(img.is_filled(area, BLANK_RGBA), "the field's box is one flat colour");

    let inbox = temp_inbox("secure");
    let root = tree.iter().position(|e| e.key.as_deref() == Some("window")).unwrap();
    let meta = model::NoteMeta {
        comment: "field looks off".into(),
        tags: vec![],
        scale: SCALE,
        dark: false,
        window_size: [400.0, 200.0],
        frame: serde_json::json!({"id": "test"}),
    };
    for picked in [root, field] {
        let id = model::write_note(&inbox, &meta, &tree, picked, &img).unwrap();
        let dir = inbox.join(id);
        for file in ["note.json", "tree.json"] {
            let body = std::fs::read_to_string(dir.join(file)).unwrap();
            assert!(!body.contains("hunter2"), "{file} never holds the secret");
            if picked == field || file == "tree.json" {
                assert!(body.contains(REDACTED), "{file} says the text was redacted");
            }
        }
        let element = Rgba::read_png(&dir.join("element.png")).unwrap();
        if picked == field {
            assert!(
                element.is_filled([0, 0, element.w, element.h], BLANK_RGBA),
                "a secure element's own crop is blank"
            );
        }
        let window = Rgba::read_png(&dir.join("window.png")).unwrap();
        assert!(window.is_filled(area, BLANK_RGBA));
    }
}

#[test]
fn a_secure_field_that_cannot_be_blanked_reliably_refuses_the_note() {
    let mut rt = secret_runtime(true);
    let (tree, rigid) = capture::snapshot(&rt);
    assert!(!rigid, "a scaled secure field has no box to trust");
    let mut img = capture::render_headless(&mut rt, SCALE);
    assert!(model::redact_pixels(&mut img, &tree, SCALE, rigid).is_err());
}

#[test]
fn the_panel_shows_a_status_and_reply_the_lead_wrote_but_looks_no_faster_than_2s() {
    let inbox = temp_inbox("status");
    let mut rt = runtime(&inbox);
    let dir = save_note_on_row(&mut rt, "row-1", "Folder icon is off-centre");

    key(
        &mut rt,
        Key::Function(8),
        Mods {
            shift: true,
            ..Mods::default()
        },
    );
    assert!(rt.state.feedback.panel_open && has_key(&rt, "fb-panel"));
    let shown = shown_text(&rt);
    assert!(
        shown.contains("Folder icon is off-centre") && shown.contains("New"),
        "{shown}"
    );
    assert!(shown.contains("stay on this PC"), "the privacy line is in the panel");
    assert!(shown.contains(&inbox.display().to_string()) || has_key(&rt, "fb-inbox-path"));

    std::fs::write(
        dir.join("status.json"),
        r#"{"state":"fixed","reply":"Centred, thanks.","fixed_in_commit":"0123456789abcdef","updated_at":"2026-11-11T12:00:00Z","by":"lead"}"#,
    )
    .unwrap();
    for _ in 0..30 {
        step(&mut rt);
    }
    assert!(
        !shown_text(&rt).contains("Fixed in"),
        "half a second later the folder has not been read again"
    );
    for _ in 0..100 {
        step(&mut rt);
    }
    let shown = shown_text(&rt);
    assert!(shown.contains("Fixed in 012345678"), "{shown}");
    assert!(shown.contains("Centred, thanks.") && shown.contains("Reply from lead"));
    assert_eq!(rt.state.feedback.notes[0].status.state, StatusState::Fixed);

    // Open the note: its screenshot and its folder path.
    let row = rt.find(&format!("fb-note-{}", rt.state.feedback.notes[0].id)).unwrap();
    click(&mut rt, row);
    assert!(has_key(&rt, "fb-detail-shot") && has_key(&rt, "fb-copy-path"));
    assert!(
        rt.state.feedback.opened_image.is_some(),
        "window.png was decoded for the panel"
    );
    assert!(shown_text(&rt).contains(&dir.display().to_string()));
    plain(&mut rt, Key::Escape);
    assert!(!rt.state.feedback.panel_open);
}

#[test]
fn status_json_parses_every_state_and_ignores_garbage() {
    for (name, state) in [
        ("new", StatusState::New),
        ("seen", StatusState::Seen),
        ("fixing", StatusState::Fixing),
        ("fixed", StatusState::Fixed),
        ("wontfix", StatusState::WontFix),
    ] {
        assert_eq!(
            Status::parse(&format!(r#"{{"state":"{name}","reply":null}}"#))
                .unwrap()
                .state,
            state
        );
    }
    assert!(Status::parse(r#"{"state":"done"}"#).is_none());
    assert!(Status::parse("{").is_none());
    assert_eq!(Status::parse(r#"{"state":"wontfix"}"#).unwrap().chip(), "Won't fix");
    assert_eq!(Status::parse(r#"{"state":"fixing"}"#).unwrap().chip(), "Being fixed");
}

#[test]
fn the_inbox_is_the_flag_then_the_env_then_the_apps_data_folder() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |k: &str| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| std::ffi::OsString::from(v))
        }
    };
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let base: &[(&str, &str)] = &[("HOME", "/home/t"), ("LOCALAPPDATA", "C:\\Users\\t\\AppData\\Local")];
    let both: &[(&str, &str)] = &[
        ("HOME", "/home/t"),
        ("LOCALAPPDATA", "C:\\Users\\t\\AppData\\Local"),
        (paths::DIR_ENV, "/env/inbox"),
    ];
    let test_id: &[(&str, &str)] = &[
        ("HOME", "/home/t"),
        ("LOCALAPPDATA", "C:\\Users\\t\\AppData\\Local"),
        (paths::APP_ID_ENV, "io.beebeeb.app.test"),
    ];
    assert_eq!(
        paths::inbox_from(&args(&["app", paths::DIR_FLAG, "/flag/inbox"]), env(both)),
        Some(PathBuf::from("/flag/inbox"))
    );
    assert_eq!(
        paths::inbox_from(&args(&["app"]), env(both)),
        Some(PathBuf::from("/env/inbox"))
    );
    let default = paths::inbox_from(&args(&["app"]), env(base)).unwrap();
    assert!(
        default.ends_with("feedback/inbox") && default.to_string_lossy().contains(paths::DEFAULT_APP_ID),
        "{default:?}"
    );
    let test = paths::inbox_from(&args(&["app"]), env(test_id)).unwrap();
    assert!(
        test.to_string_lossy().contains("io.beebeeb.app.test"),
        "the test build's own id: {test:?}"
    );
}

/// Notes never leave this PC: the module has no networking API in it at all.
#[test]
fn the_module_has_no_network_code() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/feedback");
    let banned: Vec<String> = [
        ["std::", "net"],
        ["Tcp", "Stream"],
        ["Tcp", "Listener"],
        ["Udp", "Socket"],
        ["req", "west"],
        ["hy", "per"],
        ["ure", "q::"],
        ["http", "://"],
        ["https", "://"],
        ["tok", "io::net"],
        ["open", "_url"],
        ["Socket", "Addr"],
        ["web", "socket"],
        ["cu", "rl"],
    ]
    .iter()
    .map(|p| p.concat())
    .collect();
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        for b in &banned {
            assert!(
                !src.to_lowercase().contains(&b.to_lowercase()),
                "{} mentions {b}",
                path.display()
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 6, "mod, model, capture, paths, ui, tests");
    // The check can fail: the same scan finds a socket in a line that has one.
    let mutant = format!("let s = {}::connect(addr);", banned[1]);
    assert!(banned.iter().any(|b| mutant.contains(b.as_str())));
}

/// Three pictures for the PR, rendered by the same path as the tests above. Written to
/// `FEEDBACK_EVIDENCE_DIR` when it is set, else to a temp folder.
#[test]
fn evidence_pngs() {
    let out = std::env::var_os("FEEDBACK_EVIDENCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| temp_inbox("evidence").with_file_name("evidence"));
    std::fs::create_dir_all(&out).unwrap();
    let inbox = temp_inbox("evidence");
    let mut rt = runtime(&inbox);
    let shot = |rt: &mut Runtime<State>, name: &str| {
        // Past the entrance fades: the picture is the mode at rest.
        for _ in 0..45 {
            step(rt);
        }
        let img = capture::render_headless(rt, SCALE);
        img.write_png(&out.join(name)).unwrap();
        assert_eq!((img.w, img.h), (2560, 1600));
    };

    // 1. Comment mode with a row outlined.
    plain(&mut rt, Key::Function(8));
    let at = hover(&mut rt, "row-7-name");
    while rt.state.feedback.target().unwrap().key.as_deref() != Some("row-7") {
        key(
            &mut rt,
            Key::Up,
            Mods {
                alt: true,
                ..Mods::default()
            },
        );
    }
    shot(&mut rt, "01-comment-mode-row-outlined.png");

    // 2. The note popover.
    click(&mut rt, at);
    plain(
        &mut rt,
        Key::Text("The date column is not aligned with its header.".into()),
    );
    let tag = rt.find("fb-tag-0").unwrap();
    click(&mut rt, tag);
    shot(&mut rt, "02-note-popover.png");
    let field = rt.find(super::ui::NOTE_INPUT).unwrap();
    click(&mut rt, field);
    plain(&mut rt, Key::Enter);
    assert_eq!(rt.state.feedback.mode, Mode::Picking, "{:?}", rt.state.feedback.message);
    plain(&mut rt, Key::Escape);

    // 3. The notes panel with that note answered "fixed".
    let dir = rt.state.feedback.notes[0].dir.clone();
    std::fs::write(
        dir.join("status.json"),
        r#"{"state":"fixed","reply":"Right-aligned the dates under the header. In the next test build.","fixed_in_commit":"4f1c2ab9d0e7","updated_at":"2026-11-11T12:00:00Z","by":"lead"}"#,
    )
    .unwrap();
    key(
        &mut rt,
        Key::Function(8),
        Mods {
            shift: true,
            ..Mods::default()
        },
    );
    step(&mut rt);
    assert!(shown_text(&rt).contains("Fixed in 4f1c2ab9d"));
    shot(&mut rt, "03-notes-panel-fixed.png");
    eprintln!("evidence: 3 PNGs in {}", out.display());
}
