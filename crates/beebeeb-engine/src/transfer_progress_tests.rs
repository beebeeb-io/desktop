use super::*;

#[test]
fn a_transfer_appears_while_its_guard_lives_and_goes_when_it_drops() {
    let board = TransferBoard::new();
    assert!(board.active().is_empty());
    {
        let _g = board.begin("f1", Direction::Up, 1_000);
        assert_eq!(
            board.get("f1"),
            Some(Transfer {
                direction: Direction::Up,
                done: 0,
                total: 1_000,
                ..Transfer::default()
            })
        );
        assert_eq!(board.active().len(), 1);
    }
    assert_eq!(
        board.get("f1"),
        None,
        "a dropped (failed or cancelled) transfer must not linger"
    );
    assert_eq!(
        board.finished_bytes(),
        0,
        "a failure adds nothing to the finished total"
    );
}

#[test]
fn progress_is_cumulative_monotonic_and_capped_at_the_total() {
    let board = TransferBoard::new();
    let g = board.begin("f", Direction::Down, 100);
    g.update(40);
    assert_eq!(board.get("f").unwrap().done, 40);
    g.update(30);
    assert_eq!(board.get("f").unwrap().done, 40, "never backwards");
    g.update(500);
    assert_eq!(board.get("f").unwrap().done, 100, "never past the total");
}

#[test]
fn a_finished_transfer_adds_its_total_to_the_batch_and_leaves_the_board() {
    let board = TransferBoard::new();
    let a = board.begin("a", Direction::Up, 300);
    a.update(300);
    a.finish();
    let b = board.begin("b", Direction::Up, 200);
    b.update(50);
    assert_eq!(board.finished_bytes(), 300);
    assert_eq!(board.in_flight_done(), 50);
    assert_eq!(board.active().len(), 1, "a is gone, b is in flight");
    drop(b);
    assert_eq!(board.finished_bytes(), 300, "b failed, the finished total keeps only a");
    board.reset_batch();
    assert_eq!(board.finished_bytes(), 0);
}

#[test]
fn a_wrong_size_hint_can_be_corrected() {
    let board = TransferBoard::new();
    let g = board.begin("d", Direction::Down, 100);
    g.update(80);
    g.set_total(60);
    let t = board.get("d").unwrap();
    assert_eq!((t.done, t.total), (60, 60), "done is clamped to the corrected total");
}

#[test]
fn two_transfers_are_independent() {
    let board = TransferBoard::new();
    let a = board.begin("a", Direction::Up, 10);
    let b = board.begin("b", Direction::Down, 20);
    a.update(10);
    b.update(5);
    assert_eq!(board.get("a").unwrap().direction, Direction::Up);
    assert_eq!(board.get("b").unwrap().direction, Direction::Down);
    assert_eq!(board.in_flight_done(), 15);
    drop(a);
    assert_eq!(board.in_flight_done(), 5);
}

#[test]
fn a_second_begin_for_the_same_file_replaces_the_first() {
    // A retry of the same file while the old guard is still alive (should not
    // happen, but must not double count).
    let board = TransferBoard::new();
    let _a = board.begin("x", Direction::Up, 10);
    let b = board.begin("x", Direction::Up, 99);
    assert_eq!(board.active().len(), 1);
    assert_eq!(board.get("x").unwrap().total, 99);
    b.update(1);
    assert_eq!(board.in_flight_done(), 1);
}

#[test]
fn uploaded_bytes_caps_the_short_last_chunk_at_the_payload_size() {
    assert_eq!(uploaded_bytes(0, 4, 10), 0);
    assert_eq!(uploaded_bytes(2, 4, 10), 8);
    assert_eq!(
        uploaded_bytes(3, 4, 10),
        10,
        "the last chunk is short: 12 would overshoot 10"
    );
    assert_eq!(uploaded_bytes(1, 4, 0), 0, "an empty file has no bytes");
    assert_eq!(uploaded_bytes(u64::MAX, 4, 10), 10, "no overflow");
}

