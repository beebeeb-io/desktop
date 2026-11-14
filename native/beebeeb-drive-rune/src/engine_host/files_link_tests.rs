//! Copy link and the Share dialog's link on the real engine (`EngineFilesSource::copy_link` / `share_link` /
//! `changes`), against the engine's fake accounts on loopback: what is sent and when, that the link is handed over
//! only once Beebeeb answered, and the session rule: a press belongs to the session and the account it was made
//! under. Held on its worker across a sign-out and another account's sign-in, or across a lock, nothing is sent
//! under any token and nothing of an earlier session is handed to the window. Requests are counted at the fake as
//! they arrive, before it looks at a token. The local state is a database of each test's own. Every name is
//! invented.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Reply, Request};
use serde_json::{Value, json};

use super::*;
use crate::engine_host::EngineFilesSource;
use crate::frames::files::source::FilesSource;

const PASSWORD: &str = "an invented password for links";
const WAIT: Duration = Duration::from_secs(30);

const NOTES: &str = "a0000000-0000-4000-8000-000000000801";
const PLAN: &str = "a0000000-0000-4000-8000-000000000802";
const FOLDER: &str = "a0000000-0000-4000-8000-000000000803";

type Source = EngineFilesSource<MemoryCredentials>;

/// Two accounts on one fake: a sign-in leg goes to the account its email names, anything else to the account whose
/// token it carries.
struct Fake {
    fake: FakeApi,
    first: Arc<FakeAccount>,
    first_phrase: String,
    second: Arc<FakeAccount>,
    second_phrase: String,
    /// Making a link fails at Beebeeb.
    refuse: Arc<AtomicBool>,
}

fn links(req: &Request) -> bool {
    req.route().starts_with("POST /api/v1/shares") || req.route().starts_with("GET /api/v1/shares/mine")
}

fn fake(tag: &str) -> Fake {
    let (first, first_phrase) = FakeAccount::register(&format!("{tag}-one@example.eu"), PASSWORD, TEST_PHRASE);
    // Another phrase: another vault key, so nothing of the first account opens under the second.
    let (second, second_phrase) = FakeAccount::register(
        &format!("{tag}-two@example.eu"),
        PASSWORD,
        "legal winner thank year wave sausage worth useful legal winner thank yellow",
    );
    let (first, second) = (Arc::new(first), Arc::new(second));
    assert_ne!(first.master_key, second.master_key);
    let (one, two): (Handler, Handler) = (account_handler(first.clone()), account_handler(second.clone()));
    let (second_email, second_token) = (second.email.clone(), second.session_token.clone());
    let refuse = Arc::new(AtomicBool::new(false));
    let refuse_in = refuse.clone();
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        let theirs = if route.starts_with("POST /api/v1/opaque/") {
            req.json()
                .get("email")
                .and_then(Value::as_str)
                .is_some_and(|sent| sent.eq_ignore_ascii_case(&second_email))
        } else {
            req.bearer() == Some(second_token.as_str())
        };
        if route == "POST /api/v1/shares" && refuse_in.load(Ordering::SeqCst) {
            return Reply::json(500, json!({ "error": "a server body" }));
        }
        if theirs { two(req) } else { one(req) }
    }));
    first.add_item(NOTES, None, "Invented notes.txt", false, 10);
    first.add_item(PLAN, None, "Invented plan.pdf", false, 10);
    first.add_item(FOLDER, None, "Invented folder", true, 0);
    Fake {
        fake,
        first,
        first_phrase,
        second,
        second_phrase,
        refuse,
    }
}

fn engine_at(base: &str) -> Arc<Engine<MemoryCredentials>> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "files-link-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: base.to_string(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    Arc::new(engine)
}

fn sign_in(engine: &Engine<MemoryCredentials>, account: &FakeAccount, phrase: &str) {
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    unlock(engine, phrase);
}

fn unlock(engine: &Engine<MemoryCredentials>, phrase: &str) {
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the phrase unlocks");
}

