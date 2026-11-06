//! LD-0105 U1 board wiring (release 0.8.22, streaming OFF): the Windows whole-file fetch, `hydrate_file`
//! and uploads put a NAMED row on the transfer board, and the whole-file fetch's row moves, ends on
//! success and is marked failed otherwise. Before (e93800c) the whole-file fetch never registered a
//! row, so the tray and Activity never showed a Windows download, and an upload's row was nameless.
//!
//! The Cloud Files callback itself is Windows-only, so these tests run its exact board sequence
//! (`whole_fetch_row` → `follow` beside `hydrate_file_to_memory_metered` → `finished()`, else the
//! row's drop) on every platform, and `the_cloud_files_callback_runs_that_sequence` pins the callback
//! to it.

use super::*;
use crate::direct_download::test_support::{ApiScript, Pair, StorageScript};
use crate::direct_download::Transport;
use crate::transfer_progress::{ActiveTransferDto, Direction, ReceiveMeter};

const KEY: [u8; 32] = [41; 32];
const SIZE: usize = 10 * 1024 + 77;

fn bridge_at(dir: &Path, pair: &Pair, path: &str) -> EngineBridge {
    let bridge = test_bridge_with_api(&dir.join("state.db"), pair.api.base.clone(), KEY);
    seed_bridge_row(&bridge, TEST_FILE_ID, path, None, FileStatus::CloudOnly, SIZE as i64);
    bridge
}

/// The Cloud Files callback's board sequence around its download: the row first, `follow` beside the
/// download, `finished()` on success; an `Err` returns the row to the caller, whose drop is the failure.
async fn callback_fetch(bridge: &EngineBridge) -> (anyhow::Result<FetchedBytes>, crate::transfer_progress::FetchRow) {
    let meter = ReceiveMeter::default();
    let row = bridge.whole_fetch_row(TEST_FILE_ID, SIZE as u64);
    let result = tokio::select! {
        biased;
        result = bridge.hydrate_file_to_memory_metered(TEST_FILE_ID, Some(&meter)) => result,
        never = row.follow(&meter) => match never {},
    };
    (result, row)
}

fn rows(bridge: &EngineBridge) -> Vec<ActiveTransferDto> {
    bridge.transfers().snapshot_dtos()
}

