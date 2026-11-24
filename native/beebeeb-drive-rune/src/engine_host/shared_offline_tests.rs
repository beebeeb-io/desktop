//! Slice B4: Make available offline on a file someone else shares, on the real engine against the fake accounts of
//! `shared_tests.rs` (a child of it, so its fake server and helpers are these tests' own). The local state is a
//! database of the test's own with one shared-with-me row (the file the fake lists as shared with the account) and
//! one own file; the sync folder is a folder of the test's own. Every name is invented.

use std::collections::HashSet;

use beebeeb_engine::app::bindings::shared_offline::SharedKeepRefused;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, Namespace, PERMISSION_READ, StateDb};

use super::*;

/// A local state for the signed-in first account: PLAN is its own file, OTHERS is a file shared with it. Returns the
/// state's folder (kept alive by the caller), the sync folder, and the database's path to read what was kept.
fn with_state(
    engine: &Engine<MemoryCredentials>,
    account: &FakeAccount,
) -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
    use beebeeb_engine::account_binding::Identity;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    let entry = |id: &str, name: &str| FileEntry {
        file_id: id.into(),
        path: name.into(),
        status: FileStatus::CloudOnly,
        size_bytes: 2_048,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    let own = entry(PLAN, "Invented plan.pdf");
    db.upsert_file_with_contract(&own, |contract| contract.item_kind = own.item_kind.clone())
        .unwrap();
    let shared = entry(OTHERS, "Shared with me/Invented gift.pdf");
    db.upsert_file_with_contract(&shared, |contract| {
        contract.namespace = Namespace::SharedWithMe;
        contract.parent_id = None;
        contract.shared_root_id = Some(OTHERS.into());
        contract.share_id = Some(INV_GOT.into());
        contract.owner_email = Some("giver@example.eu".into());
        contract.permission_bits = PERMISSION_READ;
        contract.item_kind = ItemKind::File;
    })
    .unwrap();
    db.set_owner(&Identity::new(Some(&account.user_id), Some(&account.email)))
        .unwrap();
    drop(db);
    engine.use_local_state_for_test(path.clone());
    let sync = tempfile::tempdir().unwrap();
    engine.use_sync_root_for_test(sync.path().to_path_buf());
    (dir, sync, path)
}

fn kept(path: &std::path::Path) -> Vec<String> {
    StateDb::open(path)
        .unwrap()
        .kept_rows()
        .unwrap()
        .into_iter()
        .map(|entry| entry.file_id)
        .collect()
}

/// The press of a shared row reaches the engine once, and the file is kept; a row that is not in the Shared with me
/// list sends nothing; the engine itself refuses an id that is not a shared-with-me row (an own file).
#[test]
fn shared_keep_offline_calls_the_engine() {
    let fake = fake("keepshared");
    let engine = signed_in(&fake);
    let (_state, _sync, path) = with_state(&engine, &fake.first);
    let mut source = EngineSharedSource::new(engine.clone());
    opened(&mut source);
    assert!(kept(&path).is_empty());

    // Not a row of the current lists (an own file, an unknown id): nothing is sent, nothing is said.
    assert!(!source.keep_offline(PLAN));
    assert!(!source.keep_offline("no-such-row"));
    assert!(!source.busy());

    assert!(source.keep_offline(OTHERS));
    assert!(
        !source.keep_offline(OTHERS),
        "one change per id at a time: a second press sends nothing"
    );
    assert!(source.busy());
    let notices = settle(&mut source);
    assert!(notices.is_empty(), "a kept file says nothing is wrong: {notices:?}");
    assert_eq!(kept(&path), [OTHERS], "the shared file is kept, and nothing else");

    // The engine alone refuses what is not a shared-with-me row, whoever asks: an own file is Files' to keep.
    let session = engine.session_turn();
    assert_eq!(
        engine.keep_shared_offline_for(session, PLAN),
        Err(SharedKeepRefused::NotShared)
    );
    assert_eq!(
        engine.keep_shared_offline_for(session, "no-such-row"),
        Err(SharedKeepRefused::NotShared)
    );
    assert_eq!(kept(&path), [OTHERS], "a refused call kept nothing");
}

/// An answer that lands after another account signed in is dropped: no notice, no read. The engine itself refuses
/// a press made under a session that has ended, before it touches the local state.
#[test]
fn shared_keep_offline_late_answer_dropped() {
    let fake = fake("keeplate");
    let engine = signed_in(&fake);
    let (_state, _sync, path) = with_state(&engine, &fake.first);
    let mut source = EngineSharedSource::new(engine.clone());
    opened(&mut source);
    let reads = |fake: &FakeApi| fake.routes().iter().filter(|r| *r == PATHS[0]).count();
    let before = reads(&fake.fake);
    let pressed = engine.session_turn();

    // The press is sent for the first account's session; before its answer is taken, another account signs in.
    assert!(source.keep_offline(OTHERS));
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    // Whatever the worker answered (kept, or refused because the session had ended) it answered for the first
    // session: dropped when it lands. Give the worker time, then take in what came.
    std::thread::sleep(Duration::from_millis(500));
    let pumped = source.pump();
    assert_eq!(pumped.notice, None, "an answer of another session says nothing");
    assert_eq!(reads(&fake.fake), before, "nothing is read for a dropped answer");
    assert!(
        source.with_me().is_empty(),
        "nothing of the first account is drawn for the second"
    );

    // Asked for the session that ended: refused before the local state is touched, nothing kept.
    let had = kept(&path);
    assert_eq!(
        engine.keep_shared_offline_for(pressed, OTHERS),
        Err(SharedKeepRefused::NotSignedIn)
    );
    assert_eq!(kept(&path), had, "a call for a session that ended kept nothing");
}

