//! The preview run and the setting at the engine (`Engine::make_previews_for`, `Engine::photo_previews_for`,
//! `Engine::set_photo_previews_for`, `Engine::measure_sample_for`), against the engine's own doubles: an invented
//! account on the fake API with the preview routes (`test_api::fake_previews`), a state database of each test's own,
//! a sync folder and a settings folder of each test's own with real JPEG originals in it. Sessions are put in memory
//! as a sign-in and an unlock leave them. Every name, key and picture is invented.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use super::super::{Engine, EngineConfig};
use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::config::{CONFIG_DIR_OVERRIDE, DesktopConfig};
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::photo_previews::runner::Stopped;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use crate::test_api::fake_previews::PreviewRoutes;
use crate::test_api::{FakeApi, Request};

type TestEngine = Engine<MemoryCredentials>;

const ONE: &str = "d0000000-0000-4000-8000-000000000001";
const TWO: &str = "d0000000-0000-4000-8000-000000000002";
const THREE: &str = "d0000000-0000-4000-8000-000000000003";
const CLOUD: &str = "d0000000-0000-4000-8000-000000000004";

const JPEG_1920: PreviewSetting = PreviewSetting {
    enabled: true,
    format: PreviewFormat::Jpeg,
    px: 1920,
};

struct World {
    api: FakeApi,
    account: Arc<FakeAccount>,
    previews: Arc<PreviewRoutes>,
    engine: Arc<TestEngine>,
    root: tempfile::TempDir,
    config_dir: tempfile::TempDir,
    db_dir: tempfile::TempDir,
}

fn engine_at(base: &str) -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-previews-slot".into(),
        state_dir,
        api_base: base.to_string(),
    };
    Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

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

/// A lock and an unlock: the same keys in memory, a new turn of the session.
fn lock_and_unlock(engine: &TestEngine) {
    let _writes = engine.writes.write().unwrap();
    engine
        .acct
        .advance_session_generation_holding_session_write_lock(SessionTransition::Lock);
}

/// The photos of the world: (id, name, status, picture). Each picture is written into the sync folder, and recorded
/// in the local state with its own size.
fn world(tag: &str, photos: &[(&str, &str, FileStatus, Vec<u8>)]) -> World {
    let (account, _) = FakeAccount::register(&format!("{tag}@example.eu"), "an invented password", TEST_PHRASE);
    let account = Arc::new(account);
    let previews = Arc::new(PreviewRoutes::new(account.clone()));
    let ids: Vec<&str> = photos.iter().map(|(id, ..)| *id).collect();
    previews.list_media(&ids);
    for (id, _, _, picture) in photos {
        // The server's version is the one this PC holds: unchanged since (0) the row's version, the row's size.
        previews.set_version(id, 0, picture.len() as u64);
    }
    let (routes, own) = (previews.clone(), account_handler(account.clone()));
    let api = FakeApi::start(Arc::new(move |req: &Request| {
        // The thumbnail upload (not the preview's): stored, answered.
        if req.method == "PUT" && req.path.ends_with("/thumbnail") {
            return crate::test_api::Reply::ok(serde_json::json!({}));
        }
        routes.answer(req).unwrap_or_else(|| own(req))
    }));
    let engine = engine_at(&api.base);
    signed_in(&engine, &account);
    let root = tempfile::tempdir().unwrap();
    let db_dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(db_dir.path().join("state.db")).unwrap();
    std::fs::create_dir_all(root.path().join("Photos")).unwrap();
    for (at, (id, name, status, picture)) in photos.iter().enumerate() {
        let rel = format!("Photos/{name}");
        std::fs::write(root.path().join(&rel), picture).unwrap();
        let entry = FileEntry {
            file_id: id.to_string(),
            path: rel,
            status: status.clone(),
            size_bytes: picture.len() as i64,
            modified_at: 1_000 - at as i64,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        };
        db.upsert_file_with_contract(&entry, |contract| contract.item_kind = ItemKind::File)
            .unwrap();
    }
    db.set_owner(&Identity::new(Some(&account.user_id), Some(&account.email)))
        .unwrap();
    engine.use_local_state_for_test(db_dir.path().join("state.db"));
    World {
        api,
        account,
        previews,
        engine,
        root,
        config_dir: tempfile::tempdir().unwrap(),
        db_dir,
    }
}

impl World {
    /// `body` with this world's settings folder, its sync folder named in it.
    fn run<T>(&self, body: impl FnOnce() -> T) -> T {
        let root = self.root.path().to_path_buf();
        CONFIG_DIR_OVERRIDE.sync_scope(self.config_dir.path().to_path_buf(), || {
            DesktopConfig::update_for(&TEST, |config| {
                config.sync_root = Some(root);
                (true, ())
            })
            .unwrap();
            body()
        })
    }

    fn make(&self, force: bool) -> RunEnd {
        let stop = AtomicBool::new(false);
        let session = self.engine.session_turn();
        self.run(|| self.engine.make_previews_for(session, force, &stop, &|_, _| {}))
    }

    fn preview_puts(&self) -> Vec<Request> {
        self.api
            .requests()
            .into_iter()
            .filter(|req| req.method == "PUT" && req.path.contains("/thumbnail/preview"))
            .collect()
    }
}

fn jpeg(w: u32, h: u32) -> Vec<u8> {
    crate::photo_previews::tests::jpeg(w, h)
}

