# Graph Report - desktop-1664  (2026-09-30)

## Corpus Check
- 168 files · ~380,548 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 2527 nodes · 5262 edges · 31 communities detected
- Extraction: 84% EXTRACTED · 16% INFERRED · 0% AMBIGUOUS · INFERRED: 828 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 36|Community 36]]
- [[_COMMUNITY_Community 37|Community 37]]
- [[_COMMUNITY_Community 45|Community 45]]
- [[_COMMUNITY_Community 48|Community 48]]
- [[_COMMUNITY_Community 49|Community 49]]
- [[_COMMUNITY_Community 50|Community 50]]
- [[_COMMUNITY_Community 52|Community 52]]
- [[_COMMUNITY_Community 55|Community 55]]
- [[_COMMUNITY_Community 57|Community 57]]
- [[_COMMUNITY_Community 74|Community 74]]
- [[_COMMUNITY_Community 87|Community 87]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 76 edges
2. `load()` - 70 edges
3. `load()` - 70 edges
4. `EngineBridge` - 62 edges
5. `StateDb` - 56 edges
6. `command()` - 47 edges
7. `test_bridge_with_api()` - 41 edges
8. `run()` - 30 edges
9. `api_client_from_session()` - 25 edges
10. `commandUnavailableLabel()` - 24 edges

## Surprising Connections (you probably didn't know these)
- `visit()` --calls--> `walk()`  [INFERRED]
  tests/syncRoot.test.ts → eslint-rules/no-ad-hoc-error-surface.mjs
- `main()` --calls--> `run()`  [INFERRED]
  windows/src/main.rs → src-tauri/src/runner.rs
- `handle_connection()` --calls--> `load()`  [INFERRED]
  src-tauri/src/ipc_socket.rs → src/windows/views/ActivityView.tsx
- `handle_connection()` --calls--> `load()`  [INFERRED]
  src-tauri/src/ipc_socket.rs → src/pages/SelectiveSync.tsx
- `emit()` --calls--> `openActivity()`  [INFERRED]
  src-tauri/src/browser_login.rs → src/WindowsTray.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.01
Nodes (338): DomainControlTool, load(), provenance_headers(), ensure_directory(), linux_thumbnail_source_path_for_entry(), legacy_platform_keychain_store(), migrate_legacy_keychain_to_account(), platform_keychain_store_for() (+330 more)

### Community 1 - "Community 1"
Cohesion: 0.02
Nodes (216): is_text_file(), apply_metadata_file_row(), apply_shared_context(), apply_shared_metadata_file_row(), apply_snapshot(), apply_sync_op(), assert_rename_then_update_in_one_tick_applies_update(), audit_1244_apply_sync_op_trash_echo_preserves_local_trashing_owner() (+208 more)

### Community 2 - "Community 2"
Cohesion: 0.02
Nodes (114): toggleFolder(), openRoot(), formatBytes(), openSetup(), refresh(), chooseFolderClick(), installFinder(), openFinder() (+106 more)

### Community 3 - "Community 3"
Cohesion: 0.02
Nodes (113): check(), main(), self_test(), account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity (+105 more)

### Community 4 - "Community 4"
Cohesion: 0.03
Nodes (69): tray_recent_activity_from_db_returns_more_than_eight_rows(), bandwidth_samples_insert_and_history(), bandwidth_samples_prune(), BandwidthSample, count_queue_groups(), decode_os_state(), decode_os_state_status_only_for_user_owned_states(), delete_file_subtree_of_a_plain_file_removes_only_that_row() (+61 more)

### Community 5 - "Community 5"
Cohesion: 0.03
Nodes (43): api_client_list_files_sends_provenance_headers_on_the_wire(), api_client_zeroizes_master_key_on_drop(), ApiClient, CreatedSession, DesktopUploadInitRequest, DesktopUploadInitResponse, header_secs(), header_secs_parses_numeric_and_rejects_dates() (+35 more)

### Community 6 - "Community 6"
Cohesion: 0.04
Nodes (40): AndroidKeyboard(), IOSKeyboard(), account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S> (+32 more)

### Community 7 - "Community 7"
Cohesion: 0.03
Nodes (91): DesktopSettings, engine_internal_filters_state_dir_and_lock(), relative_db_path_is_slash_joined_without_leading_slash(), encode_freedesktop_png(), file_uri(), fixture_png(), FreedesktopThumbnailSize, md5_hex() (+83 more)

### Community 8 - "Community 8"
Cohesion: 0.04
Nodes (53): AccountConfig, AccountId, AccountRuntime, active_account_after_synthesis_is_default_shape(), active_account_empty_registry_errs(), fixed_id(), synthesize_is_idempotent(), synthesize_single_account() (+45 more)

### Community 9 - "Community 9"
Cohesion: 0.04
Nodes (51): auto_resolution_deadline(), ConflictRecord, is_conflict(), Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed() (+43 more)

### Community 10 - "Community 10"
Cohesion: 0.04
Nodes (43): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), initialPage(), refresh() (+35 more)

### Community 11 - "Community 11"
Cohesion: 0.04
Nodes (30): FileProviderEnumerator, FileProviderExtension, BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts (+22 more)

