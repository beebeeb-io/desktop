//! Photo sources' folders at the engine (slice K4): what a folder switched off takes out of the library (its
//! subfolders too), what can be added, and whose the rows are. Sessions are put in memory as a sign-in and an unlock
//! leave them; each test has a state database of its own. Every name is invented.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind};
use beebeeb_types::backup::SystemRole;

type TestEngine = Engine<MemoryCredentials>;

const A: (Option<&str>, Option<&str>) = (Some("user-a"), Some("a@example.eu"));
const B: (Option<&str>, Option<&str>) = (Some("user-b"), Some("b@example.eu"));

const TRIP: &str = "e0000000-0000-4000-8000-000000000101";
const DAY_TWO: &str = "e0000000-0000-4000-8000-000000000102";
const HARBOUR: &str = "e0000000-0000-4000-8000-000000000103";
const TRAM: &str = "e0000000-0000-4000-8000-000000000104";
const NOTE: &str = "e0000000-0000-4000-8000-000000000105";
const TRIP_TWO: &str = "e0000000-0000-4000-8000-000000000106";
const BEACH: &str = "e0000000-0000-4000-8000-000000000107";
const LETTER: &str = "e0000000-0000-4000-8000-000000000108";
const BACKUPS: &str = "e0000000-0000-4000-8000-000000000109";
const PHONE: &str = "e0000000-0000-4000-8000-000000000110";
const SHOT: &str = "e0000000-0000-4000-8000-000000000111";
const SHARED: &str = "e0000000-0000-4000-8000-000000000112";
const SHARED_SHOT: &str = "e0000000-0000-4000-8000-000000000113";

fn engine() -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-photo-folders-slot".into(),
        state_dir,
        api_base: "http://127.0.0.1:9".into(),
    };
    Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

/// A session in memory, as a sign-in and an unlock leave it. Its number.
fn signed_in(engine: &TestEngine, who: (Option<&str>, Option<&str>)) -> SessionGeneration {
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: "an-invented-token".into(),
            master_key: [7; 32],
            email: who.1.map(str::to_string),
        });
        engine.remember_session_user_id(who.0.map(str::to_string));
        engine.set_auth_present(true);
    }
    engine.session_turn()
}

fn signed_out(engine: &TestEngine) {
    engine.forget_session(SessionTransition::SignOut);
    engine.remember_session_user_id(None);
    engine.set_auth_present(false);
}

fn row(db: &StateDb, id: &str, path: &str, modified: i64, folder: bool, namespace: Namespace) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::CloudOnly,
        size_bytes: if folder { 0 } else { 1_000 },
        modified_at: modified,
        content_hash: None,
        remote_updated_at: modified,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| {
        contract.item_kind = entry.item_kind.clone();
        contract.namespace = namespace;
    })
    .unwrap();
}

/// A state database recorded as `owner`'s: a trip folder with a subfolder, a folder whose name begins with the
/// trip's, a letter, the Backups root with a phone's folder, and a folder someone else shares.
fn state(dir: &Path, owner: (Option<&str>, Option<&str>)) -> PathBuf {
    let path = dir.join("state.db");
    let db = StateDb::open(&path).unwrap();
    let mine = || Namespace::MyFiles;
    row(&db, TRIP, "Trip", 100, true, mine());
    row(&db, DAY_TWO, "Trip/Day two", 100, true, mine());
    row(&db, HARBOUR, "Trip/Invented harbour.jpg", 600, false, mine());
    row(&db, TRAM, "Trip/Day two/Invented tram.mov", 500, false, mine());
    row(&db, NOTE, "Trip/Invented note.txt", 900, false, mine());
    row(&db, TRIP_TWO, "Trip2", 100, true, mine());
    row(&db, BEACH, "Trip2/Invented beach.png", 400, false, mine());
    row(&db, LETTER, "Invented letter.pdf", 300, false, mine());
    row(&db, BACKUPS, "Backups", 100, true, mine());
    db.set_system_role(BACKUPS, Some(SystemRole::BackupsRoot)).unwrap();
    row(&db, PHONE, "Backups/Invented phone", 100, true, mine());
    row(
        &db,
        SHOT,
        "Backups/Invented phone/Invented shot.heic",
        200,
        false,
        mine(),
    );
    row(&db, SHARED, "Shared trip", 100, true, Namespace::SharedWithMe);
    row(
        &db,
        SHARED_SHOT,
        "Shared trip/Invented shared.jpg",
        150,
        false,
        Namespace::SharedWithMe,
    );
    db.set_owner(&Identity::new(owner.0, owner.1)).unwrap();
    path
}

