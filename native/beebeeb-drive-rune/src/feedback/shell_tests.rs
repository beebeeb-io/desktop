//! The feedback mode on the shell: the frame and route a note names come from the route machine,
//! comment mode is on only where the frame registry's line allows it, the hotkeys do not take
//! the shell's shortcuts (or the other way round), and the inbox is the test identity's.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use rune::{Key, KeyEvent, Mods};
use serde_json::Value;

use super::tests::*;
use super::{Mode, Policy, frame, model, paths};
use crate::shell::app::{AppMachine, Dialog, Event, GateState, Route, Screen, Section, SettingsPane};
use crate::shell::host;
use crate::shell::identity::AppIdentity;
use crate::shell::instance::{self, FEEDBACK_DIR_FLAG};
use crate::shell::registry::{self, FrameGroup};

fn machine(events: impl IntoIterator<Item = Event>) -> AppMachine {
    let mut app = AppMachine::new(GateState::Ready);
    for event in events {
        app.apply(event);
    }
    app
}

fn section(section: Section) -> Event {
    Event::Navigate(Route::section(section))
}

/// Every state of the route machine the shell can show, with the frame id and route a note
/// would name and whether comment mode may run there. Files is the ONE allowed frame.
#[test]
fn every_screen_reports_its_frame_and_only_files_allows_comment_mode() {
    let rows: Vec<(AppMachine, &str, &str, bool)> = vec![
        (machine([]), "DW-0401", "files", true),
        (machine([section(Section::Shared)]), "DW-0501", "shared", false),
        (machine([section(Section::Photos)]), "DW-0601", "photos", false),
        (machine([section(Section::Backups)]), "DW-0737", "backups", false),
        (machine([section(Section::Trash)]), "DW-0747", "trash", false),
        (
            machine([section(Section::SyncHistory)]),
            "DW-1301",
            "sync-history",
            false,
        ),
        (
            machine([section(Section::AvailableOffline)]),
            "no board frame (Available offline)",
            "available-offline",
            false,
        ),
        (
            machine([Event::Navigate(Route::folder(Section::Files, "f-1"))]),
            "DW-0401",
            "files/folder",
            true,
        ),
        (machine([Event::OpenItem("i-1".into())]), "DW-0408", "files/item", false),
        (
            machine([Event::OpenSettings(SettingsPane::Security)]),
            "DW-0701",
            "settings/security",
            false,
        ),
        (
            machine([Event::OpenDialog(Dialog::Shortcuts)]),
            "DW-1232",
            "files#dialog=shortcuts",
            false,
        ),
        (machine([Event::Gate(GateState::SignedOut)]), "DW-A01", "sign-in", false),
        (
            machine([Event::Gate(GateState::FirstRun)]),
            "DW-0201",
            "gate/first-run",
            false,
        ),
        (
            machine([Event::Gate(GateState::Locked)]),
            "DW-0009",
            "gate/locked",
            false,
        ),
        (
            machine([Event::Gate(GateState::NeedsRecoveryPhrase)]),
            "DW-A23",
            "gate/recovery-phrase",
            false,
        ),
        (
            machine([Event::Gate(GateState::NoPlan)]),
            "DW-0014",
            "gate/no-plan",
            false,
        ),
    ];
    let mut allowed = 0;
    for (app, id, route, allow) in &rows {
        let info = frame::on_screen(app);
        assert_eq!(
            (info.id.as_str(), info.route.as_str()),
            (*id, *route),
            "{:?}",
            app.screen()
        );
        assert_eq!(
            info.feedback == Policy::Allowed,
            *allow,
            "{id} {route}: {:?}",
            info.feedback
        );
        allowed += usize::from(info.feedback == Policy::Allowed);
    }
    assert_eq!(
        (rows.len(), allowed),
        (16, 2),
        "Files and a folder of Files, nothing else"
    );
    // No id or name of a folder or a file is in the route.
    assert!(rows.iter().all(|(app, ..)| {
        let route = frame::route(app);
        !route.contains("f-1") && !route.contains("i-1")
    }));
}

