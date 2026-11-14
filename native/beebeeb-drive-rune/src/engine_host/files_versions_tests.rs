//! Version history on the real engine (`EngineFilesSource::ask_versions` / `versions` / `versions_state` /
//! `restore_version` / `changes`), against the engine's fake accounts on loopback, and in the Files window's element
//! tree: what is read and when, what is drawn before the engine answered, what a restore sends, and the session rule:
//! a press belongs to the session and the account it was made under. The local state is a database of each test's own
//! (`Engine::use_local_state_for_test`), recorded as the first account's. Requests are counted at the fake as they
//! arrive. Every name, id and date is invented.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Reply, Request};
use rune::{Env, Runtime};
use zeroize::Zeroizing;

use super::*;
use crate::State;
use crate::engine_host::open::Opened;
use crate::engine_host::{EngineFilesSource, LiveEngine};
use crate::frames::files::source::FilesSource;
use crate::shell::actions::{ActionError as ShellError, EngineActions, HostActions};
use crate::shell::app::Session;
use crate::shell::host::{Shell, Wiring};
use crate::shell::identity::engine_identity;
use crate::shell::instance::ProfileHolder;
use crate::shell::platform::fake::FakePlatform;
use crate::shell::tray::TrayInputs;
use crate::shell::{host, registry, state};

const PASSWORD: &str = "an invented password for versions";
/// A second valid 12-word phrase (a BIP-39 test vector), for the second account.
const OTHER_PHRASE: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";
const WAIT: Duration = Duration::from_secs(30);

const NOTES: &str = "c0000000-0000-4000-8000-000000000801";
const NOTES_NAME: &str = "versions-of-the-first-account.txt";
const OTHERS: &str = "c0000000-0000-4000-8000-000000000802";
const OTHERS_NAME: &str = "versions-of-the-second-account.txt";

/// The first account's versions, as the server wrote them (oldest first, the current one last).
const VERSIONS: [(&str, &str, u64); 3] = [
    ("ver-a", "2026-09-01T10:00:00Z", 1_100),
    ("ver-c", "2026-09-03T10:00:00Z", 3_300),
    ("ver-b", "2026-09-02T10:00:00Z", 2_200),
];

const NORMAL: u8 = 0;
const SERVER_ERROR: u8 = 1;
const HANG_UP: u8 = 2;

type Source = EngineFilesSource<MemoryCredentials>;
type Hold = Arc<Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>>;

struct World {
    fake: FakeApi,
    first: Arc<FakeAccount>,
    second: Arc<FakeAccount>,
    /// How a list of versions is answered.
    list_mode: Arc<AtomicU8>,
    /// The next list of versions says on the first that it reached the fake, then waits on the second.
    hold: Hold,
    engine: Arc<Engine<MemoryCredentials>>,
    _dir: tempfile::TempDir,
}

fn is_list(request: &Request) -> bool {
    request.method == "GET" && request.path.ends_with("/versions")
}

