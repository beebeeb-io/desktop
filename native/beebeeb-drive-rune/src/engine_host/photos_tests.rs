//! Photos and the thumbnails on the real engine (`EnginePhotos`, `EngineThumbStore`, `EngineThumbs`), against the
//! engine's fake accounts on loopback and a local state of each test's own (`Engine::use_local_state_for_test`):
//! what the library shows, what is asked and when, per item answers, and the session rule: what is shown, asked or
//! answered belongs to the session and the account it was made under. A move to Trash waits at the fake, a
//! thumbnail on its worker, so "while it is on its way" is a state the test is in. Every name and pixel is invented.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Reply, Request};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::thumbs::EngineThumbs;
use crate::frames::global::grid::ThumbSource;

const PASSWORD: &str = "an invented password for photos";
const WAIT: Duration = Duration::from_secs(30);

const ANSWER: u8 = 0;
const SERVER_ERROR: u8 = 1;

const HARBOUR: &str = "c0000000-0000-4000-8000-000000000801";
const TRAM: &str = "c0000000-0000-4000-8000-000000000802";
const OLD: &str = "c0000000-0000-4000-8000-000000000803";
const SHOT: &str = "c0000000-0000-4000-8000-000000000804";
const BACKUPS: &str = "c0000000-0000-4000-8000-000000000805";
const NOTE: &str = "c0000000-0000-4000-8000-000000000806";
const OTHERS: &str = "c0000000-0000-4000-8000-000000000901";

type Source = EnginePhotos<MemoryCredentials>;
type Store = Arc<EngineThumbStore<MemoryCredentials>>;
type Hold = Arc<Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>>;

struct Fake {
    fake: FakeApi,
    first: Arc<FakeAccount>,
    first_phrase: String,
    second: Arc<FakeAccount>,
    second_phrase: String,
    /// How a move to Trash answers.
    trash_mode: Arc<AtomicU8>,
    /// The next move to Trash or restore says on the first that it reached the fake, then waits on the second.
    hold: Hold,
}

fn moves(req: &Request) -> bool {
    let route = req.route();
    route.starts_with("DELETE /api/v1/files/")
        || (route.starts_with("POST /api/v1/files/") && route.ends_with("/restore"))
}

fn is_thumbnail(req: &Request) -> bool {
    req.method == "GET" && req.path.contains("/thumbnail/")
}

/// A picture `w` by `h`, all one colour, as a PNG.
fn picture(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[40, 90, 160, 255].repeat((w * h) as usize))
            .unwrap();
    }
    out
}

fn fake(tag: &str) -> Fake {
    let (first, first_phrase) = FakeAccount::register(&format!("{tag}-one@example.eu"), PASSWORD, TEST_PHRASE);
    let (second, second_phrase) = FakeAccount::register(&format!("{tag}-two@example.eu"), PASSWORD, TEST_PHRASE);
    let (first, second) = (Arc::new(first), Arc::new(second));
    for (id, name) in [
        (HARBOUR, "Invented harbour.jpg"),
        (TRAM, "Invented tram.mov"),
        (OLD, "Invented old.png"),
        (SHOT, "Invented shot.heic"),
    ] {
        first.add_file_with_content(id, None, name, b"not read here");
    }
    second.add_file_with_content(OTHERS, None, "Second account shot.jpg", b"not read here");
    // The stored pictures, per (owner's token, path's end): only the owner's token is answered with one.
    let stored: Vec<(String, String, Vec<u8>)> = vec![
        (
            first.session_token.clone(),
            format!("{HARBOUR}/thumbnail/medium"),
            first.seal_thumbnail(HARBOUR, &picture(40, 20)),
        ),
        (
            first.session_token.clone(),
            format!("{HARBOUR}/thumbnail/large"),
            first.seal_thumbnail(HARBOUR, &picture(40, 20)),
        ),
        (
            second.session_token.clone(),
            format!("{OTHERS}/thumbnail/medium"),
            second.seal_thumbnail(OTHERS, &picture(8, 8)),
        ),
    ];
    let (one, two): (Handler, Handler) = (account_handler(first.clone()), account_handler(second.clone()));
    let (second_email, second_token) = (second.email.clone(), second.session_token.clone());
    let trash_mode = Arc::new(AtomicU8::new(ANSWER));
    let hold = Hold::default();
    let (mode_in, hold_in) = (trash_mode.clone(), hold.clone());
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        if is_thumbnail(req) {
            let found = stored
                .iter()
                .find(|(token, path, _)| req.bearer() == Some(token.as_str()) && req.path.ends_with(path.as_str()));
            return match found {
                Some((_, _, blob)) => Reply::bytes(200, blob.clone()),
                None => Reply::json(404, json!({ "error": "a server body" })),
            };
        }
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
        if route.starts_with("DELETE ") && mode_in.load(Ordering::SeqCst) == SERVER_ERROR {
            return Reply::json(500, json!({ "error": "a server body" }));
        }
        rest(req)
    }));
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
        account_id: "photos-test-slot".into(),
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
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the phrase unlocks");
}

