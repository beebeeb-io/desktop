//! Move to Trash and its Undo on the real engine (`EngineFilesSource::trash` / `restore` / `changes`), against the
//! engine's fake accounts on loopback: what is sent and when, per item answers, and the session rule: a press
//! belongs to the session and the account it was made under. Held on its worker, or at the fake, across a sign-out
//! and another account's sign-in, or across a lock, nothing is sent under any token and nothing of an earlier
//! session is handed to the window. Requests are counted at the fake as they arrive, before it looks at a token.
//! Engine handles are on the process's state folder, which nothing here writes in. Every name is invented.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Reply, Request};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::EngineFilesSource;
use crate::frames::files::source::FilesSource;

const PASSWORD: &str = "an invented password for files";
const WAIT: Duration = Duration::from_secs(30);

const ANSWER: u8 = 0;
const SERVER_ERROR: u8 = 1;
const PROTECTED: u8 = 2;
const SESSION_ENDED: u8 = 3;

const NOTES: &str = "a0000000-0000-4000-8000-000000000401";
const PLAN: &str = "a0000000-0000-4000-8000-000000000402";
const PHOTO: &str = "a0000000-0000-4000-8000-000000000403";
const NOT_THERE: &str = "a0000000-0000-4000-8000-000000000499";
const OTHERS: &str = "a0000000-0000-4000-8000-000000000501";

type Source = EngineFilesSource<MemoryCredentials>;
type Hold = Arc<Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>>;

/// Two accounts on one fake: a sign-in leg goes to the account its email names, anything else to the account whose
/// token it carries.
struct Fake {
    fake: FakeApi,
    first: Arc<FakeAccount>,
    first_phrase: String,
    second: Arc<FakeAccount>,
    second_phrase: String,
    /// How a move to Trash answers.
    trash_mode: Arc<AtomicU8>,
    /// The next move to Trash or restore says on the first that it reached the fake, then waits on the second
    /// before anything is done with it.
    hold: Hold,
}

fn moves(req: &Request) -> bool {
    let route = req.route();
    route.starts_with("DELETE /api/v1/files/")
        || (route.starts_with("POST /api/v1/files/") && route.ends_with("/restore"))
}

fn fake(tag: &str) -> Fake {
    let (first, first_phrase) = FakeAccount::register(&format!("{tag}-one@example.eu"), PASSWORD, TEST_PHRASE);
    let (second, second_phrase) = FakeAccount::register(&format!("{tag}-two@example.eu"), PASSWORD, TEST_PHRASE);
    let (first, second) = (Arc::new(first), Arc::new(second));
    let (one, two): (Handler, Handler) = (account_handler(first.clone()), account_handler(second.clone()));
    let (second_email, second_token) = (second.email.clone(), second.session_token.clone());
    let trash_mode = Arc::new(AtomicU8::new(ANSWER));
    let hold = Hold::default();
    let (mode_in, hold_in) = (trash_mode.clone(), hold.clone());
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
        let rest = if theirs { &two } else { &one };
        if !moves(req) {
            return rest(req);
        }
        let held = hold_in.lock().unwrap().take();
        if let Some((reached, release)) = held {
            let _ = reached.send(());
            let _ = release.recv_timeout(WAIT);
        }
        if route.starts_with("DELETE ") {
            match mode_in.load(Ordering::SeqCst) {
                SERVER_ERROR => return Reply::json(500, json!({ "error": "a server body" })),
                SESSION_ENDED => return Reply::json(401, json!({ "error": "a server body" })),
                PROTECTED => return Reply::json(409, json!({ "error": "protected_folder", "message": "words" })),
                _ => {}
            }
        }
        rest(req)
    }));
    for (id, name) in [
        (NOTES, "Invented notes.txt"),
        (PLAN, "Invented plan.pdf"),
        (PHOTO, "IMG_0002.HEIC"),
    ] {
        first.add_item(id, None, name, false, 10);
    }
    second.add_item(OTHERS, None, "Second account budget.xlsx", false, 10);
    Fake {
        fake,
        first,
        first_phrase,
        second,
        second_phrase,
        trash_mode,
        hold,
    }
}

fn engine_at(base: &str) -> Arc<Engine<MemoryCredentials>> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "files-trash-test-slot".into(),
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

/// The first account signed in, and the Files source on its handle.
fn signed_in(fake: &Fake) -> (Arc<Engine<MemoryCredentials>>, Source) {
    let engine = engine_at(&fake.fake.base);
    sign_in(&engine, &fake.first, &fake.first_phrase);
    let source = EngineFilesSource::new(engine.clone());
    (engine, source)
}

fn rows(ids: &[&str]) -> Vec<DriveItem> {
    ids.iter()
        .map(|id| DriveItem {
            id: id.to_string(),
            parent_id: None,
            name: "a row".into(),
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
        })
        .collect()
}