/// The first account signed in, its local state (a database of this test's own, with the three rows), and the
/// Files source on its handle.
fn signed_in(fake: &Fake) -> (Arc<Engine<MemoryCredentials>>, Source, tempfile::TempDir) {
    let engine = engine_at(&fake.fake.base);
    sign_in(&engine, &fake.first, &fake.first_phrase);
    let dir = tempfile::tempdir().expect("a temp folder");
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).expect("a state database");
    for (id, name, folder) in [
        (NOTES, "Invented notes.txt", false),
        (PLAN, "Invented plan.pdf", false),
        (FOLDER, "Invented folder", true),
    ] {
        let entry = FileEntry {
            file_id: id.into(),
            path: name.into(),
            status: FileStatus::CloudOnly,
            size_bytes: 10,
            modified_at: 1_794_000_000,
            content_hash: None,
            remote_updated_at: 1_794_000_000,
            parent_id: None,
            item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
        };
        db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
            .expect("a row");
    }
    db.set_owner(&Identity::new(Some(&fake.first.user_id), Some(&fake.first.email)))
        .expect("the owner");
    drop(db);
    engine.use_local_state_for_test(path);
    let source = EngineFilesSource::new(engine.clone());
    (engine, source, dir)
}

fn row(id: &str, is_folder: bool) -> DriveItem {
    DriveItem {
        id: id.to_string(),
        parent_id: None,
        name: "a row".into(),
        is_folder,
        size: Some(10),
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

fn file(id: &str) -> Vec<DriveItem> {
    vec![row(id, false)]
}

/// The next call waits on its worker before it asks the engine anything, until the first is sent on; the second
/// says its answer is in the source's inbox.
fn hold_next_call(source: &mut Source) -> (mpsc::Sender<()>, mpsc::Receiver<()>) {
    let ((release, hold), (ended, has_ended)) = (mpsc::channel(), mpsc::channel());
    source.links.next_call = Some((hold, ended));
    (release, has_ended)
}

/// The next call is not held, and says when its answer is in the inbox.
fn watch_next_call(source: &mut Source) -> mpsc::Receiver<()> {
    let (release, ended) = hold_next_call(source);
    release.send(()).expect("the hook is set");
    ended
}

/// Every request about links that reached the fake, whatever token it carried: its route and whose token.
fn sent(fake: &Fake) -> Vec<(String, &'static str)> {
    fake.fake
        .requests()
        .iter()
        .filter(|r| links(r))
        .map(|r| {
            let whose = match r.bearer() {
                Some(token) if token == fake.first.session_token => "first",
                Some(token) if token == fake.second.session_token => "second",
                _ => "nobody",
            };
            (r.route(), whose)
        })
        .collect()
}

fn read() -> (String, &'static str) {
    ("GET /api/v1/shares/mine".into(), "first")
}

fn made() -> (String, &'static str) {
    ("POST /api/v1/shares".into(), "first")
}

/// What the source hands the window now: per answer the action and the link, or why there is none.
fn answers(source: &mut Source) -> Vec<(Action, Result<String, ActionError>)> {
    source
        .changes()
        .into_iter()
        .map(|change| match change {
            Change::Link { action, answer } => (action, answer.map(|link| link.expose().to_string())),
            other => panic!("not a link's answer: {other:?}"),
        })
        .collect()
}

/// Ask for the plain link of one file and wait for the answer to be in the inbox.
fn copy_now(source: &mut Source, id: &str) {
    let ended = watch_next_call(source);
    assert_eq!(source.copy_link(&file(id)), Ok(()), "accepted");
    ended.recv_timeout(WAIT).expect("the call ended");
}

#[test]
fn a_link_is_asked_for_on_a_worker_and_handed_over_only_once_beebeeb_answered() {
    let fake = fake("made");
    let (_engine, mut source, _state) = signed_in(&fake);
    assert!(
        source.answers_later(),
        "the window says and copies nothing at the press"
    );

    let (release, ended) = hold_next_call(&mut source);
    assert_eq!(source.copy_link(&file(NOTES)), Ok(()), "accepted");
    assert_eq!(
        answers(&mut source),
        [],
        "nothing is handed over before Beebeeb answered"
    );
    assert_eq!(sent(&fake), [], "and nothing is sent on the window's thread");
    release.send(()).expect("the worker waits");
    ended.recv_timeout(WAIT).expect("the call ended");

    assert_eq!(sent(&fake), [read(), made()]);
    let changes = source.changes();
    let printed = format!("{changes:?}");
    let [
        Change::Link {
            action: Action::CopyLink,
            answer: Ok(link),
        },
    ] = &changes[..]
    else {
        panic!("one link, for Copy link: {printed}");
    };
    let posted = fake.first.links();
    let token = posted[0]["token"].as_str().expect("the posted token");
    assert!(
        link.expose()
            .starts_with(&format!("http://localhost:5173/s/{token}#key=")),
        "the link Beebeeb accepted"
    );
    // What a `{:?}` of the answer prints holds nothing of the link.
    assert!(!printed.contains(token) && !printed.contains("#key="), "{printed}");
    assert_eq!(answers(&mut source), [], "handed over once");
}

#[test]
fn a_file_that_has_a_link_and_a_link_beebeeb_refuses_are_said_as_what_they_are() {
    let fake = fake("said");
    let (_engine, mut source, _state) = signed_in(&fake);
    copy_now(&mut source, NOTES);
    assert!(matches!(&answers(&mut source)[..], [(Action::CopyLink, Ok(_))]));
    // The file has a live link now, which cannot be copied again yet: no second one is made.
    copy_now(&mut source, NOTES);
    assert_eq!(answers(&mut source), [(Action::CopyLink, Err(ActionError::HasLink))]);
    assert_eq!(fake.first.links().len(), 1);
    // Beebeeb fails: failed, in the window's words.
    fake.refuse.store(true, Ordering::SeqCst);
    copy_now(&mut source, PLAN);
    assert_eq!(answers(&mut source), [(Action::CopyLink, Err(ActionError::Failed))]);
}

#[test]
fn the_dialogs_choices_are_sent_and_a_view_only_link_is_refused_unsent() {
    let fake = fake("choices");
    let (_engine, mut source, _state) = signed_in(&fake);
    let options = |access| ShareOptions {
        access,
        expires: LinkExpiry::Week,
        password: Some("an invented link password".into()),
    };
    // "Can view" cannot be kept by a link: it is not made as if it could.
    assert_eq!(
        source.share_link(&file(NOTES), &options(LinkAccess::View)),
        Err(ActionError::NotYet)
    );
    assert_eq!(sent(&fake), [], "nothing was sent");

    let ended = watch_next_call(&mut source);
    assert_eq!(source.share_link(&file(NOTES), &options(LinkAccess::Download)), Ok(()));
    ended.recv_timeout(WAIT).expect("the call ended");
    assert!(matches!(&answers(&mut source)[..], [(Action::ShareLink, Ok(_))]));
    assert_eq!(sent(&fake), [made()], "a link with choices is a new link");
    let posted = fake.first.links();
    assert_eq!(posted[0]["expires_in_hours"], 24 * 7);
    assert_eq!(posted[0]["passphrase"], "an invented link password");
}

#[test]
fn the_dialog_with_nothing_chosen_makes_no_second_link_and_a_file_not_here_offers_no_retry_kind() {
    let fake = fake("plain-dialog");
    let (_engine, mut source, _state) = signed_in(&fake);
    let nothing = ShareOptions {
        access: LinkAccess::Download,
        expires: LinkExpiry::Never,
        password: None,
    };
    for expected in [None, Some(ActionError::HasLink), Some(ActionError::HasLink)] {
        let ended = watch_next_call(&mut source);
        assert_eq!(source.share_link(&file(NOTES), &nothing), Ok(()));
        ended.recv_timeout(WAIT).expect("the call ended");
        let answer = answers(&mut source).pop().expect("an answer").1;
        assert_eq!(answer.err(), expected);
    }
    assert_eq!(
        fake.first.links().len(),
        1,
        "one link, however often the dialog was pressed"
    );

    // A file that is not in this account's local state: nothing is sent, and it is not said as "failed" (which
    // would offer a Try again that changes nothing).
    const NOT_HERE: &str = "a0000000-0000-4000-8000-000000000899";
    let before = sent(&fake).len();
    copy_now(&mut source, NOT_HERE);
    assert_eq!(
        answers(&mut source),
        [(Action::CopyLink, Err(ActionError::Unavailable))]
    );
    assert_eq!(sent(&fake).len(), before, "nothing was sent");
}

#[test]
fn a_folder_several_items_and_no_session_are_refused_with_nothing_sent() {
    let fake = fake("unsent");
    let (engine, mut source, _state) = signed_in(&fake);
    assert_eq!(source.copy_link(&[row(FOLDER, true)]), Err(ActionError::NotYet));
    // Never the first of several.
    assert_eq!(
        source.copy_link(&[row(NOTES, false), row(PLAN, false)]),
        Err(ActionError::NotYet)
    );
    assert_eq!(source.copy_link(&[]), Err(ActionError::NotYet));
    engine.sign_out_keeping_local_data().expect("signed out");
    assert_eq!(source.copy_link(&file(NOTES)), Err(ActionError::Unavailable));
    assert_eq!(answers(&mut source), []);
    assert_eq!(sent(&fake), [], "nothing was sent");
}

#[test]
fn a_second_press_for_an_item_whose_link_is_on_its_way_sends_nothing() {
    let fake = fake("twice");
    let (_engine, mut source, _state) = signed_in(&fake);
    let (release, ended) = hold_next_call(&mut source);
    assert_eq!(source.copy_link(&file(NOTES)), Ok(()));
    assert_eq!(source.copy_link(&file(NOTES)), Err(ActionError::InFlight));
    // Another item is not held up by it.
    let other = watch_next_call(&mut source);
    assert_eq!(source.copy_link(&file(PLAN)), Ok(()));
    other.recv_timeout(WAIT).expect("the other call ended");
    release.send(()).expect("the worker waits");
    ended.recv_timeout(WAIT).expect("the call ended");
    assert_eq!(answers(&mut source).len(), 2);
    assert_eq!(fake.first.links().len(), 2, "one link each, no third");
    // Answered: the item can be asked for again.
    copy_now(&mut source, NOTES);
    assert_eq!(answers(&mut source), [(Action::CopyLink, Err(ActionError::HasLink))]);
}

#[test]
fn a_link_pressed_before_another_account_signed_in_is_asked_under_no_token() {
    let fake = fake("switch");
    let (engine, mut source, _state) = signed_in(&fake);
    let (release, ended) = hold_next_call(&mut source);
    assert_eq!(
        source.copy_link(&file(NOTES)),
        Ok(()),
        "pressed under the first account"
    );

    // The switch, while the call has not yet reached the engine.
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    release.send(()).expect("the worker waits");
    ended.recv_timeout(WAIT).expect("the call ended");

    assert_eq!(sent(&fake), [], "requests that reached the fake, under any token");
    assert_eq!(answers(&mut source), [], "the second account is handed nothing");
    assert_eq!(
        fake.first.links().len() + fake.second.links().len(),
        0,
        "no link exists"
    );
}

#[test]
fn a_link_pressed_before_a_lock_is_not_asked_for_after_the_unlock() {
    let fake = fake("lock");
    let (engine, mut source, _state) = signed_in(&fake);
    let (release, ended) = hold_next_call(&mut source);
    assert_eq!(source.copy_link(&file(NOTES)), Ok(()));

    assert!(source.link_pending(), "a link is on its way");
    let _ = engine.lock();
    unlock(&engine, &fake.first_phrase);
    release.send(()).expect("the worker waits");
    ended.recv_timeout(WAIT).expect("the call ended");
    assert_eq!(sent(&fake), [], "the same account, another session: not sent");
    assert_eq!(answers(&mut source), []);
    // Its answer was dropped: the source says nothing is on its way, so the window forgets the press.
    assert!(!source.link_pending());

    // Nothing of the earlier session stands in the way of the next press.
    copy_now(&mut source, NOTES);
    assert!(matches!(&answers(&mut source)[..], [(Action::CopyLink, Ok(_))]));
}

#[test]
fn a_link_that_lands_under_another_account_is_not_handed_over() {
    let fake = fake("late");
    let (engine, mut source, _state) = signed_in(&fake);
    // Made for the first account, and in the inbox; the window has not looked yet.
    copy_now(&mut source, NOTES);
    assert_eq!(fake.first.links().len(), 1);
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    assert_eq!(
        answers(&mut source),
        [],
        "made for the first account: no link is handed over under the second"
    );

    // The same when the window had looked once (the link was pumped into what it takes) and not taken it.
    let fake = self::fake("late-pumped");
    let (engine, mut source, _state) = signed_in(&fake);
    copy_now(&mut source, NOTES);
    source.links.pump();
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    assert_eq!(answers(&mut source), []);
}
