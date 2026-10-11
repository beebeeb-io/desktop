# Graph Report - desktop-graphify2  (2026-10-11)

## Corpus Check
- 279 files · ~938,221 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 5737 nodes · 15943 edges · 51 communities detected
- Extraction: 78% EXTRACTED · 22% INFERRED · 0% AMBIGUOUS · INFERRED: 3489 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 32|Community 32]]
- [[_COMMUNITY_Community 33|Community 33]]
- [[_COMMUNITY_Community 34|Community 34]]
- [[_COMMUNITY_Community 38|Community 38]]
- [[_COMMUNITY_Community 53|Community 53]]
- [[_COMMUNITY_Community 54|Community 54]]
- [[_COMMUNITY_Community 62|Community 62]]
- [[_COMMUNITY_Community 63|Community 63]]
- [[_COMMUNITY_Community 66|Community 66]]
- [[_COMMUNITY_Community 67|Community 67]]
- [[_COMMUNITY_Community 68|Community 68]]
- [[_COMMUNITY_Community 69|Community 69]]
- [[_COMMUNITY_Community 70|Community 70]]
- [[_COMMUNITY_Community 74|Community 74]]
- [[_COMMUNITY_Community 76|Community 76]]
- [[_COMMUNITY_Community 86|Community 86]]
- [[_COMMUNITY_Community 87|Community 87]]
- [[_COMMUNITY_Community 94|Community 94]]
- [[_COMMUNITY_Community 99|Community 99]]

## God Nodes (most connected - your core abstractions)
1. `err` - 233 edges
2. `spawn()` - 112 edges
3. `StateDb` - 97 edges
4. `load()` - 93 edges
5. `ApiClient` - 78 edges
6. `load()` - 72 edges
7. `clear_session_impl()` - 67 edges
8. `sleep()` - 66 edges
9. `EngineBridge` - 66 edges
10. `production_source()` - 58 edges

## Surprising Connections (you probably didn't know these)
- `classesOf()` --calls--> `String`  [INFERRED]
  tests/reauthInPlace.test.tsx → src-tauri/src/lib.rs
- `openView()` --calls--> `mount()`  [INFERRED]
  tests/onboardingAfterSignOut.test.tsx → src-tauri/src/linux_fuse/mod.rs
- `err` --calls--> `confirm_action_plaintext()`  [INFERRED]
  BeebeebFileProviderTests/main.swift → src-tauri/src/lib.rs
- `emit()` --calls--> `openActivity()`  [INFERRED]
  src-tauri/src/browser_login.rs → src/WindowsTray.tsx
- `purge()` --calls--> `finish()`  [INFERRED]
  src-tauri/src/windows_cf/signout_cleanup.rs → src/Onboarding.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.01
Nodes (662): spawn(), a_synchronous_call_waits_for_a_gate_held_less_than_the_bound_and_fails_busy_after_it(), AccountId, synthesize_single_account(), test_1882_r5_two_updates_at_once_both_land_and_an_unsaved_change_writes_nothing(), wait_for_upload_bytes(), menu_open_failure_alert(), with_an_update_in_flight() (+654 more)

### Community 1 - "Community 1"
Cohesion: 0.01
Nodes (315): is_text_file(), a_failed_upload_leaves_nothing_on_the_board_and_no_activity_row(), a_finished_upload_adds_its_bytes_to_the_batch_and_records_an_up_row(), a_hydrate_reports_per_file_bytes_and_records_a_down_row(), a_hydrate_without_a_caller_callback_still_fills_the_board(), a_resumed_upload_starts_its_bar_at_the_acknowledged_watermark(), an_upload_reports_its_bytes_chunk_by_chunk_and_a_cut_one_leaves_nothing(), apply_metadata_file_row() (+307 more)

### Community 2 - "Community 2"
Cohesion: 0.02
Nodes (180): allowed_label(), test_1885_a_shared_with_me_item_is_never_asked_of_the_file_provider(), dehydrate_excluded_subtrees_windows(), entry_basename(), free_up_space_allowed_roots(), free_up_space_windows(), open_conflict_window(), open_conflict_window_impl() (+172 more)

### Community 3 - "Community 3"
Cohesion: 0.01
Nodes (205): rustSource(), rustStr(), diagnostics(), lock(), refresh(), runAction(), signOut(), titleCasePlan() (+197 more)