/// Noon today, local time, in Unix seconds, less `days` days and `seconds` seconds.
fn noon(days: i64, seconds: i64) -> i64 {
    let noon = Local::now().date_naive().and_hms_opt(12, 0, 0).unwrap();
    Local.from_local_datetime(&noon).single().unwrap().timestamp() - days * 86_400 - seconds
}

fn row(db: &StateDb, id: &str, path: &str, modified: i64, folder: bool) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::CloudOnly,
        size_bytes: if folder { 0 } else { 2_000_000 },
        modified_at: modified,
        content_hash: None,
        remote_updated_at: modified,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

/// A local state recorded as `owner`'s with the first account's library, and the engine reading it. Today: the
/// harbour, the phone's shot, the tram (a video). Ten days ago: one more.
fn local_state(engine: &Engine<MemoryCredentials>, owner: &FakeAccount, more: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    row(&db, HARBOUR, "Trip/Invented harbour.jpg", noon(0, 0), false);
    row(&db, TRAM, "Trip/Invented tram.mov", noon(0, 120), false);
    row(&db, OLD, "Invented old.png", noon(10, 0), false);
    row(&db, NOTE, "Invented note.txt", noon(0, 0), false);
    row(&db, BACKUPS, "Backups", noon(0, 0), true);
    db.mark_backups_root_for_test(BACKUPS).unwrap();
    row(
        &db,
        SHOT,
        "Backups/Invented phone/Camera/Invented shot.heic",
        noon(0, 60),
        false,
    );
    for (id, path) in more {
        row(&db, id, path, noon(20, 0), false);
    }
    db.set_owner(&Identity::new(Some(&owner.user_id), Some(&owner.email)))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(path);
    dir
}

/// The first account signed in on its own local state, and the Photos source with its store.
fn signed_in(fake: &Fake) -> (Arc<Engine<MemoryCredentials>>, Source, Store, tempfile::TempDir) {
    let engine = engine_at(&fake.fake.base);
    sign_in(&engine, &fake.first, &fake.first_phrase);
    let dir = local_state(&engine, &fake.first, &[]);
    let store = EngineThumbStore::new(engine.clone());
    let source = EnginePhotos::new(engine.clone(), store.clone());
    (engine, source, store, dir)
}