fn put(db: &StateDb, id: &str, name: &str) {
    let entry = FileEntry {
        file_id: id.to_string(),
        path: format!("/{name}"),
        status: FileStatus::CloudOnly,
        size_bytes: 2_200,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

/// Two accounts on one fake: a request with a session's token goes to that account, a sign-in to the account its
/// email names (and its second leg to the same one).
fn world(tag: &str) -> World {
    let (first, _) = FakeAccount::register(&format!("versions-one-{tag}@example.eu"), PASSWORD, TEST_PHRASE);
    let (second, _) = FakeAccount::register(&format!("versions-two-{tag}@example.eu"), PASSWORD, OTHER_PHRASE);
    let (first, second) = (Arc::new(first), Arc::new(second));
    first.add_item(NOTES, None, NOTES_NAME, false, 2_200);
    for (id, at, size) in VERSIONS {
        first.add_version(NOTES, id, at, size);
    }
    second.add_item(OTHERS, None, OTHERS_NAME, false, 10);
    second.add_version(OTHERS, "other-a", "2026-08-01T10:00:00Z", 10);
    second.add_version(OTHERS, "other-b", "2026-08-02T10:00:00Z", 10);
    let (to_first, to_second) = (account_handler(first.clone()), account_handler(second.clone()));
    let (second_email, second_token) = (second.email.clone(), second.session_token.clone());
    let signing_in_second = Mutex::new(false);
    let list_mode = Arc::new(AtomicU8::new(NORMAL));
    let hold = Hold::default();
    let (mode_in, hold_in) = (list_mode.clone(), hold.clone());
    let handler: Handler = Arc::new(move |request: &Request| {
        let second_asked = match request.bearer() {
            Some(token) => token == second_token,
            None => {
                let mut signing_in = signing_in_second.lock().unwrap();
                if let Some(email) = request.json().get("email").and_then(|email| email.as_str()) {
                    *signing_in = email == second_email;
                }
                *signing_in
            }
        };
        if is_list(request) {
            let held = hold_in.lock().unwrap().take();
            if let Some((reached, release)) = held {
                let _ = reached.send(());
                let _ = release.recv_timeout(WAIT);
            }
            match mode_in.load(Ordering::SeqCst) {
                SERVER_ERROR => return Reply::json(500, serde_json::json!({ "error": "a server body" })),
                HANG_UP => return Reply::hang_up(),
                _ => {}
            }
        }
        if second_asked {
            to_second(request)
        } else {
            to_first(request)
        }
    });
    let fake = FakeApi::start(handler);
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: format!("files-versions-slot-{tag}"),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.base.clone(),
    };
    let engine = Arc::new(
        Engine::open(engine_identity(), config, host, MemoryCredentials::default()).expect("the engine opens"),
    );
    engine.restore();
    // The first account's local state, as its sync left it.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    put(&db, NOTES, NOTES_NAME);
    db.set_owner(&Identity::new(Some(&first.user_id), Some(&first.email)))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(path);
    World {
        fake,
        first,
        second,
        list_mode,
        hold,
        engine,
        _dir: dir,
    }
}

fn sign_in(engine: &Engine<MemoryCredentials>, account: &FakeAccount, phrase: &str) {
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the phrase unlocks");
}

fn first_signed_in(tag: &str) -> (World, Source) {
    let world = world(tag);
    sign_in(&world.engine, &world.first, TEST_PHRASE);
    let source = EngineFilesSource::new(world.engine.clone());
    (world, source)
}

fn notes() -> DriveItem {
    row(NOTES, NOTES_NAME)
}

fn row(id: &str, name: &str) -> DriveItem {
    DriveItem {
        id: id.to_string(),
        parent_id: None,
        name: name.to_string(),
        is_folder: false,
        size: Some(2_200),
        child_count: None,
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    }
}

/// The next call waits on its worker before it asks the engine anything, until the first is sent on; the second
/// says its answer is in the source's inbox.
fn hold_next_call(source: &mut Source) -> (mpsc::Sender<()>, mpsc::Receiver<()>) {
    let ((release, hold), (ended, has_ended)) = (mpsc::channel(), mpsc::channel());
    source.versions.next_call = Some((hold, ended));
    (release, has_ended)
}

/// The next call is not held, and says when its answer is in the inbox.
fn watch_next_call(source: &mut Source) -> mpsc::Receiver<()> {
    let (release, ended) = hold_next_call(source);
    release.send(()).expect("the hook is set");
    ended
}

/// Ask for the versions and wait for the answer to be in the inbox, then take it in.
fn read_now(source: &mut Source, item: &DriveItem) {
    let ended = watch_next_call(source);
    source.ask_versions(item);
    ended.recv_timeout(WAIT).expect("the call ended");
    source.changes();
}

/// The ids the window would list, in order, with whether each is the current one.
fn listed(source: &Source, item: &DriveItem) -> Vec<(String, bool)> {
    source
        .versions(item)
        .into_iter()
        .map(|version| (version.id, version.current))
        .collect()
}

/// Every request about versions that reached the fake, whatever token it carried.
fn requests(world: &World) -> Vec<String> {
    world
        .fake
        .requests()
        .iter()
        .map(Request::route)
        .filter(|route| route.contains("/versions"))
        .collect()
}

fn lists(world: &World) -> usize {
    requests(world).iter().filter(|route| route.starts_with("GET ")).count()
}

fn failures(source: &mut Source) -> Vec<(Action, ActionError)> {
    source
        .changes()
        .into_iter()
        .map(|change| match change {
            Change::Failed(action, why) => (action, why),
            other => panic!("not a failure: {other:?}"),
        })
        .collect()
}

#[test]
fn the_versions_are_read_when_asked_and_listed_current_first() {
    let (world, mut source) = first_signed_in("lists");
    assert_eq!(source.versions(&notes()), [], "nothing before the engine answered");
    assert_eq!(source.versions_state(&notes()), Read::Loading);
    read_now(&mut source, &notes());
    assert_eq!(
        listed(&source, &notes()),
        [
            ("ver-b".to_string(), true),
            ("ver-c".to_string(), false),
            ("ver-a".to_string(), false)
        ]
    );
    assert_eq!(source.versions_state(&notes()), Read::Answered);
    let shown = source.versions(&notes());
    assert_eq!(shown[1].when, crate::ui::file_list::format_date("2026-09-03T10:00:00Z"));
    assert_eq!(shown[1].size, crate::source::format_size(3_300));
    assert_eq!(
        shown[1].changed_on, None,
        "the server names no device, so none is invented"
    );
    // Fresh: opening again within the time asks nothing.
    source.ask_versions(&notes());
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(lists(&world), 1);
}

#[test]
fn a_list_of_an_earlier_account_is_dropped() {
    let (world, mut source) = first_signed_in("dropped");
    // The answer is in the inbox, and the account changes before the window takes it in.
    let ended = watch_next_call(&mut source);
    source.ask_versions(&notes());
    ended.recv_timeout(WAIT).expect("the call ended");
    world.engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&world.engine, &world.second, OTHER_PHRASE);
    assert_eq!(source.changes().len(), 0);
    assert_eq!(source.versions(&notes()), [], "nothing of the first account's list");
    assert_eq!(
        source.versions_state(&notes()),
        Read::Loading,
        "and it is not held as read"
    );
}