/// A refusal is shown, in the view's words for a change that was not made, never as a kept file: the lists stay
/// Beebeeb's, and the file is not kept (the local state here is another account's, so the engine refuses).
#[test]
fn shared_keep_offline_refusal_shown() {
    let fake = fake("keeprefused");
    let engine = signed_in(&fake);
    let (_state, _sync, path) = with_state(&engine, &fake.first);
    let mut source = EngineSharedSource::new(engine.clone());
    opened(&mut source);
    // A local state of another account's: the engine refuses before anything is written.
    let other = tempfile::tempdir().unwrap();
    let other_path = other.path().join("state.db");
    let db = StateDb::open(&other_path).unwrap();
    db.set_owner(&beebeeb_engine::account_binding::Identity::new(
        Some(&fake.second.user_id),
        Some(&fake.second.email),
    ))
    .unwrap();
    drop(db);
    engine.use_local_state_for_test(other_path);

    assert!(source.keep_offline(OTHERS));
    let notices = settle(&mut source);
    assert!(
        !notices.is_empty() && notices.iter().all(|n| *n == Notice::ChangeRefused),
        "{notices:?}"
    );
    assert!(kept(&path).is_empty(), "a refused press kept nothing");
    assert_eq!(
        source.with_me().iter().filter(|row| row.pc.is_some()).count(),
        0,
        "no row is drawn as kept on this PC"
    );
}

/// B2 (K3): after a keep succeeded and the lists were read again, the kept file's row is drawn available offline from
/// the engine's own state; before the keep no row claims anything, and no other row is marked.
#[test]
fn shared_kept_mark_drawn_after_the_engine_says_kept() {
    let fake = fake("keptmark");
    let engine = signed_in(&fake);
    let (_state, _sync, path) = with_state(&engine, &fake.first);
    let mut source = EngineSharedSource::new(engine.clone());
    opened(&mut source);
    assert!(!source.with_me().is_empty());
    assert!(
        source.with_me().iter().all(|row| row.pc.is_none()),
        "nothing is claimed before a keep"
    );

    assert!(source.keep_offline(OTHERS));
    let notices = settle(&mut source);
    assert!(notices.is_empty(), "{notices:?}");
    assert_eq!(kept(&path), [OTHERS]);
    let rows = source.with_me();
    let marked: Vec<&str> = rows
        .iter()
        .filter(|row| row.pc == Some(crate::ui::file_list::PcState::AvailableOffline))
        .map(|row| row.id.as_str())
        .collect();
    assert_eq!(marked, [OTHERS], "only the kept row is marked");
    assert!(
        rows.iter().filter(|row| row.id != OTHERS).all(|row| row.pc.is_none()),
        "every other row stays unclaimed"
    );

    // Read again later (a restart reads the same state): the mark is still drawn.
    source.opened();
    settle(&mut source);
    assert!(
        source
            .with_me()
            .iter()
            .any(|row| row.id == OTHERS && row.pc == Some(crate::ui::file_list::PcState::AvailableOffline))
    );
}

/// A refused keep draws no mark (the local state here is another account's, so the engine refuses).
#[test]
fn shared_kept_mark_not_drawn_on_refusal() {
    let fake = fake("keptmarkrefused");
    let engine = signed_in(&fake);
    let (_state, _sync, path) = with_state(&engine, &fake.first);
    let mut source = EngineSharedSource::new(engine.clone());
    opened(&mut source);
    let other = tempfile::tempdir().unwrap();
    let other_path = other.path().join("state.db");
    let db = StateDb::open(&other_path).unwrap();
    db.set_owner(&beebeeb_engine::account_binding::Identity::new(
        Some(&fake.second.user_id),
        Some(&fake.second.email),
    ))
    .unwrap();
    drop(db);
    engine.use_local_state_for_test(other_path);

    assert!(source.keep_offline(OTHERS));
    let notices = settle(&mut source);
    assert!(notices.iter().all(|n| *n == Notice::ChangeRefused), "{notices:?}");
    assert!(kept(&path).is_empty());
    assert!(
        source.with_me().iter().all(|row| row.pc.is_none()),
        "a refused keep marks nothing"
    );
}

/// An overview and kept set read for a session that is gone are never applied: another account is signed in when
/// they land, and nothing of the first account is drawn, marked or not.
#[test]
fn shared_kept_mark_of_another_session_dropped() {
    let fake = fake("keptmarklate");
    let engine = signed_in(&fake);
    let (_state, _sync, _path) = with_state(&engine, &fake.first);
    let mut source = EngineSharedSource::new(engine.clone());
    let overview = engine.shares_overview().expect("the first account's lists");
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);

    let kept: HashSet<String> = [OTHERS.to_string()].into_iter().collect();
    source.take_in(super::super::Answer::Read(1, Ok(overview), kept));
    assert!(source.with_me().is_empty(), "nothing of the first account is drawn");
    assert!(source.held.is_none(), "and nothing is held for it");
}
