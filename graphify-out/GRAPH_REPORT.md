# Graph Report - desktop-graphify  (2026-10-10)

## Corpus Check
- 275 files · ~886,037 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 5615 nodes · 15525 edges · 53 communities detected
- Extraction: 78% EXTRACTED · 22% INFERRED · 0% AMBIGUOUS · INFERRED: 3355 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 35|Community 35]]
- [[_COMMUNITY_Community 36|Community 36]]
- [[_COMMUNITY_Community 40|Community 40]]
- [[_COMMUNITY_Community 55|Community 55]]
- [[_COMMUNITY_Community 56|Community 56]]
- [[_COMMUNITY_Community 64|Community 64]]
- [[_COMMUNITY_Community 65|Community 65]]
- [[_COMMUNITY_Community 68|Community 68]]
- [[_COMMUNITY_Community 69|Community 69]]
- [[_COMMUNITY_Community 70|Community 70]]
- [[_COMMUNITY_Community 71|Community 71]]
- [[_COMMUNITY_Community 72|Community 72]]
- [[_COMMUNITY_Community 76|Community 76]]
- [[_COMMUNITY_Community 78|Community 78]]
- [[_COMMUNITY_Community 88|Community 88]]
- [[_COMMUNITY_Community 89|Community 89]]
- [[_COMMUNITY_Community 96|Community 96]]
- [[_COMMUNITY_Community 101|Community 101]]

## God Nodes (most connected - your core abstractions)
1. `err` - 225 edges
2. `spawn()` - 107 edges
3. `StateDb` - 97 edges
4. `load()` - 92 edges
5. `ApiClient` - 78 edges
6. `load()` - 72 edges
7. `clear_session_impl()` - 67 edges
8. `EngineBridge` - 66 edges
9. `sleep()` - 62 edges
10. `production_source()` - 58 edges

## Surprising Connections (you probably didn't know these)
- `openView()` --calls--> `mount()`  [INFERRED]
  tests/onboardingAfterSignOut.test.tsx → src-tauri/src/linux_fuse/mod.rs
- `emit()` --calls--> `openActivity()`  [INFERRED]
  src-tauri/src/browser_login.rs → src/WindowsTray.tsx
- `purge()` --calls--> `finish()`  [INFERRED]
  src-tauri/src/windows_cf/signout_cleanup.rs → src/Onboarding.tsx
- `openAccountPage()` --calls--> `mount()`  [INFERRED]
  tests/accountSessionWarnings.test.tsx → src-tauri/src/linux_fuse/mod.rs
- `openDisconnect()` --calls--> `mount()`  [INFERRED]
  tests/accountSessionWarnings.test.tsx → src-tauri/src/linux_fuse/mod.rs

## Communities

### Community 0 - "Community 0"
Cohesion: 0.01
Nodes (801): load(), AccountConfig, AccountId, AccountRuntime, active_account_after_synthesis_is_default_shape(), active_account_empty_registry_errs(), fixed_id(), IdentifyFlight (+793 more)

### Community 1 - "Community 1"
Cohesion: 0.01
Nodes (327): is_conflict(), is_text_file(), a_failed_upload_leaves_nothing_on_the_board_and_no_activity_row(), a_finished_upload_adds_its_bytes_to_the_batch_and_records_an_up_row(), a_hydrate_reports_per_file_bytes_and_records_a_down_row(), a_hydrate_without_a_caller_callback_still_fills_the_board(), a_resumed_upload_starts_its_bar_at_the_acknowledged_watermark(), an_upload_reports_its_bytes_chunk_by_chunk_and_a_cut_one_leaves_nothing() (+319 more)

### Community 2 - "Community 2"
Cohesion: 0.01
Nodes (212): Account, AppHandle, AppState, Attempt, Auth, AuthVault, Client, Commands (+204 more)

### Community 3 - "Community 3"
Cohesion: 0.01
Nodes (203): rustSource(), rustStr(), diagnostics(), lock(), refresh(), runAction(), signOut(), titleCasePlan() (+195 more)

### Community 4 - "Community 4"
Cohesion: 0.02
Nodes (185): change(), a_kept_folder_saved_during_a_reconciler_config_update_survives_it(), a_reconciler_config_update_during_a_kept_folder_save_keeps_the_folder(), in_flight(), saved_folder(), set_failure(), set_signed_out_by_choice(), update_config() (+177 more)

