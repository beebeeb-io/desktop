# Graph Report - desktop  (2026-07-25)

## Corpus Check
- 129 files · ~311,392 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 2166 nodes · 4452 edges · 29 communities detected
- Extraction: 84% EXTRACTED · 16% INFERRED · 0% AMBIGUOUS · INFERRED: 696 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 35|Community 35]]
- [[_COMMUNITY_Community 36|Community 36]]
- [[_COMMUNITY_Community 37|Community 37]]
- [[_COMMUNITY_Community 45|Community 45]]
- [[_COMMUNITY_Community 46|Community 46]]
- [[_COMMUNITY_Community 49|Community 49]]
- [[_COMMUNITY_Community 53|Community 53]]
- [[_COMMUNITY_Community 55|Community 55]]
- [[_COMMUNITY_Community 71|Community 71]]
- [[_COMMUNITY_Community 76|Community 76]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 74 edges
2. `load()` - 67 edges
3. `EngineBridge` - 52 edges
4. `StateDb` - 50 edges
5. `command()` - 45 edges
6. `load()` - 36 edges
7. `run()` - 30 edges
8. `test_bridge_with_api()` - 29 edges
9. `commandUnavailableLabel()` - 25 edges
10. `api_client_from_session()` - 24 edges

## Surprising Connections (you probably didn't know these)
- `linux_thumbnail_source_path_for_entry()` --calls--> `load()`  [INFERRED]
  src-tauri/src/engine_bridge.rs → src/windows/views/ActivityView.tsx
- `get_known_folder_onboarding_seen()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/windows/views/ActivityView.tsx
- `mark_known_folder_onboarding_seen()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/windows/views/ActivityView.tsx
- `main()` --calls--> `run()`  [INFERRED]
  windows/src/main.rs → src-tauri/src/runner.rs
- `handle_connection()` --calls--> `load()`  [INFERRED]
  src-tauri/src/ipc_socket.rs → src/windows/views/ActivityView.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.02
Nodes (277): DomainControlTool, load(), ensure_directory(), platform_keychain_store_for(), about_metadata(), account_activity(), account_activity_feed(), account_client_sessions() (+269 more)

### Community 1 - "Community 1"
Cohesion: 0.02
Nodes (176): is_conflict(), is_text_file(), apply_metadata_file_row(), apply_shared_context(), apply_shared_metadata_file_row(), apply_snapshot(), apply_sync_op(), audit_1244_apply_sync_op_trash_echo_preserves_local_trashing_owner() (+168 more)

### Community 2 - "Community 2"
Cohesion: 0.02
Nodes (116): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), toggleFolder(), openRoot() (+108 more)

### Community 3 - "Community 3"
Cohesion: 0.04
Nodes (42): AndroidKeyboard(), IOSKeyboard(), account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S> (+34 more)

### Community 4 - "Community 4"
Cohesion: 0.03
Nodes (64): bandwidth_samples_insert_and_history(), bandwidth_samples_prune(), BandwidthSample, count_queue_groups(), decode_os_state(), decode_os_state_status_only_for_user_owned_states(), delete_file_subtree_of_a_plain_file_removes_only_that_row(), delete_file_subtree_prunes_descendants_when_folder_row_has_leading_slash() (+56 more)

### Community 5 - "Community 5"
Cohesion: 0.03
Nodes (97): account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity, AccountActivityEvent, AccountActivitySummary, AccountProfile (+89 more)

### Community 6 - "Community 6"
Cohesion: 0.04
Nodes (20): ApiClient, CreatedSession, DesktopUploadInitRequest, DesktopUploadInitResponse, header_secs(), header_secs_parses_numeric_and_rejects_dates(), HeartbeatBody, ListFilesScope (+12 more)

### Community 7 - "Community 7"
Cohesion: 0.04
Nodes (72): engine_internal_filters_state_dir_and_lock(), relative_db_path_is_slash_joined_without_leading_slash(), build_macos_file_provider_bridge(), main(), db_placeholder_path(), decide_size_action(), fail_transfer(), fetch_data_callback() (+64 more)

### Community 8 - "Community 8"
Cohesion: 0.04
Nodes (57): AccountConfig, AccountId, AccountRuntime, active_account_after_synthesis_is_default_shape(), active_account_empty_registry_errs(), fixed_id(), synthesize_is_idempotent(), synthesize_single_account() (+49 more)

### Community 9 - "Community 9"
Cohesion: 0.04
Nodes (30): FileProviderEnumerator, FileProviderExtension, BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts (+22 more)

### Community 10 - "Community 10"
Cohesion: 0.06
Nodes (30): backup_dest_root(), backup_source_key_for_rel_path(), backup_source_key_for_this_device(), classify_copy(), classify_skip_when_size_and_mtime_match(), copy_preserving_mtime(), CopyDecision, dest_root_builds_backup_device_folder() (+22 more)