/// The ids the library shows, in its order.
fn library_ids(engine: &TestEngine) -> Vec<String> {
    engine
        .photos_library()
        .expect("the library")
        .photos
        .into_iter()
        .map(|photo| photo.id)
        .collect()
}

/// Every photo folder row in the database, whoever's, by key: (key, value).
fn photo_rows(path: &Path) -> Vec<(String, String)> {
    let conn = rusqlite::Connection::open(path).unwrap();
    let mut rows = conn
        .prepare("SELECT key, value FROM sync_state WHERE substr(key, 1, 13) = 'photo_folder:' ORDER BY key")
        .unwrap();
    rows.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn photo_folder_hidden_leaves_the_library() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(state(dir.path(), A));
    let session = signed_in(&engine, A);

    let everything = library_ids(&engine);
    for id in [HARBOUR, TRAM, BEACH, SHOT] {
        assert!(
            everything.iter().any(|shown| shown == id),
            "with no folder added, every photo shows"
        );
    }

    engine
        .add_photo_folder_for(session, TRIP)
        .expect("a folder of the account's own tree");
    assert_eq!(
        library_ids(&engine),
        everything,
        "a folder added and shown changes nothing"
    );

    engine
        .set_photo_folder_shown_for(session, TRIP, false)
        .expect("the account's own folder");
    let hidden = library_ids(&engine);
    assert!(
        !hidden.iter().any(|id| id == HARBOUR),
        "a folder switched off leaves the library"
    );
    assert!(
        hidden.iter().any(|id| id == BEACH) && hidden.iter().any(|id| id == SHOT),
        "nothing else leaves"
    );

    engine.set_photo_folder_shown_for(session, TRIP, true).unwrap();
    assert_eq!(
        library_ids(&engine),
        everything,
        "switched on again: its photos show again"
    );

    engine.set_photo_folder_shown_for(session, TRIP, false).unwrap();
    engine
        .remove_photo_folder_for(session, TRIP)
        .expect("added, so it can be removed");
    assert_eq!(library_ids(&engine), everything, "removed: its photos show again");
    assert_eq!(engine.photo_folders_for(session), Ok(Vec::new()));
}

#[test]
fn photo_folder_subfolder_hidden_too() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(state(dir.path(), A));
    let session = signed_in(&engine, A);

    engine.add_photo_folder_for(session, TRIP).unwrap();
    engine.set_photo_folder_shown_for(session, TRIP, false).unwrap();
    let ids = library_ids(&engine);
    assert!(!ids.iter().any(|id| id == HARBOUR), "the folder's own photo leaves");
    assert!(!ids.iter().any(|id| id == TRAM), "a subfolder's photo leaves with it");
    assert!(
        ids.iter().any(|id| id == BEACH),
        "a folder whose name only begins with the folder's is another folder"
    );
    // Listed with its name, the photos and videos at or under it (the note is neither), and its switch.
    assert_eq!(
        engine.photo_folders_for(session),
        Ok(vec![PhotoFolder {
            id: TRIP.into(),
            name: "Trip".into(),
            items: 2,
            shown: false,
        }])
    );

    // Only the subfolder switched off: its photo leaves, the folder above keeps its own.
    engine.set_photo_folder_shown_for(session, TRIP, true).unwrap();
    engine.add_photo_folder_for(session, DAY_TWO).unwrap();
    engine.set_photo_folder_shown_for(session, DAY_TWO, false).unwrap();
    let ids = library_ids(&engine);
    assert!(!ids.iter().any(|id| id == TRAM));
    assert!(ids.iter().any(|id| id == HARBOUR));
    let listed: Vec<(String, u64, bool)> = engine
        .photo_folders_for(session)
        .unwrap()
        .into_iter()
        .map(|folder| (folder.name, folder.items, folder.shown))
        .collect();
    assert_eq!(
        listed,
        [("Trip".to_string(), 2, true), ("Day two".to_string(), 1, false)]
    );
}