#[test]
fn run_put_matches_bytes() {
    let world = world(
        "put",
        &[(ONE, "Invented harbour.jpg", FileStatus::Local, jpeg(2400, 1800))],
    );
    world.previews.set_setting(JPEG_1920);
    let end = world.make(false);
    assert_eq!((end.made, end.failed, end.not_here, end.stopped), (1, 0, 0, None));
    let puts = world.preview_puts();
    assert_eq!(puts.len(), 1);
    assert_eq!(puts[0].query("format"), Some("jpeg"));
    assert_eq!(puts[0].query("px"), Some("1920"));
    assert_eq!(puts[0].header("content-type"), Some("application/octet-stream"));
    let stored = world.previews.stored(ONE).expect("stored");
    assert!(
        !stored.body.starts_with(&[0xFF, 0xD8]),
        "sealed, never the JPEG's bytes"
    );
    let plain = seal::open_preview(&world.account.master_key, ONE, &stored.body).expect("opens with the recipe");
    assert!(plain.starts_with(&[0xFF, 0xD8]));
    let picture = image::load_from_memory_with_format(&plain, image::ImageFormat::Jpeg).unwrap();
    assert_eq!((picture.width(), picture.height()), (1920, 1440));
    // Nothing in the request but the sealed bytes and the format and size: no name, no path.
    let request = format!("{} {:?}", puts[0].path, String::from_utf8_lossy(&puts[0].body));
    assert!(
        !request.contains("Invented") && !request.contains("Photos"),
        "{request}"
    );
    // A second run finds it made.
    assert_eq!(world.make(false).made, 0);
    assert_eq!(world.preview_puts().len(), 1);
}

#[test]
fn run_makes_nothing_while_heic() {
    let world = world(
        "heic",
        &[(ONE, "Invented harbour.jpg", FileStatus::Local, jpeg(1600, 1200))],
    );
    world.previews.set_setting(PreviewSetting {
        format: PreviewFormat::Heic,
        ..JPEG_1920
    });
    let end = world.make(true);
    assert_eq!(end.stopped, Some(Stopped::HeicNotMadeHere));
    assert_eq!(world.previews.puts(), 0);
    // The same photo with JPEG chosen is made (the check can fail).
    world.previews.set_setting(JPEG_1920);
    assert_eq!(world.make(true).made, 1);
}

#[test]
fn plan_reads_only_local_originals() {
    let world = world(
        "local",
        &[
            (ONE, "Invented here.jpg", FileStatus::Local, jpeg(1600, 1200)),
            // A placeholder: on disk here only so a read of it would succeed if it were asked.
            (CLOUD, "Invented cloud.jpg", FileStatus::CloudOnly, jpeg(1600, 1200)),
        ],
    );
    world.previews.set_setting(JPEG_1920);
    let opened: Mutex<Vec<PathBuf>> = Mutex::default();
    let read = |path: &Path| {
        opened.lock().unwrap().push(path.to_path_buf());
        read_original(path)
    };
    let stop = AtomicBool::new(false);
    let session = world.engine.session_turn();
    let end = world.run(|| {
        world
            .engine
            .make_previews_reading(session, false, &stop, &|_, _| {}, &read)
    });
    assert_eq!(end.made, 1);
    let opened = opened.lock().unwrap();
    assert_eq!(opened.len(), 1);
    assert!(opened[0].ends_with("Invented here.jpg"));
    assert!(world.previews.stored(CLOUD).is_none());
}

#[test]
fn run_stops_after_lock_unlock() {
    let world = world(
        "lock",
        &[
            (ONE, "Invented one.jpg", FileStatus::Local, jpeg(1600, 1200)),
            (TWO, "Invented two.jpg", FileStatus::Local, jpeg(1600, 1200)),
            (THREE, "Invented three.jpg", FileStatus::Local, jpeg(1600, 1200)),
        ],
    );
    world.previews.set_setting(JPEG_1920);
    let stop = AtomicBool::new(false);
    let session = world.engine.session_turn();
    // After the first photo was sent, the screen locks and unlocks: the keys are the same, the session is not.
    let progress = |done: u32, _: u32| {
        if done == 1 {
            lock_and_unlock(&world.engine);
        }
    };
    let opened = std::sync::atomic::AtomicUsize::new(0);
    let read = |path: &Path| {
        opened.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        read_original(path)
    };
    let end = world.run(|| {
        world
            .engine
            .make_previews_reading(session, false, &stop, &progress, &read)
    });
    assert!(end.session_moved);
    assert_eq!((end.made, world.preview_puts().len()), (1, 1));
    assert_eq!(
        opened.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the next photo is never opened after the lock"
    );
    // A run for the session in memory now makes the rest (the check can pass).
    assert_eq!(world.make(false).made, 2);
}

#[test]
fn run_stops_once_on_quota() {
    let world = world(
        "quota",
        &[
            (ONE, "Invented one.jpg", FileStatus::Local, jpeg(1600, 1200)),
            (TWO, "Invented two.jpg", FileStatus::Local, jpeg(1600, 1200)),
            (THREE, "Invented three.jpg", FileStatus::Local, jpeg(1600, 1200)),
        ],
    );
    world.previews.set_setting(JPEG_1920);
    world.previews.fill_storage(true);
    let end = world.make(false);
    assert_eq!(end.stopped, Some(Stopped::StorageFull));
    assert_eq!((end.made, world.previews.puts()), (0, 1));
}

/// A 429 is waited out (its `Retry-After`) and the same preview sent again: stored, never counted as failed.
#[test]
fn run_waits_out_a_429_and_sends_again() {
    let world = world(
        "rate",
        &[(ONE, "Invented one.jpg", FileStatus::Local, jpeg(1600, 1200))],
    );
    world.previews.set_setting(JPEG_1920);
    world.previews.rate_limit_next(1, 0);
    let end = world.make(false);
    assert_eq!((end.made, end.failed, end.stopped), (1, 0, None));
    assert_eq!(world.previews.puts(), 2, "the 429, then the same preview again");
    assert!(world.previews.stored(ONE).is_some());
}

