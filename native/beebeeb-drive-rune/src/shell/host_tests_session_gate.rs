//! The session gate (`shell::session_gate`), driven by the app's own input: keys and clicks in
//! the shell's runtime, the engine's gate and account, the engine's "session ended". With the
//! gate closed (locked, signed out, another account, session ended) no key and no open dialog
//! reaches a source, whether the poll has seen the change yet or not.
//!
//! How "nothing happened" is proven: the same window is run twice, once WITH the input and once
//! without, and everything observable must be equal (every source's call count, the route, what
//! is open, every text and label in the element tree). Before that, the same input under an
//! open session must change that record, or the comparison would prove nothing.
//!
//! The run without the input does not depend on the input: it is made once per (setup, way of closing, seen by
//! the poll or not) and every input of that setup is compared with it ([`Baselines`]). Each test asserts how
//! many windows it compared and how many runs without input it made.

use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use crate::frames::files::source::{BoardDrive, Call, FilesFixture};
use crate::frames::global::source::{GlobalFixture, board};
use crate::source::{DriveSource, Listing, SourceError, StorageSummary};

type Count = Arc<AtomicU32>;

/// The window's drive: the board's for the first account, one file for any other. Counts reads.
struct Drive {
    board: BoardDrive,
    engine: Arc<FixtureEngine>,
    reads: Count,
}

const FIRST_ACCOUNT: &str = "fixture-account";
const SECOND_ACCOUNT_FILE: &str = "Second account only.txt";
/// A row of the board's sample (the first account's).
const A_FILE_OF_THE_ACCOUNT: &str = "Contract draft.pdf";

impl DriveSource for Drive {
    fn list(&self, folder: Option<&str>) -> Result<Listing, SourceError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let mut listing = self.board.list(folder)?;
        if crate::shell::actions::EngineActions::session(self.engine.as_ref())
            .account
            .as_deref()
            != Some(FIRST_ACCOUNT)
        {
            listing.items.retain(|i| !i.is_folder);
            listing.items.truncate(1);
            listing.items[0].name = SECOND_ACCOUNT_FILE.to_string();
        }
        Ok(listing)
    }
    fn storage(&self) -> Option<StorageSummary> {
        None
    }
}

struct App {
    rt: Runtime<State>,
    engine: Arc<FixtureEngine>,
    /// What the Files source was asked to do.
    calls: Arc<std::sync::Mutex<Vec<Call>>>,
    /// Folders read, questions to Search, files the viewer's source was asked for.
    reads: Count,
    searches: Count,
    opened: Count,
    /// How often Search's source was told to forget what it holds.
    forgotten: Count,
}

