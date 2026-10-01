# Graph Report - cap-fix  (2026-10-01)

## Corpus Check
- 232 files · ~500,979 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 3854 nodes · 8848 edges · 47 communities detected
- Extraction: 81% EXTRACTED · 19% INFERRED · 0% AMBIGUOUS · INFERRED: 1678 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 4|Community 4]]
- [[_COMMUNITY_Community 5|Community 5]]
- [[_COMMUNITY_Community 6|Community 6]]
- [[_COMMUNITY_Community 7|Community 7]]
- [[_COMMUNITY_Community 8|Community 8]]
- [[_COMMUNITY_Community 9|Community 9]]
- [[_COMMUNITY_Community 10|Community 10]]
- [[_COMMUNITY_Community 11|Community 11]]
- [[_COMMUNITY_Community 12|Community 12]]
- [[_COMMUNITY_Community 13|Community 13]]
- [[_COMMUNITY_Community 14|Community 14]]
- [[_COMMUNITY_Community 15|Community 15]]
- [[_COMMUNITY_Community 16|Community 16]]
- [[_COMMUNITY_Community 17|Community 17]]
- [[_COMMUNITY_Community 18|Community 18]]
- [[_COMMUNITY_Community 19|Community 19]]
- [[_COMMUNITY_Community 20|Community 20]]
- [[_COMMUNITY_Community 21|Community 21]]
- [[_COMMUNITY_Community 22|Community 22]]
- [[_COMMUNITY_Community 23|Community 23]]
- [[_COMMUNITY_Community 24|Community 24]]
- [[_COMMUNITY_Community 25|Community 25]]
- [[_COMMUNITY_Community 26|Community 26]]
- [[_COMMUNITY_Community 27|Community 27]]
- [[_COMMUNITY_Community 28|Community 28]]
- [[_COMMUNITY_Community 29|Community 29]]
- [[_COMMUNITY_Community 30|Community 30]]
- [[_COMMUNITY_Community 31|Community 31]]
- [[_COMMUNITY_Community 35|Community 35]]
- [[_COMMUNITY_Community 49|Community 49]]
- [[_COMMUNITY_Community 50|Community 50]]
- [[_COMMUNITY_Community 59|Community 59]]
- [[_COMMUNITY_Community 62|Community 62]]
- [[_COMMUNITY_Community 63|Community 63]]
- [[_COMMUNITY_Community 64|Community 64]]
- [[_COMMUNITY_Community 65|Community 65]]
- [[_COMMUNITY_Community 66|Community 66]]
- [[_COMMUNITY_Community 70|Community 70]]
- [[_COMMUNITY_Community 72|Community 72]]
- [[_COMMUNITY_Community 82|Community 82]]
- [[_COMMUNITY_Community 89|Community 89]]
- [[_COMMUNITY_Community 92|Community 92]]
- [[_COMMUNITY_Community 105|Community 105]]

## God Nodes (most connected - your core abstractions)
1. `err` - 144 edges
2. `ApiClient` - 78 edges
3. `load()` - 73 edges
4. `load()` - 72 edges
5. `StateDb` - 71 edges
6. `EngineBridge` - 66 edges
7. `test_bridge_with_api()` - 51 edges
8. `command()` - 49 edges
9. `spawn()` - 43 edges
10. `run()` - 37 edges

## Surprising Connections (you probably didn't know these)
- `emit()` --calls--> `openActivity()`  [INFERRED]
  src-tauri/src/browser_login.rs → src/WindowsTray.tsx
- `purge()` --calls--> `finish()`  [INFERRED]
  src-tauri/src/windows_cf/signout_cleanup.rs → src/Onboarding.tsx
- `visit()` --calls--> `walk()`  [INFERRED]
  tests/syncRoot.test.ts → eslint-rules/no-ad-hoc-error-surface.mjs
- `rustPhases()` --calls--> `expect()`  [INFERRED]
  tests/popoverContract.test.ts → BeebeebFileProviderTests/main.swift
- `open()` --calls--> `mount()`  [INFERRED]
  tests/macSettingsTabs.test.tsx → src-tauri/src/linux_fuse/mod.rs