/// Brief U1.1: one named `down` row whose progress moves while the file arrives, gone once it is
/// transferred. Red on the mutants "never begin" (no row) and "begin unnamed" (name and folder "").
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ld0105_u1_a_whole_file_fetch_is_one_named_download_row_that_moves_and_ends() {
    let dir = tempfile::tempdir().unwrap();
    let pair = Pair::start(KEY, TEST_FILE_ID, 1024, SIZE);
    for chunk in 0..11 {
        // Each chunk's body trickles in, so the fetch takes over a second and the board is seen mid-way.
        pair.knobs()
            .storage
            .insert(chunk, std::collections::VecDeque::from([StorageScript::Trickle(128, Duration::from_millis(80))]));
    }
    let bridge = bridge_at(dir.path(), &pair, "Videos/holiday-2026.mp4");
    bridge.seams.use_transport(Transport::Auto);
    let board = bridge.transfers().clone();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sampler = {
        let (board, stop) = (board.clone(), stop.clone());
        tokio::spawn(async move {
            let mut samples = Vec::new();
            while !stop.load(Ordering::SeqCst) {
                samples.push(board.snapshot_dtos());
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            samples
        })
    };
    let (result, row) = callback_fetch(&bridge).await;
    stop.store(true, Ordering::SeqCst);
    let samples = sampler.await.unwrap();
    let fetched = result.expect("the fetch succeeds");
    assert_eq!(fetched.bytes.to_vec(), pair.plain());
    let during: Vec<ActiveTransferDto> = samples.iter().filter(|s| !s.is_empty()).map(|s| {
        assert_eq!(s.len(), 1, "one row for the file, never two: {s:?}");
        s[0].clone()
    }).collect();
    println!("ld0105-u1 whole fetch: {} samples, {} with the row, done {:?}, speed {:?}", samples.len(), during.len(),
        during.iter().map(|r| r.done).collect::<std::collections::BTreeSet<_>>(),
        during.iter().map(|r| r.speed_bps).collect::<std::collections::BTreeSet<_>>());
    assert!(!during.is_empty(), "the fetch registered its row (mutant \"never begin\")");
    for r in &during {
        assert_eq!(
            (r.file_id.as_str(), r.name.as_str(), r.folder.as_str(), r.direction, r.failed),
            (TEST_FILE_ID, "holiday-2026.mp4", "Videos", Direction::Down, false),
            "named by its vault path, the form the tray matches on (mutant \"begin unnamed\")"
        );
        assert!(r.done <= r.total && r.total == SIZE as u64, "kept progress, never past the total: {r:?}");
    }
    let mid: std::collections::BTreeSet<u64> = during.iter().map(|r| r.done).filter(|d| *d > 0 && *d < SIZE as u64).collect();
    assert!(mid.len() >= 2, "the progress moves while the file arrives: {mid:?}");
    // Self-review F7: the speed comes only from `follow` (the meter, every second); the per-chunk
    // reports alone leave it 0 and the row would read "0 B/s".
    let fastest = during.iter().map(|r| r.speed_bps).max().unwrap_or(0);
    assert!(fastest > 0, "follow gave the row a speed (mutant \"no follow\")");
    row.finished();
    assert!(rows(&bridge).is_empty(), "the row ends once the whole file is transferred");
}

/// Brief U1.1: a fetch that fails leaves one named row marked failed (the "Couldn't finish
/// downloading" row, kept 15 minutes); the next fetch of the file replaces it, never a second row, and
/// ends it. Red on the mutant "never mark_failed".
#[tokio::test]
async fn ld0105_u1_a_failed_whole_file_fetch_is_marked_failed_and_the_retry_ends_it() {
    let dir = tempfile::tempdir().unwrap();
    let pair = Pair::start(KEY, TEST_FILE_ID, 1024, SIZE);
    pair.knobs().api_chunks.insert(3, std::collections::VecDeque::from([ApiScript::Status(403)]));
    let bridge = bridge_at(dir.path(), &pair, "Videos/lecture-03.mp4");
    bridge.seams.use_transport(Transport::Api);
    let (result, row) = callback_fetch(&bridge).await;
    assert!(result.is_err(), "the scripted 403 fails the fetch");
    drop(row);
    let after = rows(&bridge);
    assert_eq!(after.len(), 1, "{after:?}");
    let failed = &after[0];
    assert!(failed.failed && failed.failed_at_secs.is_some(), "marked failed (mutant \"never mark_failed\"): {failed:?}");
    assert_eq!((failed.name.as_str(), failed.folder.as_str(), failed.direction), ("lecture-03.mp4", "Videos", Direction::Down));

    // Opened again: the retry replaces the failed row (one row), and its success ends it.
    let (result, row) = callback_fetch(&bridge).await;
    assert_eq!(result.expect("the retry succeeds").bytes.to_vec(), pair.plain());
    let during = rows(&bridge);
    assert_eq!(during.len(), 1, "the retry is the file's one row: {during:?}");
    assert!(!during[0].failed, "the old failure is not the retry's");
    row.finished();
    assert!(rows(&bridge).is_empty());
}

/// Brief U1.2: an upload's row carries the file's name (it was "" on the native rung at e93800c). The
/// UI keeps uploads off the download surfaces (`downloadProgressModel.ts`); the name is the DTO's truth.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ld0105_u1_an_upload_row_is_named_by_its_vault_path() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("resume.bin");
    std::fs::write(&payload, b"0123456789abcdefghij").unwrap();
    let server = ResumableUploadMock::start(ResumeMockMode::HangOnce);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [21u8; 32]);
    seed_bridge_row(&bridge, "local-file-resume", "Documents/resume.bin", None, FileStatus::Uploading, 20);
    bridge.db.enqueue_operation(&resumable_create_op(&payload)).unwrap();
    let board = bridge.transfers().clone();
    let seen = async {
        for _ in 0..200 {
            if let Some(row) = board.snapshot_dtos().into_iter().find(|r| r.file_id == "local-file-resume" && r.done >= 8) {
                return Some(row);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        None
    };
    let (cut, seen) = tokio::join!(tokio::time::timeout(Duration::from_millis(1500), bridge.process_due_operations(dir.path(), 200)), seen);
    assert!(cut.is_err(), "chunk 1 hangs, so the pass is cut mid-upload");
    let row = seen.expect("the upload showed on the board");
    assert_eq!((row.name.as_str(), row.folder.as_str(), row.direction), ("resume.bin", "Documents", Direction::Up));
    drop(server.finish());
}

/// Self-review F3: a new engine (sign-in, unlock, account switch) starts on the shared board without
/// the failed rows, and file names, an earlier engine left. Red on the mutant "`with_transfers` keeps them".
#[test]
fn ld0105_u1_a_new_engine_never_shows_an_earlier_engines_failed_rows() {
    let dir = tempfile::tempdir().unwrap();
    let board = crate::transfer_progress::TransferBoard::new();
    drop(crate::transfer_progress::FetchRow::new(board.begin_download(TEST_FILE_ID, "Private/payslip.pdf", 10)));
    assert_eq!(board.snapshot_dtos().len(), 1, "precondition: the earlier engine's failed row");
    let _next = test_bridge(&dir.path().join("state.db")).with_transfers(board.clone());
    let rows = serde_json::to_string(&board.snapshot_dtos()).unwrap();
    assert!(!rows.contains("payslip"), "the next session sees none of it: {rows}");
}

/// The board's rows carry names now: a locked vault's poll shows none (lead 2026-11-06, "no file
/// name while the vault is locked"). Red on the mutant "`shown_transfers` passes the board through".
#[test]
fn ld0105_u1_a_locked_poll_shows_no_name_of_a_board_row() {
    let board = crate::transfer_progress::TransferBoard::new();
    let _row = board.begin_download(TEST_FILE_ID, "Videos/holiday-2026.mp4", 10);
    let locked = serde_json::to_string(&crate::account_commands::shown_transfers(board.snapshot_dtos(), false, &[])).unwrap();
    let unlocked = serde_json::to_string(&crate::account_commands::shown_transfers(board.snapshot_dtos(), true, &[])).unwrap();
    assert!(!locked.contains("holiday-2026") && !locked.contains("Videos"), "locked: no name, no folder: {locked}");
    assert!(unlocked.contains("holiday-2026.mp4"), "the control: unlocked, the name shows: {unlocked}");
}

/// The Windows Cloud Files callback runs the sequence the tests above exercise: after the streaming
/// path (unchanged) declines the fetch, it starts the row, follows the meter beside the download, and
/// ends the row only where the whole transfer succeeded; every other exit drops it (failed). Red on the
/// mutants "never begin", "no follow" and "finished on every exit".
#[test]
fn ld0105_u1_the_cloud_files_callback_runs_that_sequence() {
    use crate::source_pin::squeeze;
    let source = squeeze(include_str!("../../windows_cf/callbacks.rs"));
    let start = source.find(&squeeze("pub unsafe extern \"system\" fn fetch_data_callback(")).unwrap();
    let body = &source[start..start + source[start..].find(&squeeze("/// Resolve a hydration size mismatch")).unwrap()];
    let at = |needle: &str| body.find(&squeeze(needle)).unwrap_or_else(|| panic!("{needle} not in fetch_data_callback"));
    let streamed = at("super::streaming::serve(&streamed, &bridge, &handle)");
    let begin = at("let row = bridge.whole_fetch_row(&file_id, u64::try_from(file_size).unwrap_or(0));");
    let block = at("let buf = handle.block_on(async {");
    let download = at("result = bridge.hydrate_file_to_memory_metered(&file_id, Some(&meter)) => result,");
    let follow = at("never = row.follow(&meter) => match never {}");
    let transfer = at("match transfer_ok {");
    let ok = at("Ok(()) => { row.finished();");
    assert!(streamed < begin && begin < block && block < download && download < follow && follow < transfer && transfer < ok);
    assert_eq!(body.matches("row.finished()").count(), 1, "the row ends as finished only after a whole transfer");
    assert_eq!(body.matches("whole_fetch_row(").count(), 1);
}

/// The `get_active_transfers` and `desktop_recent_files` JSON of a downloading row, a failed one and an
/// upload, as the commands serialise them, is `tests/fixtures/active-transfers.ld0105-u1.json`, which
/// the browser rung (`tests/render-u1-board.mjs`) renders: the fixture is this test's output, never a
/// hand-written mock. `failedAtSecs` (the wall clock) is compared as present. To write it again:
/// `BB_WRITE_U1_FIXTURE=1 cargo test ld0105_u1_the_ipc_rows_are_the_shared_fixture`.
#[test]
fn ld0105_u1_the_ipc_rows_are_the_shared_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let (down, failed, up) = ("a1050000-0000-4000-8000-000000000001", "a1050000-0000-4000-8000-000000000002", "a1050000-0000-4000-8000-000000000003");
    for (id, path, status, size) in [
        (down, "Videos/holiday-2026.mp4", FileStatus::Downloading, 349_000_000),
        (failed, "Videos/lecture-03.mp4", FileStatus::Error, 212_000_000),
        (up, "Documents/notes-draft.txt", FileStatus::Uploading, 48_000),
    ] {
        seed_bridge_row(&bridge, id, path, None, status, size);
    }
    // The downloading fetch: 118.2 of 349 MB, 9.6 MB/s over the last 2 s (the U0 design's row).
    let downloading = bridge.whole_fetch_row(down, 349_000_000);
    let t0 = std::time::Instant::now();
    for (s, kept) in [(0, 99_000_000), (1, 108_600_000), (2, 118_200_000)] {
        bridge.transfers().received_at(down, kept, t0 + Duration::from_secs(s));
    }
    drop(bridge.whole_fetch_row(failed, 212_000_000));
    let upload = bridge.transfers().begin_named(up, "Documents/notes-draft.txt", Direction::Up, 48_000);
    upload.update(16_384);

    let mut transfers = crate::account_commands::shown_transfers(bridge.transfers().snapshot_dtos(), true, &[]);
    transfers.sort_by(|a, b| a.file_id.cmp(&b.file_id));
    let recent = crate::account_commands::recent_files(&bridge.db, 15).unwrap();
    let emitted = serde_json::json!({
        "source": "src-tauri/src/engine_bridge/tests/ld0105_u1_rows.rs ld0105_u1_the_ipc_rows_are_the_shared_fixture: the commands' own JSON",
        "get_active_transfers": transfers,
        "desktop_recent_files": recent,
    });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/active-transfers.ld0105-u1.json");
    if std::env::var_os("BB_WRITE_U1_FIXTURE").is_some() {
        std::fs::write(&path, serde_json::to_string_pretty(&emitted).unwrap() + "\n").unwrap();
    }
    let clock = |mut v: serde_json::Value| {
        for row in v["get_active_transfers"].as_array_mut().unwrap() {
            if row["failedAtSecs"].is_u64() {
                row["failedAtSecs"] = "wall clock".into();
            }
        }
        v
    };
    let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(clock(emitted.clone()), clock(fixture), "the shared fixture is what the commands emit: {emitted:#}");
    drop((downloading, upload));
}
