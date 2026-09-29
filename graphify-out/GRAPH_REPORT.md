# Graph Report - desktop-1646  (2026-09-30)

## Corpus Check
- 161 files · ~376,933 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 2493 nodes · 5215 edges · 30 communities detected
- Extraction: 84% EXTRACTED · 16% INFERRED · 0% AMBIGUOUS · INFERRED: 820 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 30|Community 30]]
- [[_COMMUNITY_Community 36|Community 36]]
- [[_COMMUNITY_Community 44|Community 44]]
- [[_COMMUNITY_Community 47|Community 47]]
- [[_COMMUNITY_Community 48|Community 48]]
- [[_COMMUNITY_Community 49|Community 49]]
- [[_COMMUNITY_Community 51|Community 51]]
- [[_COMMUNITY_Community 54|Community 54]]
- [[_COMMUNITY_Community 56|Community 56]]
- [[_COMMUNITY_Community 73|Community 73]]
- [[_COMMUNITY_Community 85|Community 85]]

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
10. `commandUnavailableLabel()` - 25 edges

## Surprising Connections (you probably didn't know these)
- `walk()` --calls--> `visit()`  [INFERRED]
  eslint-rules/no-ad-hoc-error-surface.mjs → tests/syncRoot.test.ts
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
Nodes (334): load(), ensure_directory(), ipc_hydrate_roundtrip(), ipc_socket_file_is_chmod_0600_after_bind(), linux_thumbnail_source_path_for_entry(), serve_ipc(), serve_ipc_at(), legacy_platform_keychain_store() (+326 more)

### Community 1 - "Community 1"
Cohesion: 0.02
Nodes (218): is_text_file(), apply_metadata_file_row(), apply_shared_context(), apply_shared_metadata_file_row(), apply_snapshot(), apply_sync_op(), assert_rename_then_update_in_one_tick_applies_update(), audit_1244_apply_sync_op_trash_echo_preserves_local_trashing_owner() (+210 more)

### Community 2 - "Community 2"
Cohesion: 0.02
Nodes (136): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), toggleFolder(), openRoot() (+128 more)

### Community 3 - "Community 3"
Cohesion: 0.03
Nodes (69): bandwidth_samples_insert_and_history(), bandwidth_samples_prune(), BandwidthSample, count_queue_groups(), decode_os_state(), decode_os_state_status_only_for_user_owned_states(), delete_file_subtree_of_a_plain_file_removes_only_that_row(), delete_file_subtree_prunes_descendants_when_folder_row_has_leading_slash() (+61 more)

### Community 4 - "Community 4"
Cohesion: 0.03
Nodes (108): check(), main(), self_test(), account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity (+100 more)

### Community 5 - "Community 5"
Cohesion: 0.04
Nodes (40): AndroidKeyboard(), IOSKeyboard(), account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S> (+32 more)

### Community 6 - "Community 6"
Cohesion: 0.03
Nodes (35): api_client_list_files_sends_provenance_headers_on_the_wire(), api_client_zeroizes_master_key_on_drop(), ApiClient, CreatedSession, DesktopUploadInitRequest, DesktopUploadInitResponse, header_secs(), header_secs_parses_numeric_and_rejects_dates() (+27 more)

### Community 7 - "Community 7"
Cohesion: 0.03
Nodes (87): engine_internal_filters_state_dir_and_lock(), relative_db_path_is_slash_joined_without_leading_slash(), remove_linux_freedesktop_thumbnails_for_evicted_files(), encode_freedesktop_png(), file_uri(), fixture_png(), FreedesktopThumbnailSize, md5_hex() (+79 more)

### Community 8 - "Community 8"
Cohesion: 0.03
Nodes (74): AccountConfig, AccountId, AccountRuntime, active_account_after_synthesis_is_default_shape(), active_account_empty_registry_errs(), fixed_id(), synthesize_is_idempotent(), synthesize_single_account() (+66 more)

### Community 9 - "Community 9"
Cohesion: 0.03
Nodes (56): FileProviderEnumerator, FileProviderExtension, BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts (+48 more)

### Community 10 - "Community 10"
Cohesion: 0.06
Nodes (30): backup_dest_root(), backup_source_key_for_rel_path(), backup_source_key_for_this_device(), classify_copy(), classify_skip_when_size_and_mtime_match(), copy_preserving_mtime(), CopyDecision, dest_root_builds_backup_device_folder() (+22 more)

### Community 11 - "Community 11"
Cohesion: 0.09
Nodes (32): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), Session, Args, b64() (+24 more)

### Community 12 - "Community 12"
Cohesion: 0.11
Nodes (26): api_client_search_shard_urls_match_web_paths(), build_local_index(), compare_records_for_query(), DesktopSearchIndexState, DesktopSearchResponse, DesktopSearchResult, FixtureSearchStore, FixtureState (+18 more)

