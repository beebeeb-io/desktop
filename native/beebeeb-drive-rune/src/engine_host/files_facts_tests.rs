//! The sidebar's figures and the folders' sorts on the real engine (`EngineFilesSource::counts`, `storage_split`,
//! `sort_of`, `set_sort`, `changed_label`), against the engine's fake accounts on loopback and a state database of
//! the test's own (`Engine::use_local_state_for_test`), recorded as the first account's: what is read and when,
//! and the session rule: a figure and a sort belong to the session and the account they were read or set under.
//! Every name is invented.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Request};
use chrono::TimeZone;
use serde_json::Value;
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::EngineFilesSource;
use crate::frames::files::SortBy;
use crate::frames::files::source::FilesSource;

const PASSWORD: &str = "an invented password for the figures";
const WAIT: Duration = Duration::from_secs(30);

type Source = EngineFilesSource<MemoryCredentials>;

struct World {
    _fake: FakeApi,
    first: Arc<FakeAccount>,
    first_phrase: String,
    second: Arc<FakeAccount>,
    second_phrase: String,
    engine: Arc<Engine<MemoryCredentials>>,
    db: StateDb,
    _dir: tempfile::TempDir,
}

fn row(db: &StateDb, id: &str, path: &str, size: i64, folder: bool) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::Local,
        size_bytes: size,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

/// Two accounts on one fake (a sign-in leg goes to the account its email names, anything else to the account whose
/// token it carries), nobody signed in, and the first account's local state: a kept folder with a note and a shot
/// in it, a plan and a clip beside it.
fn world(tag: &str) -> World {
    let (first, first_phrase) = FakeAccount::register(&format!("{tag}-one@example.eu"), PASSWORD, TEST_PHRASE);
    let (second, second_phrase) = FakeAccount::register(&format!("{tag}-two@example.eu"), PASSWORD, TEST_PHRASE);
    let (first, second) = (Arc::new(first), Arc::new(second));
    let (one, two): (Handler, Handler) = (account_handler(first.clone()), account_handler(second.clone()));
    let (second_email, second_token) = (second.email.clone(), second.session_token.clone());
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let theirs = if req.route().starts_with("POST /api/v1/opaque/") {
            req.json()
                .get("email")
                .and_then(Value::as_str)
                .is_some_and(|sent| sent.eq_ignore_ascii_case(&second_email))
        } else {
            req.bearer() == Some(second_token.as_str())
        };
        if theirs { two(req) } else { one(req) }
    }));
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: format!("files-facts-slot-{tag}"),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    let engine = Arc::new(engine);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    row(&db, "d-trip", "Trip", 0, true);
    row(&db, "f-note", "Trip/Invented note.txt", 1_000, false);
    row(&db, "f-shot", "Trip/Invented shot.jpg", 30_000, false);
    row(&db, "f-plan", "Invented plan.pdf", 500, false);
    row(&db, "f-clip", "Invented clip.mov", 200_000, false);
    db.set_recursive_pin("d-trip", true, 1).unwrap();
    db.set_owner(&Identity::new(Some(&first.user_id), Some(&first.email)))
        .unwrap();
    engine.use_local_state_for_test(path);
    World {
        _fake: fake,
        first,
        first_phrase,
        second,
        second_phrase,
        engine,
        db,
        _dir: dir,
    }
}

fn sign_in(world: &World, account: &FakeAccount, phrase: &str) {
    world
        .engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    world
        .engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the phrase unlocks");
}

/// The first account signed in, and the Files source on its handle.
fn signed_in(tag: &str) -> (World, Source) {
    let world = world(tag);
    sign_in(&world, &world.first, &world.first_phrase);
    let source = EngineFilesSource::new(world.engine.clone());
    (world, source)
}

/// The next read says when its answer is in the inbox.
fn watch_next_read(source: &mut Source) -> mpsc::Receiver<()> {
    let (release, ended) = hold_next_read(source);
    release.send(()).unwrap();
    ended
}

/// The next read waits on its worker until the first is sent on; the second says its answer is in the inbox.
fn hold_next_read(source: &mut Source) -> (mpsc::Sender<()>, mpsc::Receiver<()>) {
    let ((release, hold), (ended, has_ended)) = (mpsc::channel(), mpsc::channel());
    source.facts.next_read = Some((hold, ended));
    (release, has_ended)
}