#[test]
fn photo_folder_add_refuses_a_file_and_a_backup_row() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    engine.use_local_state_for_test(path.clone());
    let session = signed_in(&engine, A);

    for id in [
        LETTER,
        HARBOUR,
        BACKUPS,
        PHONE,
        SHOT,
        SHARED,
        SHARED_SHOT,
        "an-invented-unknown-id",
    ] {
        assert_eq!(
            engine.add_photo_folder_for(session, id),
            Err(PhotoFoldersRefused::NotAFolder),
            "only a folder of the account's own tree is added"
        );
    }
    assert!(photo_rows(&path).is_empty(), "nothing was written for any of them");
    assert_eq!(engine.photo_folders_for(session), Ok(Vec::new()));

    // Switching or removing a folder that was never added changes nothing either.
    assert_eq!(
        engine.set_photo_folder_shown_for(session, TRIP, false),
        Err(PhotoFoldersRefused::NotAFolder)
    );
    assert_eq!(
        engine.remove_photo_folder_for(session, TRIP),
        Err(PhotoFoldersRefused::NotAFolder)
    );

    // A folder of the account's own tree is taken, so the refusals above are the check's.
    engine.add_photo_folder_for(session, TRIP).expect("an own folder");
    assert_eq!(
        photo_rows(&path),
        [(format!("photo_folder:user-a:{TRIP}"), "1".to_string())]
    );
}

#[test]
fn photo_folder_other_account_sees_none() {
    let engine = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = state(dir.path(), A);
    engine.use_local_state_for_test(path.clone());
    let first = signed_in(&engine, A);
    engine.add_photo_folder_for(first, TRIP).unwrap();
    engine.set_photo_folder_shown_for(first, TRIP, false).unwrap();
    assert!(!library_ids(&engine).iter().any(|id| id == HARBOUR));

    // The first account signs out; this PC's local data stays, and the same database is the second account's now.
    signed_out(&engine);
    StateDb::open(&path)
        .unwrap()
        .set_owner(&Identity::new(B.0, B.1))
        .unwrap();
    let second = signed_in(&engine, B);

    assert_eq!(
        engine.photo_folders_for(second),
        Ok(Vec::new()),
        "the first account's folders are not read"
    );
    assert!(
        library_ids(&engine).iter().any(|id| id == HARBOUR),
        "the first account's switched-off folder hides nothing for the second"
    );
    assert_eq!(
        engine.set_photo_folder_shown_for(second, TRIP, true),
        Err(PhotoFoldersRefused::NotAFolder)
    );
    assert_eq!(
        engine.remove_photo_folder_for(second, TRIP),
        Err(PhotoFoldersRefused::NotAFolder)
    );
    // The first account's call, after the second signed in, answers nothing.
    assert_eq!(engine.photo_folders_for(first), Err(PhotoFoldersRefused::NotSignedIn));
    assert_eq!(
        engine.set_photo_folder_shown_for(first, TRIP, true),
        Err(PhotoFoldersRefused::NotSignedIn)
    );
    // The first account's row is still there, as it was, under its own key.
    assert_eq!(
        photo_rows(&path),
        [(format!("photo_folder:user-a:{TRIP}"), "0".to_string())]
    );
}
