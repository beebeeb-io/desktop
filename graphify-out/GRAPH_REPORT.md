# Graph Report - desktop-graph-1873  (2026-10-11)

## Corpus Check
- 282 files · ~1,120,837 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 6477 nodes · 19325 edges · 52 communities detected
- Extraction: 80% EXTRACTED · 20% INFERRED · 0% AMBIGUOUS · INFERRED: 3938 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 50|Community 50]]
- [[_COMMUNITY_Community 51|Community 51]]
- [[_COMMUNITY_Community 59|Community 59]]
- [[_COMMUNITY_Community 60|Community 60]]
- [[_COMMUNITY_Community 63|Community 63]]
- [[_COMMUNITY_Community 64|Community 64]]
- [[_COMMUNITY_Community 65|Community 65]]
- [[_COMMUNITY_Community 66|Community 66]]
- [[_COMMUNITY_Community 67|Community 67]]
- [[_COMMUNITY_Community 71|Community 71]]
- [[_COMMUNITY_Community 73|Community 73]]
- [[_COMMUNITY_Community 83|Community 83]]
- [[_COMMUNITY_Community 84|Community 84]]
- [[_COMMUNITY_Community 91|Community 91]]
- [[_COMMUNITY_Community 96|Community 96]]
- [[_COMMUNITY_Community 149|Community 149]]
- [[_COMMUNITY_Community 150|Community 150]]
- [[_COMMUNITY_Community 151|Community 151]]
- [[_COMMUNITY_Community 152|Community 152]]

## God Nodes (most connected - your core abstractions)
1. `err` - 264 edges
2. `StateDb` - 143 edges
3. `test_bridge_with_api()` - 141 edges
4. `spawn()` - 119 edges
5. `seed_uploaded_row()` - 103 edges
6. `fp_save()` - 102 edges
7. `now_secs()` - 96 edges
8. `load()` - 93 edges
9. `EngineBridge` - 86 edges
10. `drain_upload_queue()` - 80 edges

## Surprising Connections (you probably didn't know these)
- `classesOf()` --calls--> `String`  [INFERRED]
  tests/reauthInPlace.test.tsx → src-tauri/src/lib.rs
- `openView()` --calls--> `mount()`  [INFERRED]
  tests/onboardingAfterSignOut.test.tsx → src-tauri/src/linux_fuse/mod.rs
- `emit()` --calls--> `openActivity()`  [INFERRED]
  src-tauri/src/browser_login.rs → src/WindowsTray.tsx
- `write_hydrated_from_reader()` --calls--> `create()`  [INFERRED]
  src-tauri/src/engine_bridge.rs → eslint-rules/no-ad-hoc-error-surface.mjs
- `purge()` --calls--> `finish()`  [INFERRED]
  src-tauri/src/windows_cf/signout_cleanup.rs → src/Onboarding.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.01
Nodes (734): a_disk_image_or_translocated_launch_is_missing_for_not_in_applications_before_any_check(), AccountId, provenance_headers(), wait_for_upload_bytes(), menu_open_failure_alert(), legacy_platform_keychain_store(), platform_keychain_store_for(), test_fail_account_email_writes() (+726 more)

### Community 1 - "Community 1"
Cohesion: 0.01
Nodes (570): ParentChange<'a>, is_conflict(), is_text_file(), a_404_on_every_new_session_parks_the_write(), a_base_unknown_predecessor_hands_over_and_the_successor_parks_without_a_request(), a_builder_clears_held_columns_only_for_the_write_it_read(), a_claim_time_park_puts_its_item_in_the_ticks_signal(), a_completed_session_is_never_abandoned_at_give_up() (+562 more)

### Community 2 - "Community 2"
Cohesion: 0.01
Nodes (255): Auth, allowed_label(), calls_in(), calls_of(), code_lines(), extension_dir(), extension_sources(), interpolations() (+247 more)

### Community 3 - "Community 3"
Cohesion: 0.01
Nodes (308): change(), a_kept_folder_saved_during_a_reconciler_config_update_survives_it(), a_reconciler_config_update_during_a_kept_folder_save_keeps_the_folder(), in_flight(), update_config(), update_config_at(), load(), account_activity_parses_events_and_summary() (+300 more)

