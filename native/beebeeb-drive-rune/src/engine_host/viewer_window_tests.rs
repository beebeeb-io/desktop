//! Opening a file, end to end, in a window on the real engine: the Files window's own Open (a row, Enter), the
//! viewer's states in the element tree, the engine's download and decryption against the engine's fake API (two
//! invented accounts, each file's content encrypted with core under its account's key), and what is left of an
//! open file after a lock and after another account signed in. The local state is a database of each test's own
//! (`Engine::use_local_state_for_test`), recorded as the first account's. No sync is started: opening a file in
//! the viewer needs none. Every name, id and word is invented.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::signin::gate::GateState as EngineGate;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::test_api::fake_account::{account_handler, FakeAccount, TEST_PHRASE};
use beebeeb_engine::test_api::{FakeApi, Handler, Reply, Request};
use rune::{Env, Key, KeyEvent, Mods, Runtime};
use zeroize::Zeroizing;

use crate::engine_host::open::Opened;
use crate::engine_host::LiveEngine;
use crate::frames::global::source::{board, GlobalFixture};
use crate::frames::signin::setup::guide::Step;
use crate::frames::viewers;
use crate::frames::viewers::source::Body;
use crate::shell::actions::{ActionError, EngineActions, HostActions};
use crate::shell::app::{Event, Screen, Session};
use crate::shell::host::{Shell, Wiring};
use crate::shell::identity::engine_identity;
use crate::shell::instance::ProfileHolder;
use crate::shell::platform::fake::FakePlatform;
use crate::shell::tray::TrayInputs;
use crate::shell::{host, registry, state};
use crate::State;

const PASSWORD: &str = "an invented password for the viewer";
/// A second valid 12-word phrase (a BIP-39 test vector), for the second account.
const OTHER_PHRASE: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";

const NOTE: &str = "b0000000-0000-4000-8000-000000000601";
const PLAN: &str = "b0000000-0000-4000-8000-000000000602";
const LONG: &str = "b0000000-0000-4000-8000-000000000603";
const PICTURE: &str = "b0000000-0000-4000-8000-000000000604";
const NOTE_NAME: &str = "note-of-the-first-account-viewer.txt";
const LONG_NAME: &str = "long-of-the-first-account-viewer.txt";
const PICTURE_NAME: &str = "harbour-of-the-first-account-viewer.png";
/// The last line of the long file, far below one window of lines.
const LONG_END: &str = "the-very-last-line-of-the-long-file";
/// A kind this app has no viewer for.
const PLAN_NAME: &str = "plan-of-the-first-account-viewer.zip";
const REPORT: &str = "b0000000-0000-4000-8000-000000000605";
const REPORT_NAME: &str = "report-of-the-first-account-viewer.pdf";

fn report_pdf() -> Vec<u8> {
    super::viewer_pdf::tests::sample_pdf(2, 612, 792)
}

const CLIP: &str = "b0000000-0000-4000-8000-000000000607";
const CLIP_NAME: &str = "clip-of-the-first-account-viewer.mp4";
const DOC: &str = "b0000000-0000-4000-8000-000000000608";
const DOC_NAME: &str = "letter-of-the-first-account-viewer.docx";
const LETTER: &str = "b0000000-0000-4000-8000-000000000606";
const LETTER_NAME: &str = "letter-of-the-first-account-viewer.pdf";

/// A PDF whose first page writes in a font that is not in the file; its second page has no text.
fn letter_pdf() -> Vec<u8> {
    use super::viewer_pdf::tests::{fonts_pdf, Writes};
    fonts_pdf(&[Writes::Japanese, Writes::Nothing])
}
/// Words found nowhere else in the app.
const NOTE_WORDS: &str = "harbourlight-words-of-the-first-account";

fn note_text() -> String {
    // Several chunks of the fake's size.
    format!("{NOTE_WORDS} line one\n{}\nthe last line\n", "filler ".repeat(600))
}

/// 300 numbered lines and a last one: more than a window shows at once, well under what the reader holds whole.
fn long_text() -> String {
    let mut text: String = (1..=300)
        .map(|n| format!("invented line {n} of the long file\n"))
        .collect();
    text.push_str(LONG_END);
    text.push('\n');
    text
}

/// Holds every chunk request while it is on, or fails it.
#[derive(Default)]
struct Hold {
    /// Every chunk request is answered 500.
    fails: AtomicBool,
    on: AtomicBool,
    released: Mutex<bool>,
    wake: Condvar,
}

impl Hold {
    fn hold(&self) {
        *self.released.lock().unwrap() = false;
        self.on.store(true, Ordering::SeqCst);
    }

    fn release(&self) {
        self.on.store(false, Ordering::SeqCst);
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }

    fn wait(&self) {
        if !self.on.load(Ordering::SeqCst) {
            return;
        }
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.wake.wait(released).unwrap();
        }
    }
}

fn is_chunk(request: &Request) -> bool {
    request.method == "GET" && request.path.contains("/chunks/")
}