/// While a 429 is waited out the screen locks: the session is asked again before the preview would be sent again,
/// and nothing more is sent.
#[test]
fn a_429_wait_sends_nothing_for_a_session_that_moved() {
    let world = world(
        "rate-lock",
        &[(ONE, "Invented one.jpg", FileStatus::Local, jpeg(1600, 1200))],
    );
    world.previews.set_setting(JPEG_1920);
    world.previews.rate_limit_next(1, 2);
    let stop = AtomicBool::new(false);
    let session = world.engine.session_turn();
    let (previews, engine) = (world.previews.clone(), world.engine.clone());
    let locker = std::thread::spawn(move || {
        let began = std::time::Instant::now();
        while previews.puts() == 0 {
            assert!(
                began.elapsed() < std::time::Duration::from_secs(20),
                "no preview PUT came"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // The 429 is answered; the run waits 2 s. The screen locks and unlocks now.
        lock_and_unlock(&engine);
    });
    let end = world.run(|| world.engine.make_previews_for(session, false, &stop, &|_, _| {}));
    locker.join().unwrap();
    assert!(end.session_moved);
    assert_eq!(
        (end.made, world.previews.puts()),
        (0, 1),
        "nothing is sent after the lock"
    );
}

#[test]
fn off_never_deletes_previews() {
    let world = world("off", &[(ONE, "Invented one.jpg", FileStatus::Local, jpeg(1600, 1200))]);
    world.previews.store(ONE, "jpeg", 1920, vec![1, 2, 3]);
    world.previews.set_setting(JPEG_1920);
    let session = world.engine.session_turn();
    let off = PreviewSetting {
        enabled: false,
        ..JPEG_1920
    };
    world.engine.set_photo_previews_for(session, off).unwrap();
    assert_eq!(world.previews.preference_puts(), [off.to_value()]);
    assert_eq!(world.previews.deletes(), 0);
    assert!(world.previews.stored(ONE).is_some());
    // And a run with the setting off makes nothing.
    assert_eq!(world.make(true), RunEnd::default());
    assert_eq!(world.previews.puts(), 0);
}

#[test]
fn failed_read_is_not_off() {
    let world = world(
        "failed",
        &[(ONE, "Invented one.jpg", FileStatus::Local, jpeg(1600, 1200))],
    );
    let session = world.engine.session_turn();
    world.previews.fail_the_preference_read(true);
    assert_eq!(
        world.engine.photo_previews_setting_for(session).unwrap_err(),
        PreviewsRefused::Failed
    );
    assert_eq!(
        world.engine.photo_previews_for(session).unwrap_err(),
        PreviewsRefused::Failed
    );
    assert_eq!(world.make(false).stopped, Some(Stopped::Failed));
    // Read again: the server's default for an unset key (the check can pass).
    world.previews.fail_the_preference_read(false);
    assert_eq!(
        world.engine.photo_previews_setting_for(session).unwrap(),
        PreviewSetting::default()
    );
}

#[test]
fn facts_are_the_setting_the_summary_and_the_averages() {
    let world = world(
        "facts",
        &[
            (ONE, "Invented one.jpg", FileStatus::Local, jpeg(8, 8)),
            (TWO, "Invented two.jpg", FileStatus::Local, jpeg(8, 8)),
        ],
    );
    world.previews.set_setting(JPEG_1920);
    world.previews.store(ONE, "jpeg", 1920, vec![0; 300]);
    world.previews.store(TWO, "jpeg", 1920, vec![0; 500]);
    let session = world.engine.session_turn();
    let facts = world.engine.photo_previews_for(session).unwrap();
    assert_eq!(facts.setting, JPEG_1920);
    assert_eq!((facts.summary.count, facts.summary.total_bytes), (2, 800));
    assert_eq!(facts.averages, [((PreviewFormat::Jpeg, 1920), 400, 2)]);
    // The summary alone (what a window reads again while a run makes previews).
    assert_eq!(world.engine.photo_previews_summary_for(session).unwrap(), facts.summary);
    // Another session's question answers nothing.
    lock_and_unlock(&world.engine);
    assert_eq!(
        world.engine.photo_previews_for(session).unwrap_err(),
        PreviewsRefused::NotSignedIn
    );
    assert_eq!(
        world.engine.photo_previews_summary_for(session).unwrap_err(),
        PreviewsRefused::NotSignedIn
    );
}

/// Every file in `dir`, recursively (not SQLite's own side files of the state database, which come and go with its
/// connections).
fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let sqlite_sidecar = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("-wal") || name.ends_with("-shm") || name.ends_with("-journal"));
            if path.is_dir() {
                found.extend(files_in(&path));
            } else if !sqlite_sidecar {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn maker_writes_no_preview_bytes_to_disk() {
    let world = world(
        "disk",
        &[(ONE, "Invented one.jpg", FileStatus::Local, jpeg(1600, 1200))],
    );
    world.previews.set_setting(JPEG_1920);
    let dirs = [world.root.path(), world.db_dir.path(), world.config_dir.path()];
    // The settings file is written by the test's own set-up: counted after it.
    world.run(|| ());
    let before: Vec<Vec<PathBuf>> = dirs.iter().map(|dir| files_in(dir)).collect();
    assert_eq!(world.make(false).made, 1);
    let session = world.engine.session_turn();
    let sample = world.run(|| world.engine.measure_sample_for(session, None)).unwrap();
    assert!(sample.photo.is_some());
    let after: Vec<Vec<PathBuf>> = dirs.iter().map(|dir| files_in(dir)).collect();
    assert_eq!(before, after);
    // The system's temporary folder is shared by every test running at once, so it is not counted: instead the
    // maker's own code names no call that writes a file (the fake server holds the only copy, sealed).
    for source in [
        include_str!("photo_previews_api.rs"),
        include_str!("../photo_previews/encode.rs"),
        include_str!("../photo_previews/plan.rs"),
        include_str!("../photo_previews/seal.rs"),
        include_str!("../photo_previews/runner.rs"),
    ] {
        for writes in [
            "tempfile",
            "temp_dir",
            "fs::write",
            "File::create",
            "OpenOptions",
            "persist(",
        ] {
            assert!(!source.contains(writes), "the maker names {writes}");
        }
    }
}

#[test]
fn the_sample_is_the_newest_own_photo_measured_here() {
    let world = world(
        "sample",
        &[
            (ONE, "Invented newest.jpg", FileStatus::Local, jpeg(1600, 1200)),
            (TWO, "Invented older.jpg", FileStatus::Local, jpeg(800, 600)),
        ],
    );
    let session = world.engine.session_turn();
    let sample = world.run(|| world.engine.measure_sample_for(session, None)).unwrap();
    let photo = sample.photo.expect("a photo of the account");
    assert_eq!(photo.original_bytes, jpeg(1600, 1200).len() as u64);
    // Four sizes measured; a 1600-wide photo is never upscaled, so the three larger sizes weigh the same.
    assert_eq!(photo.weights.len(), 4);
    assert!(photo.weights.iter().all(|(_, bytes)| *bytes > 0));
    assert_eq!(photo.weights[1].1, photo.weights[3].1);
    assert!(photo.original_png.starts_with(b"\x89PNG"));
    assert_eq!(photo.previews_png.len(), 4);
    assert_eq!(world.previews.puts(), 0, "a sample uploads nothing");
    // A picked file is measured instead.
    let picked = world.root.path().join("Photos/Invented older.jpg");
    let sample = world
        .run(|| world.engine.measure_sample_for(session, Some(picked)))
        .unwrap();
    assert_eq!(sample.photo.unwrap().original_bytes, jpeg(800, 600).len() as u64);
    // No photo of the account on this PC: no photo, said as that.
    world.previews.list_media(&[]);
    let none = world.run(|| world.engine.measure_sample_for(session, None)).unwrap();
    assert!(none.photo.is_none());
}

/// The server's file changed after the version this PC holds (another device saved a newer one; this PC has not
/// taken it yet): no preview is made from this PC's older picture.
#[test]
fn a_newer_version_on_the_server_gets_no_preview_from_this_pc() {
    let picture = jpeg(1600, 1200);
    let world = world(
        "newer",
        &[(ONE, "Invented one.jpg", FileStatus::Local, picture.clone())],
    );
    world.previews.set_setting(JPEG_1920);
    world.previews.set_version(ONE, 5, picture.len() as u64);
    let end = world.make(false);
    assert_eq!((end.made, world.previews.puts()), (0, 0));
    // The same version as here: made (the check can pass).
    world.previews.set_version(ONE, 0, picture.len() as u64);
    assert_eq!(world.make(false).made, 1);
}

/// Sync paused: the run asks the server nothing, opens nothing and sends nothing, and says why.
#[test]
fn a_paused_engine_makes_no_preview() {
    let world = world(
        "paused",
        &[(ONE, "Invented one.jpg", FileStatus::Local, jpeg(1600, 1200))],
    );
    world.previews.set_setting(JPEG_1920);
    world.run(|| world.engine.pause()).unwrap();
    let asked_before = world.api.requests().len();
    let end = world.make(false);
    assert_eq!(end.stopped, Some(Stopped::Paused));
    assert_eq!(world.previews.puts(), 0);
    assert_eq!(
        world.api.requests().len(),
        asked_before,
        "nothing is asked of the server"
    );
}

/// A pause while a run makes previews: the next photo is not opened.
#[test]
fn a_pause_during_a_run_stops_it_before_the_next_photo() {
    let world = world(
        "pause-during",
        &[
            (ONE, "Invented one.jpg", FileStatus::Local, jpeg(1600, 1200)),
            (TWO, "Invented two.jpg", FileStatus::Local, jpeg(1600, 1200)),
        ],
    );
    world.previews.set_setting(JPEG_1920);
    let stop = AtomicBool::new(false);
    let session = world.engine.session_turn();
    let opened = std::sync::atomic::AtomicUsize::new(0);
    let read = |path: &Path| {
        opened.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        read_original(path)
    };
    let end = world.run(|| {
        let progress = |done: u32, _: u32| {
            if done == 1 {
                world.engine.pause().unwrap();
            }
        };
        world
            .engine
            .make_previews_reading(session, false, &stop, &progress, &read)
    });
    assert_eq!((end.made, end.stopped), (1, Some(Stopped::Paused)));
    assert_eq!(opened.load(std::sync::atomic::Ordering::SeqCst), 1);
}

/// Before this session's first reconcile pass committed the run waits: no photo is opened (so nothing is sent); once
/// it committed, the run goes on. (A Mac runs no such pass and does not wait.)
#[cfg(not(target_os = "macos"))]
#[test]
fn a_run_waits_for_the_sessions_first_pass() {
    use std::sync::atomic::Ordering::SeqCst;
    let world = world(
        "first-pass",
        &[(ONE, "Invented one.jpg", FileStatus::Local, jpeg(1600, 1200))],
    );
    world.previews.set_setting(JPEG_1920);
    let acct = &world.engine.acct;
    acct.auth_resume_asked
        .store(acct.auth_resume_done.load(SeqCst) + 1, SeqCst);
    let engine = world.engine.clone();
    let pass = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(600));
        let asked = engine.acct.auth_resume_asked.load(SeqCst);
        engine.acct.auth_resume_done.store(asked, SeqCst);
    });
    // Each open records whether the pass had committed by then.
    let opened_before = std::sync::atomic::AtomicUsize::new(0);
    let read = |path: &Path| {
        if acct.auth_resume_done.load(SeqCst) < acct.auth_resume_asked.load(SeqCst) {
            opened_before.fetch_add(1, SeqCst);
        }
        read_original(path)
    };
    let stop = AtomicBool::new(false);
    let session = world.engine.session_turn();
    let end = world.run(|| {
        world
            .engine
            .make_previews_reading(session, false, &stop, &|_, _| {}, &read)
    });
    pass.join().unwrap();
    assert_eq!(
        opened_before.load(SeqCst),
        0,
        "no photo was opened before the pass committed"
    );
    assert_eq!(end.made, 1, "after it, the run went on");
}