### Community 12 - "Community 12"
Cohesion: 0.06
Nodes (30): ancestors(), classifyJsxUsage(), comparesToErrorPhase(), isCorrectableInput(), isErrorOrigin(), isInsideUseEffect(), isLoadPath(), isRetryCallback() (+22 more)

### Community 13 - "Community 13"
Cohesion: 0.09
Nodes (40): ipc_hydrate_roundtrip(), bind_ipc_listener(), capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path() (+32 more)

### Community 14 - "Community 14"
Cohesion: 0.11
Nodes (26): api_client_search_shard_urls_match_web_paths(), build_local_index(), compare_records_for_query(), DesktopSearchIndexState, DesktopSearchResponse, DesktopSearchResult, FixtureSearchStore, FixtureState (+18 more)

### Community 15 - "Community 15"
Cohesion: 0.12
Nodes (29): is_ignored_finder_name(), path_is_engine_internal(), relative_db_path(), assert_uploading_row(), debounce_loop(), dispatch_local_create(), engine_delete_suppress(), engine_delete_suppression_distinguishes_paths() (+21 more)

### Community 16 - "Community 16"
Cohesion: 0.13
Nodes (23): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), archive_legacy_state_dir_if_present(), beebeeb_state_dir_from_app_local_data(), copy_dir_all() (+15 more)

### Community 17 - "Community 17"
Cohesion: 0.12
Nodes (18): file_provider_domain_user_enabled(), finder_domain_user_enabled(), buffer_to_string(), call_bridge(), decide_install_step(), domain_exists(), domain_user_enabled(), domain_user_enabled_state() (+10 more)

### Community 18 - "Community 18"
Cohesion: 0.34
Nodes (13): commandForTauriCli(), decodeMinisignBase64Line(), decodeTauriBase64Text(), fail(), generateKeypair(), main(), parseTauriPubkey(), parseTauriSignature() (+5 more)

### Community 19 - "Community 19"
Cohesion: 0.15
Nodes (11): Availability, capability_wire_fixtures_cover_hosts_and_packages(), DesktopCapabilities, FilesystemIntegration, host_os(), HostOs, InstallFormat, SharingLevel (+3 more)

### Community 36 - "Community 36"
Cohesion: 0.32
Nodes (4): manualUpdateToast(), buildDowngradeConfirmationViewModel(), buildUpdateCheckViewModel(), releaseChannelLabel()

### Community 37 - "Community 37"
Cohesion: 0.39
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 45 - "Community 45"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 48 - "Community 48"
Cohesion: 0.33
Nodes (1): ApiClient

### Community 49 - "Community 49"
Cohesion: 0.4
Nodes (2): localDayIndex(), planRenewalCopy()

### Community 50 - "Community 50"
Cohesion: 0.47
Nodes (4): availableSettingsSections(), settingLabel(), shellIntegrationLabel(), withPlatformLabels()

### Community 52 - "Community 52"
Cohesion: 0.5
Nodes (2): findColorViolations(), toKebabCase()

### Community 55 - "Community 55"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

### Community 57 - "Community 57"
Cohesion: 0.7
Nodes (3): consumeStoredNav(), initialNav(), navIdFromString()

### Community 74 - "Community 74"
Cohesion: 0.67
Nodes (1): StatusUiClassFactory_Impl

### Community 87 - "Community 87"
Cohesion: 1.0
Nodes (1): StatusUiSourceFactory_Impl

## Knowledge Gaps
- **169 isolated node(s):** `namespace`, `folder`, `file`, `myFiles`, `sharedWithMe` (+164 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 48`** (6 nodes): `ApiClient`, `.delete_shard()`, `.fetch_manifest()`, `.get_shard()`, `.master_key_bytes()`, `.put_shard()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 49`** (6 nodes): `localDayIndex()`, `planRenewalCopy()`, `planStatusTone()`, `quotaPercent()`, `titleCasePlan()`, `planPresentation.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 52`** (5 nodes): `findColorViolations()`, `isCommentLine()`, `listSourceFiles()`, `toKebabCase()`, `noHardcodedThemeColors.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 55`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 74`** (3 nodes): `StatusUiClassFactory_Impl`, `.CreateInstance()`, `.LockServer()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 87`** (2 nodes): `StatusUiSourceFactory_Impl`, `.GetStatusUISource()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `load()` connect `Community 0` to `Community 1`, `Community 2`, `Community 13`, `Community 15`?**
  _High betweenness centrality (0.141) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 2` to `Community 0`, `Community 10`, `Community 11`?**
  _High betweenness centrality (0.131) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 1`, `Community 3`, `Community 5`, `Community 6`, `Community 8`, `Community 11`, `Community 16`?**
  _High betweenness centrality (0.066) - this node is a cross-community bridge._
- **Are the 69 inferred relationships involving `load()` (e.g. with `commandUnavailableLabel()` and `.do_upload_version()`) actually correct?**
  _`load()` has 69 INFERRED edges - model-reasoned connections that need verification._
- **Are the 69 inferred relationships involving `load()` (e.g. with `.upload_session_body()` and `.enforce_configured_cache_limit()`) actually correct?**
  _`load()` has 69 INFERRED edges - model-reasoned connections that need verification._
- **What connects `namespace`, `folder`, `file` to the rest of the system?**
  _169 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.01 - nodes in this community are weakly interconnected._