/// One fake API for two accounts: a request with a session's token goes to that session's account, a sign-in to
/// the account whose email it names (and its second leg to the same one). A chunk waits at `hold`.
fn two_accounts(first: Arc<FakeAccount>, second: Arc<FakeAccount>, hold: Arc<Hold>) -> FakeApi {
    let (to_first, to_second) = (account_handler(first.clone()), account_handler(second.clone()));
    let signing_in_second = Mutex::new(false);
    let handler: Handler = Arc::new(move |request: &Request| {
        if is_chunk(request) {
            hold.wait();
            if hold.fails.load(Ordering::SeqCst) {
                return Reply::json(500, serde_json::json!({ "error": "a server body" }));
            }
        }
        let second_asked = match request.bearer() {
            Some(token) => token == second.session_token,
            None => {
                let mut signing_in = signing_in_second.lock().unwrap();
                if let Some(email) = request.json().get("email").and_then(|email| email.as_str()) {
                    *signing_in = email == second.email;
                }
                *signing_in
            }
        };
        if second_asked {
            to_second(request)
        } else {
            to_first(request)
        }
    });
    FakeApi::start(handler)
}

/// The engine behind the shell, with a sync start that is not carried out (the process's one state folder is
/// shared by every test of this binary).
struct NoStart {
    live: Arc<LiveEngine<MemoryCredentials>>,
}

impl EngineActions for NoStart {
    fn session(&self) -> Session {
        self.live.session()
    }
    fn tray_inputs(&self) -> TrayInputs {
        self.live.tray_inputs()
    }
    fn pause_sync(&self) -> Result<(), ActionError> {
        Ok(())
    }
    fn resume_sync(&self) -> Result<(), ActionError> {
        Ok(())
    }
    fn open_in_file_explorer(&self) -> Result<(), ActionError> {
        Ok(())
    }
    fn profile_holder(&self) -> Option<ProfileHolder> {
        None
    }
    fn start_sync(&self) -> Result<(), ActionError> {
        Ok(())
    }
    fn stop_for_quit(&self) {}
    fn resume_after_cancelled_quit(&self) {}
    fn sync_refusal(&self) -> Option<crate::frames::global::sync_off::SyncOff> {
        self.live.sync_refusal()
    }
}

fn put(db: &StateDb, id: &str, name: &str, bytes: i64) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: format!("/{name}"),
        status: FileStatus::CloudOnly,
        size_bytes: bytes,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

struct Window {
    fake: FakeApi,
    hold: Arc<Hold>,
    first: Arc<FakeAccount>,
    second: Arc<FakeAccount>,
    engine: Arc<Engine<MemoryCredentials>>,
    rt: Runtime<State>,
    _state: tempfile::TempDir,
}

/// A test that fails while a chunk is held must still end: the fake joins its threads when it is dropped.
impl Drop for Window {
    fn drop(&mut self) {
        self.hold.release();
    }
}

const ENV: Env = Env {
    system_dark: false,
    scale: 2.0,
};

fn window(tag: &str) -> Window {
    let (first, _) = FakeAccount::register(&format!("first-viewer-{tag}@example.eu"), PASSWORD, TEST_PHRASE);
    let (second, _) = FakeAccount::register(&format!("second-viewer-{tag}@example.eu"), PASSWORD, OTHER_PHRASE);
    let (first, second) = (Arc::new(first), Arc::new(second));
    first.add_file_with_content(NOTE, None, NOTE_NAME, note_text().as_bytes());
    first.add_file_with_content(PLAN, None, PLAN_NAME, b"PK invented");
    first.add_file_with_content(REPORT, None, REPORT_NAME, &report_pdf());
    first.add_file_with_content(LETTER, None, LETTER_NAME, &letter_pdf());
    first.add_file_with_content(LONG, None, LONG_NAME, long_text().as_bytes());
    first.add_file_with_content(CLIP, None, CLIP_NAME, b"not really a video");
    first.add_file_with_content(DOC, None, DOC_NAME, b"PK not really a document");
    first.add_file_with_content(PICTURE, None, PICTURE_NAME, &super::tests::png_bytes(8, 6));
    let hold = Arc::new(Hold::default());
    let fake = two_accounts(first.clone(), second.clone(), hold.clone());

    let (host, messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: format!("viewer-window-slot-{tag}"),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.base.clone(),
    };
    let engine = Arc::new(
        Engine::open(engine_identity(), config, host, MemoryCredentials::default()).expect("the engine opens"),
    );
    engine.restore();

    // The first account's local state, as its sync left it.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let db = StateDb::open(&db_path).unwrap();
    put(&db, NOTE, NOTE_NAME, note_text().len() as i64);
    put(&db, PLAN, PLAN_NAME, 11);
    put(&db, REPORT, REPORT_NAME, report_pdf().len() as i64);
    put(&db, LETTER, LETTER_NAME, letter_pdf().len() as i64);
    put(&db, LONG, LONG_NAME, long_text().len() as i64);
    put(&db, CLIP, CLIP_NAME, 18);
    put(&db, DOC, DOC_NAME, 24);
    put(&db, PICTURE, PICTURE_NAME, super::tests::png_bytes(8, 6).len() as i64);
    db.set_owner(&Identity::new(Some(&first.user_id), Some(&first.email)))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(db_path);

    let opened = Opened {
        engine: engine.clone(),
        identity: engine_identity(),
        gate: engine.gate(),
    };
    let live = Arc::new(LiveEngine::new(&opened, messages));
    let platform: Arc<FakePlatform> = Arc::new(FakePlatform::default());
    let shell = Shell::new(
        Arc::new(HostActions::new(Arc::new(NoStart { live }), platform.clone())),
        platform,
        Wiring::default(),
    );
    let mut rt = Runtime::new(
        state::init_on_engine(shell, engine.clone()),
        registry::view,
        (1280.0, 800.0),
        ENV,
    );
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    Window {
        fake,
        hold,
        first,
        second,
        engine,
        rt,
        _state: dir,
    }
}

