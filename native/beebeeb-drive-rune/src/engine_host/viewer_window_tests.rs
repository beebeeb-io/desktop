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
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Request};
use rune::{Env, Key, KeyEvent, Mods, Runtime};
use zeroize::Zeroizing;

use crate::State;
use crate::engine_host::LiveEngine;
use crate::engine_host::open::Opened;
use crate::frames::viewers;
use crate::shell::actions::{ActionError, EngineActions, HostActions};
use crate::shell::app::{Screen, Session};
use crate::shell::host::{Shell, Wiring};
use crate::shell::identity::engine_identity;
use crate::shell::instance::ProfileHolder;
use crate::shell::platform::fake::FakePlatform;
use crate::shell::tray::TrayInputs;
use crate::shell::{host, registry, state};

const PASSWORD: &str = "an invented password for the viewer";
/// A second valid 12-word phrase (a BIP-39 test vector), for the second account.
const OTHER_PHRASE: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";

const NOTE: &str = "b0000000-0000-4000-8000-000000000601";
const PLAN: &str = "b0000000-0000-4000-8000-000000000602";
const NOTE_NAME: &str = "note-of-the-first-account-viewer.txt";
const PLAN_NAME: &str = "plan-of-the-first-account-viewer.pdf";
/// Words found nowhere else in the app.
const NOTE_WORDS: &str = "harbourlight-words-of-the-first-account";

fn note_text() -> String {
    // Several chunks of the fake's size.
    format!("{NOTE_WORDS} line one\n{}\nthe last line\n", "filler ".repeat(600))
}

/// Holds every chunk request while it is on.
#[derive(Default)]
struct Hold {
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
    first.add_file_with_content(PLAN, None, PLAN_NAME, b"%PDF-1.7 invented");
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
    put(&db, PLAN, PLAN_NAME, 17);
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
            assert!(began.elapsed() < Duration::from_secs(15), "{words:?} was never drawn");
            std::thread::sleep(Duration::from_millis(10));
        }
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
        for needle in [NOTE_WORDS, NOTE_NAME, PLAN_NAME, "the last line"] {
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
    let mut w = window("opens");
    w.settle();
    w.assert_nothing_of_the_file("signed out");

    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    assert!(w.drawn().contains(NOTE_NAME), "the first account sees its list");

    // Open: the viewer is there at once, saying what it is doing, while Beebeeb still holds the first chunk.
    w.hold.hold();
    w.open_row(NOTE_NAME);
    assert_eq!(w.route_item().as_deref(), Some(NOTE), "the route is on the file");
    assert!(w.rt.state.viewer.ui.is_some(), "the file is open in the window");
    let drawn = w.drawn();
    assert!(drawn.contains("Decrypting on this PC"), "{drawn}");
    assert!(
        !drawn.contains(NOTE_WORDS),
        "nothing of the content before it is decrypted"
    );

    // The engine downloads and decrypts; the window is handed the text.
    w.hold.release();
    w.until_drawn(NOTE_WORDS);
    let drawn = w.drawn();
    assert!(drawn.contains("the last line"), "the whole text is there");
    assert!(!drawn.contains("Decrypting on this PC"));
    assert!(w.chunk_requests() >= 2, "the content came chunk by chunk from Beebeeb");

    // Edit is not offered where an edit cannot be saved: the press says so, no editor opens.
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
        w.drawn().contains("Not built yet: Edit and save from here"),
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

    // Shown, then locked.
    w.open_row(NOTE_NAME);
    w.until_drawn(NOTE_WORDS);
    w.engine.lock();
    w.settle();
    w.assert_nothing_of_the_file("locked, the file was shown");

    // Unlocked: the route is where it was, so the file opens again, by a new download for the new session.
    let before = w.chunk_requests();
    w.engine
        .unlock_with_phrase(Zeroizing::new(TEST_PHRASE.to_string()))
        .expect("the phrase unlocks");
    w.until_drawn(NOTE_WORDS);
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
    assert!(w.drawn().contains("Decrypting on this PC"));
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
fn a_file_with_no_viewer_says_so_downloads_nothing_and_its_two_buttons_answer() {
    let mut w = window("pdf");
    sign_in(&w.engine, &w.first, TEST_PHRASE);
    w.settle();
    w.open_row(PLAN_NAME);
    assert_eq!(w.route_item().as_deref(), Some(PLAN));
    let drawn = w.drawn();
    assert!(drawn.contains("No preview for this kind of file yet"), "{drawn}");
    assert!(drawn.contains("Open in another app\u{2026}") && drawn.contains("Save a copy\u{2026}"));
    assert!(!drawn.contains("Decrypting on this PC"));
    w.settle();
    assert_eq!(w.chunk_requests(), 0, "nothing was downloaded");

    // Open in another app: asked of the engine. No sync runs here, so the file is not on this PC to open: the
    // refusal is said, in words, on screen.
    w.click("state-btn-0");
    assert!(w.drawn().contains(viewers::NOT_OPENED_ELSEWHERE), "{}", w.drawn());
    // Save a copy is not built: the press says so.
    w.click("state-btn-1");
    assert!(w.drawn().contains("Not built yet: Save a copy"), "{}", w.drawn());
    assert_eq!(w.chunk_requests(), 0);
}

#[test]
fn the_boards_sample_window_still_leaves_an_office_file_to_its_own_app() {
    // The board's fixture has no such file: the router says "not this group's", and Files opens it as before.
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
    assert!(!viewers::open_item(&mut s, &item));
    assert!(s.viewer.ui.is_none());
}
