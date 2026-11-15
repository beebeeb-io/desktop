//! S6 (R8–R10), Linux: a hydrate into a file's own place takes its start look before anything is downloaded, and the
//! place is compared with it right before it is replaced: a save made after the look (between Keep there's look and
//! its download, during the download, or a new file after Keep both's rename) is never replaced. A place written a
//! moment ago is waited for once, never refused for that alone. The look's own rules are unit-tested in
//! `engine_bridge/content.rs` (every platform).
#![cfg(all(not(target_os = "windows"), not(target_os = "macos")))]

use super::reg_1640::{regression_1640_synced_file, regression_1640_web_saves};
use super::*;

/// This PC's version at the place, and a save of the same length (rvd447 P3-D: only the times or the inode tell it).
const THIS_PC: &[u8] = b"this PC's version";
const SAVE: &[u8] = b"saved during it!!";

/// A file in conflict of unknown content (no agreed content recorded, as `app/conflicts_api_tests.rs`'s `put` seeds a
/// conflict row): the server holds `b"version one"`, this PC's own version is at its place. Returns the place.
async fn conflict_of_unknown_content(
    master_key: [u8; 32],
    file: &str,
) -> (tempfile::TempDir, PathBuf, VersionedServerMock, EngineBridge, PathBuf) {
    let (dir, sync_root, server, bridge, place) = regression_1640_synced_file(master_key, file).await;
    bridge
        .db
        .test_execute(&format!(
            "UPDATE files SET local_hash = NULL, fetched_min_version = 0, fetched_hash = NULL WHERE file_id = '{file}'"
        ))
        .unwrap();
    bridge.db.set_status(file, FileStatus::Conflict).unwrap();
    std::fs::write(&place, THIS_PC).unwrap();
    (dir, sync_root, server, bridge, place)
}

/// The start look as Keep there takes it, moved past the settle boundary (the place was written a moment ago).
fn seen_settled(place: &Path) -> PlaceSeen {
    match PlaceSeen::take_at(place, SystemTime::now() + Duration::from_secs(3)) {
        Settle::Seen(seen) => seen,
        other => panic!("the look: {other:?}"),
    }
}

/// A save of the same length: in place (the same file; only its times move), or a new file renamed over the place
/// (another inode).
fn save_same_size(place: &Path, inode_change: bool) {
    assert_eq!(SAVE.len(), THIS_PC.len());
    if inode_change {
        let staged = place.with_extension("save-tmp");
        std::fs::write(&staged, SAVE).unwrap();
        std::fs::rename(&staged, place).unwrap();
    } else {
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(place, SAVE).unwrap();
    }
}

fn refused(result: anyhow::Result<impl std::fmt::Debug>) {
    let error = result.expect_err("refused");
    assert!(engine_cause::<UnsentLocalChange>(&error).is_some(), "{error:#}");
}

/// R10: a re-download whose place was written moments before the hydrate (here: the same agreed bytes, written again)
/// waits once for the place to settle and lands; it is not refused for the wait alone.
#[tokio::test]
async fn s6_start_a_re_download_into_a_place_written_a_moment_ago_waits_and_lands() {
    let master_key = [180u8; 32];
    let file = "58000000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    assert_eq!(bridge.refresh_stale_copies(&sync_root).unwrap().queued.len(), 1);
    std::fs::write(&local, b"version one").unwrap();
    let started = std::time::Instant::now();
    drain_upload_queue(&bridge, &sync_root).await;
    let waited = started.elapsed();
    let row = bridge.db.get_file(file).unwrap().unwrap();
    drop(server.finish());
    println!(
        "s6 start look, re-download after a fresh write: took {waited:?}, status={:?}",
        row.status
    );
    assert_eq!(std::fs::read(&local).unwrap(), b"edited on the web", "it lands");
    assert_eq!(row.status, FileStatus::Local);
    assert!(bridge.db.list_operations_for_file(file).unwrap().is_empty());
}