## Communities

### Community 0 - "Community 0"
Cohesion: 0.01
Nodes (353): load(), provenance_headers(), ensure_directory(), legacy_state(), sync_status_engine(), tray_tooltip(), macos_hydrate_cache_dir(), serve_ipc() (+345 more)

### Community 1 - "Community 1"
Cohesion: 0.02
Nodes (250): is_conflict(), is_text_file(), a_failed_upload_leaves_nothing_on_the_board_and_no_activity_row(), a_finished_upload_adds_its_bytes_to_the_batch_and_records_an_up_row(), a_hydrate_reports_per_file_bytes_and_records_a_down_row(), a_hydrate_without_a_caller_callback_still_fills_the_board(), a_resumed_upload_starts_its_bar_at_the_acknowledged_watermark(), an_upload_reports_its_bytes_chunk_by_chunk_and_a_cut_one_leaves_nothing() (+242 more)

### Community 2 - "Community 2"
Cohesion: 0.01
Nodes (148): diagnostics(), lock(), refresh(), runAction(), signOut(), toggleStartAtLogin(), unlock(), toggleFolder() (+140 more)

### Community 3 - "Community 3"
Cohesion: 0.02
Nodes (144): Account, AppHandle, AppState, Attempt, Auth, AuthVault, Client, Commands (+136 more)

### Community 4 - "Community 4"
Cohesion: 0.02
Nodes (110): BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts, myFiles, offline (+102 more)

### Community 5 - "Community 5"
Cohesion: 0.03
Nodes (93): allowed_label(), seed(), allowed_group_labels(), an_exhausted_or_auth_paused_upload_is_not_work_left(), an_upload_waiting_out_a_retry_backoff_is_not_a_file_left(), bandwidth_samples_insert_and_history(), bandwidth_samples_prune(), BandwidthSample (+85 more)

### Community 6 - "Community 6"
Cohesion: 0.03
Nodes (67): err, account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S>, clear_session_removes_account_email() (+59 more)

### Community 7 - "Community 7"
Cohesion: 0.02
Nodes (111): account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity, AccountActivityEvent, AccountActivitySummary, AccountProfile (+103 more)

### Community 8 - "Community 8"
Cohesion: 0.03
Nodes (43): a_failure_then_an_answer_leaves_the_link_up(), a_typed_get_answered_with_401_is_an_answer_not_a_link_failure(), a_typed_get_answered_with_503_records_the_server_not_answering(), a_typed_get_that_is_answered_records_an_answer(), a_typed_get_with_nothing_listening_records_offline(), api_client_list_files_sends_provenance_headers_on_the_wire(), api_client_zeroizes_master_key_on_drop(), ApiClient (+35 more)

### Community 9 - "Community 9"
Cohesion: 0.05
Nodes (97): IPCWriteRequest, mount(), AccountConfig, AccountId, AccountRuntime, active_account_after_synthesis_is_default_shape(), active_account_empty_registry_errs(), fixed_id() (+89 more)

### Community 10 - "Community 10"
Cohesion: 0.03
Nodes (85): a_401_error_is_not_a_connectivity_phase(), a_401_from_the_usage_call_is_an_error_that_does_not_carry_the_token(), a_failed_fetch_with_nothing_cached_is_no_summary_and_another_sessions_is_not_reused(), a_failed_refetch_serves_the_old_number_marked_stale(), a_failure_shown_elsewhere_does_not_paint_f2_here(), a_file_on_the_board_is_syncing_even_when_the_last_event_said_idle(), a_file_uploaded_twice_is_one_finished_row_the_newest(), a_finished_transfer_of_a_file_moving_again_is_not_listed_twice() (+77 more)

### Community 11 - "Community 11"
Cohesion: 0.04
Nodes (67): api_client_search_shard_urls_match_web_paths(), build_local_index(), compare_records_for_query(), DesktopSearchIndexState, DesktopSearchResponse, DesktopSearchResult, FixtureSearchStore, FixtureState (+59 more)