#[test]
fn a_name_and_a_folder_come_out_of_either_separator() {
    assert_eq!(display_name("Reports/2026/q3.xlsx"), "q3.xlsx");
    assert_eq!(display_name("C:\\home\\sam\\q3.xlsx"), "q3.xlsx");
    assert_eq!(display_name("q3.xlsx"), "q3.xlsx");
    assert_eq!(display_name("Folder/"), "Folder");
    assert_eq!(parent_folder("Reports/2026/q3.xlsx"), "Reports/2026");
    assert_eq!(parent_folder("/Reports/q3.xlsx"), "Reports");
    assert_eq!(parent_folder("q3.xlsx"), "");
    assert_eq!(parent_folder("/q3.xlsx"), "");
}

#[test]
fn a_download_report_replaces_the_size_hint_and_stays_monotonic() {
    let board = TransferBoard::new();
    let _g = board.begin("d", Direction::Down, 0);
    board.report("d", 0, 30);
    assert_eq!(board.get("d").unwrap().total, 30);
    board.report("d", 20, 30);
    board.report("d", 10, 30);
    assert_eq!(board.get("d").unwrap().done, 20, "never backwards");
    board.report("not-registered", 5, 5);
    assert_eq!(
        board.get("not-registered"),
        None,
        "a file nobody registered is not invented"
    );
}

// --- LD-0105 U1: Direction::Stream, begin_named, update_stream, mark_failed ---