/// A pause while the run waits for the first pass ends the run as paused: nothing is asked of the server, opened or
/// sent.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_pause_while_waiting_for_the_first_pass_stops_the_run() {
    use std::sync::atomic::Ordering::SeqCst;
    let world = world(
        "pause-waiting",
        &[(ONE, "Invented one.jpg", FileStatus::Local, jpeg(64, 48))],
    );
    world.previews.set_setting(JPEG_1920);
    let acct = &world.engine.acct;
    acct.auth_resume_asked
        .store(acct.auth_resume_done.load(SeqCst) + 1, SeqCst);
    let engine = world.engine.clone();
    let pauser = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        engine.pause().unwrap();
        // Should the waiting run not see the pause, the pass commits later and the run goes on to ask the server.
        std::thread::sleep(std::time::Duration::from_secs(2));
        let asked = engine.acct.auth_resume_asked.load(SeqCst);
        engine.acct.auth_resume_done.store(asked, SeqCst);
    });
    let opened = std::sync::atomic::AtomicUsize::new(0);
    let read = |path: &Path| {
        opened.fetch_add(1, SeqCst);
        read_original(path)
    };
    let stop = AtomicBool::new(false);
    let session = world.engine.session_turn();
    let asked_before = world.api.requests().len();
    let end = world.run(|| {
        world
            .engine
            .make_previews_reading(session, false, &stop, &|_, _| {}, &read)
    });
    pauser.join().unwrap();
    assert_eq!(end.stopped, Some(Stopped::Paused));
    assert_eq!(
        world.api.requests().len(),
        asked_before,
        "nothing was asked of the server"
    );
    assert_eq!((opened.load(SeqCst), world.previews.puts()), (0, 0));
}