#[test]
fn another_account_sees_no_version_of_the_first_in_the_adapter() {
    let (world, mut source) = first_signed_in("another");
    read_now(&mut source, &notes());
    assert_eq!(source.versions(&notes()).len(), 3);
    world.engine.sign_out_keeping_local_data().expect("signed out");
    // Nothing asked and nothing taken in since: what was held is not handed out under another session.
    assert_eq!(source.versions(&notes()), []);
    sign_in(&world.engine, &world.second, OTHER_PHRASE);
    assert_eq!(source.versions(&notes()), []);
}

#[test]
fn a_failed_read_says_so_and_is_read_again_at_the_next_opening() {
    let (world, mut source) = first_signed_in("failed");
    world.list_mode.store(SERVER_ERROR, Ordering::SeqCst);
    read_now(&mut source, &notes());
    assert_eq!(source.versions_state(&notes()), Read::Failed(ActionError::Failed));
    assert_eq!(source.versions(&notes()), []);
    assert_eq!(lists(&world), 1);
    // The next opening asks again (a failure is never held as a fresh read).
    world.list_mode.store(HANG_UP, Ordering::SeqCst);
    read_now(&mut source, &notes());
    assert_eq!(lists(&world), 2, "asked again");
    assert_eq!(source.versions_state(&notes()), Read::Failed(ActionError::Offline));
    world.list_mode.store(NORMAL, Ordering::SeqCst);
    read_now(&mut source, &notes());
    assert_eq!(lists(&world), 3);
    assert_eq!(source.versions(&notes()).len(), 3);
}

#[test]
fn a_restore_makes_the_chosen_version_current_and_the_list_shows_it() {
    let (world, mut source) = first_signed_in("restores");
    read_now(&mut source, &notes());
    let ended = watch_next_call(&mut source);
    assert_eq!(source.restore_version(&notes(), "ver-a"), Ok(()));
    ended.recv_timeout(WAIT).expect("the call ended");
    assert_eq!(failures(&mut source), [], "nothing to say: the list shows it");
    let now = listed(&source, &notes());
    assert_eq!(now.len(), 4, "one more version was minted");
    assert_eq!(now.iter().filter(|(_, current)| *current).count(), 1);
    assert!(
        now[0].1 && now[0].0.contains("ver-a"),
        "the restored one is current, first: {now:?}"
    );
    assert_eq!(world.first.restore_requests(), 1);
}

#[test]
fn a_second_restore_while_one_is_on_its_way_sends_nothing() {
    let (world, mut source) = first_signed_in("inflight");
    read_now(&mut source, &notes());
    let (release, ended) = hold_next_call(&mut source);
    assert_eq!(source.restore_version(&notes(), "ver-a"), Ok(()));
    assert_eq!(source.restore_version(&notes(), "ver-c"), Err(ActionError::InFlight));
    assert_eq!(source.restore_version(&notes(), "ver-a"), Err(ActionError::InFlight));
    release.send(()).expect("the worker waits");
    ended.recv_timeout(WAIT).expect("the call ended");
    source.changes();
    assert_eq!(world.first.restore_requests(), 1, "one request");
    // Answered: the mark is gone, and another restore may go.
    let now = listed(&source, &notes());
    let other = now
        .iter()
        .find(|(id, current)| !current && id == "ver-c")
        .expect("ver-c is listed")
        .0
        .clone();
    assert_eq!(source.restore_version(&notes(), &other), Ok(()));
}