### Community 4 - "Community 4"
Cohesion: 0.01
Nodes (149): ErrorFields, ExtensionLog, WriteCall, create, modify, WriteOptionsLine, DeleteDisposition, directoryNotEmpty (+141 more)

### Community 5 - "Community 5"
Cohesion: 0.01
Nodes (205): rustSource(), rustStr(), diagnostics(), lock(), refresh(), runAction(), signOut(), titleCasePlan() (+197 more)

### Community 6 - "Community 6"
Cohesion: 0.02
Nodes (212): err, DomainControlTool, KeptFolder, empty, hasEntries, missing, noPath, notReported (+204 more)

### Community 7 - "Community 7"
Cohesion: 0.02
Nodes (201): ipc_hydrate_roundtrip(), ipc_socket_file_is_chmod_0600_after_bind(), item_version_identifier(), a_frame_split_across_two_reads_is_reassembled(), a_partial_value_at_eof_is_returned_for_the_caller_to_reject(), an_oversized_frame_is_refused_instead_of_buffered_forever(), an_unframed_complete_value_is_accepted_without_waiting_for_eof(), blank_lines_are_skipped() (+193 more)

### Community 8 - "Community 8"
Cohesion: 0.03
Nodes (128): a_call_that_never_returns_is_cut_off_at_its_limit_with_a_redacted_timeout_error(), a_confirmed_removal_reaches_the_port_and_a_failed_one_does_not(), a_dmg_launch_publishes_not_in_applications_at_start_with_no_trigger(), a_domain_turned_off_while_ready_is_noticed_by_the_poll_without_a_relaunch(), a_handle_says_so_when_its_reconciler_is_gone_or_silent_and_refuses_what_is_not_a_removal(), a_launch_runs_one_check_and_publishes_ready(), a_panicking_port_ends_in_one_failed_unknown_view_and_a_handle_that_says_so(), a_persisted_failure_is_in_the_first_view_and_ready_clears_it() (+120 more)

### Community 9 - "Community 9"
Cohesion: 0.02
Nodes (136): api_client_search_shard_urls_match_web_paths(), build_local_index(), compare_records_for_query(), DesktopSearchIndexState, DesktopSearchResponse, DesktopSearchResult, FixtureSearchStore, FixtureState (+128 more)

### Community 10 - "Community 10"
Cohesion: 0.02
Nodes (130): Account, AppHandle, AppState, Attempt, AuthVault, Client, Commands, ensure_reactivation_allowed() (+122 more)

### Community 11 - "Community 11"
Cohesion: 0.02
Nodes (103): elementsOf(), finderBus(), finderView(), tick(), useFinderSetupModule(), BeebeebFS, dir_attr(), file_attr() (+95 more)

### Community 12 - "Community 12"
Cohesion: 0.03
Nodes (60): a_full_clear_after_a_token_only_clear_leaves_no_vault_key(), a_keyless_legacy_token_never_lands_next_to_a_kept_vault_key(), a_session_token_is_wiped_when_it_goes(), account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault (+52 more)

### Community 13 - "Community 13"
Cohesion: 0.03
Nodes (62): a_failure_then_an_answer_leaves_the_link_up(), a_metadata_update_body_names_each_parent_state(), a_typed_get_answered_with_401_is_an_answer_not_a_link_failure(), a_typed_get_answered_with_503_records_the_server_not_answering(), a_typed_get_that_is_answered_records_an_answer(), a_typed_get_with_nothing_listening_records_offline(), an_upload_refusal_keeps_the_servers_code_and_never_its_body(), api_client_list_files_sends_provenance_headers_on_the_wire() (+54 more)

### Community 14 - "Community 14"
Cohesion: 0.03
Nodes (91): requestPayload(), AndroidKeyboard(), IOSKeyboard(), check(), main(), self_test(), call_spans(), check() (+83 more)

### Community 15 - "Community 15"
Cohesion: 0.03
Nodes (90): is_gate_busy(), auto_resolution_deadline(), ConflictRecord, Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed() (+82 more)

### Community 16 - "Community 16"
Cohesion: 0.02
Nodes (89): ancestors(), classifyJsxUsage(), comparesToErrorPhase(), create(), isErrorOrigin(), isInsideUseEffect(), isLoadPath(), isRetryCallback() (+81 more)