/// A row the engine changed after the plan read it (here: a newer server version pulled) is not opened in that run.
#[test]
fn a_row_that_changed_after_the_plan_is_not_opened() {
    use std::sync::atomic::Ordering::SeqCst;
    let world = world(
        "row-moved",
        &[
            (ONE, "Invented one.jpg", FileStatus::Local, jpeg(64, 48)),
            (TWO, "Invented two.jpg", FileStatus::Local, jpeg(64, 48)),
        ],
    );
    world.previews.set_setting(JPEG_1920);
    let stop = AtomicBool::new(false);
    let session = world.engine.session_turn();
    // After the first photo: both rows move to a newer server version.
    let db_path = world.db_dir.path().join("state.db");
    let progress = |done: u32, _: u32| {
        if done == 1 {
            let db = StateDb::open(&db_path).unwrap();
            for id in [ONE, TWO] {
                let mut row = db.get_file(id).unwrap().unwrap();
                row.remote_updated_at += 100;
                db.upsert_file(&row).unwrap();
            }
        }
    };
    let opened = std::sync::atomic::AtomicUsize::new(0);
    let read = |path: &Path| {
        opened.fetch_add(1, SeqCst);
        read_original(path)
    };
    let end = world.run(|| {
        world
            .engine
            .make_previews_reading(session, false, &stop, &progress, &read)
    });
    assert_eq!((end.made, end.not_here), (1, 1));
    assert_eq!(opened.load(SeqCst), 1, "the changed row's photo was not opened");
    assert_eq!(world.previews.puts(), 1);
}

/// A copy whose recorded base is behind its row's version (a pull wrote the row; the bytes on disk are the older
/// version's, here of the same size) is no source: the stale-copy rule names it, and nothing of it is sent. With its
/// re-download queued the stale-copy rule leaves it alone, and it is still no source. A copy at its version is made
/// (the check can pass).
#[test]
fn a_copy_behind_its_version_gets_no_preview() {
    use crate::state_db::{OperationKind, PendingOperation};
    let world = world(
        "behind",
        &[
            (ONE, "Invented one.jpg", FileStatus::Local, jpeg(64, 48)),
            (TWO, "Invented two.jpg", FileStatus::Local, jpeg(64, 48)),
        ],
    );
    world.previews.set_setting(JPEG_1920);
    let db = StateDb::open(world.db_dir.path().join("state.db")).unwrap();
    let agreed_at = |id: &str, base: i64, current: i64| {
        let mut state = db.get_file_contract_state(id).unwrap().unwrap();
        state.local_hash = Some("an invented hash".into());
        state.local_base_version = base;
        state.current_version = current;
        db.set_file_contract_state(&state).unwrap();
    };
    agreed_at(ONE, 1, 2);
    agreed_at(TWO, 2, 2);
    let named: Vec<String> = db
        .list_stale_agreed_copies(10)
        .unwrap()
        .into_iter()
        .map(|copy| copy.file_id)
        .collect();
    assert_eq!(named, [ONE], "the stale-copy rule names the copy behind");
    let make = |force: bool| {
        let stop = AtomicBool::new(false);
        let session = world.engine.session_turn();
        world.run(|| world.engine.make_previews_for(session, force, &stop, &|_, _| {}))
    };
    assert_eq!(make(false).made, 1);
    assert!(
        world.previews.stored(ONE).is_none(),
        "nothing of the copy behind was sent"
    );
    assert!(world.previews.stored(TWO).is_some(), "the copy at its version was made");
    db.enqueue_operation(&PendingOperation {
        op_id: "an-invented-op".into(),
        kind: OperationKind::HydrateFile,
        file_id: Some(ONE.into()),
        parent_id: None,
        target_path: None,
        metadata_json: None,
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 1,
        updated_at: 1,
    })
    .unwrap();
    assert!(
        db.list_stale_agreed_copies(10).unwrap().is_empty(),
        "a queued re-download: not the rule's"
    );
    assert_eq!(
        make(true).made,
        1,
        "a rebuild makes the copy at its version again, and only it"
    );
    assert!(world.previews.stored(ONE).is_none());
}