#[test]
fn a_version_that_is_not_in_the_list_is_not_sent() {
    let (world, mut source) = first_signed_in("not-listed");
    // Before the list is known.
    assert_eq!(source.restore_version(&notes(), "ver-a"), Err(ActionError::Unavailable));
    read_now(&mut source, &notes());
    assert_eq!(
        source.restore_version(&notes(), "a-version-nobody-listed"),
        Err(ActionError::Unavailable)
    );
    assert_eq!(
        source.restore_version(&notes(), "ver-b"),
        Err(ActionError::Unavailable),
        "the current one"
    );
    assert_eq!(world.first.restore_requests(), 0, "nothing was sent");
}

#[test]
fn a_lost_answer_is_not_called_failed_when_the_list_shows_the_restore() {
    let (world, mut source) = first_signed_in("lost");
    read_now(&mut source, &notes());
    world.first.make_next_restore_then_hang_up();
    let ended = watch_next_call(&mut source);
    assert_eq!(source.restore_version(&notes(), "ver-a"), Ok(()));
    ended.recv_timeout(WAIT).expect("the call ended");
    assert_eq!(failures(&mut source), [], "the list shows it was made");
    assert_eq!(listed(&source, &notes()).len(), 4);
    assert_eq!(world.first.restore_requests(), 1, "and it was not sent again");
}

#[test]
fn a_lost_answer_is_called_failed_when_it_does_not() {
    let (world, mut source) = first_signed_in("not-made");
    read_now(&mut source, &notes());
    // A gateway's 502 and the server made nothing: the list is as it was.
    world.first.refuse_next_restore_with(502);
    let ended = watch_next_call(&mut source);
    assert_eq!(source.restore_version(&notes(), "ver-a"), Ok(()));
    ended.recv_timeout(WAIT).expect("the call ended");
    assert_eq!(failures(&mut source), [(Action::RestoreVersion, ActionError::Failed)]);
    assert_eq!(
        listed(&source, &notes()).len(),
        3,
        "the list is read again, and unchanged"
    );
    // The server refused (409): failed, and the list kept.
    world.first.refuse_next_restore_with(409);
    let ended = watch_next_call(&mut source);
    assert_eq!(source.restore_version(&notes(), "ver-a"), Ok(()));
    ended.recv_timeout(WAIT).expect("the call ended");
    assert_eq!(failures(&mut source), [(Action::RestoreVersion, ActionError::Failed)]);
}

#[test]
fn a_restore_with_no_session_is_refused_unsent() {
    let (world, mut source) = first_signed_in("signed-out");
    read_now(&mut source, &notes());
    world.engine.sign_out_keeping_local_data().expect("signed out");
    assert_eq!(source.restore_version(&notes(), "ver-a"), Err(ActionError::Unavailable));
    source.ask_versions(&notes());
    assert_eq!(world.first.restore_requests(), 0);
    assert_eq!(lists(&world), 1, "and nothing was asked");
}

// ── The Files window ─────────────────────────────────────────────────────────────────────────────

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
    fn pause_sync(&self) -> Result<(), ShellError> {
        Ok(())
    }
    fn resume_sync(&self) -> Result<(), ShellError> {
        Ok(())
    }
    fn open_in_file_explorer(&self) -> Result<(), ShellError> {
        Ok(())
    }
    fn profile_holder(&self) -> Option<ProfileHolder> {
        None
    }
    fn start_sync(&self) -> Result<(), ShellError> {
        Ok(())
    }
    fn stop_for_quit(&self) {}
    fn resume_after_cancelled_quit(&self) {}
    fn sync_refusal(&self) -> Option<crate::frames::global::sync_off::SyncOff> {
        self.live.sync_refusal()
    }
}

const ENV: Env = Env {
    system_dark: false,
    scale: 2.0,
};