### Community 13 - "Community 13"
Cohesion: 0.1
Nodes (35): bind_ipc_listener(), capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path(), FileProviderItemPayload (+27 more)

### Community 14 - "Community 14"
Cohesion: 0.1
Nodes (32): is_ignored_finder_name(), path_is_engine_internal(), relative_db_path(), assert_uploading_row(), debounce_loop(), dispatch_local_create(), engine_delete_suppress(), engine_delete_suppression_distinguishes_paths() (+24 more)

### Community 15 - "Community 15"
Cohesion: 0.11
Nodes (20): build_ws_request(), decrypt_payload(), device_code_recv_errors_immediately_on_closed_connection(), device_code_recv_returns_promptly_on_healthy_connection(), device_code_recv_times_out_when_server_stalls_after_connect(), emit(), HealthyStream, open_browser() (+12 more)

### Community 16 - "Community 16"
Cohesion: 0.12
Nodes (17): file_provider_domain_user_enabled(), finder_domain_user_enabled(), buffer_to_string(), call_bridge(), decide_install_step(), domain_exists(), domain_user_enabled(), domain_user_enabled_state() (+9 more)

### Community 17 - "Community 17"
Cohesion: 0.34
Nodes (13): commandForTauriCli(), decodeMinisignBase64Line(), decodeTauriBase64Text(), fail(), generateKeypair(), main(), parseTauriPubkey(), parseTauriSignature() (+5 more)

### Community 18 - "Community 18"
Cohesion: 0.15
Nodes (11): Availability, capability_wire_fixtures_cover_hosts_and_packages(), DesktopCapabilities, FilesystemIntegration, host_os(), HostOs, InstallFormat, SharingLevel (+3 more)

### Community 30 - "Community 30"
Cohesion: 0.33
Nodes (3): DomainControlTool, file_provider_installed(), install_file_provider_domain()

### Community 36 - "Community 36"
Cohesion: 0.39
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 44 - "Community 44"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 47 - "Community 47"
Cohesion: 0.33
Nodes (1): ApiClient

### Community 48 - "Community 48"
Cohesion: 0.4
Nodes (2): localDayIndex(), planRenewalCopy()

### Community 49 - "Community 49"
Cohesion: 0.47
Nodes (4): availableSettingsSections(), settingLabel(), shellIntegrationLabel(), withPlatformLabels()

### Community 51 - "Community 51"
Cohesion: 0.5
Nodes (2): findColorViolations(), toKebabCase()

### Community 54 - "Community 54"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

### Community 56 - "Community 56"
Cohesion: 0.7
Nodes (3): consumeStoredNav(), initialNav(), navIdFromString()

### Community 73 - "Community 73"
Cohesion: 0.67
Nodes (1): StatusUiClassFactory_Impl

### Community 85 - "Community 85"
Cohesion: 1.0
Nodes (1): StatusUiSourceFactory_Impl

## Knowledge Gaps
- **169 isolated node(s):** `namespace`, `folder`, `file`, `myFiles`, `sharedWithMe` (+164 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 47`** (6 nodes): `ApiClient`, `.delete_shard()`, `.fetch_manifest()`, `.get_shard()`, `.master_key_bytes()`, `.put_shard()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 48`** (6 nodes): `localDayIndex()`, `planRenewalCopy()`, `planStatusTone()`, `quotaPercent()`, `titleCasePlan()`, `planPresentation.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 51`** (5 nodes): `findColorViolations()`, `isCommentLine()`, `listSourceFiles()`, `toKebabCase()`, `noHardcodedThemeColors.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 54`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 73`** (3 nodes): `StatusUiClassFactory_Impl`, `.CreateInstance()`, `.LockServer()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 85`** (2 nodes): `StatusUiSourceFactory_Impl`, `.GetStatusUISource()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `load()` connect `Community 0` to `Community 1`, `Community 2`, `Community 13`?**
  _High betweenness centrality (0.163) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 2` to `Community 0`, `Community 9`?**
  _High betweenness centrality (0.152) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 1`, `Community 4`, `Community 5`, `Community 8`, `Community 9`, `Community 11`?**
  _High betweenness centrality (0.068) - this node is a cross-community bridge._
- **Are the 69 inferred relationships involving `load()` (e.g. with `commandUnavailableLabel()` and `.do_upload_version()`) actually correct?**
  _`load()` has 69 INFERRED edges - model-reasoned connections that need verification._
- **Are the 69 inferred relationships involving `load()` (e.g. with `.upload_session_body()` and `.enforce_configured_cache_limit()`) actually correct?**
  _`load()` has 69 INFERRED edges - model-reasoned connections that need verification._
- **What connects `namespace`, `folder`, `file` to the rest of the system?**
  _169 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.01 - nodes in this community are weakly interconnected._