/// A version pulled after the photo was read and before its send: the bytes read were the older version's, so
/// nothing is sent.
#[test]
fn a_version_pulled_between_the_open_and_the_send_sends_nothing() {
    let world = world(
        "pulled-in-hand",
        &[(ONE, "Invented one.jpg", FileStatus::Local, jpeg(64, 48))],
    );
    world.previews.set_setting(JPEG_1920);
    let db_path = world.db_dir.path().join("state.db");
    let read = |path: &Path| {
        let original = read_original(path);
        // The pull lands while the photo is in hand: the row now carries a newer version.
        let db = StateDb::open(&db_path).unwrap();
        let mut row = db.get_file(ONE).unwrap().unwrap();
        row.remote_updated_at += 100;
        db.upsert_file(&row).unwrap();
        original
    };
    let stop = AtomicBool::new(false);
    let session = world.engine.session_turn();
    let end = world.run(|| {
        world
            .engine
            .make_previews_reading(session, false, &stop, &|_, _| {}, &read)
    });
    assert_eq!((end.made, end.not_here), (0, 1));
    assert_eq!(world.previews.puts(), 0);
}

/// A lock during the speed limit's wait after a send ends the run at once (its copy of the key goes with it), not
/// when the wait is over.
#[test]
fn a_lock_during_a_wait_ends_the_run_at_once() {
    let world = world(
        "lock-in-wait",
        &[
            (ONE, "Invented one.jpg", FileStatus::Local, jpeg(256, 192)),
            (TWO, "Invented two.jpg", FileStatus::Local, jpeg(256, 192)),
        ],
    );
    world.previews.set_setting(JPEG_1920);
    let stop = AtomicBool::new(false);
    let session = world.engine.session_turn();
    let (engine, previews) = (world.engine.clone(), world.previews.clone());
    // Once the first preview is sent (the wait of about 5 s at 1 KiB/s has begun), the screen locks.
    let locked_at = std::sync::Arc::new(std::sync::Mutex::new(None::<std::time::Instant>));
    let locked_at_writer = std::sync::Arc::clone(&locked_at);
    let locker = std::thread::spawn(move || {
        let began = std::time::Instant::now();
        while previews.puts() == 0 && began.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        *locked_at_writer.lock().unwrap() = Some(std::time::Instant::now());
        lock_and_unlock(&engine);
    });
    let end = world.run(|| {
        DesktopConfig::update_for(&TEST, |config| {
            config.upload_kbps_limit = 1;
            (true, ())
        })
        .unwrap();
        world.engine.make_previews_for(session, false, &stop, &|_, _| {})
    });
    locker.join().unwrap();
    assert!(end.session_moved);
    assert_eq!(world.previews.puts(), 1);
    assert!(
        locked_at.lock().unwrap().expect("the lock happened").elapsed() < std::time::Duration::from_secs(3),
        "the wait ended with the session: {:?}",
        locked_at.lock().unwrap().map(|t| t.elapsed())
    );
}

/// The session moves while the photo is read and encoded: nothing is sealed for it.
#[test]
fn nothing_is_sealed_for_a_session_that_moved_during_the_encode() {
    let world = world(
        "moved-in-encode",
        &[(ONE, "Invented one.jpg", FileStatus::Local, jpeg(64, 48))],
    );
    world.previews.set_setting(JPEG_1920);
    let stop = AtomicBool::new(false);
    let session = world.engine.session_turn();
    let read = |path: &Path| {
        let original = read_original(path);
        lock_and_unlock(&world.engine);
        original
    };
    SEALED.with(|sealed| sealed.set(0));
    let end = world.run(|| {
        world
            .engine
            .make_previews_reading(session, false, &stop, &|_, _| {}, &read)
    });
    assert!(end.session_moved);
    assert_eq!(SEALED.with(|sealed| sealed.get()), 0, "nothing was sealed");
    assert_eq!(world.previews.puts(), 0);
}

/// Off while a photo is in hand: its preview is not sent.
#[test]
fn a_stop_while_a_photo_is_in_hand_sends_nothing() {
    let world = world(
        "stop-in-hand",
        &[(ONE, "Invented one.jpg", FileStatus::Local, jpeg(1600, 1200))],
    );
    world.previews.set_setting(JPEG_1920);
    let stop = AtomicBool::new(false);
    let session = world.engine.session_turn();
    let read = |path: &Path| {
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        read_original(path)
    };
    world.run(|| {
        world
            .engine
            .make_previews_reading(session, false, &stop, &|_, _| {}, &read)
    });
    assert_eq!(world.previews.puts(), 0);
}

/// A panic while one photo is read or encoded ends that photo only: it counts as failed and the run goes on.
#[test]
fn a_panic_on_one_photo_skips_that_photo_only() {
    let world = world(
        "panic",
        &[
            (ONE, "Invented one.jpg", FileStatus::Local, jpeg(1600, 1200)),
            (TWO, "Invented two.jpg", FileStatus::Local, jpeg(1600, 1200)),
        ],
    );
    world.previews.set_setting(JPEG_1920);
    let stop = AtomicBool::new(false);
    let session = world.engine.session_turn();
    let read = |path: &Path| {
        if path.ends_with("Invented one.jpg") {
            panic!("an invented decoder panic");
        }
        read_original(path)
    };
    let end = world.run(|| {
        world
            .engine
            .make_previews_reading(session, false, &stop, &|_, _| {}, &read)
    });
    assert_eq!((end.made, end.failed, end.stopped), (1, 1, None));
    assert!(world.previews.stored(TWO).is_some());
}

/// Beebeeb answers every preview with a server error: the run waits longer each time and stops after three in a
/// row, as when it cannot be reached, instead of reading, encoding and sending every photo.
#[test]
fn server_errors_stop_the_run_after_three_in_a_row() {
    let world = world(
        "server-errors",
        &[
            (ONE, "Invented one.jpg", FileStatus::Local, jpeg(64, 48)),
            (TWO, "Invented two.jpg", FileStatus::Local, jpeg(64, 48)),
            (THREE, "Invented three.jpg", FileStatus::Local, jpeg(64, 48)),
            (CLOUD, "Invented four.jpg", FileStatus::Local, jpeg(64, 48)),
        ],
    );
    world.previews.set_setting(JPEG_1920);
    world.previews.fail_puts_with(503);
    let end = world.make(false);
    assert_eq!(end.stopped, Some(Stopped::Offline));
    assert_eq!((world.previews.puts(), end.failed), (3, 3));
}

