# Graph Report - desktop-1612  (2026-09-30)

## Corpus Check
- 176 files · ~389,727 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 2613 nodes · 5491 edges · 36 communities detected
- Extraction: 83% EXTRACTED · 17% INFERRED · 0% AMBIGUOUS · INFERRED: 911 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 34|Community 34]]
- [[_COMMUNITY_Community 40|Community 40]]
- [[_COMMUNITY_Community 41|Community 41]]
- [[_COMMUNITY_Community 49|Community 49]]
- [[_COMMUNITY_Community 52|Community 52]]
- [[_COMMUNITY_Community 53|Community 53]]
- [[_COMMUNITY_Community 54|Community 54]]
- [[_COMMUNITY_Community 56|Community 56]]
- [[_COMMUNITY_Community 59|Community 59]]
- [[_COMMUNITY_Community 61|Community 61]]
- [[_COMMUNITY_Community 79|Community 79]]
- [[_COMMUNITY_Community 82|Community 82]]
- [[_COMMUNITY_Community 93|Community 93]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 76 edges
2. `load()` - 72 edges
3. `load()` - 70 edges
4. `EngineBridge` - 62 edges
5. `StateDb` - 56 edges
6. `command()` - 48 edges
7. `test_bridge_with_api()` - 41 edges
8. `run()` - 30 edges
9. `api_client_from_session()` - 25 edges
10. `spawn()` - 24 edges

## Surprising Connections (you probably didn't know these)
- `corpus_oracle_rejects_wrong_hash_and_missing_scenario()` --calls--> `load()`  [INFERRED]
  src-tauri/tests/support/bridge.rs → src/windows/views/ActivityView.tsx
- `emit()` --calls--> `openActivity()`  [INFERRED]
  src-tauri/src/browser_login.rs → src/WindowsTray.tsx
- `text()` --calls--> `hydration_mock_response()`  [INFERRED]
  tests/recoveryEntry.test.ts → src-tauri/src/engine_bridge.rs
- `text()` --calls--> `parse_release_version()`  [INFERRED]
  tests/recoveryEntry.test.ts → src-tauri/src/lib.rs
- `corpus_size_mismatch_reports_expected_and_actual_bytes_and_hashes()` --calls--> `load()`  [INFERRED]
  src-tauri/tests/support/bridge.rs → src/windows/views/ActivityView.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.01
Nodes (328): load(), ensure_directory(), hydrate_dest_is_allowed(), legacy_platform_keychain_store(), migrate_legacy_keychain_to_account(), platform_keychain_store_for(), about_metadata(), account_activity() (+320 more)

### Community 1 - "Community 1"
Cohesion: 0.02
Nodes (226): is_conflict(), is_text_file(), apply_metadata_file_row(), apply_shared_context(), apply_shared_metadata_file_row(), apply_snapshot(), apply_sync_op(), assert_rename_then_update_in_one_tick_applies_update() (+218 more)

### Community 2 - "Community 2"
Cohesion: 0.02
Nodes (127): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), toggleFolder(), openRoot() (+119 more)

### Community 3 - "Community 3"
Cohesion: 0.02
Nodes (94): AccountConfig, AccountId, AccountRuntime, active_account_after_synthesis_is_default_shape(), active_account_empty_registry_errs(), fixed_id(), synthesize_is_idempotent(), synthesize_single_account() (+86 more)

### Community 4 - "Community 4"
Cohesion: 0.03
Nodes (68): bandwidth_samples_insert_and_history(), bandwidth_samples_prune(), BandwidthSample, decode_os_state(), decode_os_state_status_only_for_user_owned_states(), delete_file_subtree_of_a_plain_file_removes_only_that_row(), delete_file_subtree_prunes_descendants_when_folder_row_has_leading_slash(), delete_file_subtree_removes_folder_and_descendants_children_first() (+60 more)

### Community 5 - "Community 5"
Cohesion: 0.03
Nodes (43): api_client_list_files_sends_provenance_headers_on_the_wire(), api_client_zeroizes_master_key_on_drop(), ApiClient, CreatedSession, DesktopUploadInitRequest, DesktopUploadInitResponse, header_secs(), header_secs_parses_numeric_and_rejects_dates() (+35 more)