/// A draw and a poll, then the read the poll started (it must start one), then the poll that takes its answer.
fn drawn_and_read(source: &mut Source) {
    let ended = watch_next_read(source);
    let _ = source.counts();
    source.changes();
    assert!(source.facts.busy(), "the poll starts the read");
    ended.recv_timeout(WAIT).expect("the read answers");
    source.changes();
    assert!(!source.facts.busy());
}

fn kept(source: &Source) -> Option<u32> {
    source.counts().map(|counts| counts.available_offline)
}

fn by_size() -> Sort {
    Sort {
        by: SortBy::Size,
        reversed: true,
        folders_first: false,
    }
}

#[test]
fn a_draw_reads_nothing_and_the_poll_after_it_brings_the_figures() {
    let (_world, mut source) = signed_in("facts-happy");
    let ended = watch_next_read(&mut source);
    assert_eq!(source.counts(), None, "nothing is known before the first read");
    assert_eq!(source.storage_split(), None);
    assert!(!source.facts.busy(), "a draw starts no read");
    assert!(source.facts.next_read.is_some(), "a draw starts no read");

    source.changes();
    assert!(source.facts.busy(), "the poll after a draw starts it");
    ended.recv_timeout(WAIT).expect("the read answers");
    assert_eq!(source.counts(), None, "an answer is taken by the poll, not by a draw");
    source.changes();
    assert_eq!(
        source.counts(),
        Some(Counts {
            shared_waiting: 0,
            trash: 0,
            available_offline: 2
        }),
        "the note and the shot are kept; the two other counts are not known here"
    );
    assert_eq!(
        source.storage_split(),
        Some(StorageSplit {
            files_bytes: 1_500,
            photos_bytes: 230_000
        })
    );
}

#[test]
fn a_poll_nobody_drew_for_reads_nothing() {
    let (_world, mut source) = signed_in("facts-undrawn");
    let _ended = hold_next_read(&mut source);
    source.changes();
    source.changes();
    assert!(!source.facts.busy());
    assert!(source.facts.next_read.is_some(), "no read was started");
}

#[test]
fn signed_out_nothing_is_read_and_nothing_is_known() {
    let world = world("facts-out");
    let mut source: Source = EngineFilesSource::new(world.engine.clone());
    let _ended = hold_next_read(&mut source);
    assert_eq!(source.counts(), None);
    assert_eq!(source.storage_split(), None);
    source.changes();
    assert!(!source.facts.busy());
    assert!(source.facts.next_read.is_some(), "no read was started");
    source.set_sort(None, by_size());
    assert_eq!(source.sort_of(None), None, "a sort set signed out is nobody's");
    sign_in(&world, &world.first, &world.first_phrase);
    assert_eq!(source.sort_of(None), None, "and no account that signs in after gets it");
}

#[test]
fn the_figures_are_read_once_and_again_only_after_a_change() {
    let (world, mut source) = signed_in("facts-again");
    drawn_and_read(&mut source);
    assert_eq!(kept(&source), Some(2));

    // Something else changed the state: a look alone does not read again.
    row(&world.db, "f-more", "Trip/Invented more.txt", 2_000, false);
    world.db.set_recursive_pin("d-trip", true, 2).unwrap();
    let _ended = hold_next_read(&mut source);
    assert_eq!(kept(&source), Some(2));
    source.changes();
    assert!(!source.facts.busy(), "what is held is not read again by a look");
    assert!(source.facts.next_read.is_some());
    source.facts.next_read = None;

    // A change this source made was answered: the next look reads again.
    let ended = watch_next_read(&mut source);
    assert!(!source.facts.pump(true), "nothing new to draw yet");
    assert!(source.facts.busy(), "a change reads the figures again");
    ended.recv_timeout(WAIT).expect("the read answers");
    assert!(source.facts.pump(false), "the new figures are to be drawn");
    assert_eq!(kept(&source), Some(3));
    assert_eq!(source.storage_split().map(|split| split.files_bytes), Some(3_500));
}

#[test]
fn another_accounts_state_gives_no_figures_and_takes_the_held_ones_away() {
    let (world, mut source) = signed_in("facts-other");
    drawn_and_read(&mut source);
    assert_eq!(kept(&source), Some(2));

    // The state on disk is recorded as someone else's: the next read is refused, and the figures go.
    world
        .db
        .set_owner(&Identity::new(None, Some("someone-else@example.eu")))
        .unwrap();
    let ended = watch_next_read(&mut source);
    source.facts.pump(true);
    ended.recv_timeout(WAIT).expect("the read answers");
    assert!(source.facts.pump(false), "the figures going is a change to draw");
    assert_eq!(source.counts(), None, "a refused read keeps nothing");
    assert_eq!(source.storage_split(), None);
}

