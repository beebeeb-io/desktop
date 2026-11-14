//! The photo library and thumbnails at the engine (`Engine::photos_library`, `Engine::thumbnail_pixels_for`,
//! `Engine::media_facts_for`, `Engine::set_kept_for`), against the engine's own doubles: two invented accounts on
//! the fake API on loopback, each stored picture encrypted by core's thumbnail sealer under that file's key, a state
//! database of each test's own (`Engine::use_local_state_for_test`) recorded as one account's. Sessions are put in
//! memory as a sign-in and an unlock leave them. Every name, key and pixel is invented.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};

use super::super::{Engine, EngineConfig};
use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use crate::test_api::{FakeApi, Handler, Reply, Request};
use beebeeb_types::backup::SystemRole;

type TestEngine = Engine<MemoryCredentials>;

const HARBOUR: &str = "b0000000-0000-4000-8000-000000000601";
const TRAM: &str = "b0000000-0000-4000-8000-000000000602";
const NOTE: &str = "b0000000-0000-4000-8000-000000000603";
const BACKUPS: &str = "b0000000-0000-4000-8000-000000000604";
const PHONE_SHOT: &str = "b0000000-0000-4000-8000-000000000605";
const NO_PICTURE: &str = "b0000000-0000-4000-8000-000000000606";
const WIDE: &str = "b0000000-0000-4000-8000-000000000607";
const BROKEN: &str = "b0000000-0000-4000-8000-000000000608";
const THEIRS: &str = "b0000000-0000-4000-8000-000000000701";

const ANSWER: u8 = 0;
const SERVER_ERROR: u8 = 1;
const SESSION_ENDED: u8 = 2;

/// A picture `w` by `h`: red on its left half, blue on its right, as a PNG.
fn picture(w: u32, h: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_fn(w, h, |x, _| {
        if x < w / 2 {
            image::Rgba([200, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 200, 255])
        }
    });
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    png.into_inner()
}

/// The stored picture of `id`, sealed under `account`'s key for that file.
fn sealed(account: &FakeAccount, id: &str, plain: &[u8]) -> Vec<u8> {
    account.seal_thumbnail(id, plain)
}

/// Holds every thumbnail request until released, and says when one arrived.
#[derive(Default)]
struct Hold {
    on: AtomicBool,
    released: Mutex<bool>,
    wake: Condvar,
    arrived: Mutex<Option<mpsc::Sender<()>>>,
}

impl Hold {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }

    fn wait(&self) {
        if !self.on.load(Ordering::SeqCst) {
            return;
        }
        if let Some(arrived) = self.arrived.lock().unwrap().as_ref() {
            let _ = arrived.send(());
        }
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.wake.wait(released).unwrap();
        }
    }
}

struct Fake {
    api: FakeApi,
    first: Arc<FakeAccount>,
    second: Arc<FakeAccount>,
    /// How a thumbnail request answers.
    mode: Arc<AtomicU8>,
    hold: Arc<Hold>,
}

fn is_thumbnail(req: &Request) -> bool {
    req.method == "GET" && req.path.contains("/thumbnail/")
}