### Community 17 - "Community 17"
Cohesion: 0.03
Nodes (101): a_home_on_an_external_disk_is_a_real_install(), at(), current(), current_bundle_path(), is_at_volume_root(), is_under_volume_applications(), launch_location(), LaunchLocation (+93 more)

### Community 18 - "Community 18"
Cohesion: 0.03
Nodes (70): backup_dest_root(), backup_source_key_for_rel_path(), backup_source_key_for_this_device(), classify_copy(), classify_skip_when_size_and_mtime_match(), copy_preserving_mtime(), CopyDecision, dest_root_builds_backup_device_folder() (+62 more)

### Community 19 - "Community 19"
Cohesion: 0.04
Nodes (82): a_401_error_is_not_a_connectivity_phase(), a_401_from_the_usage_call_is_an_error_that_does_not_carry_the_token(), a_failed_fetch_with_nothing_cached_is_no_summary_and_another_sessions_is_not_reused(), a_failed_refetch_serves_the_old_number_marked_stale(), a_failure_shown_elsewhere_does_not_paint_f2_here(), a_file_on_the_board_is_syncing_even_when_the_last_event_said_idle(), a_file_uploaded_twice_is_one_finished_row_the_newest(), a_finished_transfer_of_a_file_moving_again_is_not_listed_twice() (+74 more)

### Community 20 - "Community 20"
Cohesion: 0.1
Nodes (69): a_cached_create_is_not_returned_for_a_row_parked_trashing(), a_cached_create_is_not_returned_once_its_row_is_gone(), a_cached_create_is_not_returned_while_a_trash_op_is_pending_for_its_row(), a_cached_reply_reports_the_row_as_it_is_now_not_as_it_was(), a_create_after_the_trash_op_finished_still_uploads_the_new_file(), a_create_still_in_flight_when_the_socket_stops_keeps_its_copy_until_the_engine_has_read_it(), a_deletion_conflicted_create_over_the_socket_modifies_the_template_item(), a_failed_create_is_not_remembered_so_the_retry_actually_runs() (+61 more)

### Community 21 - "Community 21"
Cohesion: 0.07
Nodes (53): assert_uploading_row(), debounce_loop(), dispatch_local_create(), engine_delete_suppress(), engine_delete_suppression_distinguishes_paths(), engine_delete_suppression_is_consumed_once(), engine_delete_suppression_prunes_stale_entries(), NotifyEvent (+45 more)

### Community 22 - "Community 22"
Cohesion: 0.07
Nodes (54): a_forged_name_marker_in_input_is_stripped(), assert_credential_shape(), boundary_ok(), classify_error_code(), credential_authorization_bearer_uuid_keeps_following_words(), credential_authorization_colon_bearer_uuid(), credential_authorization_equals_bearer(), credential_bearer_colon() (+46 more)

### Community 23 - "Community 23"
Cohesion: 0.05
Nodes (27): allElements(), click(), installMiniDom(), MiniComment, MiniDocument, MiniNode, Modal(), activeRail() (+19 more)

### Community 24 - "Community 24"
Cohesion: 0.11
Nodes (35): a_centre_exactly_on_the_shared_edge_belongs_to_the_right_display(), a_centre_off_every_display_uses_the_largest_overlap(), a_display_left_of_the_primary_has_a_negative_origin(), a_screen_narrower_than_the_popover_keeps_the_top_left_on_screen(), ambiguous_two_display_case_picks_the_external_display(), an_icon_on_no_display_falls_back_to_the_primary_not_the_centre(), an_icon_straddling_two_displays_goes_with_its_centre(), centres_under_the_icon_with_the_gap() (+27 more)

### Community 25 - "Community 25"
Cohesion: 0.11
Nodes (29): a_payload_without_a_message_still_deserializes_and_round_trips_without_one(), display_keeps_the_message_for_stdout_tracing(), FailureRecord, FpError, FpErrorCode, redacted_is_domain_and_code_and_never_the_message(), redacted_names_the_underlying_pair_and_still_not_the_message(), serializing_an_fp_error_never_carries_the_message() (+21 more)

### Community 26 - "Community 26"
Cohesion: 0.09
Nodes (24): a_bad_scale_factor_is_treated_as_one_not_a_division_by_zero(), a_window_spanning_two_displays_is_not_fully_inside_either(), displays(), entries_outside_every_work_area_are_ignored(), every_other_reason_is_reported_on_its_own(), flags_are_size_and_position_only(), IgnoreReason, outer_rect() (+16 more)