### Community 5 - "Community 5"
Cohesion: 0.02
Nodes (185): reset(), round7_held_recovery_reply_allows_prompt_teardown_without_key_or_runner(), round7_late_verified_recovery_key_is_fenced_before_persistence(), Zeroizing<T>, Config, config_path(), load_config(), save_config() (+177 more)

### Community 6 - "Community 6"
Cohesion: 0.02
Nodes (105): DeleteDisposition, directoryNotEmpty, queueServerTrash, FileProviderEnumerator, FileProviderExtension, MaterializedSetCollector, ModifyRoute, metadataUpdate (+97 more)

### Community 7 - "Community 7"
Cohesion: 0.03
Nodes (58): a_full_clear_after_a_token_only_clear_leaves_no_vault_key(), a_keyless_legacy_token_never_lands_next_to_a_kept_vault_key(), a_session_token_is_wiped_when_it_goes(), account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault (+50 more)

### Community 8 - "Community 8"
Cohesion: 0.02
Nodes (83): BeebeebItemKind, file, folder, BeebeebProviderItem, IPCCancellation, IPCExchange, IPCFrameReader, IPCFraming (+75 more)

### Community 9 - "Community 9"
Cohesion: 0.03
Nodes (117): err, DomainControlTool, KeptFolder, empty, hasEntries, missing, noPath, notReported (+109 more)

### Community 10 - "Community 10"
Cohesion: 0.03
Nodes (52): a_failure_then_an_answer_leaves_the_link_up(), a_typed_get_answered_with_401_is_an_answer_not_a_link_failure(), a_typed_get_answered_with_503_records_the_server_not_answering(), a_typed_get_that_is_answered_records_an_answer(), a_typed_get_with_nothing_listening_records_offline(), api_client_list_files_sends_provenance_headers_on_the_wire(), api_client_zeroizes_master_key_on_drop(), ApiClient (+44 more)

### Community 11 - "Community 11"
Cohesion: 0.03
Nodes (73): requestPayload(), AndroidKeyboard(), IOSKeyboard(), check(), main(), self_test(), a_failed_reset_blocks_the_engine_and_records_nothing(), adopt_unbound() (+65 more)

### Community 12 - "Community 12"
Cohesion: 0.03
Nodes (84): is_gate_busy(), auto_resolution_deadline(), ConflictRecord, Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed() (+76 more)

### Community 13 - "Community 13"
Cohesion: 0.05
Nodes (63): a_call_that_never_returns_is_cut_off_at_its_limit_with_a_redacted_timeout_error(), a_confirmed_removal_reaches_the_port_and_a_failed_one_does_not(), a_disk_image_or_translocated_launch_is_missing_for_not_in_applications_before_any_check(), a_dmg_launch_publishes_not_in_applications_at_start_with_no_trigger(), a_domain_turned_off_while_ready_is_noticed_by_the_poll_without_a_relaunch(), a_handle_says_so_when_its_reconciler_is_gone_or_silent_and_refuses_what_is_not_a_removal(), a_launch_runs_one_check_and_publishes_ready(), a_panicking_port_ends_in_one_failed_unknown_view_and_a_handle_that_says_so() (+55 more)

### Community 14 - "Community 14"
Cohesion: 0.04
Nodes (83): a_401_error_is_not_a_connectivity_phase(), a_401_from_the_usage_call_is_an_error_that_does_not_carry_the_token(), a_failed_fetch_with_nothing_cached_is_no_summary_and_another_sessions_is_not_reused(), a_failed_refetch_serves_the_old_number_marked_stale(), a_failure_shown_elsewhere_does_not_paint_f2_here(), a_file_on_the_board_is_syncing_even_when_the_last_event_said_idle(), a_file_uploaded_twice_is_one_finished_row_the_newest(), a_finished_transfer_of_a_file_moving_again_is_not_listed_twice() (+75 more)