fn fake(tag: &str) -> Fake {
    let (first, _) = FakeAccount::register(&format!("{tag}-one@example.eu"), "an invented password", TEST_PHRASE);
    let (second, _) = FakeAccount::register(
        &format!("{tag}-two@example.eu"),
        "an invented password",
        "legal winner thank year wave sausage worth useful legal winner thank yellow",
    );
    let (first, second) = (Arc::new(first), Arc::new(second));
    for (id, name) in [
        (HARBOUR, "Invented harbour.jpg"),
        (NO_PICTURE, "Invented plain.jpg"),
        (WIDE, "Invented wide.jpg"),
        (BROKEN, "Invented broken.jpg"),
    ] {
        first.add_file_with_content(id, None, name, b"not read here");
    }
    second.add_file_with_content(THEIRS, None, "Second account shot.jpg", b"not read here");
    // What the server stores per (owner's token, file, size): the sealed pictures.
    let stored: Vec<(String, String, Vec<u8>)> = vec![
        (
            first.session_token.clone(),
            format!("{HARBOUR}/thumbnail/medium"),
            sealed(&first, HARBOUR, &picture(40, 20)),
        ),
        (
            first.session_token.clone(),
            format!("{HARBOUR}/thumbnail/small"),
            sealed(&first, HARBOUR, &picture(40, 20)),
        ),
        (
            first.session_token.clone(),
            format!("{HARBOUR}/thumbnail/large"),
            sealed(&first, HARBOUR, &picture(40, 20)),
        ),
        (
            first.session_token.clone(),
            format!("{WIDE}/thumbnail/medium"),
            sealed(&first, WIDE, &picture(MAX_SIDE + 1, 2)),
        ),
        (
            first.session_token.clone(),
            format!("{BROKEN}/thumbnail/medium"),
            sealed(&first, BROKEN, b"these bytes are no picture"),
        ),
        (
            second.session_token.clone(),
            format!("{THEIRS}/thumbnail/medium"),
            sealed(&second, THEIRS, &picture(8, 8)),
        ),
    ];
    let (one, two): (Handler, Handler) = (account_handler(first.clone()), account_handler(second.clone()));
    let second_token = second.session_token.clone();
    let (mode, hold) = (Arc::new(AtomicU8::new(ANSWER)), Arc::new(Hold::default()));
    let (mode_in, hold_in) = (mode.clone(), hold.clone());
    let api = FakeApi::start(Arc::new(move |req: &Request| {
        if is_thumbnail(req) {
            hold_in.wait();
            match mode_in.load(Ordering::SeqCst) {
                SERVER_ERROR => return Reply::json(500, serde_json::json!({ "error": "a server body" })),
                SESSION_ENDED => return Reply::json(401, serde_json::json!({ "error": "a server body" })),
                _ => {}
            }
            // Only the owner's token is answered with a picture.
            let found = stored
                .iter()
                .find(|(token, path, _)| req.bearer() == Some(token.as_str()) && req.path.ends_with(path.as_str()));
            return match found {
                Some((_, _, blob)) => Reply::bytes(200, blob.clone()),
                None => Reply::json(404, serde_json::json!({ "error": "a server body" })),
            };
        }
        if req.bearer() == Some(second_token.as_str()) {
            two(req)
        } else {
            one(req)
        }
    }));
    Fake {
        api,
        first,
        second,
        mode,
        hold,
    }
}

fn engine_at(base: &str) -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-photos-slot".into(),
        state_dir,
        api_base: base.to_string(),
    };
    Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

/// A session in memory, as a sign-in and an unlock of `account` leave it.
fn signed_in(engine: &TestEngine, account: &FakeAccount) {
    let _writes = engine.writes.write().unwrap();
    engine
        .acct
        .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    *engine.acct.session.lock().unwrap() = Some(Session {
        token: account.session_token.clone(),
        master_key: account.master_key,
        email: Some(account.email.clone()),
    });
    engine.remember_session_user_id(Some(account.user_id.clone()));
    engine.set_auth_present(true);
}

fn row(db: &StateDb, id: &str, path: &str, modified: i64, status: FileStatus, folder: bool) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status,
        size_bytes: if folder { 0 } else { 1_000 },
        modified_at: modified,
        content_hash: None,
        remote_updated_at: modified,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

/// A state database recorded as `owner`'s, and the engine reading it.
fn local_state(engine: &TestEngine, owner: &FakeAccount) -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    row(
        &db,
        HARBOUR,
        "Trip/Invented harbour.jpg",
        300,
        FileStatus::CloudOnly,
        false,
    );
    row(&db, TRAM, "Trip/Invented tram.mov", 500, FileStatus::Uploading, false);
    row(&db, NOTE, "Trip/Invented note.txt", 900, FileStatus::CloudOnly, false);
    row(&db, BACKUPS, "Backups", 100, FileStatus::CloudOnly, true);
    db.set_system_role(BACKUPS, Some(SystemRole::BackupsRoot)).unwrap();
    row(
        &db,
        PHONE_SHOT,
        "Backups/Invented phone/Camera/Invented shot.heic",
        400,
        FileStatus::Local,
        false,
    );
    for id in [NO_PICTURE, WIDE, BROKEN, THEIRS] {
        row(&db, id, &format!("Other/{id}.jpg"), 10, FileStatus::CloudOnly, false);
    }
    db.set_recursive_pin(PHONE_SHOT, true, 1).unwrap();
    db.set_owner(&Identity::new(Some(&owner.user_id), Some(&owner.email)))
        .unwrap();
    engine.use_local_state_for_test(path);
    (dir, db)
}

fn thumbnail_requests(fake: &Fake) -> usize {
    fake.api.requests().iter().filter(|req| is_thumbnail(req)).count()
}

fn pixel(pixels: &ThumbPixels, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * pixels.width + x) * 4) as usize;
    pixels.rgba[at..at + 4].try_into().unwrap()
}