/// A session mutex a panic poisoned: nothing is copied and nothing is sent.
#[test]
fn a_poisoned_session_sends_nothing() {
    let world = world(
        "poisoned",
        &[(ONE, "Invented one.jpg", FileStatus::Local, jpeg(1600, 1200))],
    );
    world.previews.set_setting(JPEG_1920);
    let session = world.engine.session_turn();
    let engine = world.engine.clone();
    let _ = std::thread::spawn(move || {
        let _held = engine.acct.session.lock().unwrap();
        panic!("an invented panic while the session is held");
    })
    .join();
    assert!(world.engine.acct.session.is_poisoned());
    let stop = AtomicBool::new(false);
    let end = world.run(|| world.engine.make_previews_for(session, false, &stop, &|_, _| {}));
    assert!(end.session_moved);
    assert_eq!(world.previews.puts(), 0);
}

/// The uploads' speed limit paces the previews: a preview's send takes at least its size at that speed.
#[test]
fn the_upload_speed_limit_paces_the_previews() {
    let world = world("paced", &[(ONE, "Invented one.jpg", FileStatus::Local, jpeg(256, 192))]);
    world.previews.set_setting(JPEG_1920);
    let stop = AtomicBool::new(false);
    let session = world.engine.session_turn();
    let began = std::time::Instant::now();
    let end = world.run(|| {
        DesktopConfig::update_for(&TEST, |config| {
            config.upload_kbps_limit = 1;
            (true, ())
        })
        .unwrap();
        world.engine.make_previews_for(session, false, &stop, &|_, _| {})
    });
    assert_eq!(end.made, 1);
    let sent = world.previews.stored(ONE).unwrap().body.len() as f64;
    assert!(
        began.elapsed().as_secs_f64() >= sent / 1024.0,
        "{} bytes at 1 KiB/s took {:?}",
        sent,
        began.elapsed()
    );
}

// ---- 115: a thumbnail made on this PC is sent once --------------------------------------------------------------

use crate::app::photos_api::ThumbPixels;
use crate::photo_previews::thumbs_upload::NotNow;

impl World {
    fn thumb_puts(&self) -> Vec<Request> {
        self.api
            .requests()
            .into_iter()
            .filter(|req| req.method == "PUT" && req.path.split('?').next().is_some_and(|p| p.ends_with("/thumbnail")))
            .collect()
    }

    fn keep_made(&self, id: &str, version: i64) {
        let session = self.engine.session_turn();
        let pixels = ThumbPixels {
            session,
            width: 32,
            height: 24,
            rgba: zeroize::Zeroizing::new(vec![120u8; 32 * 24 * 4]),
        };
        self.engine.keep_local_thumbnail(session, id, version, &pixels).unwrap();
    }

    fn upload_made(&self, id: &str, version: i64, server_404: bool) -> MadeUpload {
        let session = self.engine.session_turn();
        self.run(|| self.engine.upload_made_thumbnail_for(session, id, version, server_404))
    }
}

#[test]
fn made_thumbnail_is_sent_once_sealed_with_the_thumbnail_recipe() {
    let world = world("thumb1", &[(ONE, "Invented harbour.jpg", FileStatus::Local, jpeg(800, 600))]);
    world.previews.set_setting(JPEG_1920);
    world.keep_made(ONE, 0);
    assert_eq!(world.upload_made(ONE, 0, true), MadeUpload::Sent);
    let puts = world.thumb_puts();
    assert_eq!(puts.len(), 1);
    let key = beebeeb_core::kdf::derive_file_key(
        &beebeeb_core::kdf::MasterKey::from_bytes(world.account.master_key),
        ONE.as_bytes(),
    );
    let webp = beebeeb_core::encrypt::decrypt_thumbnail(&key, &puts[0].body).expect("opens as a thumbnail");
    assert_eq!(&webp[..4], b"RIFF");
    // Once: a later look sends nothing more.
    assert_eq!(world.upload_made(ONE, 0, true), MadeUpload::NothingToSend);
    assert_eq!(world.thumb_puts().len(), 1);
}

const M1: &str = "d0000000-0000-4000-8000-000000000101";
#[test]
fn made_thumbnail_is_not_sent_unless_the_server_had_none() {
    let world = world("thumb2", &[(M1, "Invented harbour.jpg", FileStatus::Local, jpeg(800, 600))]);
    world.previews.set_setting(JPEG_1920);
    world.keep_made(M1, 0);
    assert_eq!(world.upload_made(M1, 0, false), MadeUpload::NothingToSend);
    assert!(world.thumb_puts().is_empty());
}

const M2: &str = "d0000000-0000-4000-8000-000000000102";
#[test]
fn made_thumbnail_waits_while_the_setting_is_off_and_goes_when_it_is_on() {
    let world = world("thumb3", &[(M2, "Invented harbour.jpg", FileStatus::Local, jpeg(800, 600))]);
    world.keep_made(M2, 0);
    assert_eq!(world.upload_made(M2, 0, true), MadeUpload::NotNow(NotNow::SettingOff));
    assert!(world.thumb_puts().is_empty());
    let session = world.engine.session_turn();
    world.engine.set_photo_previews_for(session, JPEG_1920).unwrap();
    // The 404 was remembered: a later look without it still sends.
    assert_eq!(world.upload_made(M2, 0, false), MadeUpload::Sent);
    assert_eq!(world.thumb_puts().len(), 1);
}

const M9: &str = "d0000000-0000-4000-8000-000000000199";