/// The Files window on the engine of `world`, as the app builds it (signed out until a test signs in).
fn window(world: &World) -> Runtime<State> {
    let (_host, messages) = ChannelHost::new(|| {});
    let opened = Opened {
        engine: world.engine.clone(),
        identity: engine_identity(),
        gate: world.engine.gate(),
    };
    let live = Arc::new(LiveEngine::new(&opened, messages));
    let platform: Arc<FakePlatform> = Arc::new(FakePlatform::default());
    let shell = Shell::new(
        Arc::new(HostActions::new(Arc::new(NoStart { live }), platform.clone())),
        platform,
        Wiring::default(),
    );
    let mut rt = Runtime::new(
        state::init_on_engine(shell, world.engine.clone()),
        registry::view,
        (1280.0, 800.0),
        ENV,
    );
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    rt
}

fn settle(rt: &mut Runtime<State>) {
    for _ in 0..2 {
        rt.frame(0.016);
        rt.settle_all();
        rt.frame(0.016);
    }
}

/// Everything the window holds as text: each element's text and its accessibility label.
fn drawn(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .flat_map(|e| [e.text, e.label].into_iter().flatten())
        .collect::<Vec<_>>()
        .join("\n")
}

fn until_drawn(rt: &mut Runtime<State>, words: &str) {
    let began = Instant::now();
    loop {
        settle(rt);
        if drawn(rt).contains(words) {
            return;
        }
        assert!(
            began.elapsed() < Duration::from_secs(15),
            "{words:?} was never drawn; the window draws:\n{}",
            drawn(rt)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}; the window draws:\n{}", drawn(rt)))
        .rect;
    let (x, y) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

/// Open Version history for the first account's file: select its row, then the dialog.
fn open_versions(rt: &mut Runtime<State>) {
    until_drawn(rt, NOTES_NAME);
    let at = rt
        .state
        .listing
        .items
        .iter()
        .position(|item| item.name == NOTES_NAME)
        .expect("the file is listed");
    click(rt, &format!("row-{at}-date"));
    assert!(rt.state.begin_versions(), "the dialog opens");
    settle(rt);
}

#[test]
fn the_dialog_says_loading_then_lists_the_versions() {
    let world = world("window");
    let mut rt = window(&world);
    sign_in(&world.engine, &world.first, TEST_PHRASE);
    // The list is held at the fake: the dialog has nothing to draw but the loading line.
    let (reached, was_reached) = mpsc::channel();
    let (release, released) = mpsc::channel();
    *world.hold.lock().unwrap() = Some((reached, released));
    open_versions(&mut rt);
    was_reached.recv_timeout(WAIT).expect("the request reached the fake");
    settle(&mut rt);
    let before = drawn(&rt);
    assert!(
        before.contains("Loading the versions\u{2026}"),
        "the window draws:\n{before}"
    );
    assert!(!before.contains("can't be shown"), "no false sentence");
    assert!(
        !rt.elements()
            .iter()
            .any(|e| e.key.as_deref().is_some_and(|k| k.starts_with("item-0-0"))),
        "no version row before the engine answered"
    );
    release.send(()).expect("the fake holds the reply");
    until_drawn(&mut rt, "3 versions");
    let after = drawn(&rt);
    assert!(!after.contains("Loading the versions"), "{after}");
    let keys: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.key)
        .filter(|k| k.starts_with("item-0-") && k.matches('-').count() == 2)
        .collect();
    assert_eq!(keys.len(), 3, "three version rows: {keys:?}");
    assert!(after.contains(&crate::ui::file_list::format_date("2026-09-03T10:00:00Z")));
    assert!(after.contains("Current version"));
}

#[test]
fn another_account_sees_no_version_of_the_first() {
    let world = world("window-other");
    let mut rt = window(&world);
    sign_in(&world.engine, &world.first, TEST_PHRASE);
    open_versions(&mut rt);
    until_drawn(&mut rt, "3 versions");
    let first_date = crate::ui::file_list::format_date("2026-09-03T10:00:00Z");
    assert!(drawn(&rt).contains(&first_date));
    world.engine.sign_out_keeping_local_data().expect("signed out");
    settle(&mut rt);
    sign_in(&world.engine, &world.second, OTHER_PHRASE);
    settle(&mut rt);
    let now = drawn(&rt);
    assert!(
        !now.contains(&first_date),
        "a date of the first account's versions is drawn:\n{now}"
    );
    assert!(!now.contains(NOTES_NAME), "the first account's file is drawn");
    assert!(
        !rt.elements()
            .iter()
            .any(|e| e.key.as_deref().is_some_and(|k| k.starts_with("item-0-"))),
        "a version row of the first account is in the tree"
    );
}
