# Graph Report - desktop-1639  (2026-09-30)

## Corpus Check
- 192 files · ~399,503 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 2825 nodes · 6033 edges · 40 communities detected
- Extraction: 81% EXTRACTED · 19% INFERRED · 0% AMBIGUOUS · INFERRED: 1130 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 29|Community 29]]
- [[_COMMUNITY_Community 44|Community 44]]
- [[_COMMUNITY_Community 45|Community 45]]
- [[_COMMUNITY_Community 53|Community 53]]
- [[_COMMUNITY_Community 56|Community 56]]
- [[_COMMUNITY_Community 57|Community 57]]
- [[_COMMUNITY_Community 58|Community 58]]
- [[_COMMUNITY_Community 60|Community 60]]
- [[_COMMUNITY_Community 63|Community 63]]
- [[_COMMUNITY_Community 65|Community 65]]
- [[_COMMUNITY_Community 83|Community 83]]
- [[_COMMUNITY_Community 86|Community 86]]
- [[_COMMUNITY_Community 96|Community 96]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 77 edges
2. `load()` - 72 edges
3. `load()` - 70 edges
4. `StateDb` - 65 edges
5. `EngineBridge` - 62 edges
6. `command()` - 49 edges
7. `test_bridge_with_api()` - 44 edges
8. `run()` - 34 edges
9. `clear_session_impl()` - 30 edges
10. `spawn()` - 29 edges

## Surprising Connections (you probably didn't know these)
- `corpus_oracle_rejects_wrong_hash_and_missing_scenario()` --calls--> `load()`  [INFERRED]
  src-tauri/tests/support/bridge.rs → src/windows/views/ActivityView.tsx
- `self_test()` --calls--> `Fixture`  [INFERRED]
  scripts/assert-cargo-test-counts.py → src-tauri/tests/support/native_parity.rs
- `round7_held_recovery_reply_allows_prompt_teardown_without_key_or_runner()` --calls--> `desktop_unlock_with_recovery_phrase()`  [INFERRED]
  scripts/fixtures/recovery-transition.rs → src-tauri/src/lib.rs
- `round7_late_verified_recovery_key_is_fenced_before_persistence()` --calls--> `desktop_unlock_with_recovery_phrase()`  [INFERRED]
  scripts/fixtures/recovery-transition.rs → src-tauri/src/lib.rs
- `text()` --calls--> `parse_release_version()`  [INFERRED]
  tests/recoveryEntry.test.ts → src-tauri/src/lib.rs

## Communities

### Community 0 - "Community 0"
Cohesion: 0.01
Nodes (303): load(), provenance_headers(), ensure_directory(), hydrate_dest_is_allowed(), linux_thumbnail_source_path_for_entry(), legacy_platform_keychain_store(), migrate_legacy_keychain_to_account(), platform_keychain_store_for() (+295 more)

### Community 1 - "Community 1"
Cohesion: 0.02
Nodes (223): is_conflict(), is_text_file(), apply_shared_context(), apply_shared_metadata_file_row(), apply_snapshot(), apply_sync_op(), assert_rename_then_update_in_one_tick_applies_update(), audit_1244_apply_sync_op_trash_echo_preserves_local_trashing_owner() (+215 more)

### Community 2 - "Community 2"
Cohesion: 0.02
Nodes (136): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), toggleFolder(), openRoot() (+128 more)

### Community 3 - "Community 3"
Cohesion: 0.02
Nodes (100): local_index_builds_from_state_db_leaf_file_names_and_skips_folders(), local_query_uses_core_tokenization_and_returns_ranked_results(), seed_file(), apply_metadata_file_row(), decode_image_thumbnail_source(), hydrate_file_fails_closed_on_symlink_destination_toctou(), ipc_allows_hydrate_write_under_allowed_root(), ipc_rejects_hydrate_write_outside_allowed_roots() (+92 more)

### Community 4 - "Community 4"
Cohesion: 0.03
Nodes (131): engine_internal_filters_state_dir_and_lock(), ipc_hydrate_roundtrip(), ipc_socket_file_is_chmod_0600_after_bind(), relative_db_path_is_slash_joined_without_leading_slash(), bind_ipc_listener(), bind_ipc_listener_with(), capabilities_for_status(), file_entry_payload() (+123 more)

### Community 5 - "Community 5"
Cohesion: 0.03
Nodes (46): AndroidKeyboard(), IOSKeyboard(), account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S> (+38 more)

### Community 6 - "Community 6"
Cohesion: 0.02
Nodes (96): AccountConfig, AccountId, AccountRuntime, active_account_after_synthesis_is_default_shape(), active_account_empty_registry_errs(), fixed_id(), synthesize_is_idempotent(), synthesize_single_account() (+88 more)