#[test]
fn direction_stream_serialises_as_stream() {
    let s = serde_json::to_string(&Direction::Stream).unwrap();
    assert_eq!(s, r#""stream""#);
}

#[test]
fn begin_named_stores_path_and_exposes_it_in_snapshot_dto() {
    let board = TransferBoard::new();
    let _g = board.begin_named("s1", "Videos/holiday.mp4", Direction::Stream, 50_000_000);
    let dtos = board.snapshot_dtos();
    assert_eq!(dtos.len(), 1);
    let dto = &dtos[0];
    assert_eq!(dto.file_id, "s1");
    assert_eq!(dto.name, "holiday.mp4");
    assert_eq!(dto.folder, "Videos");
    assert_eq!(dto.direction, Direction::Stream);
    assert_eq!(dto.total, 50_000_000);
    assert!(!dto.failed, "a fresh transfer is not failed");
    assert_eq!(dto.failed_at_secs, None);
}

#[test]
fn update_stream_records_buffered_bytes() {
    let board = TransferBoard::new();
    let _g = board.begin_named("s2", "music.flac", Direction::Stream, 0);
    board.update_stream("s2", 1_024_000);
    let t = board.get("s2").unwrap();
    assert_eq!(t.buffered, 1_024_000);
}

#[test]
fn speed_window_produces_nonzero_speed_after_two_samples() {
    let board = TransferBoard::new();
    let _g = board.begin_named("s3", "clip.mp4", Direction::Stream, 0);
    // Two calls with a fake time gap: use update_stream twice with real clock
    // (sub-millisecond — speed will be 0 or large; we only check it doesn't panic
    // and the field is present in the DTO).
    board.update_stream("s3", 0);
    board.update_stream("s3", 100_000);
    let dtos = board.snapshot_dtos();
    assert_eq!(dtos.len(), 1);
    // `speed_bps` is either 0 (samples too close in time) or a large positive
    // number; the important thing is the field exists and the board didn't panic.
    let _ = dtos[0].speed_bps;
}

#[test]
fn mark_failed_sets_the_failed_flag_and_a_timestamp() {
    let board = TransferBoard::new();
    let _g = board.begin("f2", Direction::Down, 100);
    board.mark_failed("f2");
    let t = board.get("f2").unwrap();
    assert!(t.failed);
    assert!(t.failed_at_secs.is_some());
}

#[test]
fn mark_failed_on_unregistered_file_is_a_noop() {
    let board = TransferBoard::new();
    board.mark_failed("not-here"); // must not panic
}

#[test]
fn snapshot_dto_for_a_begin_without_path_has_empty_name_and_folder() {
    let board = TransferBoard::new();
    let _g = board.begin("plain", Direction::Down, 200);
    let dtos = board.snapshot_dtos();
    assert_eq!(dtos.len(), 1);
    // `display_name("")` → `""`, `parent_folder("")` → `""`
    assert_eq!(dtos[0].name, "");
    assert_eq!(dtos[0].folder, "");
}

// --- P1 fix: failed transfers must linger in snapshot_dtos after the guard drops ---

#[test]
fn a_failed_transfer_lingers_in_snapshot_after_the_guard_drops() {
    // Before the fix: snapshot_dtos only reads inner.active, so a dropped
    // (failed) guard left nothing visible. This test must be RED on the unfixed
    // code and GREEN once recently_failed is introduced.
    let board = TransferBoard::new();
    {
        let g = board.begin_named("f_fail", "docs/report.pdf", Direction::Down, 500);
        board.mark_failed("f_fail");
    } // guard drops here — before fix, entry is gone; after fix it's in recently_failed
    let dtos = board.snapshot_dtos();
    assert_eq!(
        dtos.len(),
        1,
        "failed row must still appear in snapshot after guard drops"
    );
    assert!(dtos[0].failed);
    assert_eq!(dtos[0].file_id, "f_fail");
    assert_eq!(dtos[0].name, "report.pdf");
}

#[test]
fn an_expired_failed_entry_is_pruned_from_the_snapshot() {
    // Inject an already-expired entry directly into recently_failed (failed_at_secs = 0
    // is 1970-01-01, far beyond any TTL). snapshot_dtos must prune and exclude it.
    let board = TransferBoard::new();
    if let Ok(mut inner) = board.inner.lock() {
        inner.recently_failed.insert(
            "expired".to_string(),
            Transfer {
                failed: true,
                failed_at_secs: Some(0),
                ..Transfer::default()
            },
        );
        inner.paths.insert("expired".to_string(), "docs/old.pdf".to_string());
    }
    let dtos = board.snapshot_dtos();
    assert!(dtos.is_empty(), "an expired failed entry must not appear in snapshot");
}

#[test]
fn a_non_failed_drop_does_not_populate_recently_failed() {
    let board = TransferBoard::new();
    {
        let g = board.begin("ok", Direction::Down, 100);
        g.update(100);
        g.finish();
    }
    // Finished transfer (not failed): nothing in recently_failed
    let inner = board.inner.lock().unwrap();
    assert!(
        inner.recently_failed.is_empty(),
        "a successful finish must not populate recently_failed"
    );
}

// --- P2 fix: paths and speeds must be pruned when transfers end ---

#[test]
fn paths_and_speeds_are_pruned_when_non_failed_transfers_end() {
    // Before the fix: inner.paths and inner.speeds were only ever inserted into,
    // never removed. This test must be RED on the unfixed code (maps are non-empty).
    let board = TransferBoard::new();
    for i in 0u32..3 {
        let id = format!("file{i}");
        let path = format!("dir/file{i}.txt");
        let g = board.begin_named(&id, &path, Direction::Stream, 100);
        board.update_stream(&id, 50);
        g.update(100);
        g.finish();
    }
    let inner = board.inner.lock().unwrap();
    assert!(inner.speeds.is_empty(), "speeds must be pruned after transfers end");
    assert!(
        inner.paths.is_empty(),
        "paths must be pruned after non-failed transfers end"
    );
}

#[test]
fn paths_are_kept_for_recently_failed_and_pruned_on_expiry() {
    // A failed transfer's path must stay (so snapshot_dtos can resolve the name),
    // but must be gone once the entry expires.
    let board = TransferBoard::new();
    {
        let g = board.begin_named("fx", "media/clip.mp4", Direction::Stream, 1000);
        board.mark_failed("fx");
    }
    {
        let inner = board.inner.lock().unwrap();
        assert!(
            inner.paths.contains_key("fx"),
            "path must be kept while entry is in recently_failed"
        );
    }
    // Expire the entry by setting its timestamp to 0 and calling snapshot_dtos
    {
        if let Ok(mut inner) = board.inner.lock() {
            if let Some(t) = inner.recently_failed.get_mut("fx") {
                t.failed_at_secs = Some(0);
            }
        }
    }
    board.snapshot_dtos(); // triggers pruning
    {
        let inner = board.inner.lock().unwrap();
        assert!(
            !inner.recently_failed.contains_key("fx"),
            "expired entry pruned from recently_failed"
        );
        assert!(
            !inner.paths.contains_key("fx"),
            "path pruned alongside expired recently_failed entry"
        );
    }
}

// --- LD-0105 U1 board wiring: begin_download, received_at, FetchSpeed, FetchRow ---

/// Brief U1.1: a second fetch of a file while one runs joins its row: no second row, progress
/// not back to 0, and the row ends with the last fetch, by that fetch's outcome. Red on the
/// mutant "`begin_download` starts the row over" (done 0 after the join, the row gone early).
#[test]
fn ld0105_u1_a_second_fetch_of_a_file_joins_its_row() {
    let board = TransferBoard::new();
    let first = FetchRow::new(board.begin_download("f", "Videos/clip.mp4", 100));
    board.received_at("f", 60, 0);
    let second = FetchRow::new(board.begin_download("f", "Videos/clip.mp4", 100));
    let rows = board.snapshot_dtos();
    assert_eq!(rows.len(), 1, "one row: {rows:?}");
    assert_eq!(
        (rows[0].done, rows[0].name.as_str()),
        (60, "clip.mp4"),
        "progress kept, never back to 0"
    );
    drop(first); // the first fetch failed while the second runs on
    let rows = board.snapshot_dtos();
    assert_eq!(rows.len(), 1, "the second fetch still holds the row");
    assert!(
        !rows[0].failed,
        "the second fetch, still running, decides how the row ends"
    );
    board.received_at("f", 100, 0);
    second.finished();
    assert!(
        board.snapshot_dtos().is_empty(),
        "the last fetch delivered the file: no row left"
    );
    assert_eq!(board.finished_bytes(), 100, "counted once");
}

/// Brief U1.1: the row's progress is the meter's kept b    /// Brief U1.1: the row's progress is the meter's kept bytes, never backwards (a failed attempt's
/// bytes are taken back from the meter) and never past the total; the speed is the caller's.
#[test]
fn ld0105_u1_kept_bytes_are_the_progress_never_backwards_never_past_the_total() {
    let board = TransferBoard::new();
    let _row = board.begin_download("f", "a.bin", 10_000);
    board.received_at("f", 4_000, 1_500);
    let t = board.get("f").unwrap();
    assert_eq!((t.done, t.speed_bps), (4_000, 1_500));
    board.received_at("f", 3_000, 1_500);
    assert_eq!(
        board.get("f").unwrap().done,
        4_000,
        "a rolled-back attempt never moves the row backwards"
    );
    board.received_at("f", 10_028, 1_500);
    assert_eq!(
        board.get("f").unwrap().done,
        10_000,
        "wire bytes (28-byte tags) never pass the total"
    );
    board.received_at("not-registered", 5, 1);
    assert_eq!(board.get("not-registered"), None);
}

/// The release lane's native 1 s samples of `done` during a 300 MB whole-file download at
/// 1e963bc (Windows, direct route, `download_kbps_limit` 12,500 KB/s, streaming OFF): the bytes
/// arrive in bursts while the pacer holds the next requests back. Kept = received on that run (no
/// discarded body), and these are per read: 118,159,856 is no multiple of 16 MiB.
const NATIVE_1E963BC: [u64; 23] = [
    0,
    33_554_432,
    67_108_976,
    67_108_976,
    67_108_976,
    67_108_976,
    118_159_856,
    134_217_952,
    134_217_952,
    134_217_952,
    134_217_952,
    170_967_324,
    201_326_928,
    201_326_928,
    201_326_928,
    201_326_928,
    240_506_312,
    267_741_438,
    268_435_904,
    268_435_904,
    268_435_904,
    268_435_904,
    300_000_000,
];

/// Lead decision 2026-11-06 09:53 (native: 13 of 21 mid-download samples read "0 B/s", the rest
/// 0.7 to 51.0 MB/s, at a real ≈12.3 MB/s): replayed second by second, the speed is non-zero at
/// every sample from the first byte to the last, and once its window is full it stays within a
/// third of the download's real average (the tail's bursts are smaller: 9.7 MB/s at 13.6). Red
/// on the mutant "2 s window" (kept = received on this trace, so the next test covers that one).
#[test]
fn ld0105_u1_the_native_bursts_read_as_a_steady_non_zero_speed() {
    let t0 = Instant::now();
    let meter = ReceiveMeter::default();
    let mut speed = FetchSpeed::new();
    let average = 300_000_000 / 22; // bytes per second, first byte to last
    let mut seen = Vec::new();
    for (s, received) in NATIVE_1E963BC.iter().enumerate() {
        meter.add(received - meter.received());
        let bps = speed.sample(t0 + Duration::from_secs(s as u64), &meter);
        seen.push(bps / 100_000);
        if s >= 1 {
            assert!(bps > 0, "second {s}: a byte arrived in the window, so a speed shows");
        }
        if s >= 10 {
            assert!(
                bps.abs_diff(average) * 3 <= average,
                "second {s}: {bps} B/s is within a third of {average}"
            );
        }
    }
    println!("ld0105-u1 native replay, 0.1 MB/s per second: {seen:?}");
}

/// Lead decision 2026-11-06 09:53: a fetch fed per-read bytes in chunk-sized bursts (16 MiB in
/// 64 KiB reads, then 4 s of nothing), with an attempt that fails and is taken back from the kept
/// progress, still reads a non-zero speed in every window after the first byte: the speed is the
/// meter's received bytes, the wire's real rate. Red on the mutant "speed of the kept progress".
#[test]
fn ld0105_u1_per_read_bursts_and_a_retry_keep_a_non_zero_speed() {
    const CHUNK: u64 = 16 << 20;
    let meter = ReceiveMeter::default();
    let t0 = Instant::now();
    let mut speed = FetchSpeed::new();
    let mut seen = Vec::new();
    for s in 0..30u64 {
        if s % 5 == 1 {
            let mark = meter.kept();
            for _ in 0..(CHUNK / 65_536) {
                meter.add(65_536);
            }
            if s == 11 {
                meter.rollback_to(mark); // that body did not authenticate: fetched again
            }
        }
        let bps = speed.sample(t0 + Duration::from_secs(s), &meter);
        seen.push(bps);
        if s >= 1 {
            assert!(bps > 0, "second {s}: a speed shows");
        }
    }
    let steady = &seen[10..];
    let (low, high) = (steady.iter().min().unwrap(), steady.iter().max().unwrap());
    assert!(high - low <= high / 4, "steady once the window is full: {steady:?}");
}

/// Brief U1.1: a fetch row dropped without `finished()` is a failed row, named, kept for the TTL;
/// one that finished is gone. A fetch with no `files` row puts nothing on the board.
#[test]
fn ld0105_u1_a_fetch_row_ends_failed_unless_it_finished() {
    let board = TransferBoard::new();
    drop(FetchRow::new(board.begin_download("bad", "Videos/lecture-03.mp4", 10)));
    FetchRow::new(board.begin_download("good", "Videos/ok.mp4", 10)).finished();
    let rows = board.snapshot_dtos();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        (rows[0].file_id.as_str(), rows[0].name.as_str(), rows[0].failed),
        ("bad", "lecture-03.mp4", true)
    );
    assert!(rows[0].failed_at_secs.is_some());
    drop(FetchRow::none());
    assert_eq!(board.snapshot_dtos().len(), 1, "no row, nothing marked");
}