/// R8, R9: a same-size save made while Keep there downloads (after the look, before the place is replaced) is kept,
/// whether only its times moved or it is another file renamed over the place.
#[tokio::test]
async fn s6_start_a_save_during_a_keep_there_download_is_kept() {
    for (n, inode_change) in [(1u8, false), (2, true)] {
        let file = format!("5810000{n}-0000-4000-8000-000000000001");
        let (_dir, sync_root, server, bridge, place) = conflict_of_unknown_content([180 + n; 32], &file).await;
        let seen = seen_settled(&place);
        let at = place.clone();
        bridge
            .seams
            .arm("hydrate:before_write", move || save_same_size(&at, inode_change));
        let result = bridge.resolve_theirs_seen(&file, &sync_root, seen).await;
        drop(server.finish());
        println!("s6 start look, keep there, save during (inode change {inode_change}): {result:?}");
        refused(result);
        assert_eq!(std::fs::read(&place).unwrap(), SAVE, "the save is kept");
    }
}

/// R9: a save made between Keep there's look (taken before its safety copy) and its download is kept: the hydrate
/// compares with that look, not with one of its own.
#[tokio::test]
async fn s6_start_a_save_between_keep_theres_look_and_its_download_is_kept() {
    let file = "58200000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, place) = conflict_of_unknown_content([183u8; 32], file).await;
    let seen = seen_settled(&place);
    save_same_size(&place, true);
    let result = bridge.resolve_theirs_seen(file, &sync_root, seen).await;
    drop(server.finish());
    println!("s6 start look, keep there, save before the download: {result:?}");
    refused(result);
    assert_eq!(std::fs::read(&place).unwrap(), SAVE, "the save is kept");
}

/// R9: Keep there refuses a place it did not look at. Here the look is of another name of the same file (a hard
/// link: the same size, times and inode), so only the place check can tell them apart.
#[tokio::test]
async fn s6_start_keep_there_refuses_a_place_it_did_not_look_at() {
    let file = "58300000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, place) = conflict_of_unknown_content([184u8; 32], file).await;
    let other = sync_root.join("a.txt");
    std::fs::hard_link(&place, &other).unwrap();
    let seen = seen_settled(&other);
    let result = bridge.resolve_theirs_seen(file, &sync_root, seen).await;
    drop(server.finish());
    println!("s6 start look, keep there, another place: {result:?}");
    refused(result);
    assert_eq!(std::fs::read(&place).unwrap(), THIS_PC, "the row's place is untouched");
}

/// R8: Keep both renames this PC's version away, then hydrates into the empty place. A file made at the original
/// name during the download is kept, and so is the conflict copy; the row reads `error`.
#[tokio::test]
async fn s6_start_a_save_during_a_keep_both_download_is_kept() {
    let file = "58400000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, place) = conflict_of_unknown_content([185u8; 32], file).await;
    let entry = bridge.db.get_file(file).unwrap().unwrap();
    let at = place.clone();
    bridge.seams.arm("hydrate:before_write", move || {
        std::fs::write(&at, b"a new file at the original name").unwrap();
    });
    let result = bridge.auto_resolve_keep_both(&sync_root, &entry).await;
    let row = bridge.db.get_file(file).unwrap().unwrap();
    drop(server.finish());
    let names: Vec<String> = std::fs::read_dir(&sync_root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    println!(
        "s6 start look, keep both: {result:?} folder={names:?} status={:?}",
        row.status
    );
    refused(result);
    assert_eq!(
        std::fs::read(&place).unwrap(),
        b"a new file at the original name",
        "the new file is kept"
    );
    let copy = names
        .iter()
        .find(|name| name.contains("(conflict - "))
        .expect("the conflict copy");
    assert_eq!(
        std::fs::read(sync_root.join(copy)).unwrap(),
        THIS_PC,
        "and the conflict copy"
    );
    assert_eq!(row.status, FileStatus::Error);
}