/// The shell's window on the board's sample, every source counting, the seventh row selected.
fn app() -> App {
    let engine = Arc::new(FixtureEngine::ready());
    let mut state = state::init_with(Shell::fixture_on(engine.clone(), Arc::new(FakePlatform::default())));
    let fixture = FilesFixture::default();
    let calls = fixture.calls.clone();
    let (reads, searches, opened, forgotten) = (Count::default(), Count::default(), Count::default(), Count::default());
    state.source = Box::new(Drive {
        board: BoardDrive::sharing(&fixture.edits),
        engine: engine.clone(),
        reads: reads.clone(),
    });
    state.actions.source = Box::new(fixture);
    state.frames.find.source = Box::new(crate::frames::search::source::Counted::board(
        searches.clone(),
        forgotten.clone(),
    ));
    let asked = opened.clone();
    crate::frames::viewers::wire(
        &mut state,
        Box::new(move || {
            asked.fetch_add(1, Ordering::SeqCst);
            Box::new(crate::frames::viewers::source::FixtureViewerSource::new())
        }),
    );
    let mut rt = runtime(state);
    settle(&mut rt);
    assert_eq!(rt.state.shell.app.account(), Some(FIRST_ACCOUNT));
    let mut app = App {
        rt,
        engine,
        calls,
        reads,
        searches,
        opened,
        forgotten,
    };
    click(&mut app.rt, "row-6-date");
    app
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let e = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"));
    let (x, y) = (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn type_text(rt: &mut Runtime<State>, text: &str) {
    for c in text.chars() {
        press(rt, Key::Text(c.to_string()), Mods::default());
    }
}

fn none() -> Mods {
    Mods::default()
}

fn ctrl_shift() -> Mods {
    Mods { shift: true, ..ctrl() }
}

/// Every text and label in the element tree, leaving elements too.
fn words(rt: &Runtime<State>) -> Vec<String> {
    let mut all: Vec<String> = rt
        .elements()
        .into_iter()
        .flat_map(|e| [e.text, e.label])
        .flatten()
        .collect();
    all.sort();
    all
}

/// Everything observable of the window: what each source was asked, where the route is, what
/// is open, and every word drawn.
fn record(app: &App) -> String {
    format!("{} | {:?}", record_without_words(app), words(&app.rt))
}

/// [`record`] without the words drawn: the lock screen and sign-in take keys themselves (Enter
/// presses their own button), so there the words may differ; no file name is among them.
fn record_without_words(app: &App) -> String {
    let s = &app.rt.state;
    format!(
        "calls {:?} | reads {} | searches {} | opened {} | screen {:?} | dialog {} rename {} menu {} picker {} \
         find {} typed {:?} viewer {} new-file {}",
        app.calls.lock().unwrap(),
        app.reads.load(Ordering::SeqCst),
        app.searches.load(Ordering::SeqCst),
        app.opened.load(Ordering::SeqCst),
        s.shell.app.screen(),
        s.actions.dialog.is_some(),
        s.actions.rename.is_some(),
        s.actions.menu.is_some(),
        s.actions.picker.is_some(),
        s.frames.find.open,
        s.search.text,
        s.viewer.ui.is_some(),
        s.viewer.new_file.is_some(),
    )
}

/// The ways the session gate closes under the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Closing {
    Lock,
    SignOut,
    OtherAccount,
    /// The server no longer takes the session: the gate stays Ready (DW-1109).
    Ended,
}

const CLOSINGS: [Closing; 4] = [Closing::Lock, Closing::SignOut, Closing::OtherAccount, Closing::Ended];

/// The gate closes. No frame runs here: the poll has not seen it yet.
fn close(app: &mut App, closing: Closing) {
    match closing {
        Closing::Lock => app.engine.set_gate(GateState::Locked),
        Closing::SignOut => app.engine.set_gate(GateState::SignedOut),
        Closing::OtherAccount => app.engine.set_account("user-b"),
        // The engine's status, as the window reads it.
        Closing::Ended => app.rt.update(|s, _| {
            s.global = crate::frames::global::Ui::new(Box::new(GlobalFixture::of(board::signed_out())));
        }),
    }
}

type Step = fn(&mut App);
type Input = (&'static str, Key, Mods);

/// The window after `setup` and the gate closed by `closing`, with `input` sent or not: its record, and the
/// window. `seen`: frames ran between the change and the input. Not seen: the input is in the same frame as the
/// poll that sees the change, sent to the tree drawn before it.
fn closed_run(setup: Step, closing: Closing, seen: bool, input: Option<(&Key, Mods)>) -> (String, App) {
    let mut app = app();
    setup(&mut app);
    close(&mut app, closing);
    if seen {
        settle(&mut app.rt);
        settle(&mut app.rt);
    }
    if let Some((key, mods)) = input {
        app.rt.key(KeyEvent { key: key.clone(), mods });
    }
    settle(&mut app.rt);
    settle(&mut app.rt);
    // The gate's own screen (locked, sign-in) answers keys itself.
    let own_screen = matches!(closing, Closing::Lock | Closing::SignOut);
    let record = if own_screen {
        assert!(
            words(&app.rt).iter().all(|w| !w.contains(A_FILE_OF_THE_ACCOUNT)),
            "{closing:?}: a file name is drawn"
        );
        record_without_words(&app)
    } else {
        record(&app)
    };
    (record, app)
}

/// The runs WITHOUT an input, one per (setup, closing, seen): the record every input of that setup must leave
/// the window equal to. Keyed by the setup's own function, so two setups never share a run.
#[derive(Default)]
struct Baselines {
    without: std::collections::HashMap<(usize, Closing, bool), String>,
    /// Windows compared with their run without the input.
    compared: usize,
}

/// `input` after `setup`: under an open session it does something; with the gate closed in each
/// way, seen by the poll or not yet, the window is exactly what it is without the input.
fn refused_when_closed(runs: &mut Baselines, name: &str, setup: Step, input: &Input) {
    let (what, key, mods) = input.clone();
    let name = format!("{name}, {what}");
    // The control: the input does something while the session is open.
    let mut with = app();
    setup(&mut with);
    let before = record(&with);
    press(&mut with.rt, key.clone(), mods);
    settle(&mut with.rt);
    assert_ne!(record(&with), before, "{name}: under an open session it does nothing");

    // Another account is an open session again once the poll has seen it, and the poll runs
    // before a frame's input: that window is the next account's own (the test of the next
    // account below).
    for closing in [Closing::Lock, Closing::SignOut, Closing::Ended] {
        for seen in [false, true] {
            let without = runs
                .without
                .entry((setup as usize, closing, seen))
                .or_insert_with(|| closed_run(setup, closing, seen, None).0)
                .clone();
            let (with, app) = closed_run(setup, closing, seen, Some((&key, mods)));
            assert_eq!(with, without, "{name}: {closing:?}, seen by the poll: {seen}");
            runs.compared += 1;
            let s = &app.rt.state;
            assert!(
                s.actions.dialog.is_none() && s.actions.rename.is_none() && !s.frames.find.open,
                "{name}: {closing:?}: something stayed open"
            );
            assert!(s.search.text.is_empty() && s.actions.name_edit.text.is_empty());
        }
    }
}

fn nothing(_: &mut App) {}

/// A row was moved to Trash: Undo holds it.
fn trashed(app: &mut App) {
    press(&mut app.rt, Key::Delete, none());
}

/// Inside a folder of the board's sample: Back has somewhere to go.
fn in_a_folder(app: &mut App) {
    click(&mut app.rt, "row-3-date");
    assert_eq!(app.rt.state.listing.items[3].name, "Lisbon trip");
    press(&mut app.rt, Key::Enter, none());
    assert_eq!(app.rt.state.listing.items[0].name, "Day 1", "the folder opened");
}

/// The shortcuts of a selected row. Asked in two tests (each builds a window per case, and a window's first
/// frame is the slow part): together they are every input of this list, once.
fn on_a_row() -> [Input; 14] {
    [
        ("Enter", Key::Enter, none()),
        ("Space", Key::Text(" ".into()), none()),
        ("Delete", Key::Delete, none()),
        ("Ctrl+D", Key::Command('d'), ctrl()),
        ("F2", Key::Function(2), none()),
        ("Alt+Enter", Key::Enter, alt()),
        ("Ctrl+Shift+N", Key::Command('n'), ctrl_shift()),
        ("Ctrl+N", Key::Command('n'), ctrl()),
        ("Ctrl+O", Key::Command('o'), ctrl()),
        ("Ctrl+F", Key::Command('f'), ctrl()),
        ("Ctrl+2", Key::Command('2'), ctrl()),
        ("Ctrl+3", Key::Command('3'), ctrl()),
        ("Ctrl+4", Key::Command('4'), ctrl()),
        ("Ctrl+5", Key::Command('5'), ctrl()),
    ]
}
/// How many of [`on_a_row`] the first test asks; the second asks the rest.
const FIRST_HALF: usize = 7;

#[test]
fn with_the_session_gate_closed_no_shortcut_reaches_a_source_or_changes_the_window() {
    let mut runs = Baselines::default();
    for input in &on_a_row()[..FIRST_HALF] {
        refused_when_closed(&mut runs, "a row selected", nothing, input);
    }
    // 7 inputs, each closed 3 ways, seen by the poll or not; 1 setup, run once per way without an input.
    assert_eq!((runs.compared, runs.without.len()), (FIRST_HALF * 6, 6));
}

#[test]
fn with_the_session_gate_closed_no_other_shortcut_reaches_a_source_or_changes_the_window() {
    let mut runs = Baselines::default();
    for input in &on_a_row()[FIRST_HALF..] {
        refused_when_closed(&mut runs, "a row selected", nothing, input);
    }
    refused_when_closed(
        &mut runs,
        "after a move to Trash",
        trashed,
        &("Ctrl+Z", Key::Command('z'), ctrl()),
    );
    refused_when_closed(&mut runs, "in a folder", in_a_folder, &("Alt+Left", Key::Left, alt()));
    refused_when_closed(
        &mut runs,
        "in a folder",
        in_a_folder,
        &("Backspace", Key::Backspace, none()),
    );
    // The other 7 row inputs and 3 more, each closed 3 ways, seen by the poll or not; 3 setups.
    assert_eq!(
        (runs.compared, runs.without.len()),
        ((on_a_row().len() - FIRST_HALF + 3) * 6, 3 * 6)
    );
    assert_eq!(
        on_a_row().len(),
        14,
        "with the first test: all 14 row inputs, 17 inputs in all"
    );
}

/// A name being typed in a row.
fn renaming(app: &mut App) {
    press(&mut app.rt, Key::Function(2), none());
    type_text(&mut app.rt, "typed-under-the-session");
    assert!(app.rt.state.actions.rename.is_some());
}

fn new_folder(app: &mut App) {
    press(&mut app.rt, Key::Command('n'), ctrl_shift());
    type_text(&mut app.rt, "typed-under-the-session");
    assert!(app.rt.state.actions.dialog.is_some());
}

/// Move… picked from the file menu: the dialog on the root's folders.
fn moving(app: &mut App) {
    press(&mut app.rt, Key::ContextMenu, none());
    let item = app
        .rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.role == "menuitem" && e.label.as_deref() == Some("Move\u{2026}"))
        .expect("the file menu has Move");
    let (x, y) = (item.rect[0] + item.rect[2] / 2.0, item.rect[1] + item.rect[3] / 2.0);
    app.rt.pointer_down(x, y);
    app.rt.pointer_up(x, y);
    settle(&mut app.rt);
    assert!(app.rt.state.actions.dialog.is_some(), "Move to is open");
}

/// Search with results, the first one selected by the keyboard.
fn searching(app: &mut App) {
    press(&mut app.rt, Key::Command('f'), ctrl());
    type_text(&mut app.rt, "Contr");
    press(&mut app.rt, Key::Down, none());
    assert!(app.rt.state.frames.find.open && app.rt.state.frames.find.selected.is_some());
}

#[test]
fn what_was_open_when_the_session_gate_closed_does_not_reach_a_source() {
    let enter: Input = ("Enter", Key::Enter, none());
    let mut runs = Baselines::default();
    refused_when_closed(&mut runs, "a name being typed in a row", renaming, &enter);
    refused_when_closed(&mut runs, "New folder", new_folder, &enter);
    refused_when_closed(&mut runs, "Move to", moving, &enter);
    refused_when_closed(&mut runs, "Move to", moving, &("Right", Key::Right, none()));
    // (The arrows are not asked here: once Search has ended they are the list's own keys, and
    // selecting a row of the list the window still holds is allowed while the session ended.)
    refused_when_closed(&mut runs, "Search on a result", searching, &enter);
    // 5 inputs, each closed 3 ways, seen by the poll or not; 4 setups, each run once per way without an input.
    assert_eq!((runs.compared, runs.without.len()), (5 * 6, 4 * 6));
}

/// The typed words are in no text or label once the poll saw the gate close, in each way; the
/// session that ended keeps the rows (the board's DW-1109) and nothing else that was open.
#[test]
fn nothing_that_was_open_is_drawn_after_the_session_gate_closed() {
    for setup in [renaming as Step, new_folder, moving, searching] {
        for closing in CLOSINGS {
            let mut app = app();
            setup(&mut app);
            close(&mut app, closing);
            settle(&mut app.rt);
            settle(&mut app.rt);
            let drawn = words(&app.rt);
            assert!(
                drawn
                    .iter()
                    .all(|w| !w.contains("typed-under-the-session") && !w.contains("Contr\u{0}")),
                "{closing:?}: {drawn:?}"
            );
            let s = &app.rt.state;
            assert!(s.actions.dialog.is_none() && s.actions.rename.is_none() && !s.frames.find.open);
            assert!(app.rt.find("dialog").is_none() && app.rt.find("name-input").is_none());
            if closing == Closing::Ended {
                assert_eq!(s.listing.items.len(), 11, "the rows stay as they were");
            }
        }
    }
}

/// Another account signs in under the window: no name of the earlier account's files is in the
/// element tree, and Ctrl+Z puts nothing of the earlier account back.
#[test]
fn the_next_account_sees_nothing_of_the_earlier_one_and_undo_restores_nothing() {
    for through in [None, Some(Closing::Lock), Some(Closing::SignOut)] {
        let mut app = app();
        let earlier: Vec<String> = app.rt.state.listing.items.iter().map(|i| i.name.clone()).collect();
        assert_eq!(earlier.len(), 11);
        press(&mut app.rt, Key::Delete, none());
        assert!(matches!(app.calls.lock().unwrap()[..], [Call::Trash(_)]));
        // The control: Undo restores it while the account is still there.
        let mut control = self::app();
        press(&mut control.rt, Key::Delete, none());
        press(&mut control.rt, Key::Command('z'), ctrl());
        assert!(matches!(
            control.calls.lock().unwrap()[..],
            [Call::Trash(_), Call::Restore(_)]
        ));

        if let Some(closing) = through {
            close(&mut app, closing);
            settle(&mut app.rt);
            settle(&mut app.rt);
        }
        app.engine.set_account("user-b");
        app.engine.set_gate(GateState::Ready);
        settle(&mut app.rt);
        settle(&mut app.rt);
        assert_eq!(app.rt.state.shell.app.account(), Some("user-b"), "{through:?}");
        let names: Vec<String> = app.rt.state.listing.items.iter().map(|i| i.name.clone()).collect();
        assert_eq!(names, [SECOND_ACCOUNT_FILE], "{through:?}");
        let drawn = words(&app.rt);
        assert!(earlier.iter().any(|name| name == A_FILE_OF_THE_ACCOUNT));
        // A folder named like a section of the sidebar is a word of the window itself.
        let sections = ["Files", "Shared", "Photos", "Backups", "Trash"];
        for name in earlier.iter().filter(|name| !sections.contains(&name.as_str())) {
            assert!(
                drawn.iter().all(|w| !w.contains(name.as_str())),
                "{through:?}: {name} of the earlier account is drawn"
            );
        }
        press(&mut app.rt, Key::Command('z'), ctrl());
        assert!(
            matches!(app.calls.lock().unwrap()[..], [Call::Trash(_)]),
            "{through:?}: Undo acted on the earlier account's row"
        );
        let names: Vec<String> = app.rt.state.listing.items.iter().map(|i| i.name.clone()).collect();
        assert_eq!(names, [SECOND_ACCOUNT_FILE], "{through:?}");
    }
}

/// With the session ended the person is not held in the window: Settings and the two views of
/// this PC open, and Esc and Back return to the Files window, which reads nothing.
#[test]
fn with_the_session_ended_settings_and_this_pc_open_and_the_way_back_works() {
    let mut app = app();
    close(&mut app, Closing::Ended);
    settle(&mut app.rt);
    let (reads, calls) = (app.reads.load(Ordering::SeqCst), app.calls.lock().unwrap().len());
    let files = app.rt.state.shell.app.screen();
    press(&mut app.rt, Key::Command(','), ctrl());
    assert!(matches!(app.rt.state.shell.app.screen(), Screen::Settings(_)));
    press(&mut app.rt, Key::Escape, none());
    assert_eq!(app.rt.state.shell.app.screen(), files);
    press(&mut app.rt, Key::Command('6'), ctrl());
    assert_eq!(
        app.rt.state.shell.app.screen(),
        Screen::Drive(Route::section(Section::SyncHistory))
    );
    press(&mut app.rt, Key::Left, alt());
    assert_eq!(app.rt.state.shell.app.screen(), files, "Back returns to Files");
    assert_eq!(app.rt.state.listing.items.len(), 11);
    assert_eq!(
        (app.reads.load(Ordering::SeqCst), app.calls.lock().unwrap().len()),
        (reads, calls),
        "nothing was read or done on the way"
    );
}

/// A request from outside the window (a link, the b's menu) that names a place of the account's
/// files is not followed while the session ended; Settings is.
#[test]
fn a_request_from_outside_is_not_followed_to_account_data_while_the_session_ended() {
    // The control: the same request (the shell's entry for a link or the b) is followed while
    // the session is open.
    let mut control = app();
    control.rt.update(|s, _| {
        request(s, Intent::Go(Route::section(Section::Photos)));
    });
    settle(&mut control.rt);
    assert_eq!(
        control.rt.state.shell.app.screen(),
        Screen::Drive(Route::section(Section::Photos))
    );

    let mut app = app();
    close(&mut app, Closing::Ended);
    settle(&mut app.rt);
    let files = app.rt.state.shell.app.screen();
    let opened = app.opened.load(Ordering::SeqCst);
    app.rt.update(|s, _| {
        request(s, Intent::Go(Route::section(Section::Photos)));
    });
    settle(&mut app.rt);
    assert_eq!(app.rt.state.shell.app.screen(), files);
    assert_eq!(app.opened.load(Ordering::SeqCst), opened);
    app.rt.update(|s, tx| on_menu(s, MenuAction::Settings, tx));
    settle(&mut app.rt);
    assert!(matches!(app.rt.state.shell.app.screen(), Screen::Settings(_)));
}

/// An open file with a question over it (unsaved changes) when the account leaves: the file is
/// dropped, and the next account's window neither shows it nor asks a source for it.
#[test]
fn an_open_file_goes_with_the_account_that_leaves_also_with_a_question_open() {
    for closing in [Closing::SignOut, Closing::OtherAccount] {
        let mut app = app();
        press(&mut app.rt, Key::Enter, none());
        assert!(app.rt.state.viewer.ui.is_some(), "the file opened in the window");
        let opened = app.opened.load(Ordering::SeqCst);
        assert!(opened >= 1);
        // The state the editor is in while its "unsaved changes" question shows.
        app.rt.update(|s, _| {
            s.viewer.ui.as_mut().unwrap().dialog = Some(crate::frames::viewers::Dialog::Unsaved);
        });
        settle(&mut app.rt);
        let name = app.rt.state.viewer.ui.as_ref().unwrap().file.name.clone();

        close(&mut app, closing);
        settle(&mut app.rt);
        settle(&mut app.rt);
        assert!(app.rt.state.viewer.ui.is_none(), "{closing:?}: the open file was kept");
        app.engine.set_account("user-b");
        app.engine.set_gate(GateState::Ready);
        settle(&mut app.rt);
        settle(&mut app.rt);
        assert_eq!(app.rt.state.shell.app.account(), Some("user-b"));
        assert!(app.rt.state.viewer.ui.is_none(), "{closing:?}");
        assert_eq!(
            app.rt.state.shell.app.screen(),
            Screen::Drive(Route::section(Section::Files)),
            "{closing:?}: the route names no file"
        );
        assert_eq!(
            app.opened.load(Ordering::SeqCst),
            opened,
            "{closing:?}: a source was asked"
        );
        assert!(words(&app.rt).iter().all(|w| !w.contains(name.as_str())), "{closing:?}");
    }
}

/// A lock, then a sign-out: the account left, so what it was uploading is forgotten (once), as
/// after a sign-out from an open window.
#[test]
fn a_sign_out_after_a_lock_still_forgets_the_uploads() {
    let mut app = app();
    // What a drop on the list calls.
    app.rt
        .update(|s, tx| assert!(s.upload(None, &["a.heic".into(), "b.heic".into()], tx)));
    settle(&mut app.rt);
    assert!(app.rt.state.actions.source.uploads().is_some());
    close(&mut app, Closing::Lock);
    settle(&mut app.rt);
    settle(&mut app.rt);
    assert!(
        app.rt.state.actions.source.uploads().is_some(),
        "a lock cancels nothing"
    );
    close(&mut app, Closing::SignOut);
    settle(&mut app.rt);
    settle(&mut app.rt);
    let forgotten = app
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| **c == Call::ForgetUploads)
        .count();
    assert_eq!(forgotten, 1);
    assert!(app.rt.state.actions.source.uploads().is_none());
}

