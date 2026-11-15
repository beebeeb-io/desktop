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
use crate::identity::REAL;
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
    Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"))
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
    let (routes, own) = (previews.clone(), account_handler(account.clone()));
    let api = FakeApi::start(Arc::new(move |req: &Request| {
        routes.answer(req).unwrap_or_else(|| own(req))
    }));
    let engine = engine_at(&api.base);
    signed_in(&engine, &account);
    let root = tempfile::tempdir().unwrap();
    let db_dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(&db_dir.path().join("state.db")).unwrap();
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
            DesktopConfig::update_for(&REAL, |config| {
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
    let end = world.run(|| world.engine.make_previews_for(session, false, &stop, &progress));
    assert!(end.session_moved);
    assert_eq!((end.made, world.preview_puts().len()), (1, 1));
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
    assert_eq!(facts.summary.by_setting, [((PreviewFormat::Jpeg, 1920), 2)]);
    assert_eq!(facts.averages, [((PreviewFormat::Jpeg, 1920), 400, 2)]);
    // Another session's question answers nothing.
    lock_and_unlock(&world.engine);
    assert_eq!(
        world.engine.photo_previews_for(session).unwrap_err(),
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