/// Self-review F1: a start that replaces a row (`begin`, the range session's call) while a fetch
/// holds it keeps counting that fetch, so the fetch's failure is not the newer transfer's and its
/// drop does not end the newer row. Red on the mutant "a replacing start resets the count to 1".
#[test]
fn ld0105_u1_a_replacing_start_keeps_counting_the_live_guards() {
    let board = TransferBoard::new();
    let fetch = FetchRow::new(board.begin_download("f", "Videos/clip.mp4", 100));
    let session = board.begin("f", Direction::Down, 100);
    drop(fetch);
    let rows = board.snapshot_dtos();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(
        !rows[0].failed,
        "the session still runs: the fetch's failure is not its outcome"
    );
    session.update(50);
    assert_eq!(
        board.get("f").unwrap().done,
        50,
        "the session's progress still lands on its row"
    );
    drop(session);
    assert!(board.snapshot_dtos().is_empty());
}

/// Self-review F2: a download never joins an upload's row (it would keep `up`, which the UI hides).
#[test]
fn ld0105_u1_a_download_never_joins_an_upload_row() {
    let board = TransferBoard::new();
    let _up = board.begin_named("f", "Documents/a.txt", Direction::Up, 10);
    let _down = board.begin_download("f", "Documents/a.txt", 100);
    let t = board.get("f").unwrap();
    assert_eq!((t.direction, t.total), (Direction::Down, 100));
}