### Community 27 - "Community 27"
Cohesion: 0.17
Nodes (19): Clock, Leader<T>, a_cache_hit_is_refreshed_on_every_serve_but_the_stored_copy_is_left_as_built(), a_failed_result_is_shared_with_waiters_but_not_remembered(), a_key_reused_for_a_different_request_shape_is_not_merged(), a_repeat_after_completion_returns_the_stored_result_without_running_again(), a_stored_result_expires_exactly_at_the_ttl(), a_stored_result_that_fails_validation_is_dropped_and_the_work_runs_again() (+11 more)

### Community 28 - "Community 28"
Cohesion: 0.13
Nodes (21): classify_sign_in_after(), a_completed_key_proof_decides_and_an_unavailable_one_settles_nothing(), a_retained_vault_key_alone_is_never_fresh_and_goes_to_the_server(), after_a_revoked_token_with_nothing_queued(), after_key_proof(), another_account_after_a_revoked_token_with_nothing_queued_is_a_switch_never_fresh(), credentials_with_no_owner_record_are_judged_by_the_same_order_as_an_owner(), every_retained_trace_counts() (+13 more)

### Community 29 - "Community 29"
Cohesion: 0.19
Nodes (12): CallbackGate, CallbackGate<T>, CallbackLease, CallbackLease<T>, drain_waits_for_transfer_and_releases_credentials(), Generation, LeaseState, revoke_denies_new_work_and_cancels_existing_work() (+4 more)

### Community 30 - "Community 30"
Cohesion: 0.17
Nodes (12): glob_match(), granted_window_patterns(), macos_merge_still_creates_the_startup_windows_and_merges_the_platform_file(), manifest_dir(), merged_config(), only_the_macos_platform_file_exists(), patterns_granted_by(), read_json() (+4 more)

### Community 31 - "Community 31"
Cohesion: 0.34
Nodes (13): commandForTauriCli(), decodeMinisignBase64Line(), decodeTauriBase64Text(), fail(), generateKeypair(), main(), parseTauriPubkey(), parseTauriSignature() (+5 more)

### Community 35 - "Community 35"
Cohesion: 0.33
Nodes (8): a_click_on_a_visible_popover_hides_it_and_clears_a_stale_blur(), a_click_right_after_a_blur_hide_does_not_reopen(), a_clock_that_steps_backwards_counts_as_the_same_gesture(), BlurClickGuard, one_blur_suppresses_one_click(), the_window_is_250_ms_exclusive(), TrayClickAction, with_no_blur_a_click_opens()

### Community 50 - "Community 50"
Cohesion: 0.39
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 51 - "Community 51"
Cohesion: 0.32
Nodes (4): manualUpdateToast(), buildDowngradeConfirmationViewModel(), buildUpdateCheckViewModel(), releaseChannelLabel()

### Community 59 - "Community 59"
Cohesion: 0.43
Nodes (1): UploadStaging

### Community 60 - "Community 60"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 63 - "Community 63"
Cohesion: 0.4
Nodes (2): declOf(), rulesFor()

### Community 64 - "Community 64"
Cohesion: 0.4
Nodes (2): findColorViolations(), toKebabCase()

### Community 65 - "Community 65"
Cohesion: 0.33
Nodes (5): Claim, Entry, Leader, State, WriteDedup

### Community 66 - "Community 66"
Cohesion: 0.33
Nodes (1): ApiClient

### Community 67 - "Community 67"
Cohesion: 0.47
Nodes (4): availableSettingsSections(), settingLabel(), shellIntegrationLabel(), withPlatformLabels()

### Community 71 - "Community 71"
Cohesion: 0.7
Nodes (4): Find-SevenZip(), Invoke-InstallerExtraction(), Invoke-SelfTest(), Test-Signature()

### Community 73 - "Community 73"
Cohesion: 0.7
Nodes (3): consumeStoredNav(), initialNav(), navIdFromString()

### Community 83 - "Community 83"
Cohesion: 0.67
Nodes (2): file(), folder()

### Community 84 - "Community 84"
Cohesion: 0.67
Nodes (2): Install-ArtifactSigningDlib(), Stop-Signing()

### Community 91 - "Community 91"
Cohesion: 1.0
Nodes (2): extractBalancedBlock(), extractFunctionBody()

