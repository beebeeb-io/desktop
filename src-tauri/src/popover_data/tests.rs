//! The snapshot assembly's tests: the cached storage summary (TTL, backoff and
//! in-flight dedupe), the activity rows and conflicts from a state-DB view, the
//! assembled DTO against the fixtures in `tests/fixtures`, and the review
//! retarget payload.
//!
//! They reach the parent's private helpers through `use super::*`, so they test
//! the same items `crate::popover_data` exposes; nothing here talks to the
//! network, the keychain or a window.

    use super::*;
    use crate::state_db::{FileStatus, ItemKind};
    use serde_json::json;

    fn entry(id: &str, path: &str, status: FileStatus, size: i64, at: i64) -> FileEntry {
        FileEntry {
            file_id: id.into(),
            path: path.into(),
            status,
            size_bytes: size,
            modified_at: at,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        }
    }

    fn done(id: &str, dir: &str, path: &str, at: i64) -> TransferActivity {
        TransferActivity {
            id: at,
            direction: dir.into(),
            file_id: Some(id.into()),
            file_name: display_name(path),
            rel_path: Some(path.into()),
            bytes: 10,
            occurred_at: at,
        }
    }

    fn view(payload: serde_json::Value) -> EngineStatusView {
        let mut v = EngineStatusView::default();
        v.apply(&payload);
        v
    }

    fn healthy() -> SnapshotInputs {
        SnapshotInputs {
            now: 1_000,
            logged_in: true,
            vault_unlocked: true,
            auth_expired: false,
            paused: false,
            email: Some("sam@example.eu".into()),
            engine: view(
                json!({ "state": "idle", "files_remaining": 0, "bytes_total": 0, "bytes_done": 0, "last_tick_ok_at": 990 }),
            ),
            finder: FinderSetup::Ready,
            finder_reason: None,
            finder_failure_elsewhere: false,
            storage: Some(StorageDto {
                used_bytes: 84_300_000_000,
                quota_bytes: 200_000_000_000,
                fetched_at: 900,
                stale: false,
            }),
            backlog: TransferBacklog::default(),
            conflicts: vec![],
            in_flight: vec![],
            failed: vec![],
            recent: vec![],
            board: vec![],
            activity_limit: DEFAULT_ACTIVITY_LIMIT,
        }
    }

    // ── phase, one test per state ───────────────────────────────────────────

    fn phase_of(inputs: SnapshotInputs) -> PopoverPhase {
        assemble(inputs).phase
    }

    #[test]
    fn every_phase_is_reachable_from_the_inputs_the_command_gathers() {
        use PopoverPhase::*;
        assert_eq!(phase_of(healthy()), Synced);

        let mut i = healthy();
        i.logged_in = false;
        i.vault_unlocked = false;
        assert_eq!(phase_of(i), SignedOut);

        let mut i = healthy();
        i.auth_expired = true;
        assert_eq!(phase_of(i), SessionEnded);

        let mut i = healthy();
        i.vault_unlocked = false;
        assert_eq!(phase_of(i), Locked);

        let mut i = healthy();
        i.finder = FinderSetup::Failed;
        assert_eq!(phase_of(i), FinderFailed);

        let mut i = healthy();
        i.finder = FinderSetup::UserDisabled;
        assert_eq!(
            phase_of(i),
            FinderUserDisabled,
            "lead ruling: Finder user_disabled has its own phase"
        );

        let mut i = healthy();
        i.finder = FinderSetup::Adding;
        assert_eq!(phase_of(i), FinderAdding);

        let mut i = healthy();
        i.finder = FinderSetup::Missing;
        assert_eq!(phase_of(i), FinderMissing);

        let mut i = healthy();
        i.paused = true;
        assert_eq!(phase_of(i), Paused);

        let mut i = healthy();
        i.storage = Some(StorageDto {
            used_bytes: 200,
            quota_bytes: 200,
            fetched_at: 1,
            stale: false,
        });
        assert_eq!(phase_of(i), StorageFull);

        let mut i = healthy();
        i.engine = view(json!({ "state": "offline", "reason": { "code": "connect", "detail": null } }));
        assert_eq!(phase_of(i), Offline);

        let mut i = healthy();
        i.engine = view(json!({ "state": "error", "reason": { "code": "timeout", "detail": "timeout after 30 s" } }));
        assert_eq!(phase_of(i), Error);

        let mut i = healthy();
        i.engine = view(json!({ "state": "syncing", "files_remaining": 3 }));
        assert_eq!(phase_of(i), Syncing);
    }

    #[test]
    fn a_file_on_the_board_is_syncing_even_when_the_last_event_said_idle() {
        let mut i = healthy();
        i.board = vec![(
            "u1".into(),
            Transfer {
                direction: Direction::Up,
                done: 1,
                total: 9,
            },
        )];
        assert_eq!(phase_of(i.clone()), PopoverPhase::Syncing);
        // Higher phases still win.
        i.paused = true;
        assert_eq!(phase_of(i), PopoverPhase::Paused);
    }

    #[test]
    fn a_quota_paused_upload_is_storage_full_even_when_the_usage_call_failed() {
        let mut i = healthy();
        i.storage = None;
        i.backlog.paused_for_quota = 2;
        let s = assemble(i);
        assert!(s.storage_full);
        assert_eq!(s.phase, PopoverPhase::StorageFull);
    }

    #[test]
    fn a_zero_quota_is_unknown_not_full() {
        assert!(!storage_is_full(&StorageUsage {
            used_bytes: 0,
            quota_bytes: 0
        }));
        assert!(!storage_is_full(&StorageUsage {
            used_bytes: 5,
            quota_bytes: -1
        }));
        assert!(storage_is_full(&StorageUsage {
            used_bytes: 200,
            quota_bytes: 200
        }));
        assert!(!storage_is_full(&StorageUsage {
            used_bytes: 199,
            quota_bytes: 200
        }));
    }

    #[test]
    fn a_failure_shown_elsewhere_does_not_paint_f2_here() {
        let mut i = healthy();
        i.finder = FinderSetup::Failed;
        i.finder_failure_elsewhere = true;
        assert_eq!(phase_of(i), PopoverPhase::FinderMissing);
    }

    /// Task 2179: the sync part of the state alone is the same reducer with the Finder setup taken as ready, for every
    /// Finder setup and every sync state, so the main window keeps no second copy of the precedence.
    #[test]
    fn the_sync_phase_is_the_phase_with_the_finder_taken_as_ready() {
        type Set = fn(&mut SnapshotInputs);
        let sync_states: [(&str, Set, PopoverPhase); 8] = [
            ("synced", |_| {}, PopoverPhase::Synced),
            ("paused", |i| i.paused = true, PopoverPhase::Paused),
            ("offline", |i| i.engine = view(json!({ "state": "offline" })), PopoverPhase::Offline),
            ("error", |i| i.engine = view(json!({ "state": "error", "reason": { "code": "timeout", "detail": null } })), PopoverPhase::Error),
            // An error with the `auth` reason is the server answering 401: online (engine_status `connectivity`).
            ("error, auth", |i| i.engine = view(json!({ "state": "error", "reason": { "code": "auth", "detail": null } })), PopoverPhase::Synced),
            // The engine's own word is not the pause: only the live flag is (phase.rs).
            ("engine says paused, flag off", |i| i.engine = view(json!({ "state": "paused" })), PopoverPhase::Synced),
            ("syncing", |i| i.engine = view(json!({ "state": "syncing" })), PopoverPhase::Syncing),
            // #139 review P3-13: storage full was missing from the matrix.
            ("storage full", |i| i.backlog.paused_for_quota = 1, PopoverPhase::StorageFull),
        ];
        let setups = [
            (FinderSetup::Ready, None),
            (FinderSetup::Failed, Some(PopoverPhase::FinderFailed)),
            (FinderSetup::UserDisabled, Some(PopoverPhase::FinderUserDisabled)),
            (FinderSetup::Adding, Some(PopoverPhase::FinderAdding)),
            (FinderSetup::Missing, Some(PopoverPhase::FinderMissing)),
        ];
        let mut cases = 0;
        for (label, set, sync) in sync_states {
            for (setup, finder_phase) in setups {
                let mut i = healthy();
                i.finder = setup;
                set(&mut i);
                let dto = assemble(i);
                assert_eq!(dto.sync_phase, sync, "{label} under {setup:?}: the sync part");
                assert_eq!(dto.phase, finder_phase.unwrap_or(sync), "{label} under {setup:?}: the popover's phase");
                cases += 1;
            }
        }
        assert_eq!(cases, 40);
    }

    #[test]
    fn a_401_error_is_not_a_connectivity_phase() {
        let mut i = healthy();
        i.engine = view(json!({ "state": "error", "reason": { "code": "auth", "detail": null } }));
        assert_eq!(phase_of(i), PopoverPhase::Synced);
    }

    // ── the serialized contract ─────────────────────────────────────────────

    /// The JSON the TypeScript side validates (`tests/popoverContract.test.ts` runs the same
    /// files through `parsePopoverSnapshot`), so the two languages cannot drift apart silently.
    fn fixture(name: &str) -> serde_json::Value {
        let text = match name {
            "synced" => include_str!("../../../tests/fixtures/popover-snapshot.synced.json"),
            "syncing" => include_str!("../../../tests/fixtures/popover-snapshot.syncing.json"),
            "error" => include_str!("../../../tests/fixtures/popover-snapshot.error.json"),
            other => panic!("no fixture {other}"),
        };
        serde_json::from_str(text).expect("fixture is JSON")
    }

    #[test]
    fn the_healthy_snapshot_serializes_to_exactly_the_shared_fixture() {
        assert_eq!(serde_json::to_value(assemble(healthy())).unwrap(), fixture("synced"));
    }

    fn error_inputs() -> SnapshotInputs {
        let mut i = healthy();
        i.engine = view(
            json!({ "state": "error", "reason": { "code": "timeout", "detail": "timeout after 30 s" }, "last_tick_ok_at": 500 }),
        );
        i
    }

    #[test]
    fn the_error_snapshot_serializes_to_exactly_the_shared_fixture() {
        assert_eq!(
            serde_json::to_value(assemble(error_inputs())).unwrap(),
            fixture("error")
        );
    }

    fn syncing_inputs() -> SnapshotInputs {
        let mut i = healthy();
        i.engine = view(json!({
            "state": "syncing", "files_remaining": 14,
            "bytes_total": 1_200_000_000_u64, "bytes_done": 456_000_000_u64
        }));
        i.in_flight = vec![
            entry("u1", "Investigations/ledger.xlsx", FileStatus::Uploading, 100, 50),
            entry("d1", "Notes/meeting.md", FileStatus::Downloading, 200, 40),
        ];
        i.board = vec![(
            "u1".into(),
            Transfer {
                direction: Direction::Up,
                done: 62,
                total: 100,
            },
        )];
        i.failed = vec![entry("e1", "Notes/bad.bin", FileStatus::Error, 10, 20)];
        i.recent = vec![done("r1", "up", "Reports/q3.pdf", 10)];
        i
    }

    #[test]
    fn the_syncing_snapshot_serializes_to_exactly_the_shared_fixture() {
        assert_eq!(
            serde_json::to_value(assemble(syncing_inputs())).unwrap(),
            fixture("syncing")
        );
    }

    #[test]
    fn the_reason_is_omitted_when_the_link_is_fine() {
        let mut i = healthy();
        // A stale reason left in the view must not leak into a healthy snapshot.
        i.engine.reason_code = Some("timeout".into());
        assert_eq!(assemble(i).reason, None);
    }

    #[test]
    fn the_finder_failure_carries_the_reason_line() {
        let mut i = healthy();
        i.finder = FinderSetup::Failed;
        i.finder_reason = Some(FinderFailureReason::Timeout);
        let s = serde_json::to_value(assemble(i)).unwrap();
        assert_eq!(
            s["finder"],
            json!({ "setup": "failed", "reason": "timeout", "reason_line": "reason: timeout" })
        );
    }

    #[test]
    fn pending_changes_and_conflicts_are_reported() {
        let mut i = healthy();
        i.backlog.queued_ops = 7;
        i.conflicts = vec![
            entry("c2", "Docs/b.txt", FileStatus::Conflict, 1, 200),
            entry("c1", "Docs/a.txt", FileStatus::Conflict, 1, 100),
        ];
        let s = assemble(i);
        assert_eq!(s.pending_changes, 7);
        assert_eq!(s.conflicts.count, 2);
        assert_eq!(
            s.conflicts.files[0],
            ConflictFile {
                file_id: "c1".into(),
                file_name: "a.txt".into()
            }
        );
        assert_eq!(s.conflicts.files[1].file_id, "c2");
    }

    #[test]
    fn only_a_handful_of_conflicts_are_named_but_all_are_counted() {
        let mut i = healthy();
        i.conflicts = (0..9)
            .map(|n| entry(&format!("c{n}"), &format!("f{n}.txt"), FileStatus::Conflict, 1, n))
            .collect();
        let s = assemble(i);
        assert_eq!(s.conflicts.count, 9);
        assert_eq!(s.conflicts.files.len(), CONFLICT_FILES_LIMIT);
    }

    // ── activity rows ───────────────────────────────────────────────────────

    #[test]
    fn rows_are_active_first_then_failed_then_done_with_direction_each() {
        let mut i = healthy();
        i.in_flight = vec![
            entry("d1", "A/down.bin", FileStatus::Downloading, 10, 30),
            entry("u1", "A/up.bin", FileStatus::Uploading, 10, 40),
        ];
        i.failed = vec![entry("e1", "A/bad.bin", FileStatus::Error, 10, 20)];
        i.recent = vec![done("r1", "up", "B/sent.txt", 10), done("r2", "down", "B/got.txt", 5)];
        let rows = build_activity_rows(&i);
        let shape: Vec<(&str, ActivityState, Option<Direction>)> =
            rows.iter().map(|r| (r.id.as_str(), r.state, r.direction)).collect();
        assert_eq!(
            shape,
            vec![
                ("u1", ActivityState::Active, Some(Direction::Up)),
                ("d1", ActivityState::Active, Some(Direction::Down)),
                ("e1", ActivityState::Failed, None),
                ("r1", ActivityState::Done, Some(Direction::Up)),
                ("r2", ActivityState::Done, Some(Direction::Down)),
            ]
        );
    }

    #[test]
    fn an_active_row_without_board_bytes_has_no_percentage_to_show() {
        let mut i = healthy();
        i.in_flight = vec![entry("u1", "x.bin", FileStatus::Uploading, 10, 1)];
        let rows = build_activity_rows(&i);
        assert_eq!(
            (rows[0].done_bytes, rows[0].total_bytes),
            (None, None),
            "state b', never a guessed percentage"
        );
    }

    #[test]
    fn a_finished_transfer_of_a_file_moving_again_is_not_listed_twice() {
        let mut i = healthy();
        i.in_flight = vec![entry("x", "f.bin", FileStatus::Uploading, 10, 50)];
        i.recent = vec![done("x", "up", "f.bin", 40), done("y", "up", "g.bin", 30)];
        let ids: Vec<String> = build_activity_rows(&i).into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["x".to_string(), "y".to_string()]);
    }

    #[test]
    fn a_file_uploaded_twice_is_one_finished_row_the_newest() {
        let mut i = healthy();
        i.recent = vec![done("x", "up", "f.bin", 40), done("x", "up", "f.bin", 10)];
        let rows = build_activity_rows(&i);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].at, 40);
    }

    #[test]
    fn the_list_is_capped_and_the_cap_is_clamped() {
        let mut i = healthy();
        i.recent = (0..30)
            .map(|n| done(&format!("r{n}"), "up", &format!("f{n}.txt"), 100 - n))
            .collect();
        i.activity_limit = 5;
        assert_eq!(build_activity_rows(&i).len(), 5);
        i.activity_limit = 0;
        assert_eq!(build_activity_rows(&i).len(), 1, "a zero limit still returns one row");
        i.activity_limit = 500;
        assert_eq!(build_activity_rows(&i).len(), MAX_ACTIVITY_LIMIT);
    }

    #[test]
    fn a_row_path_splits_into_name_folder_name_and_path() {
        let mut i = healthy();
        i.recent = vec![
            done("r", "down", "Work/2026/Q3/ledger.xlsx", 3),
            done("s", "up", "/Top/notes.md", 2),
            done("t", "up", "root.txt", 1),
        ];
        let rows = build_activity_rows(&i);
        let shape: Vec<(&str, &str, &str)> = rows
            .iter()
            .map(|r| (r.name.as_str(), r.folder.as_str(), r.path.as_str()))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("ledger.xlsx", "Q3", "Work/2026/Q3/ledger.xlsx"),
                ("notes.md", "Top", "Top/notes.md"),
                ("root.txt", "", "root.txt"),
            ],
            "the short label is the immediate parent folder; the path has no leading slash"
        );
    }

    // ── finder ──────────────────────────────────────────────────────────────

    // The Finder state itself is the reconciler's published view (spec 2026-10-06): the saved
    // install state and the "adding" flag these tests used to map are gone with `finder_setup_for`
    // and `PopoverRuntime::finder_adding`. `lib.rs`'s popover command tests cover the view.

    #[test]
    fn the_finder_reason_line_adds_the_typed_reason_and_fits_30_characters() {
        assert_eq!(
            finder_reason_line(FinderSetup::Failed, Some(FinderFailureReason::Timeout)).as_deref(),
            Some("reason: timeout")
        );
        assert_eq!(finder_reason_line(FinderSetup::Failed, None), None);
        assert_eq!(
            finder_reason_line(FinderSetup::Missing, Some(FinderFailureReason::Timeout)),
            None,
            "only a failure has a reason line"
        );
    }

    #[test]
    fn every_reason_line_fits_30_characters() {
        for reason in FinderFailureReason::ALL {
            let line =
                finder_reason_line(FinderSetup::Failed, Some(reason)).expect("a failure with a reason has a line");
            assert!(line.chars().count() <= 30, "{line}");
            assert_eq!(
                line,
                format!("reason: {}", reason.as_str()),
                "the typed reason, whole, never a cut word"
            );
        }
    }

    // ── the storage cache ───────────────────────────────────────────────────

    fn usage(used: i64) -> StorageUsage {
        StorageUsage {
            used_bytes: used,
            quota_bytes: 200,
        }
    }

    fn block<T>(f: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn a_fresh_summary_is_served_without_fetching() {
        let cache = Arc::new(Mutex::new(StorageCache::default()));
        let calls = std::cell::Cell::new(0);
        let fetch = || {
            calls.set(calls.get() + 1);
            async { Ok(usage(10)) }
        };
        let a = block(cached_storage(&cache, 1_000, 7, fetch)).unwrap();
        assert_eq!((a.used_bytes, a.stale, a.fetched_at), (10, false, 1_000));
        // 20 "opens" inside the TTL: still one fetch.
        for n in 1..=20 {
            let again = block(cached_storage(&cache, 1_000 + n, 7, || {
                calls.set(calls.get() + 1);
                async { Ok(usage(99)) }
            }))
            .unwrap();
            assert_eq!(again.used_bytes, 10, "served from the cache");
        }
        assert_eq!(calls.get(), 1, "21 snapshots inside the TTL must fetch exactly once");
    }

    #[test]
    fn the_summary_is_refetched_after_the_ttl() {
        let cache = Arc::new(Mutex::new(StorageCache::default()));
        block(cached_storage(&cache, 0, 1, || async { Ok(usage(10)) })).unwrap();
        let just_inside = block(cached_storage(&cache, STORAGE_TTL_SECS - 1, 1, || async {
            Ok(usage(20))
        }))
        .unwrap();
        assert_eq!(just_inside.used_bytes, 10);
        let at_ttl = block(cached_storage(&cache, STORAGE_TTL_SECS, 1, || async { Ok(usage(30)) })).unwrap();
        assert_eq!(at_ttl.used_bytes, 30, "at the TTL the summary is refetched");
    }

    #[test]
    fn a_new_session_refetches_even_inside_the_ttl() {
        // Spec: "refetched after unlock". The session revision changes on unlock.
        let cache = Arc::new(Mutex::new(StorageCache::default()));
        block(cached_storage(&cache, 0, 1, || async { Ok(usage(10)) })).unwrap();
        let after_unlock = block(cached_storage(&cache, 5, 2, || async { Ok(usage(55)) })).unwrap();
        assert_eq!(after_unlock.used_bytes, 55);
    }

    #[test]
    fn a_failed_refetch_serves_the_old_number_marked_stale() {
        let cache = Arc::new(Mutex::new(StorageCache::default()));
        block(cached_storage(&cache, 0, 1, || async { Ok(usage(10)) })).unwrap();
        let stale = block(cached_storage(&cache, STORAGE_TTL_SECS + 50, 1, || async {
            Err("offline".to_string())
        }))
        .unwrap();
        assert_eq!((stale.used_bytes, stale.stale, stale.fetched_at), (10, true, 0));
    }

    #[test]
    fn a_failed_fetch_with_nothing_cached_is_no_summary_and_another_sessions_is_not_reused() {
        let cache = Arc::new(Mutex::new(StorageCache::default()));
        assert_eq!(
            block(cached_storage(&cache, 0, 1, || async { Err("offline".to_string()) })),
            None
        );
        // Task 2179: after a failure the API is not asked again for the backoff.
        block(cached_storage(&cache, STORAGE_FAILURE_BACKOFF_SECS, 1, || async { Ok(usage(10)) })).unwrap();
        assert_eq!(
            block(cached_storage(&cache, STORAGE_FAILURE_BACKOFF_SECS + 1, 2, || async {
                Err("offline".to_string())
            })),
            None,
            "a summary from another session must never be shown"
        );
    }

    // ── task 2179 item 1: a slow or failing API never holds a snapshot ──────

    fn counting(calls: &Arc<std::sync::atomic::AtomicUsize>) -> Arc<std::sync::atomic::AtomicUsize> {
        Arc::clone(calls)
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_fetch_holds_a_snapshot_for_the_wait_at_most_and_serves_the_next_one() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = Arc::new(Mutex::new(StorageCache::default()));
        let calls = Arc::new(AtomicUsize::new(0));
        let slow = |calls: Arc<AtomicUsize>| {
            move || async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_secs(8)).await;
                Ok(usage(42))
            }
        };
        let started = tokio::time::Instant::now();
        let first = cached_storage(&cache, 1_000, 1, slow(counting(&calls))).await;
        assert_eq!(first, None, "nothing cached yet: no summary rather than an 8 s wait");
        assert_eq!(started.elapsed(), STORAGE_WAIT, "the snapshot waited exactly the wait");
        // A snapshot while the fetch is still running waits for that one, never starts another.
        let second = cached_storage(&cache, 1_001, 1, slow(counting(&calls))).await;
        assert_eq!(second, None);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "one fetch in flight at a time");
        // The fetch finishes in the background and fills the cache: the next snapshot is served at once.
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        let before = tokio::time::Instant::now();
        let third = cached_storage(&cache, 1_010, 1, slow(counting(&calls))).await.unwrap();
        assert_eq!((third.used_bytes, third.stale), (42, false));
        assert_eq!(before.elapsed(), std::time::Duration::ZERO, "served from the cache, no wait");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn the_wait_is_the_callers_a_test_passes_its_own() {
        // #139 review P3-11: the wait is an argument, so no test depends on the real STORAGE_WAIT.
        use std::sync::atomic::AtomicUsize;
        let cache = Arc::new(Mutex::new(StorageCache::default()));
        let calls = Arc::new(AtomicUsize::new(0));
        let wait = std::time::Duration::from_millis(250);
        let started = tokio::time::Instant::now();
        assert_eq!(cached_storage_with(&cache, 1_000, 1, wait, slow_ok(&calls, 8, 1)).await, None);
        assert_eq!(started.elapsed(), wait);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_fetch_is_not_tried_again_for_the_backoff() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = Arc::new(Mutex::new(StorageCache::default()));
        let calls = Arc::new(AtomicUsize::new(0));
        let failing = |calls: Arc<AtomicUsize>| {
            move || async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err::<StorageUsage, String>("offline".to_string())
            }
        };
        assert_eq!(cached_storage(&cache, 0, 1, failing(counting(&calls))).await, None);
        for t in [1, 10, STORAGE_FAILURE_BACKOFF_SECS - 1] {
            let started = tokio::time::Instant::now();
            assert_eq!(cached_storage(&cache, t, 1, failing(counting(&calls))).await, None);
            assert_eq!(started.elapsed(), std::time::Duration::ZERO, "t={t}: answered at once");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1, "not asked again inside the backoff");
        cached_storage(&cache, STORAGE_FAILURE_BACKOFF_SECS, 1, failing(counting(&calls))).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2, "asked again after it");
        // Another session is not held back by this one's failure.
        cached_storage(&cache, STORAGE_FAILURE_BACKOFF_SECS + 1, 2, failing(counting(&calls))).await;
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    fn slow_ok(calls: &Arc<std::sync::atomic::AtomicUsize>, secs: u64, used: i64) -> impl FnOnce() -> std::pin::Pin<Box<dyn Future<Output = Result<StorageUsage, String>> + Send>> {
        let calls = Arc::clone(calls);
        move || {
            Box::pin(async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                Ok(usage(used))
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_late_answer_from_an_older_session_does_not_replace_the_newer_ones_summary() {
        // #139 review P2-3: a sign-out and a sign-in happen while session 1's fetch is still out.
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = Arc::new(Mutex::new(StorageCache::default()));
        let (old, new) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        assert_eq!(cached_storage(&cache, 1_000, 1, slow_ok(&old, 10, 1)).await, None);
        let fresh = cached_storage(&cache, 1_001, 2, slow_ok(&new, 0, 2)).await.unwrap();
        assert_eq!(fresh.used_bytes, 2);
        tokio::time::sleep(std::time::Duration::from_secs(10)).await; // session 1's answer arrives now
        let again = cached_storage(&cache, 1_012, 2, slow_ok(&new, 0, 3)).await.unwrap();
        assert_eq!((again.used_bytes, again.stale), (2, false), "still session 2's own summary");
        assert_eq!((old.load(Ordering::SeqCst), new.load(Ordering::SeqCst)), (1, 1), "no extra fetch for session 2");
    }

    #[tokio::test(start_paused = true)]
    async fn a_late_answer_from_an_older_session_keeps_the_newer_ones_backoff() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = Arc::new(Mutex::new(StorageCache::default()));
        let (old, new) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        assert_eq!(cached_storage(&cache, 1_000, 1, slow_ok(&old, 10, 1)).await, None);
        let failing = { let new = Arc::clone(&new); move || async move { new.fetch_add(1, Ordering::SeqCst); Err::<StorageUsage, String>("offline".into()) } };
        assert_eq!(cached_storage(&cache, 1_001, 2, failing).await, None);
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        let inside = { let new = Arc::clone(&new); move || async move { new.fetch_add(1, Ordering::SeqCst); Ok::<StorageUsage, String>(usage(9)) } };
        assert_eq!(cached_storage(&cache, 1_012, 2, inside).await, None, "session 2 is still inside its backoff");
        assert_eq!(new.load(Ordering::SeqCst), 1, "not asked again inside the backoff");
    }

    #[tokio::test(start_paused = true)]
    async fn a_fetch_still_running_at_sign_out_answers_into_nothing_even_after_a_new_fetch_started() {
        // #139 review P2-3: sign-out clears the cache; the old account's fetch must not write after it, also not once a
        // new fetch (the next sign-in, the same epoch if nothing bumped it) holds the slot.
        use std::sync::atomic::AtomicUsize;
        let cache = Arc::new(Mutex::new(StorageCache::default()));
        let calls = Arc::new(AtomicUsize::new(0));
        assert_eq!(cached_storage(&cache, 1_000, 1, slow_ok(&calls, 10, 5)).await, None);
        clear_storage(&cache);
        assert!(cache.lock().unwrap().is_empty());
        assert_eq!(cached_storage(&cache, 1_001, 1, slow_ok(&calls, 20, 6)).await, None);
        tokio::time::sleep(std::time::Duration::from_secs(10)).await; // the old account's answer arrives
        assert!(cache.lock().unwrap().entry.is_none(), "the old fetch wrote nothing: {:?}", cache.lock().unwrap());
        tokio::time::sleep(std::time::Duration::from_secs(11)).await;
        assert_eq!(cache.lock().unwrap().entry.map(|e| e.usage.used_bytes), Some(6), "the new fetch's answer lands");
    }

    #[tokio::test(start_paused = true)]
    async fn a_fetch_task_that_panics_frees_the_slot_for_the_next_snapshot() {
        // #139 review P3-8: a panicked (or cancelled) fetch task must not hold the single-flight slot forever.
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = Arc::new(Mutex::new(StorageCache::default()));
        let boom = || async {
            if std::hint::black_box(true) {
                panic!("the fetch panicked (on purpose, in a test)");
            }
            Ok::<StorageUsage, String>(usage(0))
        };
        assert_eq!(cached_storage(&cache, 1_000, 1, boom).await, None);
        tokio::task::yield_now().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let next = cached_storage(&cache, 1_001, 1, slow_ok(&calls, 0, 7)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1, "a new fetch, not a wait on the dead one");
        assert_eq!(next.map(|s| s.used_bytes), Some(7));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failure_is_dated_when_it_comes_not_when_the_fetch_started() {
        // #139 review P3-9: an 8 s fetch that fails at 1 008 keeps the next one off until 1 038, not 1 030.
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = Arc::new(Mutex::new(StorageCache::default()));
        let calls = Arc::new(AtomicUsize::new(0));
        let slow_fail = { let calls = Arc::clone(&calls); move || async move {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
            Err::<StorageUsage, String>("timeout".into())
        } };
        assert_eq!(cached_storage(&cache, 1_000, 1, slow_fail).await, None);
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        let probe = { let calls = Arc::clone(&calls); move || async move { calls.fetch_add(1, Ordering::SeqCst); Ok::<StorageUsage, String>(usage(1)) } };
        assert_eq!(cached_storage(&cache, 1_000 + STORAGE_FAILURE_BACKOFF_SECS + 1, 1, probe).await, None);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "1 031 is 23 s after the failure: still inside the backoff");
    }

    // ── the real fetch, against a loopback API ──────────────────────────────

    fn serve_once(raw: &'static str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let _ = stream.write_all(raw.as_bytes());
            request
        });
        (base, handle)
    }

    #[test]
    fn the_storage_fetch_calls_billing_usage_with_the_bearer_token_and_parses_it() {
        let (base, server) = serve_once(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 53\r\nconnection: close\r\n\r\n{\"used_bytes\":84300000000,\"quota_bytes\":200000000000}",
        );
        // reqwest honours proxy env vars; a loopback API must be reached directly.
        let got = block(fetch_storage_usage(&base, "tok-123")).unwrap();
        assert_eq!(
            got,
            StorageUsage {
                used_bytes: 84_300_000_000,
                quota_bytes: 200_000_000_000
            }
        );
        let request = server.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /api/v1/billing/usage"), "{request}");
        assert!(request.contains("authorization: bearer tok-123"), "{request}");
    }

    #[test]
    fn a_401_from_the_usage_call_is_an_error_that_does_not_carry_the_token() {
        let (base, server) = serve_once("HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        let err = block(fetch_storage_usage(&base, "secret-token-abc")).unwrap_err();
        server.join().unwrap();
        assert!(err.contains("401"), "{err}");
        assert!(!err.contains("secret-token-abc"), "{err}");
    }

    // ── the review:open contract ────────────────────────────────────────────

    #[test]
    fn the_review_event_is_camel_case_like_the_url_it_replaces() {
        let ev = ReviewOpen::new("f-1", "ledger.xlsx", false, ReviewMode::Conflict).unwrap();
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            json!({ "fileId": "f-1", "fileName": "ledger.xlsx", "isText": false, "mode": "conflict" })
        );
        // The same file the TypeScript test parses.
        let shared: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/review-open.conflict.json")).unwrap();
        assert_eq!(serde_json::to_value(&ev).unwrap(), shared);
        let back: ReviewOpen =
            serde_json::from_value(json!({ "fileId": "f-2", "fileName": "a", "isText": true, "mode": "versions" }))
                .unwrap();
        assert_eq!(back.mode, ReviewMode::Versions);
        assert!(back.is_text);
        assert_eq!(REVIEW_OPEN_EVENT, "review:open");
    }

    #[test]
    fn a_review_event_with_no_file_id_is_refused_and_a_missing_name_gets_the_window_default() {
        assert_eq!(ReviewOpen::new("", "x", false, ReviewMode::Conflict), None);
        assert_eq!(ReviewOpen::new("   ", "x", false, ReviewMode::Conflict), None);
        assert_eq!(
            ReviewOpen::new("f", "", false, ReviewMode::Versions).unwrap().file_name,
            "Unknown file"
        );
    }

    // ── from a real state DB ────────────────────────────────────────────────

    fn seed(db: &crate::state_db::StateDb, id: &str, path: &str, status: FileStatus, size: i64, at: i64) {
        db.upsert_file(&entry(id, path, status, size, at)).unwrap();
    }

    fn queued(id: &str, kind: crate::state_db::OperationKind, file_id: &str) -> crate::state_db::PendingOperation {
        crate::state_db::PendingOperation {
            op_id: id.into(),
            kind,
            file_id: Some(file_id.into()),
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
        }
    }

    #[test]
    fn the_snapshot_reads_the_queue_as_of_now_and_skips_an_upload_in_retry_backoff() {
        use crate::state_db::{OperationKind, StateDb};
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed(&db, "a", "A/due.bin", FileStatus::Local, 100, 1);
        seed(&db, "b", "A/waiting.bin", FileStatus::Local, 5_000, 1);
        let mut due = queued("op-a", OperationKind::UploadFile, "a");
        due.next_retry_at = 50;
        let mut waiting = queued("op-b", OperationKind::UploadFile, "b");
        waiting.attempts = 1;
        waiting.next_retry_at = 500;
        db.enqueue_operation(&due).unwrap();
        db.enqueue_operation(&waiting).unwrap();

        let at_100 = gather_db_view(&db, 5, 100).unwrap().backlog;
        assert_eq!(
            (at_100.upload_files, at_100.upload_bytes),
            (1, 100),
            "only op-a is due at t=100"
        );
        let at_500 = gather_db_view(&db, 5, 500).unwrap().backlog;
        assert_eq!(
            (at_500.upload_files, at_500.upload_bytes),
            (2, 5_100),
            "op-b is due again at t=500"
        );
    }

    #[test]
    fn a_real_state_db_becomes_the_snapshot_the_popover_shows() {
        use crate::state_db::{OperationKind, OperationPauseReason, StateDb, TransferActivityInput};
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed(&db, "u1", "Investigations/ledger.xlsx", FileStatus::Uploading, 100, 50);
        seed(&db, "d1", "Notes/meeting.md", FileStatus::Downloading, 200, 40);
        seed(&db, "e1", "Notes/bad.bin", FileStatus::Error, 10, 20);
        seed(&db, "c1", "Docs/plan.txt", FileStatus::Conflict, 5, 30);
        seed(&db, "l1", "Reports/q3.pdf", FileStatus::Local, 10, 10);
        seed(&db, "p1", "Big/paused.bin", FileStatus::Local, 999, 5);
        db.enqueue_operation(&queued("op-u1", OperationKind::UploadFile, "u1"))
            .unwrap();
        db.enqueue_operation(&queued("op-p1", OperationKind::UploadFile, "p1"))
            .unwrap();
        db.record_operation_pause("op-p1", OperationPauseReason::Quota, Some("quota exceeded"), 9)
            .unwrap();
        db.record_transfer_activity(TransferActivityInput {
            direction: "up",
            file_id: Some("l1".into()),
            file_name: "q3.pdf".into(),
            rel_path: Some("Reports/q3.pdf".into()),
            bytes: 10,
            occurred_at: 10,
        })
        .unwrap();

        let view = gather_db_view(&db, 5, 100).unwrap();
        assert_eq!(view.backlog.upload_files, 1, "op-u1 is due, op-p1 is paused for quota");
        assert_eq!(view.backlog.download_files, 1);
        assert_eq!(view.backlog.queued_ops, 2);
        assert_eq!(view.backlog.paused_for_quota, 1);
        assert_eq!(view.in_flight.len(), 2);
        assert_eq!(view.failed.len(), 1);
        assert_eq!(view.conflicts.len(), 1);
        assert_eq!(view.recent.len(), 1);

        let mut inputs = healthy();
        inputs.engine = view_of_syncing();
        inputs.backlog = view.backlog;
        inputs.conflicts = view.conflicts;
        inputs.in_flight = view.in_flight;
        inputs.failed = view.failed;
        inputs.recent = view.recent;
        inputs.board = vec![(
            "u1".into(),
            Transfer {
                direction: Direction::Up,
                done: 62,
                total: 100,
            },
        )];
        let snapshot = assemble(inputs);
        // A quota-paused upload outranks syncing (spec precedence: storage full above syncing).
        assert_eq!(snapshot.phase, PopoverPhase::StorageFull);
        assert!(snapshot.storage_full);
        assert_eq!(snapshot.pending_changes, 2);
        assert_eq!(
            snapshot.conflicts.files,
            vec![ConflictFile {
                file_id: "c1".into(),
                file_name: "plan.txt".into()
            }]
        );
        let rows: Vec<(&str, ActivityState)> = snapshot.activity.iter().map(|r| (r.id.as_str(), r.state)).collect();
        assert_eq!(
            rows,
            vec![
                ("u1", ActivityState::Active),
                ("d1", ActivityState::Active),
                ("e1", ActivityState::Failed),
                ("l1", ActivityState::Done)
            ]
        );
        assert_eq!(
            snapshot.activity[0].done_bytes,
            Some(62),
            "the board's bytes reach the row"
        );
        assert_eq!(
            snapshot.activity[1].done_bytes, None,
            "a download the board does not know has no percentage"
        );
    }

    fn view_of_syncing() -> EngineStatusView {
        view(json!({ "state": "syncing", "files_remaining": 2, "bytes_total": 300, "bytes_done": 62 }))
    }

    #[test]
    fn an_empty_state_db_is_an_empty_snapshot_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        let view = gather_db_view(&db, 8, 100).unwrap();
        assert_eq!(view.backlog, TransferBacklog::default());
        assert!(
            view.conflicts.is_empty() && view.in_flight.is_empty() && view.failed.is_empty() && view.recent.is_empty()
        );
    }