#[test]
fn the_library_is_the_photos_and_videos_of_the_local_state_newest_first() {
    let fake = fake("library");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);

    let library = engine.photos_library().expect("the library");
    assert_eq!(library.session, engine.session_turn());
    let ids: Vec<&str> = library.photos.iter().map(|photo| photo.id.as_str()).collect();
    // Newest first; the note and the folder are no photos. The four at the same second: by path.
    assert_eq!(&ids[..3], [TRAM, PHONE_SHOT, HARBOUR]);
    assert_eq!(ids.len(), 7);
    assert!(!ids.contains(&NOTE) && !ids.contains(&BACKUPS));

    let tram = &library.photos[0];
    assert!(tram.video && tram.uploading && !tram.kept && !tram.on_this_pc);
    assert_eq!(tram.name, "Invented tram.mov");
    assert_eq!(tram.folders, ["Trip"]);
    assert_eq!((tram.bytes, tram.modified_at, tram.versions), (1_000, 500, 1));
    assert_eq!(tram.device, None);

    let shot = &library.photos[1];
    assert!(!shot.video && shot.kept && shot.on_this_pc && !shot.uploading);
    assert_eq!(shot.device.as_deref(), Some("Invented phone"));
    assert_eq!(shot.folders, ["Backups", "Invented phone", "Camera"]);

    // A `{:?}` prints no name and no path.
    let printed = format!("{library:?} {tram:?}");
    assert!(!printed.contains("Invented") && !printed.contains("Trip"), "{printed}");
}

#[test]
fn the_library_answers_nothing_signed_out_or_for_another_accounts_state() {
    let fake = fake("library-refused");
    let engine = engine_at(&fake.api.base);
    assert_eq!(engine.photos_library().unwrap_err(), LocalStateRefused::SignedOut);
    signed_in(&engine, &fake.second);
    let _state = local_state(&engine, &fake.first);
    assert_eq!(engine.photos_library().unwrap_err(), LocalStateRefused::OtherAccount);
}

#[test]
fn a_thumbnail_is_decrypted_in_memory_and_cut_as_asked() {
    let fake = fake("pixels");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let (_dir, db) = local_state(&engine, &fake.first);
    let session = engine.session_turn();

    // The stored picture is 40 by 20, red left and blue right. The square is its middle 20 by 20.
    let square = engine
        .thumbnail_pixels_for(session, HARBOUR, ThumbCut::Square)
        .expect("the square");
    assert_eq!((square.width, square.height, square.rgba.len()), (20, 20, 20 * 20 * 4));
    assert_eq!(square.session, session);
    assert_eq!(pixel(&square, 0, 0), [200, 0, 0, 255]);
    assert_eq!(pixel(&square, 19, 19), [0, 0, 200, 255]);

    // The strip: upright, 36 by 52 of the middle (13 wide at 20 high).
    let strip = engine
        .thumbnail_pixels_for(session, HARBOUR, ThumbCut::Strip)
        .expect("the strip");
    assert_eq!((strip.width, strip.height), (13, 20));

    // The whole picture, at its own proportions.
    let whole = engine
        .thumbnail_pixels_for(session, HARBOUR, ThumbCut::Whole)
        .expect("the whole");
    assert_eq!((whole.width, whole.height), (40, 20));
    assert_eq!(pixel(&whole, 0, 0), [200, 0, 0, 255]);
    assert_eq!(pixel(&whole, 39, 0), [0, 0, 200, 255]);

    let asked: Vec<String> = fake
        .api
        .requests()
        .iter()
        .filter(|req| is_thumbnail(req))
        .map(|req| req.path.rsplit('/').next().unwrap().to_string())
        .collect();
    assert_eq!(asked, ["medium", "small", "large"]);

    // Nothing about the file is recorded, and a `{:?}` prints no pixel.
    assert_eq!(db.get_file(HARBOUR).unwrap().unwrap().status, FileStatus::CloudOnly);
    assert_eq!(db.kept_rows().unwrap().len(), 1, "only the row the test kept");
    assert!(!format!("{square:?}").contains("200"));
}

#[test]
fn a_file_with_no_stored_picture_and_another_accounts_file_hand_over_nothing() {
    let fake = fake("none");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);
    let session = engine.session_turn();
    let ask = |id: &str| engine.thumbnail_pixels_for(session, id, ThumbCut::Square).map(|_| ());

    assert_eq!(ask(NO_PICTURE), Err(ThumbRefused::NoThumbnail));
    // The second account's file, by its id, under the first account's session: as if it did not exist.
    assert_eq!(ask(THEIRS), Err(ThumbRefused::NoThumbnail));
    // And the second account does get its own.
    signed_in(&engine, &fake.second);
    let _theirs = local_state(&engine, &fake.second);
    let theirs = engine
        .thumbnail_pixels_for(engine.session_turn(), THEIRS, ThumbCut::Square)
        .expect("the owner's picture");
    assert_eq!((theirs.width, theirs.height), (8, 8));
}