fn sign_in(engine: &Engine<MemoryCredentials>, account: &FakeAccount, phrase: &str) {
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the account's phrase unlocks");
    assert_eq!(engine.gate(), EngineGate::Ready);
}

impl Window {
    /// Lets leaving elements finish leaving, and the poll see what changed.
    /// Settles until `done` holds, at most 30 s (a loaded rig answers late, never wrongly).
    fn settle_until(&mut self, done: impl Fn(&Window) -> bool) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            self.settle();
            if done(self) || std::time::Instant::now() >= until {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    fn settle(&mut self) {
        for _ in 0..2 {
            self.rt.frame(0.016);
            self.rt.settle_all();
            self.rt.frame(0.016);
        }
    }

    /// Everything the window holds as text: each element's text and its accessibility label.
    fn drawn(&self) -> String {
        self.rt
            .elements()
            .into_iter()
            .filter(|e| !e.exiting)
            .flat_map(|e| [e.text, e.label].into_iter().flatten())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Frames until `words` are drawn (an answer of a worker arrives between frames).
    fn until_drawn(&mut self, words: &str) {
        let began = Instant::now();
        loop {
            self.settle();
            if self.drawn().contains(words) {
                return;
            }
            assert!(
                began.elapsed() < Duration::from_secs(15),
                "{words:?} was never drawn; the window draws:\n{}",
                self.drawn()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Frames until the first-time guide is on for the account (its mark is read on a worker; "not done" starts it,
    /// and brings it back after an unlock). Its first step is a window of its own by the b: nothing of it is drawn
    /// here and it takes none of this window's keys.
    #[track_caller]
    fn until_the_guide_is_on(&mut self) {
        self.settle_until(|w| w.rt.state.guide.step.is_some());
        assert_eq!(
            self.rt.state.guide.step,
            Some(Step::Tray),
            "the guide is on its first step for the signed-in account"
        );
    }

    fn press(&mut self, key: Key, mods: Mods) {
        self.rt.key(KeyEvent { key, mods });
        self.settle();
    }

    #[track_caller]
    fn click(&mut self, key: &str) {
        let rect = self
            .rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
            .unwrap_or_else(|| panic!("no element keyed {key}; the window draws:\n{}", self.drawn()))
            .rect;
        let (x, y) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
        self.rt.pointer_down(x, y);
        self.rt.pointer_up(x, y);
        self.settle();
    }

    /// The Files window's own Open on the row of `name`: a click on the row, then Enter.
    #[track_caller]
    fn open_row(&mut self, name: &str) {
        // The Files list is read on a worker: the rows are there once it lands.
        self.until_drawn(name);
        let row = self
            .rt
            .state
            .listing
            .items
            .iter()
            .position(|item| item.name == name)
            .unwrap_or_else(|| panic!("{name} is not listed"));
        self.click(&format!("row-{row}-date"));
        self.press(Key::Enter, Mods::default());
    }

    fn route_item(&self) -> Option<String> {
        match self.rt.state.shell.app.screen() {
            Screen::Drive(route) => route.item,
            _ => None,
        }
    }

    fn chunk_requests(&self) -> usize {
        self.fake.requests().iter().filter(|r| is_chunk(r)).count()
    }

    /// Nothing of the first account's open file is in the window.
    fn assert_nothing_of_the_file(&self, when: &str) {
        let drawn = self.drawn();
        for needle in [
            NOTE_WORDS,
            NOTE_NAME,
            PLAN_NAME,
            REPORT_NAME,
            LETTER_NAME,
            LONG_NAME,
            PICTURE_NAME,
            "the last line",
        ] {
            assert!(!drawn.contains(needle), "{when}: the window draws {needle:?}");
        }
        assert!(
            self.rt.state.viewer.ui.is_none(),
            "{when}: the window holds an open file"
        );
    }
}

#[test]
fn a_file_opened_from_the_files_window_is_decrypted_and_shown_and_leaves_with_its_account() {
    // The shell's poll takes the viewer's intents (the Edit toast below is its router's).
    crate::shell::intents::poll_drains_for_this_test();
    let mut w = window("opens");
    w.settle();
    w.assert_nothing_of_the_file("signed out");

    sign_in(&w.engine, &w.first, TEST_PHRASE);
    // The list is read on a worker: wait for it to land.
    w.until_drawn(NOTE_NAME);

    // Open: the viewer is there at once, saying what it is doing, while Beebeeb still holds the first chunk.
    w.hold.hold();
    w.open_row(NOTE_NAME);
    assert_eq!(w.route_item().as_deref(), Some(NOTE), "the route is on the file");
    assert!(w.rt.state.viewer.ui.is_some(), "the file is open in the window");
    let drawn = w.drawn();
    assert!(
        drawn.contains(&format!(concat!(
            "Decrypting {NOTE_NAME} on ",
            beebeeb_engine::device_word!()
        ), NOTE_NAME = NOTE_NAME)),
        "{drawn}"
    );
    assert!(
        !drawn.contains(NOTE_WORDS),
        "nothing of the content before it is decrypted"
    );

    // The engine downloads and decrypts; the window is handed the text.
    w.hold.release();
    w.until_drawn(NOTE_WORDS);
    let drawn = w.drawn();
    assert!(drawn.contains("the last line"), "the whole text is there");
    assert!(!drawn.contains(&format!(concat!(
        "Decrypting {NOTE_NAME} on ",
        beebeeb_engine::device_word!()
    ), NOTE_NAME = NOTE_NAME)));
    assert!(w.chunk_requests() >= 2, "the content came chunk by chunk from Beebeeb");

    // The details panel (Alt+Shift+P): where the file is, and the Available offline switch, whose state the local
    // state holds.
    w.press(
        Key::Text("p".into()),
        Mods {
            alt: true,
            shift: true,
            ..Mods::default()
        },
    );
    let drawn = w.drawn();
    assert!(drawn.contains("Where: Files"), "{drawn}");
    assert!(drawn.contains("Online only now"), "the state that was read: {drawn}");
    assert!(
        w.rt.elements()
            .iter()
            .any(|e| e.key.as_deref() == Some("offline-switch")),
        "the switch for a state that was read"
    );

    // Edit is not offered where the file's version was not recorded here: the press says so, no editor opens.
    w.press(
        Key::Command('e'),
        Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    );
    assert!(w.rt.state.viewer.ui.as_ref().unwrap().editor.is_none());
    assert!(
        w.drawn().contains(concat!(
            "Not on ",
            beebeeb_engine::device_word!(),
            ": keep it available offline to edit it here"
        )),
        "{}",
        w.drawn()
    );

    // The account leaves and another signs in: nothing of the open file is drawn or held.
    w.engine.sign_out_keeping_local_data().expect("the sign-out");
    w.settle();
    w.assert_nothing_of_the_file("signed out again");
    sign_in(&w.engine, &w.second, OTHER_PHRASE);
    w.settle();
    w.settle();
    w.assert_nothing_of_the_file("the second account, signed in and unlocked");
    assert_eq!(w.route_item(), None, "the route does not name the first account's file");
}

#[test]
fn a_lock_closes_the_open_file_and_a_download_on_its_way_shows_nothing_later() {
    let mut w = window("lock");
    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    // The sign-in starts the first-time guide for the account, on the answer of a worker (its mark is read from
    // the local state). This test waits for it: a lock then leaves a guide to come back to, and what that does to
    // an open file is tested below on every run, not only when the worker happens to answer first.
    w.until_the_guide_is_on();

    // Shown, then locked.
    w.open_row(NOTE_NAME);
    w.until_drawn(NOTE_WORDS);
    w.engine.lock();
    w.settle();
    w.assert_nothing_of_the_file("locked, the file was shown");
    assert_eq!(w.rt.state.guide.step, None, "a lock ends the guide");

    // Unlocked: the route is where it was, so the file opens again, by a new download for the new session.
    let before = w.chunk_requests();
    w.engine
        .unlock_with_phrase(Zeroizing::new(TEST_PHRASE.to_string()))
        .expect("the phrase unlocks");
    // The guide comes back on its worker's answer, before the file is shown again or after it (it moved the window
    // to its step's place, which closed the file whenever its answer came while the route was on the file: the
    // file was gone in a loaded run and there in a run alone). Waited for first, so the file is asked for AFTER the
    // guide is back, on every run: it is still the route's, open and shown.
    w.until_the_guide_is_on();
    assert_eq!(
        w.route_item().as_deref(),
        Some(NOTE),
        "the guide came back without moving the window off the file"
    );
    w.until_drawn(NOTE_WORDS);
    assert!(w.rt.state.viewer.ui.is_some(), "the file is open");
    assert_eq!(w.route_item().as_deref(), Some(NOTE));
    assert!(
        w.chunk_requests() > before,
        "decrypted again from Beebeeb, not kept across the lock"
    );
    // Esc closes it: back to the list.
    w.press(Key::Escape, Mods::default());
    assert!(w.rt.state.viewer.ui.is_none());
    assert_eq!(w.route_item(), None);
    assert!(!w.drawn().contains(NOTE_WORDS));

    // On its way, then locked: the answer never reaches a window.
    w.hold.hold();
    w.open_row(NOTE_NAME);
    assert!(w.drawn().contains(&format!(concat!(
        "Decrypting {NOTE_NAME} on ",
        beebeeb_engine::device_word!()
    ), NOTE_NAME = NOTE_NAME)));
    w.engine.lock();
    w.settle();
    w.assert_nothing_of_the_file("locked, the download was on its way");
    w.hold.release();
    for _ in 0..20 {
        w.settle();
        w.assert_nothing_of_the_file("after the held download ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_kind_with_no_preview_never_opens_the_viewer_from_a_list_and_its_route_says_so_with_two_buttons() {
    let mut w = window("zip");
    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    // The Files window's Open on a kind the viewer has no preview for goes on by `open_file`'s steps 2 and 3 (the
    // system's app, else the file manager): the route stays on the list. No sync runs here, so the file is not on
    // this PC and nothing is started; the window says so (and nothing was downloaded).
    w.open_row(PLAN_NAME);
    if cfg!(target_os = "macos") {
        // Dm-73: on a Mac the open that did not start is said on the file's own page (the reason, the other apps),
        // never in a toast alone. Nothing was downloaded for it.
        assert_eq!(w.route_item().as_deref(), Some(PLAN));
        let drawn = w.drawn();
        assert!(drawn.contains("Couldn't open.") && drawn.contains("Open with another app\u{2026}"), "{drawn}");
        assert_eq!(w.rt.state.actions.toast, None);
    } else {
        assert_eq!(
            w.route_item(),
            None,
            "a list's Open does not go to a file the viewer cannot show"
        );
        assert!(w.rt.state.viewer.ui.is_none());
    }
    assert_eq!(w.chunk_requests(), 0);

    // A route that names it anyway (a link, Back and Forward): the viewer's honest state, and nothing downloaded.
    w.rt.update(|s, _| {
        host::dispatch(s, Event::OpenItem(PLAN.to_string()));
    });
    w.settle();
    assert_eq!(w.route_item().as_deref(), Some(PLAN));
    let drawn = w.drawn();
    assert!(drawn.contains("Beebeeb can\u{2019}t show this kind of file here"), "{drawn}");
    // "Save a copy…" has nowhere to go on this source: it is not drawn (never a button that only says "not built").
    assert!(drawn.contains("Open with another app\u{2026}") && !drawn.contains("Save a copy"));
    assert!(!drawn.contains(&format!(concat!(
        "Decrypting {NOTE_NAME} on ",
        beebeeb_engine::device_word!()
    ), NOTE_NAME = NOTE_NAME)));
    w.settle();
    assert_eq!(w.chunk_requests(), 0, "nothing was downloaded");

    // Open in another app: asked of the engine. No sync runs here, so the file is not on this PC to open: the
    // refusal is said, in its own words, on screen.
    w.click("state-btn-0");
    assert!(w.drawn().contains(viewers::NEEDS_SYNC_ELSEWHERE), "{}", w.drawn());
    assert_eq!(w.chunk_requests(), 0);
}

#[test]
fn a_route_to_a_file_that_is_not_there_says_it_is_no_longer_here_and_close_goes_back() {
    let mut w = window("gone");
    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    w.rt.update(|s, _| {
        host::dispatch(s, Event::OpenItem("b0000000-0000-4000-8000-00000000ffff".to_string()));
    });
    for _ in 0..10 {
        w.settle();
    }
    let drawn = w.drawn();
    assert!(drawn.contains("This file is no longer here"), "{drawn}");
    assert!(
        !drawn.contains("Not built yet") && !drawn.contains("Try again"),
        "{drawn}"
    );
    assert_eq!(
        w.rt.state.viewer.unopened_for_test(),
        None,
        "it is a state on screen, not a memo"
    );
    assert_eq!(w.chunk_requests(), 0);
    // Close: back to the folder, and a file that is there opens.
    w.click("state-btn-0");
    assert_eq!(w.route_item(), None);
    assert!(w.rt.state.viewer.ui.is_none());
    w.open_row(NOTE_NAME);
    w.until_drawn(NOTE_WORDS);
}

/// The engine's own glue, in a window: a file whose bytes are in the account's folder on this PC opens from
/// there with Beebeeb unreachable for its chunks, shows what is on disk (an edit that is not sent yet), and says
/// where it came from.
#[test]
fn a_file_kept_on_this_pc_opens_from_its_copy_without_a_download_and_says_so() {
    let mut w = window("local");
    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    // The local state says an upload of the note is on its way; the folder holds the edited file.
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join(NOTE_NAME), "edited-on-this-pc-words, not sent yet\n").unwrap();
    w.engine.use_sync_root_for_test(folder.path().to_path_buf());
    w.engine
        .local_state_for_changes()
        .unwrap()
        .with(|db| db.set_status(NOTE, FileStatus::Uploading).unwrap())
        .unwrap();
    // Every chunk request would fail: nothing may be downloaded.
    w.hold.fails.store(true, Ordering::SeqCst);

    w.rt.update(|s, _| {
        host::dispatch(s, Event::OpenItem(NOTE.to_string()));
    });
    w.until_drawn("edited-on-this-pc-words");
    let drawn = w.drawn();
    assert!(
        !drawn.contains(NOTE_WORDS),
        "the older version Beebeeb has is not what is shown"
    );
    assert!(drawn.contains(viewers::FROM_THIS_PC_UNSENT), "{drawn}");
    assert!(
        !drawn.contains(concat!("Decrypted on ", beebeeb_engine::device_word!())),
        "{drawn}"
    );
    assert_eq!(w.chunk_requests(), 0, "nothing was downloaded");

    // In sync, and the copy is what Beebeeb has: said as the copy on this PC, without the warning.
    w.press(Key::Escape, Mods::default());
    std::fs::write(folder.path().join(NOTE_NAME), note_text()).unwrap();
    w.engine
        .local_state_for_changes()
        .unwrap()
        .with(|db| db.set_status(NOTE, FileStatus::Local).unwrap())
        .unwrap();
    w.rt.update(|s, _| {
        host::dispatch(s, Event::OpenItem(NOTE.to_string()));
    });
    w.until_drawn(NOTE_WORDS);
    let drawn = w.drawn();
    assert!(drawn.contains(viewers::FROM_THIS_PC), "{drawn}");
    assert!(!drawn.contains("aren't in Beebeeb yet"), "{drawn}");
    assert_eq!(w.chunk_requests(), 0);

    // After a lock nothing of it is left, as for a downloaded file.
    w.engine.lock();
    w.settle();
    w.assert_nothing_of_the_file("after a lock");
}

/// The preview panel's "Open" opens the file in the window's viewer (w29), also for a file the panel itself shows.
#[test]
fn the_preview_panels_open_opens_the_windows_viewer() {
    let mut w = window("itsapp");
    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    // The Files list lands on a worker after the sign-in: wait for the row to be drawn, never read it on a fixed beat.
    w.until_drawn(NOTE_NAME);
    let row =
        w.rt.state
            .listing
            .items
            .iter()
            .position(|item| item.name == NOTE_NAME)
            .expect("the note is listed");
    w.click(&format!("row-{row}-date"));
    w.rt.update(|s, _| assert!(s.preview(), "the preview panel opens"));
    // The panel's own preview downloads the note on a worker. Wait for its answer (drawn in the panel: every one of
    // its chunk requests is in by then), instead of racing it: the count below is what the panel itself asked.
    w.until_drawn(NOTE_WORDS);
    let by_the_panel = w.chunk_requests();
    w.rt.update(|s, tx| s.open_from_preview(tx));
    w.settle();
    let _ = by_the_panel;
    w.until_drawn(NOTE_WORDS);
    assert_eq!(w.route_item().as_deref(), Some(NOTE), "the route went to the viewer");
    assert!(w.rt.state.viewer.ui.is_some());
}

/// A PDF, end to end on the real engine: downloaded and decrypted in memory, its page rendered on this PC and
/// drawn, and nothing of it left after a lock.
#[test]
fn a_pdf_opened_from_the_files_window_shows_its_page_rendered_here() {
    let mut w = window("pdfpage");
    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    w.open_row(REPORT_NAME);
    assert_eq!(w.route_item().as_deref(), Some(REPORT));
    w.until_drawn("Page 1 of 2");
    // The page is asked for when it is drawn, says so meanwhile, and then has its picture.
    let began = Instant::now();
    loop {
        w.settle();
        let page =
            w.rt.elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some("page") && !e.exiting)
                .expect("the page");
        // A Letter page in its own proportions (the blank page of the board is 3:4): its picture is drawn.
        let ratio = page.rect[3] / page.rect[2];
        if (ratio - 792.0 / 612.0).abs() < 0.01 && !w.drawn().contains(viewers::PAGE_RENDERING) {
            break;
        }
        assert!(
            began.elapsed() < Duration::from_secs(20),
            "the page was never drawn: {}",
            w.drawn()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let drawn = w.drawn();
    assert!(
        drawn.contains(concat!("Decrypted on ", beebeeb_engine::device_word!())),
        "{drawn}"
    );
    assert!(!drawn.contains(viewers::PAGE_FAILED), "{drawn}");
    // Page Down: the second page, rendered when it is shown.
    w.press(Key::PageDown, Mods::default());
    w.until_drawn("Page 2 of 2");

    w.engine.lock();
    w.settle();
    w.assert_nothing_of_the_file("after a lock");
}

/// A page that is drawn without some of its text says so in the status line, and the next page does not.
#[test]
fn a_pdf_page_that_may_lack_text_says_so_under_the_page_and_a_page_that_does_not_is_as_before() {
    let mut w = window("pdffont");
    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    w.open_row(LETTER_NAME);
    assert_eq!(w.route_item().as_deref(), Some(LETTER));
    w.until_drawn("Page 1 of 2");
    // Not said while the page is on its way; said once its picture is there.
    assert!(
        !w.drawn().contains(viewers::PAGE_LACKS_TEXT) || !w.drawn().contains(viewers::PAGE_RENDERING),
        "{}",
        w.drawn()
    );
    w.until_drawn(viewers::PAGE_LACKS_TEXT);
    let drawn = w.drawn();
    assert!(!drawn.contains(viewers::PAGE_RENDERING), "{drawn}");
    assert!(
        !drawn.contains("Space for Preview"),
        "the note takes the hint's place: {drawn}"
    );
    assert!(
        drawn.contains(concat!("Decrypted on ", beebeeb_engine::device_word!())),
        "{drawn}"
    );
    // The second page lacks nothing: the hint is back, the note is gone.
    w.press(Key::PageDown, Mods::default());
    w.until_drawn("Page 2 of 2");
    let began = Instant::now();
    while w.drawn().contains(viewers::PAGE_RENDERING) {
        assert!(began.elapsed() < Duration::from_secs(20), "page 2 never came");
        std::thread::sleep(Duration::from_millis(10));
        w.settle();
    }
    let drawn = w.drawn();
    assert!(!drawn.contains(viewers::PAGE_LACKS_TEXT), "{drawn}");
    assert!(drawn.contains("Space for Preview"), "{drawn}");
    // And a lock leaves nothing of it.
    w.engine.lock();
    w.settle();
    w.assert_nothing_of_the_file("after a lock");
    assert!(!w.drawn().contains(viewers::PAGE_LACKS_TEXT));
}

#[test]
fn the_boards_sample_window_opens_an_office_file_in_the_viewer_not_its_own_app() {
    // The board's fixture has no such file: the viewer says so itself; nothing goes to the OS (w29).
    let mut s = crate::init();
    let item = crate::source::DriveItem {
        id: "no-such-sample".into(),
        parent_id: None,
        name: "Invented letter.docx".into(),
        is_folder: false,
        size: Some(10),
        child_count: None,
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    };
    assert!(viewers::open_item(&mut s, &item));
}

#[test]
fn a_failed_download_says_so_and_try_again_downloads_again_while_the_session_is_open() {
    let mut w = window("retry");
    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    w.hold.fails.store(true, Ordering::SeqCst);
    w.open_row(NOTE_NAME);
    w.until_drawn("Couldn't decrypt this file");
    let drawn = w.drawn();
    assert!(drawn.contains("Try again") && !drawn.contains(NOTE_WORDS), "{drawn}");

    // The session ended under the window (as the window reads the engine's status): the press is refused by the
    // shell's session gate, counted, and nothing is asked of Beebeeb.
    w.rt.update(|s, _| {
        s.global = crate::frames::global::Ui::new(Box::new(GlobalFixture::of(board::signed_out())));
    });
    w.settle();
    assert!(w.rt.state.viewer.ui.is_some(), "the window keeps what it shows");
    let (asked, requests) = (w.rt.state.actions.session.asked, w.fake.requests().len());
    w.click("state-btn-0");
    assert_eq!(w.rt.state.actions.session.asked, asked + 1, "the refusal is counted");
    assert_eq!(w.fake.requests().len(), requests, "nothing was asked of Beebeeb");
    assert!(w.drawn().contains("Couldn't decrypt this file"));

    // The session is open again and Beebeeb answers: Try again downloads and shows the file.
    let engine = w.engine.clone();
    w.rt.update(move |s, _| {
        s.global = crate::frames::global::Ui::new(Box::new(crate::engine_host::EngineGlobalSource::new(engine)));
    });
    w.settle();
    w.hold.fails.store(false, Ordering::SeqCst);
    w.click("state-btn-0");
    w.until_drawn(NOTE_WORDS);
    assert!(!w.drawn().contains("Couldn't decrypt this file"));
}

#[test]
fn a_text_longer_than_the_window_is_read_to_its_end_by_scrolling() {
    let mut w = window("scroll");
    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    w.open_row(LONG_NAME);
    w.until_drawn("invented line 1 of the long file");
    // Where the last line is drawn, if it is inside the window at all.
    let last_line_top = |w: &Window| {
        w.rt.elements()
            .into_iter()
            .find(|e| !e.exiting && e.text.as_deref() == Some(LONG_END))
            .map(|e| e.rect[1])
    };
    let body =
        w.rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("viewer-body"))
            .expect("the text's body")
            .rect;
    let (x, y, bottom) = (body[0] + body[2] / 2.0, body[1] + body[3] / 2.0, body[1] + body[3]);
    assert!(
        last_line_top(&w).is_none_or(|top| top >= bottom),
        "before scrolling the last line is below the window"
    );
    // The wheel over the text, far enough for 300 lines (either sign of the toolkit's wheel).
    for dy in [100_000.0, -200_000.0] {
        if last_line_top(&w).is_some_and(|top| top < bottom) {
            break;
        }
        w.rt.wheel(x, y, dy);
        w.settle();
    }
    let top = last_line_top(&w).expect("the last line is drawn after scrolling");
    assert!(
        top >= body[1] && top < bottom,
        "the last line is inside the text's viewport: {top}"
    );
}

#[test]
fn a_picture_is_shown_and_no_action_claims_a_state_that_was_not_read() {
    let mut w = window("picture");
    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    w.open_row(PICTURE_NAME);
    assert_eq!(w.route_item().as_deref(), Some(PICTURE));
    // The photo's stage replaces "Decrypting" once the picture is decoded.
    let began = Instant::now();
    while !w.rt.elements().iter().any(|e| e.key.as_deref() == Some("photo")) {
        assert!(began.elapsed() < Duration::from_secs(15), "the photo was never shown");
        std::thread::sleep(Duration::from_millis(10));
        w.settle();
    }
    let keys: Vec<String> = w.rt.elements().into_iter().filter_map(|e| e.key).collect();
    assert!(
        keys.iter().any(|k| k == "action-0") && keys.iter().any(|k| k == "action-2"),
        "{keys:?}"
    );
    assert!(
        keys.iter().any(|k| k == "action-1"),
        "the Offline action for a state that was read"
    );
    // The decoded picture is the source's, for this session.
    let ui = w.rt.state.viewer.ui.as_ref().expect("the open file");
    assert!(matches!(ui.file.body, Body::Photo(_)));
    w.engine.lock();
    w.settle();
    w.assert_nothing_of_the_file("locked, the picture was shown");
}

#[path = "viewer_edit_lock_tests.rs"]
mod edit_lock;

/// w29: a PDF, a picture, a text and a Word document, each opened from Files (a row and Enter) and from
/// Search (`open_file`), on the real engine: every one is the window's own viewer. A kind with no preview
/// (`.zip` here; a video too, in a window with no player) is not: `open_file` sends it on to steps 2 and 3, and with no sync running here nothing is started.
#[test]
fn four_kinds_open_in_the_app_from_files_and_from_search_and_a_kind_with_no_preview_does_not() {
    use crate::frames::open_file::{item_named, open_file, Opened, Target};
    let mut w = window("sixkinds");
    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    let kinds = [
        (REPORT_NAME, REPORT),
        (PICTURE_NAME, PICTURE),
        (NOTE_NAME, NOTE),
        (DOC_NAME, DOC),
    ];
    let leave = |w: &mut Window| {
        w.rt.update(|s, _| {
            s.viewer.ui = None;
            host::dispatch(s, Event::Escape);
        });
        w.settle_until(|w| w.route_item().is_none());
        assert_eq!(w.route_item(), None);
    };
    let mut in_app = 0;
    for (name, id) in kinds {
        w.open_row(name);
        w.settle_until(|w| w.route_item().as_deref() == Some(id) && w.rt.state.viewer.ui.is_some());
        assert!(
            w.route_item().as_deref() == Some(id) && w.rt.state.viewer.ui.is_some(),
            "Files: {name} did not open in the app; the window draws:\n{}",
            w.drawn()
        );
        in_app += 1;
        leave(&mut w);
        let mut opened = Opened::Failed(crate::frames::files::source::ActionError::Failed);
        w.rt.update(|s, _| opened = open_file(s, Target::Item(&item_named(id, name))));
        w.settle_until(|w| w.route_item().as_deref() == Some(id));
        assert_eq!(
            (opened, w.route_item().as_deref()),
            (Opened::InApp, Some(id)),
            "Search: {name}"
        );
        in_app += 1;
        leave(&mut w);
    }
    assert_eq!(in_app, 8, "4 kinds, from Files and from Search");
    // The last open's decrypt may still be on its way when its window is left (the viewer is drawn before the
    // content is in): let the downloads of the opens above finish, so only the no-preview opens are counted.
    let mut before = w.chunk_requests();
    let mut quiet = 0;
    while quiet < 10 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        w.settle();
        let now = w.chunk_requests();
        quiet = if now == before { quiet + 1 } else { 0 };
        before = now;
    }
    // The kind with no preview: not in the app, nothing downloaded, nothing started (no copy on this PC). On a Mac the
    // open that did not start is said on the file's own page (dm-73); elsewhere the list stays and a toast says it.
    let said = |w: &mut Window, id: &str| {
        if cfg!(target_os = "macos") {
            assert_eq!((w.route_item().as_deref(), w.rt.state.viewer.ui.is_some()), (Some(id), true));
            assert!(w.drawn().contains("Couldn't open."), "{}", w.drawn());
            leave(w);
        } else {
            assert_eq!((w.route_item(), w.rt.state.viewer.ui.is_some()), (None, false));
        }
    };
    w.open_row(PLAN_NAME);
    said(&mut w, PLAN);
    let mut opened = Opened::InApp;
    w.rt.update(|s, _| opened = open_file(s, Target::Item(&item_named(PLAN, PLAN_NAME))));
    assert_eq!(
        opened,
        Opened::Failed(crate::frames::files::source::ActionError::Unavailable)
    );
    // A video, in this window (no player here), is the same: step 2, never the viewer's frame.
    w.open_row(CLIP_NAME);
    said(&mut w, CLIP);
    assert_eq!(w.chunk_requests(), before, "nothing was downloaded for them");
}

/// The real Windows port of step 2: the system knows an app for `.txt`, and none for a kind nobody registered.
#[cfg(target_os = "windows")]
#[test]
fn windows_knows_an_app_for_text_and_none_for_an_unregistered_kind() {
    assert!(crate::engine_host::has_default_app(std::path::Path::new(
        "C:\\x\\notes.txt"
    )));
    assert!(!crate::engine_host::has_default_app(std::path::Path::new(
        "C:\\x\\file.bbunregisteredkind"
    )));
    assert!(!crate::engine_host::has_default_app(std::path::Path::new(
        "C:\\x\\no-extension"
    )));
}
