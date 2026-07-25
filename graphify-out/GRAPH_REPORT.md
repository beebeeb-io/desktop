# Graph Report - desktop  (2026-07-25)

## Corpus Check
- 131 files · ~313,619 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 2191 nodes · 4500 edges · 28 communities detected
- Extraction: 84% EXTRACTED · 16% INFERRED · 0% AMBIGUOUS · INFERRED: 700 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 29|Community 29]]
- [[_COMMUNITY_Community 35|Community 35]]
- [[_COMMUNITY_Community 43|Community 43]]
- [[_COMMUNITY_Community 44|Community 44]]
- [[_COMMUNITY_Community 45|Community 45]]
- [[_COMMUNITY_Community 48|Community 48]]
- [[_COMMUNITY_Community 52|Community 52]]
- [[_COMMUNITY_Community 54|Community 54]]
- [[_COMMUNITY_Community 70|Community 70]]
- [[_COMMUNITY_Community 75|Community 75]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 75 edges
2. `load()` - 67 edges
3. `EngineBridge` - 52 edges
4. `StateDb` - 50 edges
5. `command()` - 45 edges
6. `load()` - 36 edges
7. `run()` - 30 edges
8. `test_bridge_with_api()` - 29 edges
9. `api_client_from_session()` - 25 edges
10. `commandUnavailableLabel()` - 25 edges

## Surprising Connections (you probably didn't know these)
- `linux_thumbnail_source_path_for_entry()` --calls--> `load()`  [INFERRED]
  src-tauri/src/engine_bridge.rs → src/windows/views/ActivityView.tsx
- `get_known_folder_onboarding_seen()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/windows/views/ActivityView.tsx
- `mark_known_folder_onboarding_seen()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/windows/views/ActivityView.tsx
- `desktop_search_files()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/windows/views/ActivityView.tsx
- `handle_connection()` --calls--> `load()`  [INFERRED]
  src-tauri/src/ipc_socket.rs → src/windows/views/ActivityView.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.02
Nodes (281): load(), ensure_account_id_mints_persists_and_is_stable_across_relaunch(), ensure_directory(), legacy_platform_keychain_store(), migrate_legacy_keychain_to_account(), platform_keychain_store_for(), about_metadata(), account_activity() (+273 more)

### Community 1 - "Community 1"
Cohesion: 0.02
Nodes (175): apply_metadata_file_row(), apply_shared_context(), apply_shared_metadata_file_row(), apply_snapshot(), apply_sync_op(), audit_1244_apply_sync_op_trash_echo_preserves_local_trashing_owner(), audit_1244_auto_resolve_keep_both_preserves_local_copy_when_remote_hydrate_fails(), audit_1244_double_trash_restore_cycle_keeps_resnapshot_for_final_restore() (+167 more)

### Community 2 - "Community 2"
Cohesion: 0.02
Nodes (123): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), toggleFolder(), openRoot() (+115 more)

### Community 3 - "Community 3"
Cohesion: 0.03
Nodes (46): AndroidKeyboard(), IOSKeyboard(), account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S> (+38 more)

### Community 4 - "Community 4"
Cohesion: 0.03
Nodes (63): tray_recent_activity_from_db_returns_more_than_eight_rows(), bandwidth_samples_insert_and_history(), bandwidth_samples_prune(), BandwidthSample, count_queue_groups(), decode_os_state(), decode_os_state_status_only_for_user_owned_states(), delete_file_subtree_of_a_plain_file_removes_only_that_row() (+55 more)

### Community 5 - "Community 5"
Cohesion: 0.03
Nodes (93): engine_internal_filters_state_dir_and_lock(), linux_thumbnail_source_path_for_entry(), relative_db_path_is_slash_joined_without_leading_slash(), remove_linux_freedesktop_thumbnails_for_evicted_files(), session_zeroizes_master_key_on_drop(), encode_freedesktop_png(), file_uri(), fixture_png() (+85 more)

### Community 6 - "Community 6"
Cohesion: 0.03
Nodes (29): ApiClient, CreatedSession, DesktopUploadInitRequest, DesktopUploadInitResponse, header_secs(), header_secs_parses_numeric_and_rejects_dates(), HeartbeatBody, ListFilesScope (+21 more)

### Community 7 - "Community 7"
Cohesion: 0.03
Nodes (91): account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity, AccountActivityEvent, AccountActivitySummary, AccountProfile (+83 more)

### Community 8 - "Community 8"
Cohesion: 0.04
Nodes (64): AccountConfig, AccountId, AccountRuntime, active_account_after_synthesis_is_default_shape(), active_account_empty_registry_errs(), fixed_id(), synthesize_is_idempotent(), synthesize_single_account() (+56 more)

### Community 9 - "Community 9"
Cohesion: 0.04
Nodes (34): FileProviderEnumerator, FileProviderExtension, BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts (+26 more)

### Community 10 - "Community 10"
Cohesion: 0.06
Nodes (30): backup_dest_root(), backup_source_key_for_rel_path(), backup_source_key_for_this_device(), classify_copy(), classify_skip_when_size_and_mtime_match(), copy_preserving_mtime(), CopyDecision, dest_root_builds_backup_device_folder() (+22 more)

### Community 11 - "Community 11"
Cohesion: 0.1
Nodes (27): api_client_search_shard_urls_match_web_paths(), build_local_index(), compare_records_for_query(), DesktopSearchIndexState, DesktopSearchResponse, DesktopSearchResult, FixtureSearchStore, FixtureState (+19 more)

### Community 12 - "Community 12"
Cohesion: 0.13
Nodes (24): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), archive_legacy_state_dir_if_present(), beebeeb_state_dir_from_app_local_data(), copy_dir_all() (+16 more)

### Community 13 - "Community 13"
Cohesion: 0.12
Nodes (22): ApiClient, parse_response(), capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path() (+14 more)

### Community 14 - "Community 14"
Cohesion: 0.13
Nodes (28): is_ignored_finder_name(), path_is_engine_internal(), relative_db_path(), assert_uploading_row(), dispatch_local_create(), engine_delete_suppress(), engine_delete_suppression_distinguishes_paths(), engine_delete_suppression_is_consumed_once() (+20 more)

### Community 15 - "Community 15"
Cohesion: 0.11
Nodes (18): decrypt_payload(), device_code_recv_errors_immediately_on_closed_connection(), device_code_recv_returns_promptly_on_healthy_connection(), device_code_recv_times_out_when_server_stalls_after_connect(), emit(), HealthyStream, open_browser(), recv_text() (+10 more)

### Community 16 - "Community 16"
Cohesion: 0.15
Nodes (12): auto_resolution_deadline(), ConflictRecord, is_conflict(), is_text_file(), Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed() (+4 more)

### Community 17 - "Community 17"
Cohesion: 0.34
Nodes (13): commandForTauriCli(), decodeMinisignBase64Line(), decodeTauriBase64Text(), fail(), generateKeypair(), main(), parseTauriPubkey(), parseTauriSignature() (+5 more)

### Community 29 - "Community 29"
Cohesion: 0.33
Nodes (3): DomainControlTool, file_provider_installed(), install_file_provider_domain()

### Community 35 - "Community 35"
Cohesion: 0.39
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 43 - "Community 43"
Cohesion: 0.48
Nodes (6): buffer_to_string(), call_bridge(), install(), remove(), status(), visible_url()

### Community 44 - "Community 44"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 45 - "Community 45"
Cohesion: 0.33
Nodes (2): hasExcludedDescendant(), triStateFor()

### Community 48 - "Community 48"
Cohesion: 0.33
Nodes (1): ApiClient

### Community 52 - "Community 52"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

### Community 54 - "Community 54"
Cohesion: 0.7
Nodes (3): consumeStoredNav(), initialNav(), navIdFromString()

### Community 70 - "Community 70"
Cohesion: 0.67
Nodes (1): StatusUiClassFactory_Impl

### Community 75 - "Community 75"
Cohesion: 1.0
Nodes (1): StatusUiSourceFactory_Impl

## Knowledge Gaps
- **144 isolated node(s):** `namespace`, `folder`, `file`, `myFiles`, `sharedWithMe` (+139 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 45`** (7 nodes): `SelectiveSyncView.tsx`, `collectExcludedIds()`, `computeTotals()`, `hasExcludedDescendant()`, `setsEqual()`, `TriCheckbox()`, `triStateFor()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 48`** (6 nodes): `ApiClient`, `.delete_shard()`, `.fetch_manifest()`, `.get_shard()`, `.master_key_bytes()`, `.put_shard()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 52`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 70`** (3 nodes): `StatusUiClassFactory_Impl`, `.CreateInstance()`, `.LockServer()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 75`** (2 nodes): `StatusUiSourceFactory_Impl`, `.GetStatusUISource()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `run()` connect `Community 0` to `Community 1`, `Community 3`, `Community 7`, `Community 8`, `Community 9`, `Community 12`?**
  _High betweenness centrality (0.100) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 2` to `Community 0`, `Community 9`?**
  _High betweenness centrality (0.086) - this node is a cross-community bridge._
- **Why does `load()` connect `Community 0` to `Community 2`?**
  _High betweenness centrality (0.086) - this node is a cross-community bridge._
- **Are the 66 inferred relationships involving `load()` (e.g. with `.do_upload_version()` and `.enforce_configured_cache_limit()`) actually correct?**
  _`load()` has 66 INFERRED edges - model-reasoned connections that need verification._
- **What connects `namespace`, `folder`, `file` to the rest of the system?**
  _144 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.02 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.02 - nodes in this community are weakly interconnected._