#[test]
fn a_picture_too_wide_or_that_does_not_decode_is_refused() {
    let fake = fake("unreadable");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);
    let session = engine.session_turn();
    let ask = |id: &str| engine.thumbnail_pixels_for(session, id, ThumbCut::Square).map(|_| ());
    assert_eq!(ask(WIDE), Err(ThumbRefused::Unreadable));
    assert_eq!(ask(BROKEN), Err(ThumbRefused::Unreadable));
    // At the limit it decodes.
    assert!(cut_pixels(&picture(MAX_SIDE, 2), ThumbCut::Whole).is_some());
    // A square is handed over no larger than its cap.
    let (w, h, _) = cut_pixels(&picture(MAX_SQUARE * 2, MAX_SQUARE * 3), ThumbCut::Square).unwrap();
    assert_eq!((w, h), (MAX_SQUARE, MAX_SQUARE));
    // And a whole picture no wider than its cap, at its own proportions.
    let (w, h, _) = cut_pixels(&picture(MAX_WHOLE * 2, MAX_WHOLE / 2), ThumbCut::Whole).unwrap();
    assert_eq!((w, h), (MAX_WHOLE, MAX_WHOLE / 4));
}

#[test]
fn a_refusal_of_the_server_has_its_kind() {
    let fake = fake("kinds");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);
    let session = engine.session_turn();
    let ask = || {
        engine
            .thumbnail_pixels_for(session, HARBOUR, ThumbCut::Square)
            .map(|_| ())
    };
    fake.mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert_eq!(ask(), Err(ThumbRefused::Failed));
    fake.mode.store(SESSION_ENDED, Ordering::SeqCst);
    assert_eq!(ask(), Err(ThumbRefused::SessionEnded));
    fake.mode.store(ANSWER, Ordering::SeqCst);
    assert_eq!(ask(), Ok(()));
}

#[test]
fn a_thumbnail_asked_for_an_earlier_session_sends_nothing() {
    let fake = fake("earlier");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);
    let earlier = engine.session_turn();
    // The same account signs in again: another session.
    signed_in(&engine, &fake.first);
    assert_ne!(earlier, engine.session_turn());
    assert_eq!(
        engine
            .thumbnail_pixels_for(earlier, HARBOUR, ThumbCut::Square)
            .map(|_| ()),
        Err(ThumbRefused::NotSignedIn)
    );
    assert_eq!(thumbnail_requests(&fake), 0);
    // And Keep for that earlier session changes nothing (refused before anything is read).
    assert_eq!(
        engine.set_kept_for(earlier, HARBOUR, true).err().as_deref(),
        Some(crate::app::kept_api::NOT_SIGNED_IN)
    );
}

#[test]
fn a_thumbnail_whose_session_ends_while_it_is_fetched_is_not_handed_over() {
    let fake = fake("ends");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);
    let session = engine.session_turn();
    let (arrived, at_server) = mpsc::channel();
    *fake.hold.arrived.lock().unwrap() = Some(arrived);
    fake.hold.on.store(true, Ordering::SeqCst);

    let worker = {
        let engine = engine.clone();
        std::thread::spawn(move || {
            engine
                .thumbnail_pixels_for(session, HARBOUR, ThumbCut::Square)
                .map(|_| ())
        })
    };
    at_server.recv().expect("the request reached the server");
    // Another account signs in while the server holds the answer.
    signed_in(&engine, &fake.second);
    let answer = worker.join().unwrap();
    fake.hold.release();
    assert!(
        matches!(answer, Err(ThumbRefused::NotSignedIn)),
        "nothing of the first session is handed over: {answer:?}"
    );
}

#[test]
fn a_videos_length_is_read_only_from_a_copy_on_this_pc_and_only_for_the_session_asked() {
    let fake = fake("length");
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let _state = local_state(&engine, &fake.first);
    let session = engine.session_turn();
    // Online only, and a photo that is on this PC: nothing is read, nothing is downloaded.
    assert_eq!(engine.video_seconds_for(session, TRAM), Ok(None));
    assert_eq!(engine.video_seconds_for(session, PHONE_SHOT), Ok(None));
    assert_eq!(fake.api.requests().len(), 0);
    signed_in(&engine, &fake.first);
    assert_eq!(
        engine.video_seconds_for(session, TRAM),
        Err(LocalStateRefused::SessionChanged)
    );
}

#[test]
fn the_head_of_a_copy_says_a_pictures_size() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("Trip")).unwrap();
    std::fs::write(dir.path().join("Trip").join("shot.png"), picture(31, 17)).unwrap();
    let facts = facts_of_copy(dir.path(), "Trip/shot.png", false);
    assert_eq!((facts.width, facts.height), (Some(31), Some(17)));
    // A path that leaves the folder, and a file that is not there: nothing.
    assert_eq!(facts_of_copy(dir.path(), "../shot.png", false), PhotoMeta::default());
    assert_eq!(facts_of_copy(dir.path(), "Trip/none.png", false), PhotoMeta::default());
}