/// The frames that take a password, a recovery phrase or a code say so, and are denied without
/// any registry line. Every registered line is read: the count below is the registry's.
#[test]
fn sign_in_and_first_run_are_denied_with_their_reason_and_the_registry_is_default_deny() {
    for (gate, word) in [
        (GateState::SignedOut, "password"),
        (GateState::FirstRun, "recovery phrase"),
        (GateState::Locked, "password"),
        (GateState::NeedsRecoveryPhrase, "recovery phrase"),
    ] {
        let app = machine([Event::Gate(gate)]);
        match frame::on_screen(&app).feedback {
            Policy::Denied(why) => assert!(why.contains(word), "{:?}: {why}", app.screen()),
            Policy::Allowed => panic!("{:?} allows comment mode", app.screen()),
        }
    }
    assert!(matches!(
        machine([Event::Gate(GateState::SignedOut)]).screen(),
        Screen::SignIn
    ));

    let allowed: Vec<FrameGroup> = registry::FRAMES
        .iter()
        .filter(|frame| frame.feedback == Policy::Allowed)
        .map(|frame| frame.group)
        .collect();
    assert_eq!(
        allowed,
        [FrameGroup::Section(Section::Files)],
        "a group that opts in adds itself here, with its secret-redaction test"
    );
    assert_eq!(
        registry::DIALOGS
            .iter()
            .filter(|d| d.feedback == Policy::Allowed)
            .count(),
        0
    );
    // No line: denied, whatever the group. A line decides for itself.
    assert_eq!(
        frame::policy(FrameGroup::Section(Section::Shared), None),
        Policy::Denied("this screen has not opted in")
    );
    assert_eq!(
        frame::policy(FrameGroup::SignIn, Some(Policy::Denied("its own words"))),
        Policy::Denied("its own words")
    );
    // A dialog in front: both must allow it.
    let no = Policy::Denied("no");
    assert_eq!(frame::both(Policy::Allowed, None), Policy::Allowed);
    assert_eq!(frame::both(Policy::Allowed, Some(Policy::Allowed)), Policy::Allowed);
    assert_eq!(frame::both(Policy::Allowed, Some(no)), no);
    assert_eq!(frame::both(no, Some(Policy::Allowed)), no);
    assert_eq!(frame::both(no, None), no);
}

/// The note names the frame the route machine is on, and following the route out of Files ends
/// comment mode with the visible refusal.
#[test]
fn the_note_names_the_route_machines_frame_and_leaving_files_ends_comment_mode() {
    let inbox = temp_inbox("shell-frame");
    let mut rt = runtime(&inbox);
    let dir = save_note_on_row(&mut rt, "row-4", "which frame");
    let note: Value = serde_json::from_slice(&std::fs::read(dir.join("note.json")).unwrap()).unwrap();
    assert_eq!(note["frame"]["id"], "DW-0401");
    assert_eq!(note["frame"]["route"], "files");
    assert_eq!(note["frame"]["state"]["group"], "Files");
    assert_eq!(note["app"]["id"], AppIdentity::CURRENT.bundle_identifier());

    plain(&mut rt, Key::Function(8));
    assert_eq!(rt.state.feedback.mode, Mode::Picking);
    host::dispatch(&mut rt.state, section(Section::Shared));
    step(&mut rt);
    assert_eq!(rt.state.feedback.mode, Mode::Off, "Shared has not opted in");
    assert!(shown_text(&rt).contains(super::DENIED_LINE));
    plain(&mut rt, Key::Function(8));
    assert_eq!(rt.state.feedback.mode, Mode::Off);
    assert_eq!(model::scan(&inbox, &[]).len(), 1);

    // Sign-in: the refusal carries the reason.
    host::dispatch(&mut rt.state, Event::Gate(GateState::SignedOut));
    step(&mut rt);
    plain(&mut rt, Key::Function(8));
    assert_eq!(rt.state.feedback.mode, Mode::Off);
    let shown = shown_text(&rt);
    assert!(
        shown.contains(super::DENIED_LINE) && shown.contains("password"),
        "{shown}"
    );
}

fn mods(shift: bool, cmd: bool, alt: bool) -> Mods {
    Mods {
        shift,
        cmd,
        alt,
        ctrl: cmd,
        ..Mods::default()
    }
}

/// The two hotkeys never reach the shell's handlers, and no shortcut of the shell is taken by
/// the feedback mode while it is off: each still does what it does without the feature.
#[test]
fn the_hotkeys_and_the_shells_shortcuts_do_not_collide() {
    let inbox = temp_inbox("shell-keys");
    let mut rt = runtime(&inbox);

    // F8 and Shift+F8 are consumed before the shell (the handler below must never run).
    fn never(_: &mut crate::State, ev: &KeyEvent, _: &mut rune::Tx) -> bool {
        panic!("{:?} reached the app", ev.key)
    }
    fn feedback_then_never(s: &mut crate::State, ev: &KeyEvent, tx: &mut rune::Tx) -> bool {
        super::on_key(s, ev, tx, never)
    }
    let shell = rt.on_key.replace(feedback_then_never);
    for shift in [false, true, false] {
        key(&mut rt, Key::Function(8), mods(shift, false, false));
    }
    rt.on_key = shell;
    rt.state.feedback.leave();
    rt.state.feedback.panel_open = false;
    step(&mut rt);

    // The shell's shortcuts, with the feedback mode off: Ctrl+, Ctrl+/ Esc Alt+Left Alt+Right.
    key(&mut rt, Key::Command(','), mods(false, true, false));
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::General));
    key(&mut rt, Key::Escape, Mods::default());
    assert!(matches!(rt.state.shell.app.screen(), Screen::Drive(_)));
    key(&mut rt, Key::Command('/'), mods(false, true, false));
    assert_eq!(rt.state.shell.app.dialog(), Some(&Dialog::Shortcuts));
    key(&mut rt, Key::Escape, Mods::default());
    assert_eq!(rt.state.shell.app.dialog(), None);
    host::dispatch(&mut rt.state, section(Section::Shared));
    step(&mut rt);
    key(&mut rt, Key::Left, mods(false, false, true));
    assert_eq!(rt.state.shell.app.route().section, Section::Files);
    key(&mut rt, Key::Right, mods(false, false, true));
    assert_eq!(rt.state.shell.app.route().section, Section::Shared);
    key(&mut rt, Key::Left, mods(false, false, true));

    // Comment mode on: it owns the keyboard, so none of them moves the app under it.
    plain(&mut rt, Key::Function(8));
    assert_eq!(rt.state.feedback.mode, Mode::Picking);
    key(&mut rt, Key::Command(','), mods(false, true, false));
    key(&mut rt, Key::Command('/'), mods(false, true, false));
    key(&mut rt, Key::Right, mods(false, false, true));
    assert_eq!(rt.state.feedback.mode, Mode::Picking);
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Drive(Route::section(Section::Files))
    );
    assert_eq!(rt.state.shell.app.dialog(), None);
    // Esc leaves comment mode and nothing else.
    plain(&mut rt, Key::Escape);
    assert_eq!(rt.state.feedback.mode, Mode::Off);
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Drive(Route::section(Section::Files))
    );
}

fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
    move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| OsString::from(v))
}

/// The inbox is the flag, then the environment, then the TEST identity's data folder: for the
/// real identity too, so no build writes a note into the real app's folder by default.
#[test]
fn the_inbox_is_the_flag_then_the_env_then_the_test_identitys_folder() {
    let base: &[(&str, &str)] = &[("HOME", "/data/t"), ("LOCALAPPDATA", "C:\\home\\t\\AppData\\Local")];
    let both: &[(&str, &str)] = &[
        ("HOME", "/data/t"),
        ("LOCALAPPDATA", "C:\\home\\t\\AppData\\Local"),
        (paths::DIR_ENV, "/env/inbox"),
    ];
    assert_eq!(
        paths::inbox_from(Some(Path::new("/flag/inbox")), AppIdentity::Test, env(both)),
        Some(PathBuf::from("/flag/inbox"))
    );
    assert_eq!(
        paths::inbox_from(None, AppIdentity::Test, env(both)),
        Some(PathBuf::from("/env/inbox"))
    );
    let mut folders = Vec::new();
    for identity in AppIdentity::BOTH {
        let inbox = paths::inbox_from(None, identity, env(base)).unwrap();
        assert!(inbox.ends_with("feedback/inbox"), "{inbox:?}");
        let parts: Vec<String> = inbox.to_string_lossy().split(['/', '\\']).map(str::to_string).collect();
        assert!(
            parts.iter().any(|p| p == AppIdentity::Test.state_dir_id()),
            "{identity:?}: the test identity's folder: {inbox:?}"
        );
        assert!(
            !parts.iter().any(|p| p == AppIdentity::Real.state_dir_id()),
            "{identity:?}: never the real app's folder: {inbox:?}"
        );
        folders.push(inbox);
    }
    assert_eq!(folders[0], folders[1]);
    assert_ne!(AppIdentity::Real.state_dir_id(), AppIdentity::Real.feedback_dir_id());
    // A test never looks at a real folder.
    assert_eq!(paths::inbox(), None);
}

/// `--feedback-dir` is a development flag: a dev build reads it, a shipped build ignores it,
/// and a command line that carries a link honours nothing else.
#[test]
fn feedback_dir_is_read_only_by_a_dev_build_and_never_beside_a_link() {
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let dev = instance::parse_args(args(&[FEEDBACK_DIR_FLAG, "/tmp/inbox"]), true);
    assert_eq!(dev.feedback_dir, Some(PathBuf::from("/tmp/inbox")));
    assert!(!dev.tooling && dev.rune_args.is_empty(), "it is not a renderer's flag");
    let shipped = instance::parse_args(args(&[FEEDBACK_DIR_FLAG, "/tmp/inbox"]), false);
    assert_eq!(shipped.feedback_dir, None);
    assert!(shipped.rune_args.is_empty());
    let link = instance::parse_args(
        args(&[FEEDBACK_DIR_FLAG, "/tmp/inbox", "--open-link", "beebeeb://open"]),
        true,
    );
    assert_eq!(link.feedback_dir, None);
    let no_path = instance::parse_args(args(&[FEEDBACK_DIR_FLAG, "--autostart"]), true);
    assert_eq!((no_path.feedback_dir, no_path.autostart), (None, false));
    let installer = instance::parse_args(args(&[FEEDBACK_DIR_FLAG, "/tmp/inbox", "--quit-for-update"]), true);
    assert_eq!(installer.feedback_dir, None);
}