/// Pumps until `done` says so; what was answered on the way.
fn pump_until(source: &mut Source, mut done: impl FnMut(&Source, &[Answered]) -> bool) -> Vec<Answered> {
    let deadline = Instant::now() + WAIT;
    let mut answered = Vec::new();
    loop {
        answered.extend(source.pump().answered);
        if done(source, &answered) {
            return answered;
        }
        assert!(Instant::now() < deadline, "it never came");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The Photos section opens and the library is read.
fn open(source: &mut Source) {
    source.opened();
    pump_until(source, |source, _| source.loaded() != Loaded::Loading);
}

/// Pumps until nothing is on its way any more.
fn settle(source: &mut Source) -> Vec<Answered> {
    let mut answered = Vec::new();
    let deadline = Instant::now() + WAIT;
    loop {
        let pumped = source.pump();
        answered.extend(pumped.answered);
        if !pumped.busy {
            return answered;
        }
        assert!(Instant::now() < deadline, "it never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The numbers of what shows, day by day.
fn shown(source: &Source) -> Vec<Vec<PhotoId>> {
    source
        .days(None)
        .iter()
        .enumerate()
        .map(|(day, of)| {
            (0..of.len)
                .map(|index| source.photo(None, day, index).expect("a photo").id)
                .collect()
        })
        .collect()
}

fn hold_next_request(fake: &Fake) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    let ((reached, was_reached), (release, released)) = (mpsc::channel(), mpsc::channel());
    *fake.hold.lock().unwrap() = Some((reached, released));
    (was_reached, release)
}

/// Every move to Trash and restore that reached the fake: its route and whose token it carried.
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

fn thumbnail_requests(fake: &Fake) -> usize {
    fake.fake.requests().iter().filter(|r| is_thumbnail(r)).count()
}

/// Asks for something of the file `id` and waits until its fetch was taken in or dropped.
fn fetch(store: &Store, id: &str, ask: impl FnOnce()) {
    let (release, ended) = store.hold_next_fetch(id);
    ask();
    release.send(()).expect("a worker took the wish");
    ended.recv_timeout(WAIT).expect("the fetch ended");
}

#[test]
fn the_library_shows_the_days_the_devices_and_the_open_photo_of_the_local_state() {
    let fake = fake("library");
    let (_engine, mut source, _store, _dir) = signed_in(&fake);
    assert!(source.wired() && source.answers_later());

    // Nothing is read from a draw: until the section opens there is no library, and it says so.
    assert_eq!(source.loaded(), Loaded::Loading);
    assert_eq!(source.days(None), []);
    assert_eq!(source.pump().answered, []);
    assert_eq!(source.days(None), [], "a pump reads nothing either");

    open(&mut source);
    assert_eq!(source.loaded(), Loaded::Ready);
    let days = source.days(None);
    assert_eq!(days.len(), 2);
    assert_eq!((days[0].title.as_str(), days[0].len), ("Today", 3));
    assert_eq!(days[1].len, 1);
    assert_eq!(
        days[1].title,
        day_title(local(noon(10, 0)).date_naive(), Local::now().date_naive())
    );

    // Newest first: the harbour, the phone's shot, the tram (a video; online only, so no length).
    let harbour = source.photo(None, 0, 0).unwrap();
    let shot = source.photo(None, 0, 1).unwrap();
    let tram = source.photo(None, 0, 2).unwrap();
    assert_eq!(
        (harbour.kind, harbour.bytes, harbour.uploading),
        (Kind::Photo, 2_000_000, false)
    );
    assert_eq!(tram.kind, Kind::Video(0));
    assert_eq!(source.photo(None, 0, 3), None);
    assert_eq!(source.photo(None, 2, 0), None);

    // The devices are the device folders under Backups that hold a photo.
    let devices = source.devices();
    assert_eq!(devices.len(), 1);
    assert_eq!(
        (
            devices[0].name.as_str(),
            devices[0].kind,
            devices[0].photos,
            devices[0].shown
        ),
        ("Invented phone", DeviceKind::Phone, 1, true)
    );
    assert_eq!(source.days(Some(0)).iter().map(|day| day.len).collect::<Vec<_>>(), [1]);
    assert_eq!(source.photo(Some(0), 0, 0).map(|photo| photo.id), Some(shot.id));
    assert_eq!(source.days(Some(1)), [], "no such device");

    // Totals count what shows; the note and the folders are no photos.
    assert_eq!(
        source.totals(None),
        Totals {
            photos: 3,
            videos: 1,
            bytes: 8_000_000
        }
    );
    assert_eq!(
        source.totals(Some(0)),
        Totals {
            photos: 1,
            videos: 0,
            bytes: 2_000_000
        }
    );
    assert_eq!(
        source.origin(),
        "Photos from Invented phone \u{00B7} 3 from your other folders"
    );
    assert_eq!(source.folders(), []);
    assert_eq!(source.backup_status(), None);

    // The open photo: its own facts, its neighbours in order, its place among what shows.
    let viewing = source.viewing(shot.id).expect("it opens");
    assert_eq!(viewing.name, "Invented shot.heic");
    assert_eq!(viewing.device, "Invented phone");
    assert_eq!(viewing.taken, "Today 11:59");
    assert_eq!(viewing.position, (2, 4));
    assert_eq!(
        viewing
            .strip
            .iter()
            .map(|item| (item.id, item.current))
            .collect::<Vec<_>>(),
        [
            (harbour.id, false),
            (shot.id, true),
            (tram.id, false),
            (shown(&source)[1][0], false)
        ]
    );
    assert_eq!(source.viewing(harbour.id).unwrap().device, "Files");
    assert!(!format!("{viewing:?}").contains("Invented"), "a {{:?}} prints no name");

    let details: Vec<(&str, String)> = source
        .details(shot.id)
        .into_iter()
        .map(|line| (line.label, line.value))
        .collect();
    assert_eq!(details[0], ("Changed", "Today, 11:59".to_string()));
    assert_eq!(details[1], ("From", "Invented phone".to_string()));
    assert_eq!(details[3], ("Where", "Backups / Invented phone / Camera".to_string()));
    assert_eq!(details[4], ("Version history", "1 version".to_string()));
    assert!(!source.kept_offline(shot.id));

    // A hidden device's photos leave what shows; the device still lists them.
    source.set_shown(SourceRef::Device(0), false);
    assert!(source.pump().changed, "the window is told to read the days again");
    assert_eq!(source.days(None)[0].len, 2);
    assert_eq!(source.totals(None).photos, 2);
    assert_eq!(source.days(Some(0))[0].len, 1);
    assert_eq!(source.origin(), "Photos and videos from your folders");
    source.set_shown(SourceRef::Device(0), true);
    assert_eq!(source.days(None)[0].len, 3);
    // A folder is no source here: nothing changes.
    source.set_shown(SourceRef::Folder(0), false);
    assert_eq!(source.days(None)[0].len, 3);
}

#[test]
fn a_day_is_named_by_how_long_ago_it_is() {
    let today = NaiveDate::from_ymd_opt(2026, 11, 14).unwrap();
    let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
    assert_eq!(day_title(today, today), "Today");
    assert_eq!(day_title(day(2026, 11, 13), today), "Yesterday");
    assert_eq!(day_title(day(2026, 11, 9), today), "Monday");
    assert_eq!(day_title(day(2026, 11, 7), today), "7 November");
    assert_eq!(day_title(day(2025, 10, 12), today), "12 October 2025");
}

#[test]
fn a_library_that_could_not_be_read_says_why_and_is_read_again_at_the_next_opening() {
    let fake = fake("unread");
    let engine = engine_at(&fake.fake.base);
    sign_in(&engine, &fake.first, &fake.first_phrase);
    // The local state on this PC is the second account's.
    let _theirs = local_state(&engine, &fake.second, &[]);
    let store = EngineThumbStore::new(engine.clone());
    let mut source = EnginePhotos::new(engine.clone(), store);
    open(&mut source);
    assert_eq!(source.loaded(), Loaded::NotThisAccounts);
    assert_eq!(source.days(None), [], "never another account's library");
    assert_eq!(source.devices(), []);

    // A failed read stamps no freshness: the next opening reads again, at once.
    let _own = local_state(&engine, &fake.first, &[]);
    open(&mut source);
    assert_eq!(source.loaded(), Loaded::Ready);
    assert_eq!(source.days(None)[0].len, 3);

    // A library that is fresh is not read again.
    let _theirs = local_state(&engine, &fake.second, &[]);
    source.opened();
    assert!(!source.pump().busy, "no read was started");
    assert_eq!(source.loaded(), Loaded::Ready);
}

#[test]
fn signed_out_shows_nothing_and_another_account_sees_nothing_of_the_first() {
    let fake = fake("whose");
    let (engine, mut source, store, _dir) = signed_in(&fake);
    open(&mut source);
    let firsts: Vec<PhotoId> = shown(&source).into_iter().flatten().collect();
    assert_eq!(firsts.len(), 4);
    let harbour = firsts[0];
    fetch(&store, HARBOUR, || {
        assert!(source.thumbnail(harbour, Shape::Square).is_none())
    });
    assert!(
        source.thumbnail(harbour, Shape::Square).is_some(),
        "the first account's picture is here"
    );

    // Signed out: every accessor answers nothing at once, before any poll.
    engine.sign_out_keeping_local_data().expect("signed out");
    assert_eq!(source.loaded(), Loaded::SignedOut);
    assert_eq!(source.days(None), []);
    assert_eq!(source.devices(), []);
    assert_eq!(source.totals(None), Totals::default());
    assert_eq!(source.origin(), "");
    assert_eq!(source.viewing(harbour), None);
    assert_eq!(source.details(harbour), []);
    assert!(source.thumbnail(harbour, Shape::Square).is_none());
    assert_eq!(source.trash(&[harbour]), Err(Left::Unavailable));
    assert_eq!(source.keep_offline(harbour, true), Err(Left::Unavailable));

    // The second account signs in on the same PC (the local state is still the first's).
    sign_in(&engine, &fake.second, &fake.second_phrase);
    assert_eq!(source.days(None), [], "before any poll");
    assert!(
        !EngineThumbs::new(store.clone()).arrived(HARBOUR),
        "the Files grid, by the file's id: the first account's picture is not there for the second"
    );
    assert_eq!(source.viewing(harbour), None);
    assert!(
        source.thumbnail(harbour, Shape::Square).is_none(),
        "the picture went with its session"
    );
    assert_eq!(
        source.trash(&[harbour]),
        Err(Left::Unavailable),
        "a number of the first names nothing"
    );
    assert_eq!(source.restore(&[harbour]), Err(Left::Unavailable));
    source.forget();
    open(&mut source);
    assert_eq!(source.loaded(), Loaded::NotThisAccounts);
    assert_eq!(sent(&fake), [], "nothing was sent for it under any token");
    assert_eq!(
        thumbnail_requests(&fake),
        1,
        "and no picture was asked for under the second session"
    );

    // On its own local state the second account gets its own numbers: none of the first's is handed out again.
    let _own = local_state(&engine, &fake.second, &[]);
    open(&mut source);
    let seconds: Vec<PhotoId> = shown(&source).into_iter().flatten().collect();
    assert_eq!(seconds.len(), 4);
    assert!(seconds.iter().all(|id| !firsts.contains(id)));
}

#[test]
fn a_thumbnail_arrives_in_memory_once_and_a_file_without_one_is_not_asked_again() {
    let fake = fake("thumbs");
    let (_engine, mut source, store, _dir) = signed_in(&fake);
    open(&mut source);
    let ids = shown(&source);
    let (harbour, old) = (ids[0][0], ids[1][0]);
    settle(&mut source);

    // A look only wishes: nothing is fetched on the window's thread.
    let (release, ended) = store.hold_next_fetch(HARBOUR);
    assert!(source.thumbnail(harbour, Shape::Square).is_none());
    assert_eq!(thumbnail_requests(&fake), 0);
    assert!(source.pump().busy, "it is on its way");
    release.send(()).unwrap();
    ended.recv_timeout(WAIT).expect("the fetch ended");
    assert!(source.pump().changed, "the window is told a picture arrived");
    let square = source.thumbnail(harbour, Shape::Square).expect("the picture");
    assert_eq!(square.size(), (20, 20), "the middle of 40 by 20, cut by the engine");
    assert_eq!(thumbnail_requests(&fake), 1);
    assert!(source.thumbnail(harbour, Shape::Square).is_some());
    assert_eq!(thumbnail_requests(&fake), 1, "held: not asked again");

    // The preview is the whole stored picture.
    fetch(&store, HARBOUR, || {
        assert!(source.thumbnail(harbour, Shape::Preview).is_none())
    });
    assert_eq!(
        source.thumbnail(harbour, Shape::Preview).map(|p| p.size()),
        Some((40, 20))
    );

    // A file with no stored picture stays a plain tile and is not asked again.
    fetch(&store, OLD, || assert!(source.thumbnail(old, Shape::Square).is_none()));
    assert_eq!(thumbnail_requests(&fake), 3);
    assert!(source.thumbnail(old, Shape::Square).is_none());
    assert!(!store.busy(), "no second wish for it");
    assert_eq!(thumbnail_requests(&fake), 3);

    // The Files grid reads the same store, by file id.
    let grid = EngineThumbs::new(store.clone());
    assert!(grid.arrived(HARBOUR));
    assert_eq!(grid.pixels(HARBOUR).map(|p| p.size()), Some((20, 20)));
    assert!(!grid.arrived(OLD) && grid.pixels(OLD).is_none());
    assert_eq!(grid.sample(HARBOUR), None, "only a board fixture names a sample");
    // A video that is online only has no length here, and is never downloaded for one.
    assert_eq!(grid.video_length(TRAM), None);
    settle(&mut source);
    assert_eq!(grid.video_length(TRAM), None);
    assert_eq!(thumbnail_requests(&fake), 3);
}

#[test]
fn a_picture_does_not_stay_for_the_session_of_a_file_that_changes() {
    let fake = fake("stale");
    let (engine, mut source, store, _dir) = signed_in(&fake);
    open(&mut source);
    let harbour = shown(&source)[0][0];
    settle(&mut source);
    fetch(&store, HARBOUR, || {
        assert!(source.thumbnail(harbour, Shape::Square).is_none())
    });
    assert!(source.thumbnail(harbour, Shape::Square).is_some());
    assert_eq!(thumbnail_requests(&fake), 1);

    // Held for long: it is still shown, and fetched again.
    store.age_pictures(crate::engine_host::thumbs::REFRESH_AFTER);
    fetch(&store, HARBOUR, || {
        assert!(source.thumbnail(harbour, Shape::Square).is_some(), "shown meanwhile")
    });
    assert_eq!(thumbnail_requests(&fake), 2);
    assert!(source.thumbnail(harbour, Shape::Square).is_some());
    assert_eq!(thumbnail_requests(&fake), 2, "fresh again: not asked a third time");

    // The file changed (a sync check found a new version): its picture goes at the next read of the library.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    row(&db, HARBOUR, "Trip/Invented harbour.jpg", noon(0, 0) + 30, false);
    row(&db, OLD, "Invented old.png", noon(10, 0), false);
    db.set_owner(&Identity::new(Some(&fake.first.user_id), Some(&fake.first.email)))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(path);
    source.refresh();
    pump_until(&mut source, |source, _| source.days(None)[0].len == 1);
    assert_eq!(shown(&source)[0], [harbour], "the same file keeps its number");
    let (release, ended) = store.hold_next_fetch(HARBOUR);
    assert!(
        source.thumbnail(harbour, Shape::Square).is_none(),
        "the earlier version's picture is not shown"
    );
    release.send(()).unwrap();
    ended.recv_timeout(WAIT).expect("it is fetched again");
    assert_eq!(thumbnail_requests(&fake), 3);
}

#[test]
fn a_change_elsewhere_reads_the_library_again_only_once_someone_looked_at_it() {
    let fake = fake("refresh");
    let (_engine, mut source, _store, _dir) = signed_in(&fake);
    // Never opened: a finished check reads nothing.
    source.refresh();
    assert!(!source.pump().busy);
    assert_eq!(source.days(None), []);
    // Opened and fresh: a finished check reads again all the same.
    open(&mut source);
    settle(&mut source);
    source.refresh();
    assert!(source.pump().busy, "a read is on its way");
    settle(&mut source);
    assert_eq!(source.days(None)[0].len, 3);
}

#[test]
fn a_thumbnail_wished_under_one_account_is_never_fetched_or_kept_under_another() {
    let fake = fake("thumb-whose");
    let (engine, mut source, store, _dir) = signed_in(&fake);
    open(&mut source);
    let harbour = shown(&source)[0][0];

    // The wish waits on its worker; meanwhile the second account signs in.
    let (release, ended) = store.hold_next_fetch(HARBOUR);
    assert!(source.thumbnail(harbour, Shape::Square).is_none());
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    release.send(()).unwrap();
    ended.recv_timeout(WAIT).expect("the fetch ended");
    assert_eq!(thumbnail_requests(&fake), 0, "nothing was asked under any token");

    // The second account, by the first's file id: nothing of the first, and the server's refusal keeps nothing.
    let grid = EngineThumbs::new(store.clone());
    let _own = local_state(&engine, &fake.second, &[(HARBOUR, "Other/Invented harbour.jpg")]);
    fetch(&store, HARBOUR, || assert!(!grid.arrived(HARBOUR)));
    assert!(!grid.arrived(HARBOUR));
    assert!(grid.pixels(HARBOUR).is_none());
    // What was asked for it carried the second account's token, and Beebeeb answered that with no picture.
    let asked: Vec<bool> = fake
        .fake
        .requests()
        .iter()
        .filter(|r| is_thumbnail(r))
        .map(|r| r.bearer() == Some(fake.second.session_token.as_str()))
        .collect();
    assert_eq!(asked, [true]);
}

#[test]
fn a_move_to_trash_is_shown_only_once_beebeeb_answered_and_undo_puts_it_back() {
    let fake = fake("trash");
    let (_engine, mut source, _store, _dir) = signed_in(&fake);
    open(&mut source);
    let ids = shown(&source);
    let (harbour, shot, tram) = (ids[0][0], ids[0][1], ids[0][2]);

    let (reached, release) = hold_next_request(&fake);
    assert_eq!(source.trash(&[harbour, tram]), Ok(()), "accepted");
    reached.recv_timeout(WAIT).expect("the first reached Beebeeb");
    // Asked, not answered: nothing left the library, and nothing is said.
    assert_eq!(source.pump().answered, []);
    assert_eq!(shown(&source)[0], [harbour, shot, tram]);
    // A second press on one of them sends nothing and says so.
    assert_eq!(source.trash(&[harbour]), Err(Left::InFlight));
    release.send(()).unwrap();

    let answered = pump_until(&mut source, |_, answered| !answered.is_empty());
    assert_eq!(answered, [Answered::Trashed(vec![(harbour, None), (tram, None)])]);
    assert_eq!(
        sent(&fake),
        [
            (format!("DELETE /api/v1/files/{HARBOUR}"), "first"),
            (format!("DELETE /api/v1/files/{TRAM}"), "first")
        ]
    );
    assert_eq!(shown(&source)[0], [shot], "they left the library");
    assert_eq!(source.viewing(harbour), None);
    assert!(!fake.first.trash_item(HARBOUR), "Beebeeb has it in Trash already");
    // The read after the answer still finds them in the local state (it follows later): they stay out.
    assert_eq!(settle(&mut source), []);
    assert_eq!(shown(&source)[0], [shot]);

    // Undo: under the session of the move; they stand in the library again, with the numbers they had.
    assert_eq!(source.restore(&[harbour, tram]), Ok(()));
    let answered = pump_until(&mut source, |_, answered| !answered.is_empty());
    assert_eq!(answered, [Answered::Restored(vec![(harbour, None), (tram, None)])]);
    assert_eq!(shown(&source)[0], [harbour, shot, tram]);
    assert!(fake.first.trash_item(HARBOUR), "Beebeeb put it back");
    assert_eq!(settle(&mut source), []);
    assert_eq!(shown(&source)[0], [harbour, shot, tram]);

    // Put back: no longer what Undo is for.
    assert_eq!(source.restore(&[tram]), Err(Left::Unavailable));
    assert_eq!(sent(&fake).len(), 4, "a refused Undo sends nothing");
}

#[test]
fn what_beebeeb_refuses_is_said_per_item_and_another_accounts_photo_changes_nothing() {
    let fake = fake("refused");
    let engine = engine_at(&fake.fake.base);
    sign_in(&engine, &fake.first, &fake.first_phrase);
    // The local state lists a file that is the second account's at Beebeeb.
    let _dir = local_state(&engine, &fake.first, &[(OTHERS, "Other/Second account shot.jpg")]);
    let mut source = EnginePhotos::new(engine.clone(), EngineThumbStore::new(engine.clone()));
    open(&mut source);
    let ids = shown(&source);
    let (harbour, others) = (ids[0][0], ids[2][0]);

    // Another account's photo, by its id: as if it were not there. It stays, here and at Beebeeb.
    assert_eq!(source.trash(&[others]), Ok(()));
    let answered = pump_until(&mut source, |_, answered| !answered.is_empty());
    assert_eq!(answered, [Answered::Trashed(vec![(others, Some(Left::Failed))])]);
    assert_eq!(settle(&mut source), []);
    assert_eq!(shown(&source)[2], [others], "nothing left the library");
    assert!(
        fake.second.trash_item(OTHERS),
        "the second account's file was not in Trash"
    );
    fake.second.restore_item(OTHERS);
    assert_eq!(
        source.restore(&[others]),
        Err(Left::Unavailable),
        "and there is nothing to undo"
    );

    // A refusal of the server: said for that item, which stays; Try again works once it answers.
    fake.trash_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert_eq!(source.trash(&[harbour]), Ok(()));
    let answered = pump_until(&mut source, |_, answered| !answered.is_empty());
    assert_eq!(answered, [Answered::Trashed(vec![(harbour, Some(Left::Failed))])]);
    assert_eq!(shown(&source)[0][0], harbour);
    fake.trash_mode.store(ANSWER, Ordering::SeqCst);
    assert_eq!(source.trash(&[harbour]), Ok(()), "the mark of the failed one is gone");
    let answered = pump_until(&mut source, |_, answered| !answered.is_empty());
    assert_eq!(answered, [Answered::Trashed(vec![(harbour, None)])]);
}

#[test]
fn an_answer_that_lands_under_another_account_is_dropped() {
    let fake = fake("late");
    let (engine, mut source, _store, _dir) = signed_in(&fake);
    open(&mut source);
    let harbour = shown(&source)[0][0];

    // The move is at Beebeeb, sent under the first account's session, when the second account signs in.
    let (reached, release) = hold_next_request(&fake);
    assert_eq!(source.trash(&[harbour]), Ok(()));
    reached.recv_timeout(WAIT).expect("it reached Beebeeb");
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    release.send(()).unwrap();
    // Whatever the worker answers, nothing of it is handed to the window of the second account. (Looked at for a
    // while: an answer handed over would be there within it.)
    let until = Instant::now() + Duration::from_millis(400);
    while Instant::now() < until {
        assert_eq!(source.pump().answered, []);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(source.days(None), []);
    assert_eq!(
        source.restore(&[harbour]),
        Err(Left::Unavailable),
        "nothing is held for Undo"
    );
    assert_eq!(sent(&fake), [(format!("DELETE /api/v1/files/{HARBOUR}"), "first")]);
}

#[test]
fn keep_on_this_pc_is_said_only_once_this_pc_answered() {
    let fake = fake("keep");
    let (engine, mut source, _store, _dir) = signed_in(&fake);
    open(&mut source);
    let harbour = shown(&source)[0][0];

    // This PC keeps it: said once answered, and the library says it is kept.
    source.keep_says = Some(Ok(()));
    assert_eq!(source.keep_offline(harbour, true), Ok(()));
    assert!(!source.kept_offline(harbour), "not before the answer");
    let answered = pump_until(&mut source, |_, answered| !answered.is_empty());
    assert_eq!(
        answered,
        [Answered::Kept {
            id: harbour,
            keep: true,
            left: None
        }]
    );
    assert!(source.kept_offline(harbour));
    settle(&mut source);

    // The engine itself, with no sync folder set on this PC: refused, said, and nothing is drawn as kept.
    let tram = shown(&source)[0][2];
    assert_eq!(source.keep_offline(tram, true), Ok(()));
    let answered = pump_until(&mut source, |_, answered| !answered.is_empty());
    assert_eq!(
        answered,
        [Answered::Kept {
            id: tram,
            keep: true,
            left: Some(Left::Failed)
        }]
    );
    assert!(!source.kept_offline(tram));
    settle(&mut source);

    // An answer for a session that is gone is dropped.
    source.keep_says = Some(Ok(()));
    assert_eq!(source.keep_offline(tram, true), Ok(()));
    engine.lock();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(settle(&mut source), []);
    assert_eq!(
        source.keep_offline(tram, true),
        Err(Left::Unavailable),
        "locked: nothing is asked"
    );
}