### Community 11 - "Community 11"
Cohesion: 0.08
Nodes (33): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), Session, Args, b64() (+25 more)

### Community 12 - "Community 12"
Cohesion: 0.09
Nodes (36): is_ignored_finder_name(), path_is_engine_internal(), relative_db_path(), assert_uploading_row(), debounce_loop(), dispatch_local_create(), engine_delete_suppress(), engine_delete_suppression_distinguishes_paths() (+28 more)

### Community 13 - "Community 13"
Cohesion: 0.11
Nodes (26): api_client_search_shard_urls_match_web_paths(), build_local_index(), compare_records_for_query(), DesktopSearchIndexState, DesktopSearchResponse, DesktopSearchResult, FixtureSearchStore, FixtureState (+18 more)

### Community 14 - "Community 14"
Cohesion: 0.11
Nodes (19): decrypt_payload(), device_code_recv_errors_immediately_on_closed_connection(), device_code_recv_returns_promptly_on_healthy_connection(), device_code_recv_times_out_when_server_stalls_after_connect(), emit(), HealthyStream, open_browser(), recv_text() (+11 more)

### Community 15 - "Community 15"
Cohesion: 0.17
Nodes (20): capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path(), FileProviderItemPayload, handle_connection() (+12 more)

### Community 16 - "Community 16"
Cohesion: 0.18
Nodes (15): remove_linux_freedesktop_thumbnails_for_evicted_files(), encode_freedesktop_png(), file_uri(), fixture_png(), FreedesktopThumbnailSize, md5_hex(), png_output_is_rgba_resized_and_embeds_uri_and_mtime_text_chunks(), remove_freedesktop_thumbnails() (+7 more)

### Community 17 - "Community 17"
Cohesion: 0.17
Nodes (10): auto_resolution_deadline(), ConflictRecord, Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed(), v() (+2 more)

### Community 18 - "Community 18"
Cohesion: 0.34
Nodes (13): commandForTauriCli(), decodeMinisignBase64Line(), decodeTauriBase64Text(), fail(), generateKeypair(), main(), parseTauriPubkey(), parseTauriSignature() (+5 more)

### Community 35 - "Community 35"
Cohesion: 0.39
Nodes (7): file_provider_visible_location(), buffer_to_string(), call_bridge(), install(), remove(), status(), visible_url()

### Community 36 - "Community 36"
Cohesion: 0.32
Nodes (4): handleConfirm(), parseDate(), relativeFuture(), shortDate()

### Community 37 - "Community 37"
Cohesion: 0.39
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 45 - "Community 45"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 46 - "Community 46"
Cohesion: 0.33
Nodes (2): hasExcludedDescendant(), triStateFor()

### Community 49 - "Community 49"
Cohesion: 0.33
Nodes (1): ApiClient

### Community 53 - "Community 53"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

### Community 55 - "Community 55"
Cohesion: 0.7
Nodes (3): consumeStoredNav(), initialNav(), navIdFromString()

### Community 71 - "Community 71"
Cohesion: 0.67
Nodes (1): StatusUiClassFactory_Impl

### Community 76 - "Community 76"
Cohesion: 1.0
Nodes (1): StatusUiSourceFactory_Impl

## Knowledge Gaps
- **140 isolated node(s):** `namespace`, `folder`, `file`, `myFiles`, `sharedWithMe` (+135 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 46`** (7 nodes): `SelectiveSyncView.tsx`, `collectExcludedIds()`, `computeTotals()`, `hasExcludedDescendant()`, `setsEqual()`, `TriCheckbox()`, `triStateFor()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 49`** (6 nodes): `ApiClient`, `.delete_shard()`, `.fetch_manifest()`, `.get_shard()`, `.master_key_bytes()`, `.put_shard()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 53`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 71`** (3 nodes): `StatusUiClassFactory_Impl`, `.CreateInstance()`, `.LockServer()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 76`** (2 nodes): `StatusUiSourceFactory_Impl`, `.GetStatusUISource()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `run()` connect `Community 0` to `Community 1`, `Community 3`, `Community 5`, `Community 8`, `Community 9`, `Community 11`?**
  _High betweenness centrality (0.102) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 2` to `Community 0`, `Community 9`?**
  _High betweenness centrality (0.093) - this node is a cross-community bridge._
- **Why does `load()` connect `Community 0` to `Community 2`?**
  _High betweenness centrality (0.092) - this node is a cross-community bridge._
- **Are the 66 inferred relationships involving `load()` (e.g. with `.do_upload_version()` and `.enforce_configured_cache_limit()`) actually correct?**
  _`load()` has 66 INFERRED edges - model-reasoned connections that need verification._
- **What connects `namespace`, `folder`, `file` to the rest of the system?**
  _140 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.02 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.02 - nodes in this community are weakly interconnected._