### Community 12 - "Community 12"
Cohesion: 0.03
Nodes (68): backup_dest_root(), backup_source_key_for_rel_path(), backup_source_key_for_this_device(), classify_copy(), classify_skip_when_size_and_mtime_match(), copy_preserving_mtime(), CopyDecision, dest_root_builds_backup_device_folder() (+60 more)

### Community 13 - "Community 13"
Cohesion: 0.04
Nodes (76): auto_resolution_deadline(), ConflictRecord, Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed(), v() (+68 more)

### Community 14 - "Community 14"
Cohesion: 0.05
Nodes (56): DomainControlTool, a_frame_split_across_two_reads_is_reassembled(), a_partial_value_at_eof_is_returned_for_the_caller_to_reject(), an_oversized_frame_is_refused_instead_of_buffered_forever(), an_unframed_complete_value_is_accepted_without_waiting_for_eof(), blank_lines_are_skipped(), ends_like_json_value(), FrameError (+48 more)

### Community 15 - "Community 15"
Cohesion: 0.05
Nodes (38): requestPayload(), AndroidKeyboard(), IOSKeyboard(), assert_clean(), assert_corpus_endpoint_consistency(), assert_same_metadata(), bridge(), corpus_conflict_versions_and_stale_precondition() (+30 more)

