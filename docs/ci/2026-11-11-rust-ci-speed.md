# Rust CI speed: measurements and changes (2026-11-11)

Lane C1. Goal (Guus, 2026-11-11): the Rust CI jobs "more efficient and faster … SUPER FAST".
Every number here is a measured count or a measured second; the instrument is named next to it.

## 1. Baseline

### 1.1 GitHub-hosted runners (run of PR #275, measured by the lead)

| Job | Total | Setup + cache | Cargo check | Run Rust tests | libtest line |
|---|---|---|---|---|---|
| Rust test (Linux) | 295 s | ~50 s | – | 237 s | `2668 passed; 6 failed; 7 ignored; finished in 165.17s` |
| Rust check (Windows) | 661 s | ~70 s | 101 s | 482 s | `finished in 320.73s` |
| Rust check (macOS) | 666 s | ~40 s | 80 s | 538 s | `finished in 365.50s` |

### 1.2 The Legion (WSL2, Ryzen 9 9955HX3D, 16 logical CPUs), `v2-next` at `3d39dc9b`

Instrument: `cargo test --locked` with libtest's `--report-time` (per-test seconds), `XDG_CONFIG_HOME`
and `TMPDIR` on the ext4 disk as in CI; `cargo --timings`; `/usr/bin/time`; a timing wrapper around the linker.

