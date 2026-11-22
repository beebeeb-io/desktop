//! Slice B1's rules: the backup line's words, its session rule, and that nothing is invented. Slice K4's: Photo
//! sources' folders come from the engine, a switch takes a folder's photos out of the timeline once the engine
//! answered, and an answer of an earlier session or an older list is dropped. Every name is invented.

use std::time::Duration;

use super::*;

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use zeroize::Zeroizing;

fn stamped(turn: u64, account: &str) -> Stamp {
    (SessionGeneration::for_test(turn), Some(account.to_string()))
}

fn at(day: NaiveDate, hour: u32, minute: u32) -> i64 {
    let naive = day.and_hms_opt(hour, minute, 0).unwrap();
    Local.from_local_datetime(&naive).single().unwrap().timestamp_millis()
}

#[test]
fn backup_status_last_is_the_devices_last_backup() {
    let today = Local::now().date_naive();
    assert_eq!(
        last_backup_words(Some(at(today, 9, 12)), today).as_deref(),
        Some("last backup 09:12"),
        "today: the time alone"
    );
    let yesterday = today.pred_opt().unwrap();
    assert_eq!(
        last_backup_words(Some(at(yesterday, 18, 5)), today).as_deref(),
        Some("last backup Yesterday 18:05"),
        "an earlier day names the day"
    );
}

#[test]
fn backup_status_now_is_never_invented() {
    let today = Local::now().date_naive();
    // No device named a pass time: no line at all.
    assert_eq!(last_backup_words(None, today), None);
    // The held line carries an empty `now`, whatever it was read from.
    let mut held = Held::default();
    let asked = stamped(1, "first");
    held.follow(&asked);
    assert!(held.take(&asked, last_backup_words(Some(at(today, 9, 12)), today)));
    let status = held.last.clone().map(|last| BackupStatus {
        now: String::new(),
        last,
    });
    assert_eq!(status.as_ref().map(|status| status.now.as_str()), Some(""));
    assert_eq!(status.map(|status| status.last).as_deref(), Some("last backup 09:12"));
}

#[test]
fn photo_folders_late_answer_dropped() {
    let mut held = Held::default();
    let before = stamped(1, "first");
    held.follow(&before);
    held.reading = true;
    // The session moved on, then another account: the held state is for the new stamp and empty.
    let after = stamped(2, "second");
    held.follow(&after);
    assert!(
        !held.take(&before, Some("last backup 09:12".into())),
        "a late answer is dropped"
    );
    assert_eq!(held.last, None);
    assert!(held.at.is_none());
    // The same session under another account is another stamp too.
    assert!(!held.take(&stamped(2, "first"), Some("last backup 09:12".into())));
    // The answer for the current stamp is kept.
    assert!(held.take(&after, Some("last backup 10:00".into())));
    assert_eq!(held.last.as_deref(), Some("last backup 10:00"));
}

// ---- Photo sources' folders ----

const FOLDERS_PASSWORD: &str = "an invented password for photo folders";
const FOLDERS_WAIT: Duration = Duration::from_secs(30);

const TRIP: &str = "c0000000-0000-4000-8000-000000000a01";
const HARBOUR: &str = "c0000000-0000-4000-8000-000000000a02";
const TRAM: &str = "c0000000-0000-4000-8000-000000000a03";
const OLD: &str = "c0000000-0000-4000-8000-000000000a04";
const DAY_TWO: &str = "c0000000-0000-4000-8000-000000000a05";

type FolderSource = EnginePhotos<MemoryCredentials>;

fn folder(id: &str, shown: bool) -> PhotoFolder {
    PhotoFolder {
        id: id.into(),
        name: "Invented trip".into(),
        items: 3,
        shown,
    }
}

fn folder_row(db: &StateDb, id: &str, path: &str, folder: bool) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::CloudOnly,
        size_bytes: if folder { 0 } else { 2_000_000 },
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