#[test]
fn a_hundred_looks_read_the_setting_once_and_a_local_change_reads_it_again() {
    let world = world("thumb9", &[(M9, "Invented harbour.jpg", FileStatus::Local, jpeg(800, 600))]);
    world.keep_made(M9, 0);
    let gets = |world: &World| {
        world
            .api
            .requests()
            .iter()
            .filter(|req| req.method == "GET" && req.path.starts_with("/api/v1/preferences/photo_previews"))
            .count()
    };
    for _ in 0..100 {
        assert_eq!(world.upload_made(M9, 0, true), MadeUpload::NotNow(NotNow::SettingOff));
    }
    assert_eq!(gets(&world), 1);
    // The settings pane changes it here: the next look reads it again, and sends.
    let session = world.engine.session_turn();
    world.engine.set_photo_previews_for(session, JPEG_1920).unwrap();
    assert_eq!(world.upload_made(M9, 0, true), MadeUpload::Sent);
    assert_eq!(gets(&world), 2);
}

// ---- drive-complete-RS-d: thumbnails made from an opened file's pixels ------------------------------------------

const G1: &str = "d0000000-0000-4000-8000-000000000201";
const G2: &str = "d0000000-0000-4000-8000-000000000202";
const G3: &str = "d0000000-0000-4000-8000-000000000203";

impl World {
    fn set_flag(&self, id: &str, has: bool) {
        let state = self.engine.local_state().expect("the local state");
        state.with(|db| db.set_server_has_thumbnail(id, has)).unwrap().unwrap();
    }

    fn flag(&self, id: &str) -> Option<bool> {
        self.engine.server_thumbnail_known_for(self.engine.session_turn(), id)
    }

    fn opened_pixels(&self) -> ThumbPixels {
        let (w, h) = (640u32, 480u32);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 90, 255]);
            }
        }
        ThumbPixels {
            session: self.engine.session_turn(),
            width: w,
            height: h,
            rgba: zeroize::Zeroizing::new(rgba),
        }
    }

    fn upload_generated(&self, id: &str, server_404: bool) -> MadeUpload {
        let session = self.engine.session_turn();
        let pixels = self.opened_pixels();
        self.run(|| self.engine.upload_generated_thumbnails_for(session, id, &pixels, server_404))
    }

    fn medium_puts(&self) -> Vec<Request> {
        self.thumb_puts().into_iter().filter(|req| !req.path.contains("/thumbnail/")).collect()
    }
}

#[test]
fn thumbnail_of_an_opened_file_with_none_is_sent_medium_and_has_thumbnail_becomes_true() {
    let world = world("gen1", &[(G1, "Invented forest.jpg", FileStatus::CloudOnly, jpeg(800, 600))]);
    world.previews.set_setting(JPEG_1920);
    world.set_flag(G1, false);
    assert_eq!(world.flag(G1), Some(false));
    assert_eq!(world.upload_generated(G1, false), MadeUpload::Sent);
    let puts = world.medium_puts();
    assert_eq!(puts.len(), 1, "one medium thumbnail upload");
    let key = beebeeb_core::kdf::derive_file_key(
        &beebeeb_core::kdf::MasterKey::from_bytes(world.account.master_key),
        G1.as_bytes(),
    );
    let webp = beebeeb_core::encrypt::decrypt_thumbnail(&key, &puts[0].body).expect("opens as a thumbnail");
    assert_eq!(&webp[..4], b"RIFF");
    assert_eq!(world.flag(G1), Some(true));
    // The photo preview the setting asks for went too.
    assert_eq!(world.preview_puts().len(), 1);
    // Done: a second open sends nothing more.
    assert_eq!(world.upload_generated(G1, false), MadeUpload::NothingToSend);
    assert_eq!(world.medium_puts().len(), 1);
}

#[test]
fn thumbnail_is_not_generated_for_a_file_the_server_has_one_of_or_not_known_without_a_404() {
    let world = world(
        "gen2",
        &[
            (G1, "Invented a.jpg", FileStatus::CloudOnly, jpeg(80, 60)),
            (G2, "Invented b.jpg", FileStatus::CloudOnly, jpeg(80, 60)),
        ],
    );
    world.previews.set_setting(JPEG_1920);
    world.set_flag(G1, true);
    assert_eq!(world.upload_generated(G1, false), MadeUpload::NothingToSend);
    assert_eq!(world.upload_generated(G2, false), MadeUpload::NothingToSend);
    assert!(world.thumb_puts().is_empty());
    // Not known, and the caller's fetch said none just now: sent.
    assert_eq!(world.upload_generated(G2, true), MadeUpload::Sent);
    assert_eq!(world.flag(G2), Some(true));
}

#[test]
fn thumbnail_is_not_generated_while_the_setting_is_off_or_paused() {
    let world = world("gen3", &[(G1, "Invented a.jpg", FileStatus::CloudOnly, jpeg(80, 60))]);
    world.set_flag(G1, false);
    assert_eq!(world.upload_generated(G1, false), MadeUpload::NotNow(NotNow::SettingOff));
    let session = world.engine.session_turn();
    assert_eq!(
        world.run(|| world.engine.thumbnail_send_hold_for(session)),
        Some(MadeUpload::NotNow(NotNow::SettingOff))
    );
    assert!(world.thumb_puts().is_empty());
    assert_eq!(world.flag(G1), Some(false));
}

#[test]
fn thumbnail_repair_lists_pictures_with_none_or_not_known_and_leaves_the_rest() {
    let world = world(
        "gen4",
        &[
            (G1, "Invented a.jpg", FileStatus::CloudOnly, jpeg(80, 60)),
            (G2, "Invented b.jpg", FileStatus::CloudOnly, jpeg(80, 60)),
            (G3, "Invented c.jpg", FileStatus::CloudOnly, jpeg(80, 60)),
        ],
    );
    world.set_flag(G1, false);
    world.set_flag(G2, true);
    let session = world.engine.session_turn();
    let missing = world.engine.files_missing_thumbnails_for(session).unwrap();
    let ids: Vec<(&str, bool)> = missing.iter().map(|m| (m.file_id.as_str(), m.server_said_none)).collect();
    assert_eq!(ids, vec![(G1, true), (G3, false)]);
}