| What | Measured |
|---|---|
| Library suite, truth line | `test result: FAILED. 2668 passed; 6 failed; 7 ignored; 0 measured; 0 filtered out; finished in 72.46s` |
| Tests timed | 2674 (2668 passed + 6 failed; the 6 are the known red pin tests PR #279 fixes) |
| Sum of per-test seconds | 914.0 s (16 threads turn that into 72 s of wall time) |
| Top 50 tests | 517.3 s; the other 2624 tests: 396.7 s |
| Tests over 1 s | 103 tests, 596.2 s |
| CPU actually used by the run | user 119 s + sys 56 s of 660 s test-seconds (measured on the run in §3.1): the suite mostly WAITS |
| Clean compile of every test target (`cargo test --no-run`) | 58.5 s wall, 345 s user + 111 s sys, 560 crates |
| The crate's own two compiles inside that | library for tests 49.0 s, library (non-test) 28.5 s, in parallel |
| Incremental rebuild after a one-line change in `src/` | 11.7 s |
| Link of the library test binary | 1.1 s (`rust-lld`, rustc's default linker on x86_64 Linux since 1.90); `libbeebeeb_desktop_lib.so` 2.8 s |
| Size of the library test binary | 771 MB (`debuginfo=2`) |

Distribution of the 2674 per-test times:

| Seconds | Tests | Sum |
|---|---|---|
| ≥ 10 | 12 | 338.4 s |
| 5 – 10 | 15 | 100.2 s |
| 1 – 5 | 76 | 157.7 s |
| 0.5 – 1 | 154 | 98.6 s |
| 0.1 – 0.5 | 850 | 209.1 s |
| 0.01 – 0.1 | 154 | 9.2 s |
| < 0.01 | 1413 | 0.9 s |

By module: `engine_bridge::tests` 236.4 s (563 tests), `recovery_phrase_unlock_tests` 126.5 s (4),
`state_db::tests` 94.6 s (186), `direct_download::tests` 87.7 s (25), `owner_keys::tests` 66.2 s (82),
`auth_v2::password` 43.1 s (3), `own_data_proof::tests` 36.9 s (51), `signout_teardown_tests` 36.5 s (16).

### 1.3 Why the top 20 are slow (each test read)

| # | s | Test | Cause |
|---|---|---|---|
| 1 | 67.2 | `direct_download::tests::ld0105_s0_a_chunk_slower_than_a_minute_…` | **Real time by design**: an 8 MiB chunk trickled at 128 KiB/s over a real socket; asserts `took > 60 s` (Windows' fixed 60 s callback limit). No CPU. |
| 2 | 43.2 | `owner_keys::tests::move_adopt::review_p2_7_a_move_of_more_than_ten_thousand_…` | **fsync**: 10 001 rows seeded, one SQLite commit each (4.3 ms per commit on disk; 7.8 s on tmpfs) |
| 3–5 | 34.9 ×3 | `recovery_phrase_unlock_tests::*` (3 tests) | **KDF in a debug build**: Argon2id (core's production parameters) compiled at `opt-level 0` |
| 6 | 28.4 | `auth_v2::password::tests::the_password_legs_run_cores_sign_in` | **KDF in a debug build**: three OPAQUE logins (Argon2id + curve arithmetic) |
| 7 | 21.9 | `recovery_phrase_unlock_tests::account_without_stored_check_…` | KDF in a debug build |
| 8 | 21.4 | `own_data_proof::tests::stranding::a_five_percent_mixed_database_…` | **fsync**: 25 rounds × 100 rows, one commit each |
| 9 | 16.8 | `state_db::tests::open_and_cache::every_concurrent_first_open_…` | 50 rounds × 8 threads opening one fresh database: migration + fsync, serialised by SQLite's write lock |
| 10 | 14.7 | `auth_v2::password::tests::the_second_factor_leg_…` | KDF in a debug build |
| 11 | 10.0 | `signout_teardown_tests::clear_session_retry_after_unconfirmed_engine_stop_…` | **Real time by design**: an engine that cannot stop; waits the product's 3 s graceful + 2 s force abort windows, then the runtime waits for the stand-in thread's 10 s deadline |
| 12 | 10.0 | `runner::tests::abort::abort_returns_task_unconfirmed_…` | Same (a thread that spins until a 10 s deadline) |
| 13 | 9.9 | `ipc_socket::framing_tests::framing_protocol::a_reply_larger_than_64_kib_…` | fsync: 400 rows seeded one commit each |
| 14 | 7.5 | `state_db::tests::startup_and_accepts::an_accept_already_running_…` | 50 rounds, each: two database opens + a 20 ms sleep that makes the race window |
| 15–18 | 7.0 ×4 | `signout_teardown_tests::{sign_out,lock}_with_an_unconfirmed_engine_stop_…`, `finder_setup_wiring_tests::{the_stop_helper,repairs_engine_stop}_…` | Real time by design: 3 s + 2 s abort windows, stand-in deadline 7 s |
| 19 | 6.9 | `own_data_proof::tests::stranding::every_row_the_server_named_…` | fsync: 3 × 300 rows |
| 20 | 6.3 | `state_db::tests::startup_and_accepts::an_accept_waiting_behind_the_close_…` | 50 rounds of open + commit (fsync) |

Three causes cover the list: **real time by design** (1, 11, 12, 15–18: 115 s), **KDF at opt-level 0**
(3–7, 10: 170 s), **one fsync per SQLite commit** (2, 8, 9, 13, 14, 19, 20: 112 s).

**Test #1 is a floor.** It needs 65–67 s of real time whatever the machine, so no configuration of the
library suite finishes in under ~66 s while it runs on every PR. The Legion's 72 s is that one test plus 5 s.
Shortening it would change what it proves (a chunk "slower than a minute"); it is left as it is and reported
to the lead as a decision (§7).

### 1.4 The 50 slowest tests (baseline, Legion, seconds)

| # | s | test |
|---|---|---|
| 1 | 67.2 | `direct_download::tests::ld0105_s0_a_chunk_slower_than_a_minute_reports_its_bytes_as_they_arrive` |
| 2 | 43.2 | `owner_keys::tests::move_adopt::review_p2_7_a_move_of_more_than_ten_thousand_foreign_items_is_refused` |
| 3 | 34.9 | `recovery_phrase_unlock_tests::verifier_failure_fails_closed` |
| 4 | 34.9 | `recovery_phrase_unlock_tests::account_phrase_is_verified_with_session_then_persisted` |
| 5 | 34.9 | `recovery_phrase_unlock_tests::wrong_but_valid_phrase_is_rejected_and_never_persisted` |
| 6 | 28.4 | `auth_v2::password::tests::the_password_legs_run_cores_sign_in` |
| 7 | 21.9 | `recovery_phrase_unlock_tests::account_without_stored_check_is_refused_like_web` |
| 8 | 21.4 | `own_data_proof::tests::stranding::a_five_percent_mixed_database_is_refused_every_time` |
| 9 | 16.8 | `state_db::tests::open_and_cache::every_concurrent_first_open_of_a_fresh_database_succeeds` |
| 10 | 14.7 | `auth_v2::password::tests::the_second_factor_leg_keeps_the_challenge_after_a_wrong_code` |
| 11 | 10.0 | `signout_teardown_tests::clear_session_retry_after_unconfirmed_engine_stop_cannot_skip_the_stop_gate` |
| 12 | 10.0 | `runner::tests::abort::abort_returns_task_unconfirmed_when_the_task_cannot_be_aborted` |
| 13 | 9.9 | `ipc_socket::framing_tests::framing_protocol::a_reply_larger_than_64_kib_arrives_intact_and_delimited` |
| 14 | 7.5 | `state_db::tests::startup_and_accepts::an_accept_already_running_commits_before_the_close` |
| 15 | 7.0 | `signout_teardown_tests::sign_out_with_an_unconfirmed_engine_stop_purges_no_upload_staging` |
| 16 | 7.0 | `signout_teardown_tests::lock_with_an_unconfirmed_engine_stop_does_not_purge_upload_staging` |
| 17 | 7.0 | `finder_setup_wiring_tests::the_stop_helper_records_an_unconfirmed_stop_and_only_that` |
| 18 | 7.0 | `finder_setup_wiring_tests::repairs_engine_stop_reports_an_unconfirmed_stop_whoever_made_it` |
| 19 | 6.9 | `own_data_proof::tests::stranding::every_row_the_server_named_is_judged_not_a_sample` |
| 20 | 6.3 | `state_db::tests::startup_and_accepts::an_accept_waiting_behind_the_close_queues_nothing` |
| 21 | 6.3 | `ipc_socket::framing_tests::changes_and_thumbnail::list_changes_pages_with_resume_tokens_without_losing_or_repeating` |
| 22 | 6.2 | `engine_bridge::tests::finder_queue_5::a_finder_write_refused_for_a_lapsed_account_lands_once_subscribed` |
| 23 | 6.1 | `engine_bridge::tests::finder_queue_5::a_finder_write_refused_for_no_plan_lands_once_subscribed` |
| 24 | 6.1 | `engine_bridge::tests::finder_queue_5::a_finder_write_refused_after_a_cancelled_trial_lands_once_subscribed` |
| 25 | 6.0 | `state_db::tests::startup_and_accepts::a_finder_change_waiting_behind_the_close_queues_nothing` |
| 26 | 5.7 | `sign_out_revoke_tests::the_server_hears_the_logout_only_after_the_local_clear_with_the_engine_slot_free` |
| 27 | 5.2 | `direct_download::tests::the_pinned_route_rides_out_a_rate_limit_and_a_server_hiccup` |
| 28 | 4.9 | `api_client::direct::tests::ld0105_s0_both_api_chunk_routes_report_each_piece_of_the_body_as_it_arrives` |
| 29 | 4.7 | `signout_teardown_tests::clear_session_when_already_signed_out_skips_the_engine_and_succeeds` |
| 30 | 4.7 | `sign_out_unchanged_elsewhere_tests::the_unsent_gate_is_macos_only` |
| 31 | 4.7 | `state_db::tests::open_and_cache::task_2240_the_cache_is_refitted_when_the_db_grows` |
| 32 | 4.6 | `signout_teardown_tests::a_sign_out_empties_upload_staging_and_the_staged_copies_in_every_staging_root` |
| 33 | 4.0 | `direct_download::tests::no_url_or_signature_reaches_a_log_line_or_an_error` |
| 34 | 3.8 | `engine_bridge::tests::ipc::ipc_hydrate_is_cancelled_and_status_restored_when_the_client_hangs_up` |
| 35 | 3.6 | `engine_bridge::tests::sf2237d::sf2237d_t5_whole_and_range_open` |
| 36 | 3.4 | `engine_bridge::tests::sync::test_sync_tick_ops_429_backs_off_and_preserves_cursor` |
| 37 | 3.3 | `engine_bridge::tests::versions_3::twenty_rapid_saves_land_in_order_and_the_last_wins` |
| 38 | 3.3 | `runner::tests::tick::the_real_pulse_emits_syncing_with_progress_then_idle_and_resets_the_batch` |
| 39 | 3.2 | `signout_teardown_tests::sign_out_clears_the_storage_summary` |
| 40 | 3.1 | `sign_out_revoke_tests::a_revoke_that_fails_or_never_answers_changes_nothing_of_the_sign_out_and_logs_no_token` |
| 41 | 3.0 | `sign_out_revoke_tests::a_sign_out_with_no_token_anywhere_sends_nothing` |
| 42 | 3.0 | `sign_out_revoke_tests::the_revoke_gives_up_after_three_seconds` |
| 43 | 2.9 | `config_write_pin_tests::test_1889_the_real_writers_land_next_to_a_save_that_was_in_flight` |
| 44 | 2.9 | `engine_bridge::tests::refusals_2::a_404_on_every_new_session_parks_the_write` |
| 45 | 2.8 | `engine_bridge::tests::refusals_2::a_server_that_refuses_every_session_with_400_parks_the_write` |
| 46 | 2.7 | `engine_bridge::tests::finder_queue_5::a_finder_write_over_quota_at_complete_lands_once_space_is_free` |
| 47 | 2.7 | `engine_bridge::tests::finder_queue_5::a_finder_write_over_the_object_budget_lands_once_objects_are_freed` |
| 48 | 2.6 | `engine_bridge::tests::finder_queue_5::a_finder_write_over_quota_at_a_chunk_lands_once_space_is_free` |
| 49 | 2.5 | `engine_bridge::tests::backup_uploads::backup_upload_leaves_source_byte_identical` |
| 50 | 2.4 | `engine_bridge::tests::refusals_2::a_session_reaped_during_an_outage_lands_on_a_fresh_upload` |