### Community 16 - "Community 16"
Cohesion: 0.06
Nodes (59): bind_ipc_listener(), bind_ipc_listener_with(), capabilities_for_status(), dedup_write(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string() (+51 more)

### Community 17 - "Community 17"
Cohesion: 0.04
Nodes (40): ancestors(), classifyJsxUsage(), comparesToErrorPhase(), isCorrectableInput(), isErrorOrigin(), isInsideUseEffect(), isLoadPath(), isRetryCallback() (+32 more)

### Community 18 - "Community 18"
Cohesion: 0.07
Nodes (55): a_forged_name_marker_in_input_is_stripped(), assert_credential_shape(), boundary_ok(), classify_error_code(), credential_authorization_bearer_uuid_keeps_following_words(), credential_authorization_colon_bearer_uuid(), credential_authorization_equals_bearer(), credential_bearer_colon() (+47 more)

### Community 19 - "Community 19"
Cohesion: 0.05
Nodes (24): FileProviderEnumerator, FileProviderExtension, FileProviderItem, IPCCancellation, IPCContentFingerprint, IPCExchange, IPCFrameReader, IPCFraming (+16 more)

### Community 20 - "Community 20"
Cohesion: 0.15
Nodes (39): a_cached_create_is_not_returned_for_a_row_parked_trashing(), a_cached_create_is_not_returned_once_its_row_is_gone(), a_cached_create_is_not_returned_while_a_trash_op_is_pending_for_its_row(), a_cached_reply_reports_the_row_as_it_is_now_not_as_it_was(), a_create_after_the_trash_op_finished_still_uploads_the_new_file(), a_failed_create_is_not_remembered_so_the_retry_actually_runs(), a_malformed_line_gets_an_error_and_the_connection_keeps_working(), a_real_finder_delete_then_the_same_create_uploads_the_new_file() (+31 more)

### Community 21 - "Community 21"
Cohesion: 0.11
Nodes (35): a_centre_exactly_on_the_shared_edge_belongs_to_the_right_display(), a_centre_off_every_display_uses_the_largest_overlap(), a_display_left_of_the_primary_has_a_negative_origin(), a_screen_narrower_than_the_popover_keeps_the_top_left_on_screen(), ambiguous_two_display_case_picks_the_external_display(), an_icon_on_no_display_falls_back_to_the_primary_not_the_centre(), an_icon_straddling_two_displays_goes_with_its_centre(), centres_under_the_icon_with_the_gap() (+27 more)

### Community 22 - "Community 22"
Cohesion: 0.09
Nodes (24): a_bad_scale_factor_is_treated_as_one_not_a_division_by_zero(), a_window_spanning_two_displays_is_not_fully_inside_either(), displays(), entries_outside_every_work_area_are_ignored(), every_other_reason_is_reported_on_its_own(), flags_are_size_and_position_only(), IgnoreReason, outer_rect() (+16 more)

### Community 23 - "Community 23"
Cohesion: 0.18
Nodes (18): Leader<T>, a_cache_hit_is_refreshed_on_every_serve_but_the_stored_copy_is_left_as_built(), a_failed_result_is_shared_with_waiters_but_not_remembered(), a_key_reused_for_a_different_request_shape_is_not_merged(), a_repeat_after_completion_returns_the_stored_result_without_running_again(), a_stored_result_expires_exactly_at_the_ttl(), a_stored_result_that_fails_validation_is_dropped_and_the_work_runs_again(), a_table_full_of_in_flight_work_fails_open_instead_of_refusing_the_write() (+10 more)

### Community 24 - "Community 24"
Cohesion: 0.13
Nodes (23): BeebeebFS, dir_attr(), file_attr(), reply_read_slice(), archive_legacy_state_dir_if_present(), beebeeb_state_dir_from_app_local_data(), copy_dir_all(), copy_dir_contents_no_clobber() (+15 more)

### Community 25 - "Community 25"
Cohesion: 0.14
Nodes (27): check(), main(), self_test(), call_spans(), check(), function_body(), matching_close(), Yield the argument text of every `sendRequest(...)` call (not the func decl). (+19 more)

### Community 26 - "Community 26"
Cohesion: 0.11
Nodes (24): all_visibility(), every_reachable_registry_state_shows_a_finder_failure_exactly_once(), finder_failure_surfaces(), the_popover_and_the_ledger_never_show_a_finder_failure_twice(), the_popover_is_f2_when_it_is_the_one_surface_showing_the_failure(), finder_failure_shown_elsewhere(), a_finder_failure_shown_by_another_surface_is_not_repeated_as_f2(), a_stopped_or_unstarted_engine_is_not_locked() (+16 more)

### Community 27 - "Community 27"
Cohesion: 0.17
Nodes (22): assert_uploading_row(), debounce_loop(), dispatch_local_create(), engine_delete_suppress(), engine_delete_suppression_distinguishes_paths(), engine_delete_suppression_is_consumed_once(), engine_delete_suppression_prunes_stale_entries(), NotifyEvent (+14 more)

### Community 28 - "Community 28"
Cohesion: 0.19
Nodes (12): CallbackGate, CallbackGate<T>, CallbackLease, CallbackLease<T>, drain_waits_for_transfer_and_releases_credentials(), Generation, LeaseState, revoke_denies_new_work_and_cancels_existing_work() (+4 more)

### Community 29 - "Community 29"
Cohesion: 0.17
Nodes (12): glob_match(), granted_window_patterns(), macos_merge_is_todays_window_list_until_slice_6(), manifest_dir(), merged_config(), only_the_macos_platform_file_exists(), patterns_granted_by(), read_json() (+4 more)

### Community 30 - "Community 30"
Cohesion: 0.34
Nodes (13): commandForTauriCli(), decodeMinisignBase64Line(), decodeTauriBase64Text(), fail(), generateKeypair(), main(), parseTauriPubkey(), parseTauriSignature() (+5 more)

### Community 31 - "Community 31"
Cohesion: 0.36
Nodes (6): Attempt, AuthAttempts, ReopenOnDrop, round5_browser_handoff_after_signout_persists_nothing_and_starts_no_runner(), round5_lock_cancels_handoff_and_drains_credential_owner(), round7_recovery_verification_cancels_and_late_install_is_rejected()

### Community 35 - "Community 35"
Cohesion: 0.33
Nodes (8): a_click_on_a_visible_popover_hides_it_and_clears_a_stale_blur(), a_click_right_after_a_blur_hide_does_not_reopen(), a_clock_that_steps_backwards_counts_as_the_same_gesture(), BlurClickGuard, one_blur_suppresses_one_click(), the_window_is_250_ms_exclusive(), TrayClickAction, with_no_blur_a_click_opens()

### Community 49 - "Community 49"
Cohesion: 0.32
Nodes (4): manualUpdateToast(), buildDowngradeConfirmationViewModel(), buildUpdateCheckViewModel(), releaseChannelLabel()

### Community 50 - "Community 50"
Cohesion: 0.39
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 59 - "Community 59"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 62 - "Community 62"
Cohesion: 0.4
Nodes (2): declOf(), rulesFor()

### Community 63 - "Community 63"
Cohesion: 0.4
Nodes (2): findColorViolations(), toKebabCase()

### Community 64 - "Community 64"
Cohesion: 0.33
Nodes (5): Claim, Entry, Leader, State, WriteDedup

### Community 65 - "Community 65"
Cohesion: 0.33
Nodes (1): ApiClient

### Community 66 - "Community 66"
Cohesion: 0.47
Nodes (4): availableSettingsSections(), settingLabel(), shellIntegrationLabel(), withPlatformLabels()

### Community 70 - "Community 70"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

### Community 72 - "Community 72"
Cohesion: 0.7
Nodes (3): consumeStoredNav(), initialNav(), navIdFromString()

### Community 82 - "Community 82"
Cohesion: 0.67
Nodes (2): file(), folder()

### Community 89 - "Community 89"
Cohesion: 1.0
Nodes (2): extractBalancedBlock(), extractFunctionBody()

### Community 92 - "Community 92"
Cohesion: 1.0
Nodes (2): productRegionLabel(), useRegionLabel()

### Community 105 - "Community 105"
Cohesion: 1.0
Nodes (1): StatusUiSourceFactory_Impl

## Knowledge Gaps
- **259 isolated node(s):** `daemonUnavailable`, `invalidIdentifier`, `timedOut`, `cancelled`, `encodingFailed` (+254 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 62`** (6 nodes): `declOf()`, `parseCss()`, `read()`, `rulesFor()`, `macSettingsLayout.test.ts`, `walk()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 63`** (6 nodes): `findColorViolations()`, `isCommentLine()`, `listSourceFiles()`, `oklchLiteralLines()`, `toKebabCase()`, `noHardcodedThemeColors.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 65`** (6 nodes): `ApiClient`, `.delete_shard()`, `.fetch_manifest()`, `.get_shard()`, `.master_key_bytes()`, `.put_shard()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 70`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 82`** (4 nodes): `file()`, `folder()`, `state()`, `macSettingsModel.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 89`** (3 nodes): `extractBalancedBlock()`, `extractFunctionBody()`, `fileProviderHandoffCopiesToSystemTempDir.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 92`** (3 nodes): `useRegion.ts`, `productRegionLabel()`, `useRegionLabel()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 105`** (2 nodes): `StatusUiSourceFactory_Impl`, `.GetStatusUISource()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `err` connect `Community 6` to `Community 0`, `Community 1`, `Community 3`, `Community 4`, `Community 5`, `Community 7`, `Community 8`, `Community 9`, `Community 10`, `Community 11`, `Community 12`, `Community 13`, `Community 14`, `Community 15`, `Community 16`, `Community 20`, `Community 24`, `Community 28`, `Community 31`?**
  _High betweenness centrality (0.139) - this node is a cross-community bridge._
- **Why does `load()` connect `Community 0` to `Community 1`, `Community 2`, `Community 3`, `Community 16`, `Community 27`?**
  _High betweenness centrality (0.081) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 2` to `Community 0`?**
  _High betweenness centrality (0.080) - this node is a cross-community bridge._
- **Are the 143 inferred relationships involving `err` (e.g. with `.validate()` and `.run()`) actually correct?**
  _`err` has 143 INFERRED edges - model-reasoned connections that need verification._
- **Are the 72 inferred relationships involving `load()` (e.g. with `corpus_size_mismatch_reports_expected_and_actual_bytes_and_hashes()` and `corpus_oracle_rejects_wrong_hash_and_missing_scenario()`) actually correct?**
  _`load()` has 72 INFERRED edges - model-reasoned connections that need verification._
- **Are the 71 inferred relationships involving `load()` (e.g. with `corpus_size_mismatch_reports_expected_and_actual_bytes_and_hashes()` and `corpus_oracle_rejects_wrong_hash_and_missing_scenario()`) actually correct?**
  _`load()` has 71 INFERRED edges - model-reasoned connections that need verification._
- **What connects `daemonUnavailable`, `invalidIdentifier`, `timedOut` to the rest of the system?**
  _259 weakly-connected nodes found - possible documentation gaps or missing edges._