#[test]
fn another_account_never_sees_the_first_ones_figures_or_sorts() {
    let (world, mut source) = signed_in("facts-switch");
    drawn_and_read(&mut source);
    source.set_sort(Some("d-trip"), by_size());
    assert_eq!(kept(&source), Some(2));
    assert_eq!(source.sort_of(Some("d-trip")), Some(by_size()));

    world.engine.sign_out_keeping_local_data().expect("signs out");
    assert_eq!(source.counts(), None, "signed out, before any poll");
    assert_eq!(source.storage_split(), None);
    assert_eq!(source.sort_of(Some("d-trip")), None);
    source.changes();
    assert!(
        source.facts.held.is_none(),
        "the first one's figures are not kept in memory"
    );

    sign_in(&world, &world.second, &world.second_phrase);
    assert_eq!(source.counts(), None, "the second account, before any poll");
    assert_eq!(source.storage_split(), None);
    assert_eq!(source.sort_of(Some("d-trip")), None);
    // The state on disk is the first account's: the second one's read is refused.
    let ended = watch_next_read(&mut source);
    source.changes();
    assert!(source.facts.busy(), "the second account's own read starts");
    ended.recv_timeout(WAIT).expect("the read answers");
    source.changes();
    assert_eq!(source.counts(), None);
    assert_eq!(source.sort_of(Some("d-trip")), None);
    // The second account's own sort is its own.
    source.set_sort(Some("d-trip"), Sort::default());
    assert_eq!(source.sort_of(Some("d-trip")), Some(Sort::default()));
}

#[test]
fn a_read_that_answers_after_the_account_left_is_dropped() {
    let (world, mut source) = signed_in("facts-late");
    let (release, ended) = hold_next_read(&mut source);
    let _ = source.counts();
    source.changes();
    assert!(source.facts.busy());

    // Its answer lands after a sign-out and the same account's next sign-in: it is the earlier session's.
    world.engine.sign_out_keeping_local_data().expect("signs out");
    sign_in(&world, &world.first, &world.first_phrase);
    let _held = hold_next_read(&mut source);
    release.send(()).unwrap();
    ended.recv_timeout(WAIT).expect("the read answers");
    // No draw asked since: the poll takes the late answer and starts nothing.
    source.facts.wanted.set(false);
    assert!(!source.facts.pump(false));
    assert!(source.facts.held.is_none(), "the earlier session's answer is not kept");
    assert_eq!(source.counts(), None);
}

#[test]
fn a_folder_keeps_its_own_sort_for_the_session() {
    let (world, mut source) = signed_in("facts-sorts");
    assert_eq!(source.sort_of(None), None, "no sort was chosen");
    source.set_sort(None, by_size());
    source.set_sort(Some("d-trip"), Sort::default());
    assert_eq!(source.sort_of(None), Some(by_size()));
    assert_eq!(source.sort_of(Some("d-trip")), Some(Sort::default()));
    assert_eq!(source.sort_of(Some("d-other")), None);
    source.changes();
    assert_eq!(source.sort_of(None), Some(by_size()), "a poll keeps it");

    world.engine.sign_out_keeping_local_data().expect("signs out");
    sign_in(&world, &world.first, &world.first_phrase);
    assert_eq!(source.sort_of(None), None, "a new session starts without it");
}

#[test]
fn changed_within_the_last_minute_reads_just_now() {
    let now = chrono::Utc.with_ymd_and_hms(2026, 11, 14, 12, 0, 0).unwrap();
    let item = |modified_at: &str| DriveItem {
        id: "f-note".into(),
        parent_id: None,
        name: "a row".into(),
        is_folder: false,
        size: Some(10),
        child_count: None,
        modified_at: modified_at.into(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    };
    let just_now = Some("Just now".to_string());
    assert_eq!(changed_label(&item("2026-11-14T12:00:00Z"), now), just_now);
    assert_eq!(changed_label(&item("2026-11-14T11:59:01Z"), now), just_now);
    assert_eq!(changed_label(&item("2026-11-14T12:59:30+01:00"), now), just_now);
    assert_eq!(changed_label(&item("2026-11-14T11:59:00Z"), now), None, "a minute ago");
    assert_eq!(
        changed_label(&item("2026-11-14T12:00:01Z"), now),
        None,
        "ahead of this PC's clock"
    );
    assert_eq!(changed_label(&item(""), now), None);
    assert_eq!(changed_label(&item("yesterday"), now), None);
}