### Community 4 - "Community 4"
Cohesion: 0.01
Nodes (193): Wait, saved_folder(), account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity, AccountActivityEvent (+185 more)

### Community 5 - "Community 5"
Cohesion: 0.01
Nodes (118): DeleteDisposition, directoryNotEmpty, queueServerTrash, FileProviderEnumerator, FileProviderExtension, MaterializedSetCollector, ModifyRoute, metadataUpdate (+110 more)

### Community 6 - "Community 6"
Cohesion: 0.02
Nodes (198): err, DomainControlTool, KeptFolder, empty, hasEntries, missing, noPath, notReported (+190 more)

### Community 7 - "Community 7"
Cohesion: 0.02
Nodes (155): Account, AppHandle, AppState, Attempt, Auth, AuthVault, Client, Commands (+147 more)

### Community 8 - "Community 8"
Cohesion: 0.03
Nodes (122): a_call_that_never_returns_is_cut_off_at_its_limit_with_a_redacted_timeout_error(), a_confirmed_removal_reaches_the_port_and_a_failed_one_does_not(), a_disk_image_or_translocated_launch_is_missing_for_not_in_applications_before_any_check(), a_dmg_launch_publishes_not_in_applications_at_start_with_no_trigger(), a_domain_turned_off_while_ready_is_noticed_by_the_poll_without_a_relaunch(), a_handle_says_so_when_its_reconciler_is_gone_or_silent_and_refuses_what_is_not_a_removal(), a_launch_runs_one_check_and_publishes_ready(), a_panicking_port_ends_in_one_failed_unknown_view_and_a_handle_that_says_so() (+114 more)

### Community 9 - "Community 9"
Cohesion: 0.02
Nodes (100): BeebeebItemKind, file, folder, BeebeebProviderItem, IPCCancellation, IPCExchange, IPCFrameReader, IPCFraming (+92 more)

### Community 10 - "Community 10"
Cohesion: 0.02
Nodes (103): elementsOf(), finderBus(), finderView(), tick(), useFinderSetupModule(), BeebeebFS, dir_attr(), file_attr() (+95 more)

### Community 11 - "Community 11"
Cohesion: 0.03
Nodes (57): a_full_clear_after_a_token_only_clear_leaves_no_vault_key(), a_keyless_legacy_token_never_lands_next_to_a_kept_vault_key(), a_session_token_is_wiped_when_it_goes(), account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault (+49 more)

### Community 12 - "Community 12"
Cohesion: 0.03
Nodes (120): change(), op_limit(), within(), a_kept_folder_saved_during_a_reconciler_config_update_survives_it(), a_reconciler_config_update_during_a_kept_folder_save_keeps_the_folder(), a_removal_that_answers_in_time_gives_the_reconciler_its_folder_and_nothing_goes_to_the_late_sink(), a_removal_that_finishes_after_the_op_limit_still_saves_the_row_and_raises_the_alert(), a_stuck_bridge_call_holds_the_gate_so_the_next_call_fails_fast_and_spawns_nothing() (+112 more)