### Community 6 - "Community 6"
Cohesion: 0.03
Nodes (102): account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity, AccountActivityEvent, AccountActivitySummary, AccountProfile (+94 more)

### Community 7 - "Community 7"
Cohesion: 0.05
Nodes (38): account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S>, clear_session_removes_account_email(), delete() (+30 more)

### Community 8 - "Community 8"
Cohesion: 0.03
Nodes (79): engine_internal_filters_state_dir_and_lock(), relative_db_path_is_slash_joined_without_leading_slash(), build_macos_file_provider_bridge(), main(), db_placeholder_path(), decide_size_action(), fail_transfer(), fetch_data_callback() (+71 more)

### Community 9 - "Community 9"
Cohesion: 0.05
Nodes (42): AndroidKeyboard(), IOSKeyboard(), check(), main(), self_test(), assert_clean(), assert_corpus_endpoint_consistency(), assert_same_metadata() (+34 more)

### Community 10 - "Community 10"
Cohesion: 0.04
Nodes (31): FileProviderEnumerator, FileProviderExtension, BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts (+23 more)

### Community 11 - "Community 11"
Cohesion: 0.07
Nodes (49): bind_ipc_listener(), bind_ipc_listener_with(), capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path() (+41 more)

### Community 12 - "Community 12"
Cohesion: 0.11
Nodes (26): api_client_search_shard_urls_match_web_paths(), build_local_index(), compare_records_for_query(), DesktopSearchIndexState, DesktopSearchResponse, DesktopSearchResult, FixtureSearchStore, FixtureState (+18 more)

### Community 13 - "Community 13"
Cohesion: 0.08
Nodes (27): ancestors(), classifyJsxUsage(), comparesToErrorPhase(), isCorrectableInput(), isErrorOrigin(), isInsideUseEffect(), isLoadPath(), isRetryCallback() (+19 more)

### Community 14 - "Community 14"
Cohesion: 0.09
Nodes (23): build_ws_request(), decrypt_payload(), device_code_recv_errors_immediately_on_closed_connection(), device_code_recv_returns_promptly_on_healthy_connection(), device_code_recv_times_out_when_server_stalls_after_connect(), emit(), HealthyStream, open_browser() (+15 more)

### Community 15 - "Community 15"
Cohesion: 0.12
Nodes (29): is_ignored_finder_name(), path_is_engine_internal(), relative_db_path(), assert_uploading_row(), debounce_loop(), dispatch_local_create(), engine_delete_suppress(), engine_delete_suppression_distinguishes_paths() (+21 more)

### Community 16 - "Community 16"
Cohesion: 0.13
Nodes (24): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), archive_legacy_state_dir_if_present(), beebeeb_state_dir_from_app_local_data(), copy_dir_all() (+16 more)

### Community 17 - "Community 17"
Cohesion: 0.12
Nodes (18): file_provider_domain_user_enabled(), finder_domain_user_enabled(), buffer_to_string(), call_bridge(), decide_install_step(), domain_exists(), domain_user_enabled(), domain_user_enabled_state() (+10 more)

### Community 18 - "Community 18"
Cohesion: 0.18
Nodes (15): remove_linux_freedesktop_thumbnails_for_evicted_files(), encode_freedesktop_png(), file_uri(), fixture_png(), FreedesktopThumbnailSize, md5_hex(), png_output_is_rgba_resized_and_embeds_uri_and_mtime_text_chunks(), remove_freedesktop_thumbnails() (+7 more)

### Community 19 - "Community 19"
Cohesion: 0.12
Nodes (10): initialPage(), refresh(), renderPage(), supportsRoute(), useCapabilities(), compactPageFromString(), platformFromQueryParam(), resolvePlatformOnce() (+2 more)

### Community 20 - "Community 20"
Cohesion: 0.17
Nodes (10): auto_resolution_deadline(), ConflictRecord, Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed(), v() (+2 more)