### Community 15 - "Community 15"
Cohesion: 0.04
Nodes (93): bind_ipc_listener(), bind_ipc_listener_with(), capabilities_for_status(), dedup_write(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_provider_change_payloads() (+85 more)

### Community 16 - "Community 16"
Cohesion: 0.03
Nodes (87): a_home_on_an_external_disk_is_a_real_install(), at(), current(), current_bundle_path(), is_at_volume_root(), is_under_volume_applications(), launch_location(), LaunchLocation (+79 more)

### Community 17 - "Community 17"
Cohesion: 0.03
Nodes (57): allElements(), click(), installMiniDom(), MiniComment, MiniDocument, MiniNode, heldSignOutWarning(), alerts() (+49 more)

### Community 18 - "Community 18"
Cohesion: 0.03
Nodes (44): elementsOf(), finderBus(), finderView(), tick(), useFinderSetupModule(), BeebeebFS, dir_attr(), file_attr() (+36 more)

### Community 19 - "Community 19"
Cohesion: 0.07
Nodes (74): a_disk_image_launch_reports_the_reason_before_any_check_and_never_adds(), a_failed_read_after_add_is_not_evidence_and_still_waits(), a_held_ready_core_does_not_poll(), a_held_user_disabled_core_does_not_poll_and_keys_arriving_resumes_it(), a_ready_core_with_no_window_visible_sets_no_timer_and_a_focus_resumes_the_poll(), a_ready_domain_turned_off_while_running_is_read_once_when_the_app_becomes_active(), a_ready_domain_turned_off_while_running_lands_user_disabled_from_the_poll_alone(), a_registered_domain_is_confirmed_even_from_a_disk_image() (+66 more)

### Community 20 - "Community 20"
Cohesion: 0.04
Nodes (55): backup_dest_root(), backup_source_key_for_rel_path(), backup_source_key_for_this_device(), classify_copy(), classify_skip_when_size_and_mtime_match(), copy_preserving_mtime(), CopyDecision, dest_root_builds_backup_device_folder() (+47 more)

### Community 21 - "Community 21"
Cohesion: 0.08
Nodes (44): a_frame_split_across_two_reads_is_reassembled(), a_partial_value_at_eof_is_returned_for_the_caller_to_reject(), an_oversized_frame_is_refused_instead_of_buffered_forever(), an_unframed_complete_value_is_accepted_without_waiting_for_eof(), blank_lines_are_skipped(), ends_like_json_value(), FrameError, FrameReader (+36 more)

### Community 22 - "Community 22"
Cohesion: 0.05
Nodes (36): handle_new_conflict(), close_policy(), close_policy_with(), conflict_auto_opens_window(), item_from(), lib_rs(), macos_destroys_only_onboarding_and_review(), Platform (+28 more)

### Community 23 - "Community 23"
Cohesion: 0.12
Nodes (54): a_cached_create_is_not_returned_for_a_row_parked_trashing(), a_cached_create_is_not_returned_once_its_row_is_gone(), a_cached_create_is_not_returned_while_a_trash_op_is_pending_for_its_row(), a_cached_reply_reports_the_row_as_it_is_now_not_as_it_was(), a_create_after_the_trash_op_finished_still_uploads_the_new_file(), a_failed_create_is_not_remembered_so_the_retry_actually_runs(), a_malformed_line_gets_an_error_and_the_connection_keeps_working(), a_real_finder_delete_then_the_same_create_uploads_the_new_file() (+46 more)

### Community 24 - "Community 24"
Cohesion: 0.08
Nodes (54): a_forged_name_marker_in_input_is_stripped(), assert_credential_shape(), boundary_ok(), classify_error_code(), credential_authorization_bearer_uuid_keeps_following_words(), credential_authorization_colon_bearer_uuid(), credential_authorization_equals_bearer(), credential_bearer_colon() (+46 more)

### Community 25 - "Community 25"
Cohesion: 0.11
Nodes (35): a_centre_exactly_on_the_shared_edge_belongs_to_the_right_display(), a_centre_off_every_display_uses_the_largest_overlap(), a_display_left_of_the_primary_has_a_negative_origin(), a_screen_narrower_than_the_popover_keeps_the_top_left_on_screen(), ambiguous_two_display_case_picks_the_external_display(), an_icon_on_no_display_falls_back_to_the_primary_not_the_centre(), an_icon_straddling_two_displays_goes_with_its_centre(), centres_under_the_icon_with_the_gap() (+27 more)

### Community 26 - "Community 26"
Cohesion: 0.1
Nodes (27): api_client_search_shard_urls_match_web_paths(), build_local_index(), compare_records_for_query(), DesktopSearchIndexState, DesktopSearchResponse, DesktopSearchResult, FixtureSearchStore, FixtureState (+19 more)

### Community 27 - "Community 27"
Cohesion: 0.12
Nodes (28): a_payload_without_a_message_still_deserializes_and_round_trips_without_one(), display_keeps_the_message_for_stdout_tracing(), FailureRecord, FpError, FpErrorCode, redacted_is_domain_and_code_and_never_the_message(), redacted_names_the_underlying_pair_and_still_not_the_message(), serializing_an_fp_error_never_carries_the_message() (+20 more)

### Community 28 - "Community 28"
Cohesion: 0.09
Nodes (24): a_bad_scale_factor_is_treated_as_one_not_a_division_by_zero(), a_window_spanning_two_displays_is_not_fully_inside_either(), displays(), entries_outside_every_work_area_are_ignored(), every_other_reason_is_reported_on_its_own(), flags_are_size_and_position_only(), IgnoreReason, outer_rect() (+16 more)

### Community 29 - "Community 29"
Cohesion: 0.17
Nodes (19): Clock, Leader<T>, a_cache_hit_is_refreshed_on_every_serve_but_the_stored_copy_is_left_as_built(), a_failed_result_is_shared_with_waiters_but_not_remembered(), a_key_reused_for_a_different_request_shape_is_not_merged(), a_repeat_after_completion_returns_the_stored_result_without_running_again(), a_stored_result_expires_exactly_at_the_ttl(), a_stored_result_that_fails_validation_is_dropped_and_the_work_runs_again() (+11 more)

### Community 30 - "Community 30"
Cohesion: 0.11
Nodes (21): analyse(), bindingNames(), calleeName(), censusOfSource(), classify(), enclosingFunction(), familyLiterals(), guardSides() (+13 more)

### Community 31 - "Community 31"
Cohesion: 0.13
Nodes (21): classify_sign_in_after(), a_completed_key_proof_decides_and_an_unavailable_one_settles_nothing(), a_retained_vault_key_alone_is_never_fresh_and_goes_to_the_server(), after_a_revoked_token_with_nothing_queued(), after_key_proof(), another_account_after_a_revoked_token_with_nothing_queued_is_a_switch_never_fresh(), credentials_with_no_owner_record_are_judged_by_the_same_order_as_an_owner(), every_retained_trace_counts() (+13 more)

### Community 32 - "Community 32"
Cohesion: 0.14
Nodes (19): a_finder_failure_shown_by_another_surface_is_not_repeated_as_f2(), a_stopped_or_unstarted_engine_is_not_locked(), Connectivity, each_trigger_alone_yields_its_phase(), everything_wrong_at_once_is_signed_out(), finder_names_match_what_serde_writes(), finder_problems_outrank_paused_and_the_network_states(), finder_turned_off_in_system_settings_is_its_own_phase_not_missing() (+11 more)

### Community 33 - "Community 33"
Cohesion: 0.19
Nodes (12): CallbackGate, CallbackGate<T>, CallbackLease, CallbackLease<T>, drain_waits_for_transfer_and_releases_credentials(), Generation, LeaseState, revoke_denies_new_work_and_cancels_existing_work() (+4 more)

### Community 34 - "Community 34"
Cohesion: 0.17
Nodes (12): glob_match(), granted_window_patterns(), macos_merge_still_creates_the_startup_windows_and_merges_the_platform_file(), manifest_dir(), merged_config(), only_the_macos_platform_file_exists(), patterns_granted_by(), read_json() (+4 more)

### Community 35 - "Community 35"
Cohesion: 0.34
Nodes (13): commandForTauriCli(), decodeMinisignBase64Line(), decodeTauriBase64Text(), fail(), generateKeypair(), main(), parseTauriPubkey(), parseTauriSignature() (+5 more)

### Community 36 - "Community 36"
Cohesion: 0.15
Nodes (11): Availability, capability_wire_fixtures_cover_hosts_and_packages(), DesktopCapabilities, FilesystemIntegration, host_os(), HostOs, InstallFormat, SharingLevel (+3 more)

### Community 40 - "Community 40"
Cohesion: 0.33
Nodes (8): a_click_on_a_visible_popover_hides_it_and_clears_a_stale_blur(), a_click_right_after_a_blur_hide_does_not_reopen(), a_clock_that_steps_backwards_counts_as_the_same_gesture(), BlurClickGuard, one_blur_suppresses_one_click(), the_window_is_250_ms_exclusive(), TrayClickAction, with_no_blur_a_click_opens()

### Community 55 - "Community 55"
Cohesion: 0.32
Nodes (4): manualUpdateToast(), buildDowngradeConfirmationViewModel(), buildUpdateCheckViewModel(), releaseChannelLabel()

### Community 56 - "Community 56"
Cohesion: 0.39
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 64 - "Community 64"
Cohesion: 0.38
Nodes (3): compare_prerelease_parts(), ParsedReleaseVersion, PrereleasePart

### Community 65 - "Community 65"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 68 - "Community 68"
Cohesion: 0.4
Nodes (2): declOf(), rulesFor()

### Community 69 - "Community 69"
Cohesion: 0.4
Nodes (2): findColorViolations(), toKebabCase()

### Community 70 - "Community 70"
Cohesion: 0.33
Nodes (5): Claim, Entry, Leader, State, WriteDedup

### Community 71 - "Community 71"
Cohesion: 0.33
Nodes (1): ApiClient

### Community 72 - "Community 72"
Cohesion: 0.47
Nodes (4): availableSettingsSections(), settingLabel(), shellIntegrationLabel(), withPlatformLabels()

### Community 76 - "Community 76"
Cohesion: 0.7
Nodes (4): Find-SevenZip(), Invoke-InstallerExtraction(), Invoke-SelfTest(), Test-Signature()

### Community 78 - "Community 78"
Cohesion: 0.7
Nodes (3): consumeStoredNav(), initialNav(), navIdFromString()

### Community 88 - "Community 88"
Cohesion: 0.67
Nodes (2): file(), folder()

### Community 89 - "Community 89"
Cohesion: 0.67
Nodes (2): Install-ArtifactSigningDlib(), Stop-Signing()

### Community 96 - "Community 96"
Cohesion: 1.0
Nodes (2): extractBalancedBlock(), extractFunctionBody()

### Community 101 - "Community 101"
Cohesion: 1.0
Nodes (2): productRegionLabel(), useRegionLabel()

## Knowledge Gaps
- **303 isolated node(s):** `metadataUpdate`, `serverTrash`, `queueServerTrash`, `directoryNotEmpty`, `daemonUnavailable` (+298 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 68`** (6 nodes): `declOf()`, `parseCss()`, `read()`, `rulesFor()`, `macSettingsLayout.test.ts`, `walk()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 69`** (6 nodes): `findColorViolations()`, `isCommentLine()`, `listSourceFiles()`, `oklchLiteralLines()`, `toKebabCase()`, `noHardcodedThemeColors.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 71`** (6 nodes): `ApiClient`, `.delete_shard()`, `.fetch_manifest()`, `.get_shard()`, `.master_key_bytes()`, `.put_shard()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 88`** (4 nodes): `file()`, `folder()`, `macSettingsModel.test.ts`, `v()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 89`** (4 nodes): `sign-artifact.ps1`, `Find-SignTool()`, `Install-ArtifactSigningDlib()`, `Stop-Signing()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 96`** (3 nodes): `extractBalancedBlock()`, `extractFunctionBody()`, `fileProviderHandoffCopiesToSystemTempDir.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 101`** (3 nodes): `useRegion.ts`, `productRegionLabel()`, `useRegionLabel()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `commandUnavailableLabel()` connect `Community 3` to `Community 0`?**
  _High betweenness centrality (0.036) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 1`, `Community 2`, `Community 4`, `Community 5`, `Community 6`, `Community 8`, `Community 9`, `Community 10`, `Community 12`, `Community 13`, `Community 22`, `Community 27`?**
  _High betweenness centrality (0.035) - this node is a cross-community bridge._
- **Why does `load()` connect `Community 0` to `Community 1`, `Community 3`, `Community 9`, `Community 15`, `Community 26`?**
  _High betweenness centrality (0.035) - this node is a cross-community bridge._
- **Are the 224 inferred relationships involving `err` (e.g. with `.validate()` and `.run()`) actually correct?**
  _`err` has 224 INFERRED edges - model-reasoned connections that need verification._
- **Are the 82 inferred relationships involving `spawn()` (e.g. with `.start()` and `serve()`) actually correct?**
  _`spawn()` has 82 INFERRED edges - model-reasoned connections that need verification._
- **Are the 91 inferred relationships involving `load()` (e.g. with `corpus_size_mismatch_reports_expected_and_actual_bytes_and_hashes()` and `corpus_oracle_rejects_wrong_hash_and_missing_scenario()`) actually correct?**
  _`load()` has 91 INFERRED edges - model-reasoned connections that need verification._
- **What connects `metadataUpdate`, `serverTrash`, `queueServerTrash` to the rest of the system?**
  _303 weakly-connected nodes found - possible documentation gaps or missing edges._