//! Tests of the session that ended (DW-1109): it is not a sign-out, the list stays as it was,
//! and everything that needs the session is refused before any source is asked.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use super::source::{BoardDrive, FilesFixture};
use super::tests::*;
use super::{Intent, MenuKind};
use crate::State;
use crate::frames::global::source::board;
use crate::frames::global::{GlobalFixture, Ui};
use crate::shell::actions::{EngineActions, FixtureEngine};
use crate::shell::app::{GateState, Screen};
use crate::shell::host::{self, Shell};
use crate::shell::platform::fake::FakePlatform;
use crate::shell::registry;
use crate::source::{DriveSource, Listing, SourceError, StorageSummary};
use rune::{Env, Key, Mods, Runtime};

/// The board's drive, counting every folder it is asked to read.
struct Counting(BoardDrive, Arc<AtomicU32>);

impl DriveSource for Counting {
    fn list(&self, folder: Option<&str>) -> Result<Listing, SourceError> {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0.list(folder)
    }
    fn storage(&self) -> Option<StorageSummary> {
        self.0.storage()
    }
}

struct App {
    rt: Runtime<State>,
    calls: Calls,
    reads: Arc<AtomicU32>,
    engine: Arc<FixtureEngine>,
}

/// The app as it runs (the shell's view, keys and poll) on the fixture engine. `ended`: the
/// engine says the session ended.
fn app(ended: bool) -> App {
    let engine = Arc::new(FixtureEngine::ready());
    let mut state =
        crate::shell::state::init_with(Shell::fixture_on(engine.clone(), Arc::new(FakePlatform::default())));
    let fixture = FilesFixture::default();
    let calls = fixture.calls.clone();
    let reads = Arc::new(AtomicU32::new(0));
    state.source = Box::new(Counting(BoardDrive::sharing(&fixture.edits), reads.clone()));
    state.actions.source = Box::new(fixture);
    if ended {
        state.global = Ui::new(Box::new(GlobalFixture::of(board::signed_out())));
    }
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (1278.0, 798.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    settle(&mut rt);
    click(&mut rt, "row-6-date");
    reads.store(0, Ordering::SeqCst);
    App {
        rt,
        calls,
        reads,
        engine,
    }
}

fn texts(rt: &Runtime<State>) -> Vec<String> {
    rt.elements()
        .into_iter()
        .flat_map(|e| [e.text, e.label])
        .flatten()
        .collect()
}

fn shows(rt: &Runtime<State>, words: &str) -> bool {
    texts(rt).iter().any(|t| t.contains(words))
}

#[test]
fn the_session_that_ended_keeps_the_list_and_a_sign_out_shows_none_of_it() {
    // The session ended: the window as it was, and what stopped.
    let ended = app(true);
    assert!(
        matches!(ended.rt.state.shell.app.screen(), Screen::Drive(_)),
        "the gate stays open"
    );
    assert_eq!(ended.rt.state.listing.items.len(), 11);
    assert!(shows(&ended.rt, "Contract draft.pdf") && shows(&ended.rt, "11 items"));
    assert_eq!(
        info(&ended.rt, "banner").label.as_deref(),
        Some("Backup and sync have stopped. Enter your password to continue. Nothing has been lost.")
    );
    let sign_in = info(&ended.rt, "banner-action");
    assert_eq!(
        (sign_in.role, sign_in.label.as_deref(), sign_in.focusable),
        ("button", Some("Sign in"), true)
    );
    assert_eq!(info(&ended.rt, "status-text").text.as_deref(), Some("Signed out"));
    assert_eq!(
        info(&ended.rt, "status-details").text.as_deref(),
        Some("Nothing syncs until you sign in")
    );
    // Showing it read nothing again.
    assert_eq!(ended.reads.load(Ordering::SeqCst), 0);
    assert!(asked(&ended.calls).is_empty());
    // No secret is on this screen: it is not one the capture exclusion covers.
    assert!(!registry::route_shows_secret(&ended.rt.state.shell.app));
    assert!(!registry::group_for(&ended.rt.state.shell.app).shows_secret());

    // A real sign-out: the gate closes, the sign-in screen shows, nothing of the account is drawn.
    let mut out = app(false);
    assert!(shows(&out.rt, "Contract draft.pdf"));
    out.engine.set_gate(GateState::SignedOut);
    settle(&mut out.rt);
    assert_eq!(out.rt.state.shell.app.screen(), Screen::SignIn);
    for words in ["Contract draft.pdf", "Lisbon trip", "11 items", "Budget 2026.xlsx"] {
        assert!(!shows(&out.rt, words), "{words} is still drawn after a sign-out");
    }
    assert!(find(&out.rt, "sidebar").is_none() && find(&out.rt, "row-0").is_none());
}

/// Every way to ask for something that needs the session: each is refused and counted, the
/// keyboard goes to Sign in where it can, and no source hears of it.
#[test]
fn everything_that_needs_the_session_is_refused_and_no_source_is_asked() {
    type Try = fn(&mut Runtime<State>);
    fn on_state(rt: &mut Runtime<State>, f: fn(&mut State, &mut rune::Tx) -> bool) {
        let mut tx = rune::Tx::default();
        f(&mut rt.state, &mut tx);
    }
    fn pick(rt: &mut Runtime<State>, kind: MenuKind, id: &str) {
        if kind == MenuKind::File {
            rt.state.open_file_menu((400.0, 300.0));
        } else {
            rt.state.toggle_menu(kind);
        }
        let entries = rt.state.menu_entries(kind);
        let index = entries
            .iter()
            .position(|e| e.id() == Some(id))
            .unwrap_or_else(|| panic!("no menu entry {id}"));
        let mut tx = rune::Tx::default();
        rt.state.pick_menu(index, &mut tx);
    }
    const SHIFT_CTRL: Mods = Mods { shift: true, ..CTRL };
    let ways: Vec<(&str, Try)> = vec![
        ("open a file (Enter)", |rt| press(rt, Key::Enter, NONE)),
        ("open a folder (Enter)", |rt| {
            click(rt, "row-3-date");
            press(rt, Key::Enter, NONE);
        }),
        ("preview (Space)", |rt| press(rt, Key::Text(" ".into()), NONE)),
        ("delete (Delete)", |rt| press(rt, Key::Delete, NONE)),
        ("rename (F2)", |rt| press(rt, Key::Function(2), NONE)),
        ("duplicate (Ctrl+D)", |rt| press(rt, Key::Command('d'), CTRL)),
        ("undo (Ctrl+Z)", |rt| press(rt, Key::Command('z'), CTRL)),
        ("details (Alt+Enter)", |rt| press(rt, Key::Enter, ALT)),
        ("new folder (Ctrl+Shift+N)", |rt| {
            press(rt, Key::Command('n'), SHIFT_CTRL)
        }),
        ("new file (Ctrl+N)", |rt| press(rt, Key::Command('n'), CTRL)),
        ("upload (Ctrl+O)", |rt| press(rt, Key::Command('o'), CTRL)),
        ("menu: Open", |rt| pick(rt, MenuKind::File, "open")),
        ("menu: Preview", |rt| pick(rt, MenuKind::File, "preview")),
        ("menu: Share", |rt| pick(rt, MenuKind::File, "share")),
        ("menu: Copy link", |rt| pick(rt, MenuKind::File, "copy-link")),
        ("menu: Make available offline (download)", |rt| {
            pick(rt, MenuKind::File, "keep-offline")
        }),
        ("menu: Show in File Explorer", |rt| {
            pick(rt, MenuKind::File, "show-in-explorer")
        }),
        ("menu: Details", |rt| pick(rt, MenuKind::File, "details")),
        ("menu: Rename", |rt| pick(rt, MenuKind::File, "rename")),
        ("menu: Move", |rt| pick(rt, MenuKind::File, "move")),
        ("menu: Duplicate", |rt| pick(rt, MenuKind::File, "duplicate")),
        ("menu: Version history", |rt| pick(rt, MenuKind::File, "versions")),
        ("menu: Move to Trash", |rt| pick(rt, MenuKind::File, "trash")),
        ("New: New folder", |rt| pick(rt, MenuKind::New, "new-folder")),
        ("New: Upload files", |rt| pick(rt, MenuKind::New, "upload-files")),
        ("New: Upload a folder", |rt| pick(rt, MenuKind::New, "upload-folder")),
        ("free up space", |rt| on_state(rt, State::free_up_space)),
        ("the toolbar's Open in File Explorer", |rt| click(rt, "open-explorer")),
        ("a drop on the list", |rt| {
            let (x, y) = centre(rt, "rows");
            rt.drop_files(vec![std::path::PathBuf::from("a.txt")], Some((x, y)));
        }),
        ("a drop on a folder row", |rt| {
            let (x, y) = centre(rt, "row-1-date");
            rt.drop_files(vec![std::path::PathBuf::from("a.txt")], Some((x, y)));
        }),
        ("the sidebar's Backups (a folder to read)", |rt| {
            click(rt, "Backups-row")
        }),
    ];
    let mut refused = 0;
    for (name, way) in &ways {
        let mut app = app(true);
        way(&mut app.rt);
        settle(&mut app.rt);
        let s = &app.rt.state;
        assert!(asked(&app.calls).is_empty(), "{name}: {:?}", asked(&app.calls));
        assert_eq!(app.reads.load(Ordering::SeqCst), 0, "{name}: a folder was read");
        assert!(s.actions.session.asked >= 1, "{name}: not counted as refused");
        assert_eq!(s.listing.items.len(), 11, "{name}");
        assert_eq!(s.listing.folder.id, None, "{name}");
        assert!(s.actions.dialog.is_none() && s.actions.rename.is_none(), "{name}");
        assert!(s.actions.toast.is_none(), "{name}: {:?}", s.actions.toast);
        assert!(s.actions.intents.is_empty(), "{name}: {:?}", s.actions.intents);
        assert!(s.global.blocked_preview.is_none(), "{name}");
        assert!(
            find(&app.rt, "banner-action").is_some(),
            "{name}: the ask is still there"
        );
        refused += 1;
    }
    assert_eq!(refused, 31);
    // The same ways with a session: the first one does reach the source (the guard is what refuses).
    let mut with = app(false);
    press(&mut with.rt, Key::Enter, NONE);
    assert_eq!(asked(&with.calls).len(), 1);
    assert_eq!(with.rt.state.actions.session.asked, 0);
}

#[test]
fn a_refused_key_puts_the_keyboard_on_sign_in_and_sorting_stays_the_windows() {
    let mut app = app(true);
    press(&mut app.rt, Key::Delete, NONE);
    assert_eq!(focused(&app.rt).as_deref(), Some("banner-action"));
    // Sorting the rows that are here needs no session, and tells the source nothing.
    let entries = app.rt.state.menu_entries(MenuKind::Sort);
    app.rt.state.toggle_menu(MenuKind::Sort);
    let size = entries.iter().position(|e| e.id() == Some("sort-size")).unwrap();
    let mut tx = rune::Tx::default();
    app.rt.state.pick_menu(size, &mut tx);
    assert!(app.rt.state.actions.sort.by == super::SortBy::Size);
    assert!(asked(&app.calls).is_empty(), "{:?}", asked(&app.calls));
    assert_eq!(app.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn sign_in_asks_the_engine_and_the_sign_in_screen_shows() {
    let mut app = app(true);
    click(&mut app.rt, "banner-action");
    settle(&mut app.rt);
    assert_eq!(app.rt.state.actions.intents, [Intent::Frame("DW-A03")]);
    assert_eq!(
        app.engine.session().gate,
        GateState::SignedOut,
        "the engine closed its gate"
    );
    assert_eq!(app.rt.state.shell.app.screen(), Screen::SignIn);
    assert!(
        !shows(&app.rt, "Contract draft.pdf"),
        "the sign-in screen shows no file"
    );
    assert!(asked(&app.calls).is_empty());
}

#[test]
fn the_other_answer_is_one_switch_no_row_name_or_count_is_drawn() {
    let mut app = app(true);
    assert!(super::session::KEEP_THE_LIST, "the board's answer is the default");
    assert!(shows(&app.rt, "Contract draft.pdf"));
    app.rt.update(|s, _| s.actions.session.keep_the_list = Some(false));
    settle(&mut app.rt);
    let names: Vec<String> = BoardDrive::new()
        .list(None)
        .unwrap()
        .items
        .into_iter()
        .map(|i| i.name)
        // "Backups" is also the sidebar's own word; its row is checked by key below.
        .filter(|name| name != "Backups")
        .collect();
    assert_eq!(names.len(), 10);
    for name in &names {
        assert!(!shows(&app.rt, name), "{name} is still in the element tree");
    }
    assert!(!shows(&app.rt, "11 items") && find(&app.rt, "row-0").is_none());
    assert!(find(&app.rt, "banner-action").is_some(), "the ask stays");
    // The rows never left this PC: with the session back they are shown again, unread.
    app.rt.update(|s, _| s.global = Ui::default());
    settle(&mut app.rt);
    assert_eq!(app.rt.state.listing.items.len(), 11);
    assert_eq!(app.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn the_scene_is_the_state_the_engine_reports() {
    let mut state = crate::init();
    (super::session::SCENES[0].1)(&mut state);
    assert!(state.session_ended());
    assert_eq!(state.listing.items.len(), 11);
    assert!(state.selected.is_none() && !state.show_detail_pane);
    assert!(!crate::init().session_ended(), "a window starts with a valid session");
}