### Community 21 - "Community 21"
Cohesion: 0.34
Nodes (13): commandForTauriCli(), decodeMinisignBase64Line(), decodeTauriBase64Text(), fail(), generateKeypair(), main(), parseTauriPubkey(), parseTauriSignature() (+5 more)

### Community 22 - "Community 22"
Cohesion: 0.15
Nodes (11): Availability, capability_wire_fixtures_cover_hosts_and_packages(), DesktopCapabilities, FilesystemIntegration, host_os(), HostOs, InstallFormat, SharingLevel (+3 more)

### Community 34 - "Community 34"
Cohesion: 0.33
Nodes (3): DomainControlTool, file_provider_installed(), install_file_provider_domain()

### Community 40 - "Community 40"
Cohesion: 0.32
Nodes (4): manualUpdateToast(), buildDowngradeConfirmationViewModel(), buildUpdateCheckViewModel(), releaseChannelLabel()

### Community 41 - "Community 41"
Cohesion: 0.39
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 49 - "Community 49"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 52 - "Community 52"
Cohesion: 0.33
Nodes (1): ApiClient

### Community 53 - "Community 53"
Cohesion: 0.4
Nodes (2): localDayIndex(), planRenewalCopy()

### Community 54 - "Community 54"
Cohesion: 0.47
Nodes (4): availableSettingsSections(), settingLabel(), shellIntegrationLabel(), withPlatformLabels()

### Community 56 - "Community 56"
Cohesion: 0.5
Nodes (2): findColorViolations(), toKebabCase()

### Community 59 - "Community 59"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

### Community 61 - "Community 61"
Cohesion: 0.7
Nodes (3): consumeStoredNav(), initialNav(), navIdFromString()

### Community 79 - "Community 79"
Cohesion: 0.67
Nodes (1): StatusUiClassFactory_Impl

### Community 82 - "Community 82"
Cohesion: 1.0
Nodes (2): productRegionLabel(), useRegionLabel()

### Community 93 - "Community 93"
Cohesion: 1.0
Nodes (1): StatusUiSourceFactory_Impl

## Knowledge Gaps
- **175 isolated node(s):** `namespace`, `folder`, `file`, `myFiles`, `sharedWithMe` (+170 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 52`** (6 nodes): `ApiClient`, `.delete_shard()`, `.fetch_manifest()`, `.get_shard()`, `.master_key_bytes()`, `.put_shard()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 53`** (6 nodes): `localDayIndex()`, `planRenewalCopy()`, `planStatusTone()`, `quotaPercent()`, `titleCasePlan()`, `planPresentation.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 56`** (5 nodes): `findColorViolations()`, `isCommentLine()`, `listSourceFiles()`, `toKebabCase()`, `noHardcodedThemeColors.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 59`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 79`** (3 nodes): `StatusUiClassFactory_Impl`, `.CreateInstance()`, `.LockServer()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 82`** (3 nodes): `useRegion.ts`, `productRegionLabel()`, `useRegionLabel()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 93`** (2 nodes): `StatusUiSourceFactory_Impl`, `.GetStatusUISource()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `load()` connect `Community 0` to `Community 1`, `Community 2`, `Community 11`, `Community 15`?**
  _High betweenness centrality (0.086) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 2` to `Community 0`, `Community 10`?**
  _High betweenness centrality (0.080) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 1`, `Community 3`, `Community 6`, `Community 7`, `Community 10`, `Community 16`?**
  _High betweenness centrality (0.072) - this node is a cross-community bridge._
- **Are the 71 inferred relationships involving `load()` (e.g. with `corpus_size_mismatch_reports_expected_and_actual_bytes_and_hashes()` and `corpus_oracle_rejects_wrong_hash_and_missing_scenario()`) actually correct?**
  _`load()` has 71 INFERRED edges - model-reasoned connections that need verification._
- **Are the 69 inferred relationships involving `load()` (e.g. with `commandUnavailableLabel()` and `.do_upload_version()`) actually correct?**
  _`load()` has 69 INFERRED edges - model-reasoned connections that need verification._
- **What connects `namespace`, `folder`, `file` to the rest of the system?**
  _175 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.01 - nodes in this community are weakly interconnected._