/// The next call waits on its worker before it asks the engine anything, until the first is sent on; the second
/// says its answer is in the source's inbox.
fn hold_next_call(source: &mut Source) -> (mpsc::Sender<()>, mpsc::Receiver<()>) {
    let ((release, hold), (ended, has_ended)) = (mpsc::channel(), mpsc::channel());
    source.trash.next_call = Some((hold, ended));
    (release, has_ended)
}

/// The next call is not held, and says when its answer is in the inbox.
fn watch_next_call(source: &mut Source) -> mpsc::Receiver<()> {
    let (release, ended) = hold_next_call(source);
    release.send(()).expect("the hook is set");
    ended
}

/// The next move to Trash or restore waits at the fake: the first says it arrived, the second lets it go on.
fn hold_next_request(fake: &Fake) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    let ((reached, was_reached), (release, released)) = (mpsc::channel(), mpsc::channel());
    *fake.hold.lock().unwrap() = Some((reached, released));
    (was_reached, release)
}

/// Every move to Trash and restore that reached the fake, whatever token it carried: its route and whose token.
fn sent(fake: &Fake) -> Vec<(String, &'static str)> {
    fake.fake
        .requests()
        .iter()
        .filter(|r| moves(r))
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

fn trashed(id: &str) -> (String, &'static str) {
    (format!("DELETE /api/v1/files/{id}"), "first")
}

fn restored(id: &str) -> (String, &'static str) {
    (format!("POST /api/v1/files/{id}/restore"), "first")
}

/// What the source hands the window now: per change the action and, per item, its id and why it was left.
fn changes(source: &mut Source) -> Vec<(Action, Vec<(String, Option<ActionError>)>)> {
    source
        .changes()
        .into_iter()
        .map(|change| match change {
            Change::Trash { action, items } => (action, items.into_iter().map(|item| (item.id, item.left)).collect()),
            other => panic!("not a change of Trash: {other:?}"),
        })
        .collect()
}

fn done(id: &str) -> (String, Option<ActionError>) {
    (id.to_string(), None)
}

fn left(id: &str, why: ActionError) -> (String, Option<ActionError>) {
    (id.to_string(), Some(why))
}

/// Move these to Trash and wait for the answer to be in the inbox.
fn trash_now(source: &mut Source, ids: &[&str]) {
    let ended = watch_next_call(source);
    assert_eq!(source.trash(&rows(ids)), Ok(()), "accepted");
    ended.recv_timeout(WAIT).expect("the call ended");
}

#[test]
fn a_move_to_trash_is_sent_per_item_and_drawn_only_once_answered_and_undo_puts_it_back() {
    let fake = fake("made");
    let (_engine, mut source) = signed_in(&fake);
    assert!(source.answers_later(), "the window draws nothing at the press");

    let (release, ended) = hold_next_call(&mut source);
    assert_eq!(source.trash(&rows(&[NOTES, PLAN])), Ok(()), "accepted");
    assert_eq!(
        changes(&mut source),
        [],
        "nothing is handed over before Beebeeb answered"
    );
    assert_eq!(sent(&fake), [], "and nothing is sent on the window's thread");
    release.send(()).expect("the worker waits");
    ended.recv_timeout(WAIT).expect("the call ended");
    assert_eq!(sent(&fake), [trashed(NOTES), trashed(PLAN)]);
    assert_eq!(changes(&mut source), [(Action::Trash, vec![done(NOTES), done(PLAN)])]);
    assert_eq!(changes(&mut source), [], "handed over once");
    assert!(!fake.first.trash_item(NOTES), "Beebeeb has it in Trash already");

    // Undo: what was moved, under the same session.
    let ended = watch_next_call(&mut source);
    assert_eq!(source.restore(&rows(&[NOTES, PLAN])), Ok(()));
    ended.recv_timeout(WAIT).expect("the call ended");
    assert_eq!(sent(&fake)[2..], [restored(NOTES), restored(PLAN)]);
    assert_eq!(changes(&mut source), [(Action::Restore, vec![done(NOTES), done(PLAN)])]);
    assert!(fake.first.trash_item(NOTES), "Beebeeb put it back");

    // Put back: no longer what Undo is for.
    assert_eq!(source.restore(&rows(&[PLAN])), Err(ActionError::Unavailable));
    assert_eq!(sent(&fake).len(), 4, "a refused Undo sends nothing");
}

#[test]
fn nothing_asked_sends_nothing_and_signed_out_is_refused_unsent() {
    let fake = fake("nothing");
    let (engine, mut source) = signed_in(&fake);
    assert_eq!(source.trash(&[]), Ok(()));
    assert_eq!(changes(&mut source), []);

    engine.sign_out_keeping_local_data().expect("signed out");
    assert_eq!(source.trash(&rows(&[NOTES])), Err(ActionError::Unavailable));
    assert_eq!(source.restore(&rows(&[NOTES])), Err(ActionError::Unavailable));
    assert_eq!(changes(&mut source), []);
    assert_eq!(sent(&fake), [], "no session, no request");
}

#[test]
fn what_beebeeb_refuses_is_said_per_item_and_the_other_items_still_go() {
    let fake = fake("refused");
    let (_engine, mut source) = signed_in(&fake);

    // An id Beebeeb does not have for this account, between two it has.
    trash_now(&mut source, &[NOTES, NOT_THERE, PLAN]);
    assert_eq!(
        changes(&mut source),
        [(
            Action::Trash,
            vec![done(NOTES), left(NOT_THERE, ActionError::Failed), done(PLAN)]
        )]
    );
    assert_eq!(sent(&fake).len(), 3, "each sent once");

    // Undo is for what was moved, and only that.
    assert_eq!(
        source.restore(&rows(&[NOTES, NOT_THERE])),
        Err(ActionError::Unavailable),
        "an item that was not moved is not Undo's"
    );
    assert_eq!(sent(&fake).len(), 3, "refused whole, unsent");

    fake.trash_mode.store(SERVER_ERROR, Ordering::SeqCst);
    trash_now(&mut source, &[PHOTO]);
    assert_eq!(
        changes(&mut source),
        [(Action::Trash, vec![left(PHOTO, ActionError::Failed)])]
    );
    fake.trash_mode.store(PROTECTED, Ordering::SeqCst);
    trash_now(&mut source, &[PHOTO]);
    assert_eq!(
        changes(&mut source),
        [(Action::Trash, vec![left(PHOTO, ActionError::Protected)])]
    );

    // A session Beebeeb ended is the call's failure, not the item's: the items after it are not sent.
    fake.trash_mode.store(SESSION_ENDED, Ordering::SeqCst);
    let before = sent(&fake).len();
    trash_now(&mut source, &[PHOTO, NOT_THERE]);
    assert_eq!(
        changes(&mut source),
        [(
            Action::Trash,
            vec![left(PHOTO, ActionError::Failed), left(NOT_THERE, ActionError::Failed)]
        )]
    );
    assert_eq!(sent(&fake).len(), before + 1, "the item after it was not sent");
    fake.trash_mode.store(ANSWER, Ordering::SeqCst);

    // A failed move does not take away what Undo holds: the two that were moved still come back.
    let ended = watch_next_call(&mut source);
    assert_eq!(source.restore(&rows(&[NOTES, PLAN])), Ok(()));
    ended.recv_timeout(WAIT).expect("the call ended");
    assert_eq!(changes(&mut source), [(Action::Restore, vec![done(NOTES), done(PLAN)])]);
}

#[test]
fn without_a_connection_the_first_item_says_so_and_the_rest_are_not_tried() {
    let fake = fake("offline");
    let (_engine, mut source) = signed_in(&fake);
    let Fake { fake: api, .. } = fake;
    drop(api);
    trash_now(&mut source, &[NOTES, PLAN]);
    assert_eq!(
        changes(&mut source),
        [(
            Action::Trash,
            vec![left(NOTES, ActionError::Offline), left(PLAN, ActionError::Offline)]
        )]
    );
}

#[test]
fn a_second_press_on_an_item_whose_change_is_on_its_way_sends_nothing() {
    let fake = fake("twice");
    let (_engine, mut source) = signed_in(&fake);
    let (release, ended) = hold_next_call(&mut source);
    assert_eq!(source.trash(&rows(&[NOTES])), Ok(()));
    assert_eq!(
        source.trash(&rows(&[PLAN, NOTES])),
        Err(ActionError::InFlight),
        "one of them is on its way: none is sent"
    );
    release.send(()).expect("the worker waits");
    ended.recv_timeout(WAIT).expect("the call ended");
    assert_eq!(sent(&fake), [trashed(NOTES)], "sent once");
    assert_eq!(changes(&mut source), [(Action::Trash, vec![done(NOTES)])]);

    // Answered: the mark is gone, and a press on the same item is accepted again (Beebeeb no longer has it).
    trash_now(&mut source, &[NOTES, PLAN]);
    assert_eq!(
        changes(&mut source),
        [(Action::Trash, vec![left(NOTES, ActionError::Failed), done(PLAN)])]
    );
}

#[test]
fn a_move_to_trash_pressed_before_another_account_signed_in_is_sent_under_no_token() {
    let fake = fake("switch-trash");
    let (engine, mut source) = signed_in(&fake);
    let (release, ended) = hold_next_call(&mut source);
    assert_eq!(
        source.trash(&rows(&[NOTES, PLAN])),
        Ok(()),
        "pressed under the first account"
    );

    // The switch, while the call has not yet reached the engine.
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    release.send(()).expect("the worker waits");
    ended.recv_timeout(WAIT).expect("the call ended");

    assert_eq!(sent(&fake), [], "requests that reached the fake, under any token");
    assert_eq!(changes(&mut source), [], "the second account is handed nothing");
    assert!(fake.first.trash_item(NOTES), "the item was not in Trash");
}

#[test]
fn a_move_to_trash_pressed_before_a_lock_is_not_sent_after_the_unlock() {
    let fake = fake("lock-trash");
    let (engine, mut source) = signed_in(&fake);
    let (release, ended) = hold_next_call(&mut source);
    assert_eq!(source.trash(&rows(&[NOTES])), Ok(()));

    let _ = engine.lock();
    unlock(&engine, &fake.first_phrase);
    release.send(()).expect("the worker waits");
    ended.recv_timeout(WAIT).expect("the call ended");
    assert_eq!(sent(&fake), [], "the same account, another session: not sent");
    assert_eq!(changes(&mut source), []);

    // Nothing of the earlier session stands in the way of the next press.
    trash_now(&mut source, &[NOTES]);
    assert_eq!(changes(&mut source), [(Action::Trash, vec![done(NOTES)])]);
}

#[test]
fn an_answer_that_lands_under_another_account_is_not_handed_over() {
    let fake = fake("late");
    let (engine, mut source) = signed_in(&fake);
    // The engine's own send is not held, the request is: it waits at the fake with nothing done yet.
    let ended = watch_next_call(&mut source);
    let (reached, let_go) = hold_next_request(&fake);
    assert_eq!(source.trash(&rows(&[NOTES])), Ok(()));
    reached.recv_timeout(WAIT).expect("the request reached the fake");

    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    let_go.send(()).expect("the fake holds the request");
    ended.recv_timeout(WAIT).expect("the call ended");

    assert_eq!(
        sent(&fake),
        [trashed(NOTES)],
        "sent once, under the session of its press"
    );
    assert_eq!(
        changes(&mut source),
        [],
        "made for the first account: nothing of it is handed over under the second"
    );
    // And it is not what Undo is for under the second account.
    assert_eq!(source.restore(&rows(&[NOTES])), Err(ActionError::Unavailable));
    assert_eq!(sent(&fake).len(), 1);
}

#[test]
fn undo_after_another_account_signed_in_is_refused_and_sends_nothing() {
    let fake = fake("switch-undo");
    let (engine, mut source) = signed_in(&fake);
    trash_now(&mut source, &[NOTES]);
    assert_eq!(changes(&mut source), [(Action::Trash, vec![done(NOTES)])]);

    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    // The window is not asked in between: the press itself finds the other account.
    assert_eq!(
        source.restore(&rows(&[NOTES])),
        Err(ActionError::Unavailable),
        "the ids of the first account are not Undo's under the second"
    );
    assert_eq!(
        sent(&fake),
        [trashed(NOTES)],
        "no restore reached the fake, under any token"
    );
    assert_eq!(changes(&mut source), []);
    assert!(!fake.first.trash_item(NOTES), "still in the first account's Trash");
}

#[test]
fn undo_held_across_a_switch_or_a_lock_is_sent_under_no_token() {
    let fake = fake("held-undo");
    let (engine, mut source) = signed_in(&fake);
    trash_now(&mut source, &[NOTES, PLAN]);
    assert_eq!(changes(&mut source).len(), 1);

    // Undo is pressed under the session of the move, and held on its worker across a lock and an unlock.
    let (release, ended) = hold_next_call(&mut source);
    assert_eq!(source.restore(&rows(&[NOTES])), Ok(()));
    let _ = engine.lock();
    unlock(&engine, &fake.first_phrase);
    release.send(()).expect("the worker waits");
    ended.recv_timeout(WAIT).expect("the call ended");
    assert_eq!(sent(&fake).len(), 2, "no restore: the session of the move is gone");
    assert_eq!(changes(&mut source), []);

    // And across a sign-out and another account's sign-in.
    trash_now(&mut source, &[PHOTO]);
    assert_eq!(changes(&mut source), [(Action::Trash, vec![done(PHOTO)])]);
    let (release, ended) = hold_next_call(&mut source);
    assert_eq!(source.restore(&rows(&[PHOTO])), Ok(()));
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    release.send(()).expect("the worker waits");
    ended.recv_timeout(WAIT).expect("the call ended");
    assert_eq!(
        sent(&fake),
        [trashed(NOTES), trashed(PLAN), trashed(PHOTO)],
        "no restore reached the fake, under any token"
    );
    assert_eq!(changes(&mut source), []);
}