### Community 96 - "Community 96"
Cohesion: 1.0
Nodes (2): productRegionLabel(), useRegionLabel()

### Community 149 - "Community 149"
Cohesion: 1.0
Nodes (1): Index just past the delimiter closing the one at `open_idx` (strings skipped).

### Community 150 - "Community 150"
Cohesion: 1.0
Nodes (1): Text of `func <name>(...) ... { body }`, or None.

### Community 151 - "Community 151"
Cohesion: 1.0
Nodes (1): Yield the argument text of every `sendRequest(...)` call (not the func decl).

### Community 152 - "Community 152"
Cohesion: 1.0
Nodes (1): Return (ok_count, problems).

## Knowledge Gaps
- **337 isolated node(s):** `metadataUpdate`, `serverTrash`, `queueServerTrash`, `directoryNotEmpty`, `QueuedWriteCompletion` (+332 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 59`** (7 nodes): `UploadStaging.swift`, `UploadStaging`, `.directory()`, `.discard()`, `.newFileURL()`, `.reason()`, `.stage()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 63`** (6 nodes): `declOf()`, `parseCss()`, `read()`, `rulesFor()`, `macSettingsLayout.test.ts`, `walk()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 64`** (6 nodes): `findColorViolations()`, `isCommentLine()`, `listSourceFiles()`, `oklchLiteralLines()`, `toKebabCase()`, `noHardcodedThemeColors.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 66`** (6 nodes): `ApiClient`, `.delete_shard()`, `.fetch_manifest()`, `.get_shard()`, `.master_key_bytes()`, `.put_shard()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 83`** (4 nodes): `file()`, `folder()`, `macSettingsModel.test.ts`, `v()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 84`** (4 nodes): `sign-artifact.ps1`, `Find-SignTool()`, `Install-ArtifactSigningDlib()`, `Stop-Signing()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 91`** (3 nodes): `extractBalancedBlock()`, `extractFunctionBody()`, `fileProviderHandoffCopiesToSystemTempDir.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 96`** (3 nodes): `useRegion.ts`, `productRegionLabel()`, `useRegionLabel()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 149`** (1 nodes): `Index just past the delimiter closing the one at `open_idx` (strings skipped).`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 150`** (1 nodes): `Text of `func <name>(...) ... { body }`, or None.`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 151`** (1 nodes): `Yield the argument text of every `sendRequest(...)` call (not the func decl).`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 152`** (1 nodes): `Return (ok_count, problems).`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `run()` connect `Community 3` to `Community 0`, `Community 1`, `Community 2`, `Community 4`, `Community 6`, `Community 8`, `Community 9`, `Community 12`, `Community 18`, `Community 25`?**
  _High betweenness centrality (0.054) - this node is a cross-community bridge._
- **Why does `err` connect `Community 6` to `Community 0`, `Community 1`, `Community 2`, `Community 3`, `Community 4`, `Community 7`, `Community 8`, `Community 9`, `Community 10`, `Community 12`, `Community 13`, `Community 14`, `Community 15`, `Community 17`, `Community 18`, `Community 19`, `Community 21`, `Community 29`?**
  _High betweenness centrality (0.051) - this node is a cross-community bridge._
- **Why does `load()` connect `Community 3` to `Community 0`, `Community 1`, `Community 5`, `Community 6`, `Community 7`, `Community 9`, `Community 17`, `Community 21`?**
  _High betweenness centrality (0.029) - this node is a cross-community bridge._
- **Are the 263 inferred relationships involving `err` (e.g. with `.validate()` and `.run()`) actually correct?**
  _`err` has 263 INFERRED edges - model-reasoned connections that need verification._
- **Are the 3 inferred relationships involving `test_bridge_with_api()` (e.g. with `.open()` and `.open()`) actually correct?**
  _`test_bridge_with_api()` has 3 INFERRED edges - model-reasoned connections that need verification._
- **Are the 94 inferred relationships involving `spawn()` (e.g. with `.start()` and `serve()`) actually correct?**
  _`spawn()` has 94 INFERRED edges - model-reasoned connections that need verification._
- **What connects `metadataUpdate`, `serverTrash`, `queueServerTrash` to the rest of the system?**
  _337 weakly-connected nodes found - possible documentation gaps or missing edges._