/// A gate that closed reads no folder: the window holds the empty root until it opens again.
#[test]
fn a_gate_that_closes_reads_no_folder() {
    for closing in [Closing::Lock, Closing::SignOut] {
        let mut app = app();
        let reads = app.reads.load(Ordering::SeqCst);
        close(&mut app, closing);
        settle(&mut app.rt);
        settle(&mut app.rt);
        assert_eq!(app.reads.load(Ordering::SeqCst), reads, "{closing:?}");
        assert!(app.rt.state.listing.items.is_empty(), "{closing:?}");
    }
}

/// Search open with results when the session gate closes, in each way: its source is told to
/// forget what it holds, and from then on it is not told search opened, not asked a query and
/// not pumped for answers, however many frames run.
#[test]
fn search_forgets_and_is_not_asked_again_once_the_session_gate_closed() {
    for closing in [Closing::Lock, Closing::SignOut, Closing::Ended] {
        let mut app = app();
        searching(&mut app);
        let before = app.searches.load(Ordering::SeqCst);
        // The control: while the session is open, a frame asks the source (its poll pumps it).
        settle(&mut app.rt);
        assert!(
            app.searches.load(Ordering::SeqCst) > before,
            "an open search asks its source"
        );
        assert_eq!(app.forgotten.load(Ordering::SeqCst), 0);

        close(&mut app, closing);
        settle(&mut app.rt);
        assert!(
            app.forgotten.load(Ordering::SeqCst) >= 1,
            "{closing:?}: nothing was forgotten"
        );
        let asked = app.searches.load(Ordering::SeqCst);
        for _ in 0..4 {
            settle(&mut app.rt);
        }
        assert_eq!(
            app.searches.load(Ordering::SeqCst),
            asked,
            "{closing:?}: asked after it closed"
        );
    }
}