### Community 7 - "Community 7"
Cohesion: 0.03
Nodes (102): account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity, AccountActivityEvent, AccountActivitySummary, AccountProfile (+94 more)

### Community 8 - "Community 8"
Cohesion: 0.03
Nodes (34): api_client_list_files_sends_provenance_headers_on_the_wire(), api_client_zeroizes_master_key_on_drop(), ApiClient, CreatedSession, DesktopUploadInitRequest, DesktopUploadInitResponse, header_secs(), header_secs_parses_numeric_and_rejects_dates() (+26 more)

### Community 9 - "Community 9"
Cohesion: 0.04
Nodes (31): FileProviderEnumerator, FileProviderExtension, BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts (+23 more)

### Community 10 - "Community 10"
Cohesion: 0.07
Nodes (38): assert_clean(), assert_corpus_endpoint_consistency(), assert_same_metadata(), bridge(), corpus_conflict_versions_and_stale_precondition(), corpus_encrypted_bytes_namespaces_and_thumbnails(), corpus_endpoint_consistency_all_objects_and_remote_edit(), corpus_expiry_is_account_scoped() (+30 more)

### Community 11 - "Community 11"
Cohesion: 0.05
Nodes (33): ancestors(), classifyJsxUsage(), comparesToErrorPhase(), isCorrectableInput(), isErrorOrigin(), isInsideUseEffect(), isLoadPath(), isRetryCallback() (+25 more)

### Community 12 - "Community 12"
Cohesion: 0.08
Nodes (37): DomainControlTool, check(), main(), self_test(), file_provider_installed(), install_file_provider_domain(), complete_upload_placeholder(), convert_to_in_sync_placeholder() (+29 more)

### Community 13 - "Community 13"
Cohesion: 0.05
Nodes (21): Account, AppHandle, AppState, Attempt, Auth, AuthVault, Client, Commands (+13 more)

### Community 14 - "Community 14"
Cohesion: 0.11
Nodes (23): api_client_search_shard_urls_match_web_paths(), build_local_index(), compare_records_for_query(), DesktopSearchIndexState, DesktopSearchResponse, DesktopSearchResult, FixtureSearchStore, FixtureState (+15 more)

### Community 15 - "Community 15"
Cohesion: 0.13
Nodes (24): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), archive_legacy_state_dir_if_present(), beebeeb_state_dir_from_app_local_data(), copy_dir_all() (+16 more)

### Community 16 - "Community 16"
Cohesion: 0.11
Nodes (21): build_ws_request(), decrypt_payload(), device_code_recv_errors_immediately_on_closed_connection(), device_code_recv_returns_promptly_on_healthy_connection(), device_code_recv_times_out_when_server_stalls_after_connect(), emit(), HealthyStream, open_browser() (+13 more)

### Community 17 - "Community 17"
Cohesion: 0.12
Nodes (18): file_provider_domain_user_enabled(), finder_domain_user_enabled(), buffer_to_string(), call_bridge(), decide_install_step(), domain_exists(), domain_user_enabled(), domain_user_enabled_state() (+10 more)

### Community 18 - "Community 18"
Cohesion: 0.15
Nodes (24): path_is_engine_internal(), assert_uploading_row(), debounce_loop(), dispatch_local_create(), engine_delete_suppress(), engine_delete_suppression_distinguishes_paths(), engine_delete_suppression_is_consumed_once(), engine_delete_suppression_prunes_stale_entries() (+16 more)

### Community 19 - "Community 19"
Cohesion: 0.14
Nodes (11): Args, b64(), futures_task_noop_waker(), is_ignored_file(), load_master_key(), main(), sync_batch(), upload_file() (+3 more)

### Community 20 - "Community 20"
Cohesion: 0.19
Nodes (12): CallbackGate, CallbackGate<T>, CallbackLease, CallbackLease<T>, drain_waits_for_transfer_and_releases_credentials(), Generation, LeaseState, revoke_denies_new_work_and_cancels_existing_work() (+4 more)

### Community 21 - "Community 21"
Cohesion: 0.21
Nodes (13): encode_freedesktop_png(), file_uri(), fixture_png(), FreedesktopThumbnailSize, md5_hex(), png_output_is_rgba_resized_and_embeds_uri_and_mtime_text_chunks(), remove_freedesktop_thumbnails(), resize_rgba_to_fit() (+5 more)

### Community 22 - "Community 22"
Cohesion: 0.2
Nodes (8): Contract, Fixture, purge(), Row, StateDb, windows_signout_hydrated_sentinel_is_not_external_cache(), windows_signout_rejects_external_cache_before_deleting_placeholders(), windows_signout_retries_after_partial_cleanup_with_hydrated_sentinel()

### Community 23 - "Community 23"
Cohesion: 0.17
Nodes (10): auto_resolution_deadline(), ConflictRecord, Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed(), v() (+2 more)