### Community 13 - "Community 13"
Cohesion: 0.03
Nodes (123): finder_staging_cache_base(), bind_ipc_listener(), bind_ipc_listener_with(), capabilities_for_status(), dedup_write(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract() (+115 more)

### Community 14 - "Community 14"
Cohesion: 0.03
Nodes (43): a_failure_then_an_answer_leaves_the_link_up(), a_typed_get_answered_with_401_is_an_answer_not_a_link_failure(), a_typed_get_answered_with_503_records_the_server_not_answering(), a_typed_get_that_is_answered_records_an_answer(), a_typed_get_with_nothing_listening_records_offline(), api_client_list_files_sends_provenance_headers_on_the_wire(), api_client_zeroizes_master_key_on_drop(), ApiClient (+35 more)

### Community 15 - "Community 15"
Cohesion: 0.03
Nodes (77): AndroidKeyboard(), IOSKeyboard(), a_failed_reset_blocks_the_engine_and_records_nothing(), adopt_unbound(), adoption_happens_once_and_never_replaces_an_owner(), adoption_needs_a_candidate_and_uses_up_its_window_even_without_one(), bind_before_engine_start(), Binding (+69 more)

### Community 16 - "Community 16"
Cohesion: 0.03
Nodes (109): a_home_on_an_external_disk_is_a_real_install(), at(), current(), current_bundle_path(), is_at_volume_root(), is_under_volume_applications(), launch_location(), LaunchLocation (+101 more)

### Community 17 - "Community 17"
Cohesion: 0.03
Nodes (87): is_gate_busy(), auto_resolution_deadline(), ConflictRecord, is_conflict(), Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed() (+79 more)

### Community 18 - "Community 18"
Cohesion: 0.04
Nodes (82): a_401_error_is_not_a_connectivity_phase(), a_401_from_the_usage_call_is_an_error_that_does_not_carry_the_token(), a_failed_fetch_with_nothing_cached_is_no_summary_and_another_sessions_is_not_reused(), a_failed_refetch_serves_the_old_number_marked_stale(), a_failure_shown_elsewhere_does_not_paint_f2_here(), a_file_on_the_board_is_syncing_even_when_the_last_event_said_idle(), a_file_uploaded_twice_is_one_finished_row_the_newest(), a_finished_transfer_of_a_file_moving_again_is_not_listed_twice() (+74 more)

### Community 19 - "Community 19"
Cohesion: 0.07
Nodes (74): a_disk_image_launch_reports_the_reason_before_any_check_and_never_adds(), a_failed_read_after_add_is_not_evidence_and_still_waits(), a_held_ready_core_does_not_poll(), a_held_user_disabled_core_does_not_poll_and_keys_arriving_resumes_it(), a_ready_core_with_no_window_visible_sets_no_timer_and_a_focus_resumes_the_poll(), a_ready_domain_turned_off_while_running_is_read_once_when_the_app_becomes_active(), a_ready_domain_turned_off_while_running_lands_user_disabled_from_the_poll_alone(), a_registered_domain_is_confirmed_even_from_a_disk_image() (+66 more)

### Community 20 - "Community 20"
Cohesion: 0.04
Nodes (55): backup_dest_root(), backup_source_key_for_rel_path(), backup_source_key_for_this_device(), classify_copy(), classify_skip_when_size_and_mtime_match(), copy_preserving_mtime(), CopyDecision, dest_root_builds_backup_device_folder() (+47 more)

### Community 21 - "Community 21"
Cohesion: 0.07
Nodes (49): a_frame_split_across_two_reads_is_reassembled(), a_partial_value_at_eof_is_returned_for_the_caller_to_reject(), an_oversized_frame_is_refused_instead_of_buffered_forever(), an_unframed_complete_value_is_accepted_without_waiting_for_eof(), blank_lines_are_skipped(), ends_like_json_value(), FrameError, FrameReader (+41 more)

### Community 22 - "Community 22"
Cohesion: 0.12
Nodes (54): a_cached_create_is_not_returned_for_a_row_parked_trashing(), a_cached_create_is_not_returned_once_its_row_is_gone(), a_cached_create_is_not_returned_while_a_trash_op_is_pending_for_its_row(), a_cached_reply_reports_the_row_as_it_is_now_not_as_it_was(), a_create_after_the_trash_op_finished_still_uploads_the_new_file(), a_failed_create_is_not_remembered_so_the_retry_actually_runs(), a_malformed_line_gets_an_error_and_the_connection_keeps_working(), a_real_finder_delete_then_the_same_create_uploads_the_new_file() (+46 more)

### Community 23 - "Community 23"
Cohesion: 0.08
Nodes (54): a_forged_name_marker_in_input_is_stripped(), assert_credential_shape(), boundary_ok(), classify_error_code(), credential_authorization_bearer_uuid_keeps_following_words(), credential_authorization_colon_bearer_uuid(), credential_authorization_equals_bearer(), credential_bearer_colon() (+46 more)

### Community 24 - "Community 24"
Cohesion: 0.05
Nodes (27): allElements(), click(), installMiniDom(), MiniComment, MiniDocument, MiniNode, Modal(), activeRail() (+19 more)

### Community 25 - "Community 25"
Cohesion: 0.11
Nodes (35): a_centre_exactly_on_the_shared_edge_belongs_to_the_right_display(), a_centre_off_every_display_uses_the_largest_overlap(), a_display_left_of_the_primary_has_a_negative_origin(), a_screen_narrower_than_the_popover_keeps_the_top_left_on_screen(), ambiguous_two_display_case_picks_the_external_display(), an_icon_on_no_display_falls_back_to_the_primary_not_the_centre(), an_icon_straddling_two_displays_goes_with_its_centre(), centres_under_the_icon_with_the_gap() (+27 more)

### Community 26 - "Community 26"
Cohesion: 0.1
Nodes (30): api_client_search_shard_urls_match_web_paths(), build_local_index(), compare_records_for_query(), DesktopSearchIndexState, DesktopSearchResponse, DesktopSearchResult, FixtureSearchStore, FixtureState (+22 more)

### Community 27 - "Community 27"
Cohesion: 0.11
Nodes (29): a_payload_without_a_message_still_deserializes_and_round_trips_without_one(), display_keeps_the_message_for_stdout_tracing(), FailureRecord, FpError, FpErrorCode, redacted_is_domain_and_code_and_never_the_message(), redacted_names_the_underlying_pair_and_still_not_the_message(), serializing_an_fp_error_never_carries_the_message() (+21 more)

### Community 28 - "Community 28"
Cohesion: 0.09
Nodes (24): a_bad_scale_factor_is_treated_as_one_not_a_division_by_zero(), a_window_spanning_two_displays_is_not_fully_inside_either(), displays(), entries_outside_every_work_area_are_ignored(), every_other_reason_is_reported_on_its_own(), flags_are_size_and_position_only(), IgnoreReason, outer_rect() (+16 more)

### Community 29 - "Community 29"
Cohesion: 0.17
Nodes (19): Clock, Leader<T>, a_cache_hit_is_refreshed_on_every_serve_but_the_stored_copy_is_left_as_built(), a_failed_result_is_shared_with_waiters_but_not_remembered(), a_key_reused_for_a_different_request_shape_is_not_merged(), a_repeat_after_completion_returns_the_stored_result_without_running_again(), a_stored_result_expires_exactly_at_the_ttl(), a_stored_result_that_fails_validation_is_dropped_and_the_work_runs_again() (+11 more)

### Community 30 - "Community 30"
Cohesion: 0.1
Nodes (17): handle_new_conflict(), close_policy(), close_policy_with(), conflict_auto_opens_window(), item_from(), lib_rs(), macos_destroys_only_onboarding_and_review(), runner_rs() (+9 more)

### Community 31 - "Community 31"
Cohesion: 0.11
Nodes (21): analyse(), bindingNames(), calleeName(), censusOfSource(), classify(), enclosingFunction(), familyLiterals(), guardSides() (+13 more)

### Community 32 - "Community 32"
Cohesion: 0.19
Nodes (12): CallbackGate, CallbackGate<T>, CallbackLease, CallbackLease<T>, drain_waits_for_transfer_and_releases_credentials(), Generation, LeaseState, revoke_denies_new_work_and_cancels_existing_work() (+4 more)

### Community 33 - "Community 33"
Cohesion: 0.34
Nodes (13): commandForTauriCli(), decodeMinisignBase64Line(), decodeTauriBase64Text(), fail(), generateKeypair(), main(), parseTauriPubkey(), parseTauriSignature() (+5 more)

### Community 34 - "Community 34"
Cohesion: 0.36
Nodes (6): Attempt, AuthAttempts, ReopenOnDrop, round5_browser_handoff_after_signout_persists_nothing_and_starts_no_runner(), round5_lock_cancels_handoff_and_drains_credential_owner(), round7_recovery_verification_cancels_and_late_install_is_rejected()

### Community 38 - "Community 38"
Cohesion: 0.33
Nodes (8): a_click_on_a_visible_popover_hides_it_and_clears_a_stale_blur(), a_click_right_after_a_blur_hide_does_not_reopen(), a_clock_that_steps_backwards_counts_as_the_same_gesture(), BlurClickGuard, one_blur_suppresses_one_click(), the_window_is_250_ms_exclusive(), TrayClickAction, with_no_blur_a_click_opens()

### Community 53 - "Community 53"
Cohesion: 0.32
Nodes (4): manualUpdateToast(), buildDowngradeConfirmationViewModel(), buildUpdateCheckViewModel(), releaseChannelLabel()

### Community 54 - "Community 54"
Cohesion: 0.39
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 62 - "Community 62"
Cohesion: 0.38
Nodes (3): compare_prerelease_parts(), ParsedReleaseVersion, PrereleasePart

### Community 63 - "Community 63"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 66 - "Community 66"
Cohesion: 0.4
Nodes (2): declOf(), rulesFor()

### Community 67 - "Community 67"
Cohesion: 0.4
Nodes (2): findColorViolations(), toKebabCase()

### Community 68 - "Community 68"
Cohesion: 0.33
Nodes (5): Claim, Entry, Leader, State, WriteDedup

### Community 69 - "Community 69"
Cohesion: 0.33
Nodes (1): ApiClient

### Community 70 - "Community 70"
Cohesion: 0.47
Nodes (4): availableSettingsSections(), settingLabel(), shellIntegrationLabel(), withPlatformLabels()

### Community 74 - "Community 74"
Cohesion: 0.7
Nodes (4): Find-SevenZip(), Invoke-InstallerExtraction(), Invoke-SelfTest(), Test-Signature()

### Community 76 - "Community 76"
Cohesion: 0.7
Nodes (3): consumeStoredNav(), initialNav(), navIdFromString()

### Community 86 - "Community 86"
Cohesion: 0.67
Nodes (2): file(), folder()

### Community 87 - "Community 87"
Cohesion: 0.67
Nodes (2): Install-ArtifactSigningDlib(), Stop-Signing()

### Community 94 - "Community 94"
Cohesion: 1.0
Nodes (2): extractBalancedBlock(), extractFunctionBody()

### Community 99 - "Community 99"
Cohesion: 1.0
Nodes (2): productRegionLabel(), useRegionLabel()

## Knowledge Gaps
- **305 isolated node(s):** `metadataUpdate`, `serverTrash`, `queueServerTrash`, `directoryNotEmpty`, `daemonUnavailable` (+300 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 66`** (6 nodes): `declOf()`, `parseCss()`, `read()`, `rulesFor()`, `macSettingsLayout.test.ts`, `walk()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 67`** (6 nodes): `findColorViolations()`, `isCommentLine()`, `listSourceFiles()`, `oklchLiteralLines()`, `toKebabCase()`, `noHardcodedThemeColors.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 69`** (6 nodes): `ApiClient`, `.delete_shard()`, `.fetch_manifest()`, `.get_shard()`, `.master_key_bytes()`, `.put_shard()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 86`** (4 nodes): `file()`, `folder()`, `macSettingsModel.test.ts`, `v()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 87`** (4 nodes): `sign-artifact.ps1`, `Find-SignTool()`, `Install-ArtifactSigningDlib()`, `Stop-Signing()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 94`** (3 nodes): `extractBalancedBlock()`, `extractFunctionBody()`, `fileProviderHandoffCopiesToSystemTempDir.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 99`** (3 nodes): `useRegion.ts`, `productRegionLabel()`, `useRegionLabel()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `run()` connect `Community 6` to `Community 0`, `Community 1`, `Community 2`, `Community 5`, `Community 7`, `Community 8`, `Community 9`, `Community 11`, `Community 12`, `Community 13`, `Community 27`, `Community 30`?**
  _High betweenness centrality (0.050) - this node is a cross-community bridge._
- **Why does `err` connect `Community 6` to `Community 0`, `Community 1`, `Community 2`, `Community 4`, `Community 7`, `Community 8`, `Community 9`, `Community 11`, `Community 12`, `Community 13`, `Community 14`, `Community 15`, `Community 16`, `Community 17`, `Community 18`, `Community 19`, `Community 20`, `Community 21`, `Community 32`, `Community 34`?**
  _High betweenness centrality (0.040) - this node is a cross-community bridge._
- **Why does `load()` connect `Community 6` to `Community 0`, `Community 1`, `Community 3`, `Community 12`, `Community 13`, `Community 26`?**
  _High betweenness centrality (0.033) - this node is a cross-community bridge._
- **Are the 232 inferred relationships involving `err` (e.g. with `.validate()` and `.run()`) actually correct?**
  _`err` has 232 INFERRED edges - model-reasoned connections that need verification._
- **Are the 87 inferred relationships involving `spawn()` (e.g. with `.start()` and `serve()`) actually correct?**
  _`spawn()` has 87 INFERRED edges - model-reasoned connections that need verification._
- **Are the 92 inferred relationships involving `load()` (e.g. with `corpus_size_mismatch_reports_expected_and_actual_bytes_and_hashes()` and `corpus_oracle_rejects_wrong_hash_and_missing_scenario()`) actually correct?**
  _`load()` has 92 INFERRED edges - model-reasoned connections that need verification._
- **What connects `metadataUpdate`, `serverTrash`, `queueServerTrash` to the rest of the system?**
  _305 weakly-connected nodes found - possible documentation gaps or missing edges._