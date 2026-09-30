# Graph Report - desktop-1640  (2026-09-30)

## Corpus Check
- 168 files · ~385,967 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 2656 nodes · 5765 edges · 36 communities detected
- Extraction: 82% EXTRACTED · 18% INFERRED · 0% AMBIGUOUS · INFERRED: 1052 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 27|Community 27]]
- [[_COMMUNITY_Community 41|Community 41]]
- [[_COMMUNITY_Community 49|Community 49]]
- [[_COMMUNITY_Community 50|Community 50]]
- [[_COMMUNITY_Community 53|Community 53]]
- [[_COMMUNITY_Community 54|Community 54]]
- [[_COMMUNITY_Community 55|Community 55]]
- [[_COMMUNITY_Community 57|Community 57]]
- [[_COMMUNITY_Community 60|Community 60]]
- [[_COMMUNITY_Community 62|Community 62]]
- [[_COMMUNITY_Community 79|Community 79]]
- [[_COMMUNITY_Community 90|Community 90]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 77 edges
2. `load()` - 70 edges
3. `load()` - 70 edges
4. `EngineBridge` - 69 edges
5. `StateDb` - 61 edges
6. `test_bridge_with_api()` - 48 edges
7. `command()` - 47 edges
8. `run()` - 30 edges
9. `spawn()` - 29 edges
10. `clear_session_impl()` - 26 edges

## Surprising Connections (you probably didn't know these)
- `linux_thumbnail_source_path_for_entry()` --calls--> `load()`  [INFERRED]
  src-tauri/src/engine_bridge.rs → src/windows/views/ActivityView.tsx
- `linux_thumbnail_source_path_for_entry()` --calls--> `load()`  [INFERRED]
  src-tauri/src/engine_bridge.rs → src/pages/SelectiveSync.tsx
- `clear_session()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/windows/views/ActivityView.tsx
- `main()` --calls--> `run()`  [INFERRED]
  windows/src/main.rs → src-tauri/src/runner.rs
- `handle_connection()` --calls--> `load()`  [INFERRED]
  src-tauri/src/ipc_socket.rs → src/windows/views/ActivityView.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.01
Nodes (300): DomainControlTool, load(), ensure_directory(), hydrate_dest_is_allowed(), legacy_platform_keychain_store(), migrate_legacy_keychain_to_account(), platform_keychain_store_for(), about_metadata() (+292 more)

### Community 1 - "Community 1"
Cohesion: 0.02
Nodes (229): is_text_file(), apply_shared_context(), apply_shared_metadata_file_row(), apply_snapshot(), apply_sync_op(), assert_rename_then_update_in_one_tick_applies_update(), audit_1244_apply_sync_op_trash_echo_preserves_local_trashing_owner(), audit_1244_auto_resolve_keep_both_preserves_local_copy_when_remote_hydrate_fails() (+221 more)

### Community 2 - "Community 2"
Cohesion: 0.02
Nodes (138): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), toggleFolder(), openRoot() (+130 more)

### Community 3 - "Community 3"
Cohesion: 0.02
Nodes (93): local_index_builds_from_state_db_leaf_file_names_and_skips_folders(), local_query_uses_core_tokenization_and_returns_ranked_results(), seed_file(), apply_metadata_file_row(), record_moved_to_trash_activity_writes_deletion_event(), resolve_relative_path(), test_metadata_file_row_classifies_folder_rows(), test_metadata_file_row_composes_nested_path_under_parent() (+85 more)

### Community 4 - "Community 4"
Cohesion: 0.02
Nodes (98): AccountConfig, AccountId, AccountRuntime, active_account_after_synthesis_is_default_shape(), active_account_empty_registry_errs(), fixed_id(), synthesize_is_idempotent(), synthesize_single_account() (+90 more)

### Community 5 - "Community 5"
Cohesion: 0.03
Nodes (49): api_client_list_files_sends_provenance_headers_on_the_wire(), api_client_zeroizes_master_key_on_drop(), ApiClient, CreatedSession, DesktopUploadInitRequest, DesktopUploadInitResponse, header_secs(), header_secs_parses_numeric_and_rejects_dates() (+41 more)

### Community 6 - "Community 6"
Cohesion: 0.04
Nodes (41): AndroidKeyboard(), IOSKeyboard(), account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S> (+33 more)

### Community 7 - "Community 7"
Cohesion: 0.03
Nodes (101): account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity, AccountActivityEvent, AccountActivitySummary, AccountProfile (+93 more)

### Community 8 - "Community 8"
Cohesion: 0.03
Nodes (92): DesktopSettings, engine_internal_filters_state_dir_and_lock(), relative_db_path_is_slash_joined_without_leading_slash(), build_macos_file_provider_bridge(), main(), content_is_confirmed(), should_inspect_close(), db_placeholder_path() (+84 more)

### Community 9 - "Community 9"
Cohesion: 0.04
Nodes (34): FileProviderEnumerator, FileProviderExtension, BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts (+26 more)

### Community 10 - "Community 10"
Cohesion: 0.06
Nodes (61): check(), main(), self_test(), ipc_hydrate_roundtrip(), ipc_socket_file_is_chmod_0600_after_bind(), bind_ipc_listener(), capabilities_for_status(), file_entry_payload() (+53 more)

### Community 11 - "Community 11"
Cohesion: 0.12
Nodes (35): is_ignored_finder_name(), path_is_engine_internal(), relative_db_path(), assert_uploading_row(), debounce_loop(), dispatch_local_create(), engine_delete_suppress(), engine_delete_suppression_distinguishes_paths() (+27 more)

### Community 12 - "Community 12"
Cohesion: 0.11
Nodes (23): api_client_search_shard_urls_match_web_paths(), build_local_index(), compare_records_for_query(), DesktopSearchIndexState, DesktopSearchResponse, DesktopSearchResult, FixtureSearchStore, FixtureState (+15 more)

### Community 13 - "Community 13"
Cohesion: 0.11
Nodes (20): build_ws_request(), decrypt_payload(), device_code_recv_errors_immediately_on_closed_connection(), device_code_recv_returns_promptly_on_healthy_connection(), device_code_recv_times_out_when_server_stalls_after_connect(), emit(), HealthyStream, open_browser() (+12 more)

### Community 14 - "Community 14"
Cohesion: 0.19
Nodes (24): http_json(), resumable_mock_response(), regression_1640_r3_changed_download_never_establishes_baseline(), regression_1640_r3_download_baseline_survives_remote_update(), regression_1640_r3_keep_both_chain_restart_save(), regression_1640_r3_keep_mine_chain_restart_save(), regression_1640_r3_keep_theirs_chain_restart_save(), regression_1640_r3_native_append_queues_dirty_partial() (+16 more)

### Community 15 - "Community 15"
Cohesion: 0.12
Nodes (17): file_provider_domain_user_enabled(), finder_domain_user_enabled(), buffer_to_string(), call_bridge(), decide_install_step(), domain_exists(), domain_user_enabled(), domain_user_enabled_state() (+9 more)

### Community 16 - "Community 16"
Cohesion: 0.14
Nodes (18): ancestors(), classifyJsxUsage(), comparesToErrorPhase(), isCorrectableInput(), isErrorOrigin(), isInsideUseEffect(), isLoadPath(), isRetryCallback() (+10 more)

### Community 17 - "Community 17"
Cohesion: 0.19
Nodes (12): CallbackGate, CallbackGate<T>, CallbackLease, CallbackLease<T>, drain_waits_for_transfer_and_releases_credentials(), Generation, LeaseState, revoke_denies_new_work_and_cancels_existing_work() (+4 more)

### Community 18 - "Community 18"
Cohesion: 0.24
Nodes (19): archive_legacy_state_dir_if_present(), beebeeb_state_dir_from_app_local_data(), copy_dir_all(), copy_dir_contents_no_clobber(), copy_file(), fresh_sync_root_uses_app_local_state_dir(), init_from_app(), migrate_legacy_state_dir() (+11 more)

### Community 19 - "Community 19"
Cohesion: 0.18
Nodes (15): remove_linux_freedesktop_thumbnails_for_evicted_files(), encode_freedesktop_png(), file_uri(), fixture_png(), FreedesktopThumbnailSize, md5_hex(), png_output_is_rgba_resized_and_embeds_uri_and_mtime_text_chunks(), remove_freedesktop_thumbnails() (+7 more)

### Community 20 - "Community 20"
Cohesion: 0.16
Nodes (11): auto_resolution_deadline(), ConflictRecord, is_conflict(), Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed() (+3 more)

### Community 21 - "Community 21"
Cohesion: 0.22
Nodes (7): Contract, Fixture, purge(), StateDb, windows_signout_hydrated_sentinel_is_not_external_cache(), windows_signout_rejects_external_cache_before_deleting_placeholders(), windows_signout_retries_after_partial_cleanup_with_hydrated_sentinel()

### Community 22 - "Community 22"
Cohesion: 0.34
Nodes (13): commandForTauriCli(), decodeMinisignBase64Line(), decodeTauriBase64Text(), fail(), generateKeypair(), main(), parseTauriPubkey(), parseTauriSignature() (+5 more)

### Community 23 - "Community 23"
Cohesion: 0.15
Nodes (11): Availability, capability_wire_fixtures_cover_hosts_and_packages(), DesktopCapabilities, FilesystemIntegration, host_os(), HostOs, InstallFormat, SharingLevel (+3 more)

### Community 27 - "Community 27"
Cohesion: 0.29
Nodes (5): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice()

### Community 41 - "Community 41"
Cohesion: 0.39
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 49 - "Community 49"
Cohesion: 0.43
Nodes (4): body(), cloud_files_unregister_precedes_shell_unregister_and_propagates_failure(), every_credential_command_has_admission_before_credential_reads(), every_engine_start_requires_transition_and_generation_validation()

### Community 50 - "Community 50"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 53 - "Community 53"
Cohesion: 0.33
Nodes (1): ApiClient

### Community 54 - "Community 54"
Cohesion: 0.4
Nodes (2): localDayIndex(), planRenewalCopy()

### Community 55 - "Community 55"
Cohesion: 0.47
Nodes (4): availableSettingsSections(), settingLabel(), shellIntegrationLabel(), withPlatformLabels()

### Community 57 - "Community 57"
Cohesion: 0.5
Nodes (2): findColorViolations(), toKebabCase()

### Community 60 - "Community 60"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

### Community 62 - "Community 62"
Cohesion: 0.7
Nodes (3): consumeStoredNav(), initialNav(), navIdFromString()

### Community 79 - "Community 79"
Cohesion: 0.67
Nodes (1): StatusUiClassFactory_Impl

### Community 90 - "Community 90"
Cohesion: 1.0
Nodes (1): StatusUiSourceFactory_Impl

## Knowledge Gaps
- **178 isolated node(s):** `namespace`, `folder`, `file`, `myFiles`, `sharedWithMe` (+173 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 53`** (6 nodes): `ApiClient`, `.delete_shard()`, `.fetch_manifest()`, `.get_shard()`, `.master_key_bytes()`, `.put_shard()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 54`** (6 nodes): `localDayIndex()`, `planRenewalCopy()`, `planStatusTone()`, `quotaPercent()`, `titleCasePlan()`, `planPresentation.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 57`** (5 nodes): `findColorViolations()`, `isCommentLine()`, `listSourceFiles()`, `toKebabCase()`, `noHardcodedThemeColors.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 60`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 79`** (3 nodes): `StatusUiClassFactory_Impl`, `.CreateInstance()`, `.LockServer()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 90`** (2 nodes): `StatusUiSourceFactory_Impl`, `.GetStatusUISource()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `load()` connect `Community 0` to `Community 11`, `Community 1`, `Community 2`, `Community 10`?**
  _High betweenness centrality (0.120) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 2` to `Community 0`, `Community 9`?**
  _High betweenness centrality (0.108) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 1`, `Community 4`, `Community 6`, `Community 7`, `Community 9`, `Community 18`?**
  _High betweenness centrality (0.060) - this node is a cross-community bridge._
- **Are the 69 inferred relationships involving `load()` (e.g. with `commandUnavailableLabel()` and `.do_upload_version()`) actually correct?**
  _`load()` has 69 INFERRED edges - model-reasoned connections that need verification._
- **Are the 69 inferred relationships involving `load()` (e.g. with `.upload_session_body()` and `.enforce_configured_cache_limit()`) actually correct?**
  _`load()` has 69 INFERRED edges - model-reasoned connections that need verification._
- **What connects `namespace`, `folder`, `file` to the rest of the system?**
  _178 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.01 - nodes in this community are weakly interconnected._