### Community 24 - "Community 24"
Cohesion: 0.34
Nodes (13): commandForTauriCli(), decodeMinisignBase64Line(), decodeTauriBase64Text(), fail(), generateKeypair(), main(), parseTauriPubkey(), parseTauriSignature() (+5 more)

### Community 25 - "Community 25"
Cohesion: 0.15
Nodes (11): Availability, capability_wire_fixtures_cover_hosts_and_packages(), DesktopCapabilities, FilesystemIntegration, host_os(), HostOs, InstallFormat, SharingLevel (+3 more)

### Community 26 - "Community 26"
Cohesion: 0.36
Nodes (6): Attempt, AuthAttempts, ReopenOnDrop, round5_browser_handoff_after_signout_persists_nothing_and_starts_no_runner(), round5_lock_cancels_handoff_and_drains_credential_owner(), round7_recovery_verification_cancels_and_late_install_is_rejected()

### Community 29 - "Community 29"
Cohesion: 0.23
Nodes (7): auth_wiring(), body(), cloud_files_unregister_precedes_shell_unregister_and_propagates_failure(), every_credential_command_has_admission_before_credential_reads(), every_engine_start_requires_transition_and_generation_validation(), recovery_verification_is_fenced(), round7_recovery_guard_rejects_held_transition_and_missing_final_fence()

### Community 44 - "Community 44"
Cohesion: 0.32
Nodes (4): manualUpdateToast(), buildDowngradeConfirmationViewModel(), buildUpdateCheckViewModel(), releaseChannelLabel()

### Community 45 - "Community 45"
Cohesion: 0.39
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 53 - "Community 53"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 56 - "Community 56"
Cohesion: 0.33
Nodes (1): ApiClient

### Community 57 - "Community 57"
Cohesion: 0.4
Nodes (2): localDayIndex(), planRenewalCopy()

### Community 58 - "Community 58"
Cohesion: 0.47
Nodes (4): availableSettingsSections(), settingLabel(), shellIntegrationLabel(), withPlatformLabels()

### Community 60 - "Community 60"
Cohesion: 0.5
Nodes (2): findColorViolations(), toKebabCase()

### Community 63 - "Community 63"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

### Community 65 - "Community 65"
Cohesion: 0.7
Nodes (3): consumeStoredNav(), initialNav(), navIdFromString()

### Community 83 - "Community 83"
Cohesion: 0.67
Nodes (1): StatusUiClassFactory_Impl

### Community 86 - "Community 86"
Cohesion: 1.0
Nodes (2): productRegionLabel(), useRegionLabel()

### Community 96 - "Community 96"
Cohesion: 1.0
Nodes (1): StatusUiSourceFactory_Impl

## Knowledge Gaps
- **188 isolated node(s):** `AppHandle`, `Session`, `Account`, `Zeroizing`, `namespace` (+183 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 56`** (6 nodes): `ApiClient`, `.delete_shard()`, `.fetch_manifest()`, `.get_shard()`, `.master_key_bytes()`, `.put_shard()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 57`** (6 nodes): `localDayIndex()`, `planRenewalCopy()`, `planStatusTone()`, `quotaPercent()`, `titleCasePlan()`, `planPresentation.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 60`** (5 nodes): `findColorViolations()`, `isCommentLine()`, `listSourceFiles()`, `toKebabCase()`, `noHardcodedThemeColors.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 63`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 83`** (3 nodes): `StatusUiClassFactory_Impl`, `.CreateInstance()`, `.LockServer()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 86`** (3 nodes): `useRegion.ts`, `productRegionLabel()`, `useRegionLabel()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 96`** (2 nodes): `StatusUiSourceFactory_Impl`, `.GetStatusUISource()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `load()` connect `Community 0` to `Community 1`, `Community 2`, `Community 3`, `Community 4`, `Community 18`?**
  _High betweenness centrality (0.118) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 2` to `Community 0`, `Community 9`?**
  _High betweenness centrality (0.104) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 1`, `Community 5`, `Community 6`, `Community 7`, `Community 9`, `Community 15`?**
  _High betweenness centrality (0.043) - this node is a cross-community bridge._
- **Are the 71 inferred relationships involving `load()` (e.g. with `corpus_size_mismatch_reports_expected_and_actual_bytes_and_hashes()` and `corpus_oracle_rejects_wrong_hash_and_missing_scenario()`) actually correct?**
  _`load()` has 71 INFERRED edges - model-reasoned connections that need verification._
- **Are the 69 inferred relationships involving `load()` (e.g. with `commandUnavailableLabel()` and `.do_upload_version()`) actually correct?**
  _`load()` has 69 INFERRED edges - model-reasoned connections that need verification._
- **What connects `AppHandle`, `Session`, `Account` to the rest of the system?**
  _188 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.01 - nodes in this community are weakly interconnected._