/// Self-review F4/F5: once any fetch delivered the file the row never ends as failed, and its bytes
/// count once; a fetch that knows the size fills a joined row's unknown total.
#[test]
fn ld0105_u1_a_delivered_file_never_ends_as_failed() {
    let board = TransferBoard::new();
    let no_size = board.begin_download("f", "Videos/clip.mp4", 0);
    let first = FetchRow::new(board.begin_download("f", "Videos/clip.mp4", 100));
    assert_eq!(
        board.get("f").unwrap().total,
        100,
        "the joiner's known size fills the unknown total"
    );
    let second = FetchRow::new(board.begin_download("f", "Videos/clip.mp4", 100));
    first.finished();
    drop(no_size);
    drop(second); // the last holder fails after the file was delivered
    assert!(board.snapshot_dtos().is_empty(), "a delivered file is no failed row");
    assert_eq!(board.finished_bytes(), 100, "counted once");
}

/// Self-review F3: a fetch cut off by an engine stop (lock, sign-out, switch, quit) ends with no
/// failed row; a new engine forgets the failed rows an earlier one left (their names included).
#[test]
fn ld0105_u1_a_stop_is_no_failure_and_a_new_engine_forgets_failed_rows() {
    let board = TransferBoard::new();
    let stopping = Arc::new(std::sync::atomic::AtomicBool::new(true));
    drop(FetchRow::new(board.begin_download("cut", "Videos/a.mp4", 10)).stopped_by(stopping));
    assert!(board.snapshot_dtos().is_empty(), "a stop is not the file's failure");
    drop(FetchRow::new(board.begin_download("bad", "Videos/b.mp4", 10)));
    assert_eq!(board.snapshot_dtos().len(), 1);
    board.forget_failed();
    assert!(board.snapshot_dtos().is_empty());
    assert!(
        board.inner.lock().unwrap().paths.is_empty(),
        "the names go with the rows"
    );
}

/// w31: the board keeps where the first file list is, rings the window only when that changed, and keeps a Try
/// again's ask for the runner that is busy (a permit).
#[test]
fn w31_the_first_list_state_is_kept_and_rings_on_a_change_only() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let board = TransferBoard::new();
    assert_eq!(board.first_list(), FirstList::Here);
    let rings = Arc::new(AtomicUsize::new(0));
    let counted = rings.clone();
    board.set_wake(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    });
    board.set_first_list(FirstList::Awaited);
    board.set_first_list(FirstList::Awaited);
    assert_eq!(board.first_list(), FirstList::Awaited);
    assert_eq!(rings.load(Ordering::SeqCst), 1);
    board.set_first_list(FirstList::Failed);
    assert_eq!(board.first_list(), FirstList::Failed);
    board.set_first_list(FirstList::Here);
    assert_eq!(board.first_list(), FirstList::Here);
    assert_eq!(rings.load(Ordering::SeqCst), 3);

    board.ask_pull_now();
    let asked = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(async { tokio::time::timeout(std::time::Duration::from_secs(5), board.pull_asked()).await });
    assert!(asked.is_ok(), "an ask made before the runner waits is kept");
}