/// Pumps until `done` says so.
fn pump_until(source: &mut FolderSource, mut done: impl FnMut(&mut FolderSource) -> bool) {
    let deadline = Instant::now() + FOLDERS_WAIT;
    loop {
        source.pump();
        if done(source) {
            return;
        }
        assert!(Instant::now() < deadline, "it never came");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// (photos, videos) the timeline shows.
fn counted(source: &FolderSource) -> (u64, u64) {
    let totals = source.totals(None);
    (totals.photos, totals.videos)
}

#[test]
fn photo_folders_come_from_the_engine() {
    let (account, phrase) = FakeAccount::register("photo-folders-one@example.eu", FOLDERS_PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let fake = FakeApi::start(account_handler(account.clone()));
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "photo-folders-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    let engine = Arc::new(engine);
    engine
        .sign_in_password(&account.email, Zeroizing::new(FOLDERS_PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase))
        .expect("the phrase unlocks");

    // The account's local state: a trip folder with a subfolder, and a photo outside it.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    folder_row(&db, TRIP, "Trip", true);
    folder_row(&db, DAY_TWO, "Trip/Day two", true);
    folder_row(&db, HARBOUR, "Trip/Invented harbour.jpg", false);
    folder_row(&db, TRAM, "Trip/Day two/Invented tram.mov", false);
    folder_row(&db, OLD, "Invented old.png", false);
    db.set_owner(&Identity::new(Some(&account.user_id), Some(&account.email)))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(path);

    // The folder is added at the engine, not through this source: the list is the engine's answer.
    engine
        .add_photo_folder_for(engine.session_turn(), TRIP)
        .expect("a folder of the account's own tree");
    let store = EngineThumbStore::new(engine.clone());
    let mut source = EnginePhotos::new(engine.clone(), store);
    let trip_shown = Folder {
        name: "Trip".into(),
        detail: "2 items, with its subfolders".into(),
        shown: true,
    };
    pump_until(&mut source, |source| source.folders() == [trip_shown.clone()]);

    source.opened();
    pump_until(&mut source, |source| source.loaded() != Loaded::Loading);
    assert_eq!(
        counted(&source),
        (2, 1),
        "every photo shows while the folder is switched on"
    );

    // Move… reads the files behind the photos: the id and the name; a photo that is not held gives none.
    let first = source.photo(None, 0, 0).expect("a photo shows").id;
    let items = source.file_items(&[first]).expect("the photo is held");
    assert!(!items[0].id.is_empty() && !items[0].name.is_empty());
    assert!(
        source
            .file_items(&[first, crate::frames::photos::source::PhotoId(u64::MAX)])
            .is_none()
    );

    // Done hands every folder over, changed or not: a switch that is as the engine has it sends nothing.
    let reads = source.reads;
    source.set_shown(SourceRef::Folder(0), true);
    assert_eq!(source.reads, reads, "an unchanged switch is not sent");

    // Switched off: once the engine answered, the folder's photos (its subfolder's too) leave the timeline.
    source.set_shown(SourceRef::Folder(0), false);
    pump_until(&mut source, |source| {
        counted(source) == (1, 0) && source.folders().first().is_some_and(|folder| !folder.shown)
    });
    assert_eq!(
        engine
            .photo_folders_for(engine.session_turn())
            .map(|list| list.iter().map(|f| f.shown).collect::<Vec<_>>()),
        Ok(vec![false]),
        "the switch is the engine's"
    );

    // A file is no folder: the engine refuses, and the list stays what the engine has.
    assert!(source.add_folder(HARBOUR), "asked");
    pump_until(&mut source, |source| !source.pump().busy);
    assert_eq!(source.folders().len(), 1);
    assert!(source.folder_refused(), "the refusal is told to the window");
    assert!(!source.folder_refused(), "once");

    // Removed: its photos show again, and the row is gone.
    source.remove_folder(SourceRef::Folder(0));
    pump_until(&mut source, |source| {
        counted(source) == (2, 1) && source.folders().is_empty()
    });
    assert_eq!(engine.photo_folders_for(engine.session_turn()), Ok(Vec::new()));
}

#[test]
fn photo_folder_switch_late_answer_dropped() {
    let mut held = HeldFolders::default();
    let before = stamped(1, "first");
    held.follow(&before);
    let asked = held.start(&before).expect("a read for the stamp that is held");
    // The session moved on: what was asked before is not taken, and nothing is held for the new session.
    let after = stamped(2, "first");
    held.follow(&after);
    assert!(
        !held.take(&before, asked, vec![folder(TRIP, false)]),
        "a late answer is dropped"
    );
    assert_eq!(held.list, None);
    assert_eq!(held.start(&before), None, "nothing starts for a stamp that is not held");
    // Another account in the same session is another stamp too.
    assert!(!held.take(&stamped(2, "second"), asked + 1, vec![folder(TRIP, false)]));
    // Under the stamp held: the newest list wins; one read before the change that lands later does not undo it.
    let first = held.start(&after).unwrap();
    let second = held.start(&after).unwrap();
    assert!(held.take(&after, second, vec![folder(TRIP, false)]));
    assert!(
        !held.take(&after, first, vec![folder(TRIP, true)]),
        "an older list is dropped"
    );
    assert_eq!(held.list.as_ref().map(|list| list[0].shown), Some(false));
    // Nothing is drawn as changed before the engine answered: only `take` changes the list.
    assert!(held.start(&after).is_some());
    assert_eq!(held.list.as_ref().map(|list| list[0].shown), Some(false));
}
