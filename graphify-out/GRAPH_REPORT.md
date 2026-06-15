# Graph Report - desktop  (2026-06-15)

## Corpus Check
- 101 files · ~209,840 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1307 nodes · 2344 edges · 25 communities detected
- Extraction: 82% EXTRACTED · 18% INFERRED · 0% AMBIGUOUS · INFERRED: 423 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 20|Community 20]]
- [[_COMMUNITY_Community 28|Community 28]]
- [[_COMMUNITY_Community 43|Community 43]]
- [[_COMMUNITY_Community 44|Community 44]]
- [[_COMMUNITY_Community 45|Community 45]]
- [[_COMMUNITY_Community 46|Community 46]]
- [[_COMMUNITY_Community 49|Community 49]]
- [[_COMMUNITY_Community 50|Community 50]]
- [[_COMMUNITY_Community 54|Community 54]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 53 edges
2. `load()` - 36 edges
3. `load()` - 35 edges
4. `EngineBridge` - 31 edges
5. `StateDb` - 31 edges
6. `open()` - 25 edges
7. `command()` - 23 edges
8. `commandUnavailableLabel()` - 23 edges
9. `run()` - 20 edges
10. `api_client_from_session()` - 17 edges

## Surprising Connections (you probably didn't know these)
- `process_metadata_row()` --calls--> `apply()`  [INFERRED]
  src-tauri/src/engine_bridge.rs → src/windows/views/SelectiveSyncView.tsx
- `test_bridge()` --calls--> `open()`  [INFERRED]
  src-tauri/src/engine_bridge.rs → src/WindowsApp.tsx
- `test_bridge_with_api()` --calls--> `open()`  [INFERRED]
  src-tauri/src/engine_bridge.rs → src/WindowsApp.tsx
- `main()` --calls--> `run()`  [INFERRED]
  windows/src/main.rs → src-tauri/src/runner.rs
- `test_shared_namespace_lists_roots_with_permission_capabilities()` --calls--> `open()`  [INFERRED]
  src-tauri/src/ipc_socket.rs → src/WindowsApp.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.03
Nodes (164): load(), ensure_directory(), apply_metadata_file_row(), test_metadata_file_row_classifies_folder_rows(), test_metadata_file_row_composes_nested_path_under_parent(), test_metadata_file_row_decrypts_encrypted_name_into_path(), test_metadata_file_row_falls_back_when_name_undecryptable(), test_metadata_file_row_persists_contract_for_own_file() (+156 more)

### Community 1 - "Community 1"
Cohesion: 0.02
Nodes (74): account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity, AccountActivityEvent, AccountActivitySummary, AccountProfile (+66 more)

### Community 2 - "Community 2"
Cohesion: 0.04
Nodes (71): is_conflict(), is_text_file(), apply_shared_context(), CacheCleanupOutcome, CachePolicy, classify_operation_error(), classify_review_operation(), ConflictDetected (+63 more)

### Community 3 - "Community 3"
Cohesion: 0.06
Nodes (24): account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S>, clear_session_removes_account_email(), delete() (+16 more)

### Community 4 - "Community 4"
Cohesion: 0.06
Nodes (14): ApiClient, created_session_parses_minimal(), CreatedSession, DesktopUploadInitRequest, DesktopUploadInitResponse, heartbeat_body_skips_none_optionals_but_keeps_zero(), HeartbeatBody, ListFilesScope (+6 more)

### Community 5 - "Community 5"
Cohesion: 0.04
Nodes (41): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), toggleFolder(), formatBytes() (+33 more)

### Community 6 - "Community 6"
Cohesion: 0.07
Nodes (30): BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts, myFiles, offline (+22 more)

### Community 7 - "Community 7"
Cohesion: 0.06
Nodes (30): auto_resolution_deadline(), ConflictRecord, Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed(), v() (+22 more)

### Community 8 - "Community 8"
Cohesion: 0.1
Nodes (31): decide_size_action(), fail_transfer(), fetch_data_callback(), mark_in_sync(), mark_not_in_sync(), normalized_path(), notify_delete_completion_callback(), notify_file_close_completion_callback() (+23 more)

### Community 9 - "Community 9"
Cohesion: 0.12
Nodes (25): accountActivity(), accountActivityFeed(), accountClientSessions(), accountDevices(), accountNotificationPreferences(), accountNotifications(), accountProfile(), accountRevokeOtherSessions() (+17 more)

### Community 10 - "Community 10"
Cohesion: 0.12
Nodes (15): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), read_http_request(), Session, Args (+7 more)

### Community 11 - "Community 11"
Cohesion: 0.15
Nodes (21): is_ignored_finder_name(), path_is_engine_internal(), relative_db_path(), guess_mime_type(), dispatch_local_create(), handle_delete(), handle_rename(), handle_settled_path() (+13 more)

### Community 12 - "Community 12"
Cohesion: 0.13
Nodes (7): FileProviderEnumerator, FileProviderExtension, FileProviderItem, NSFileProviderEnumerator, NSFileProviderItem, NSFileProviderReplicatedExtension, NSObject

### Community 13 - "Community 13"
Cohesion: 0.17
Nodes (20): capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path(), FileProviderItemPayload, handle_connection() (+12 more)

### Community 14 - "Community 14"
Cohesion: 0.12
Nodes (13): notification_preferences_parse_and_serialize_partial(), heartbeat_body_minimal_serializes_status_only(), Config, config_path(), load_config(), save_config(), snapshot_sync_state(), debug_output_redacts_session_token_and_key_material() (+5 more)

### Community 15 - "Community 15"
Cohesion: 0.19
Nodes (8): default_sync_root_suggestion(), default_sync_root_uses_private_file_provider_state_folder(), default_sync_root_uses_user_facing_beebeeb_folder(), DesktopConfig, DesktopSettings, user_visible_home_dir(), default_sync_root(), disposable_cache_path_check_accepts_temp_files_only()

### Community 20 - "Community 20"
Cohesion: 0.2
Nodes (3): apply(), hasExcludedDescendant(), triStateFor()

### Community 28 - "Community 28"
Cohesion: 0.56
Nodes (2): ApiClient, parse_response()

### Community 43 - "Community 43"
Cohesion: 0.48
Nodes (6): buffer_to_string(), call_bridge(), install(), remove(), status(), visible_url()

### Community 44 - "Community 44"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 45 - "Community 45"
Cohesion: 0.38
Nodes (3): dayLabel(), parseDate(), relativeTime()

### Community 46 - "Community 46"
Cohesion: 0.48
Nodes (1): DomainControlTool

### Community 49 - "Community 49"
Cohesion: 0.47
Nodes (4): hostname_or_unknown(), LockContents, LockFile, pid_alive()

### Community 50 - "Community 50"
Cohesion: 0.47
Nodes (3): parseDate(), relativeFuture(), shortDate()

### Community 54 - "Community 54"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

## Knowledge Gaps
- **82 isolated node(s):** `namespace`, `folder`, `file`, `myFiles`, `sharedWithMe` (+77 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 28`** (9 nodes): `ApiClient`, `.create_folder()`, `.list_files()`, `.require_auth()`, `.upload_encrypted()`, `.url()`, `.verify_session()`, `parse_response()`, `api.rs`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 46`** (7 nodes): `DomainControlTool`, `.install()`, `.main()`, `.remove()`, `.signalRoot()`, `.status()`, `DomainControlTool.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 54`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `commandUnavailableLabel()` connect `Community 5` to `Community 0`, `Community 9`, `Community 6`?**
  _High betweenness centrality (0.116) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 3`, `Community 6`, `Community 10`, `Community 12`, `Community 14`?**
  _High betweenness centrality (0.076) - this node is a cross-community bridge._
- **Why does `open()` connect `Community 0` to `Community 2`, `Community 5`, `Community 13`?**
  _High betweenness centrality (0.071) - this node is a cross-community bridge._
- **Are the 35 inferred relationships involving `load()` (e.g. with `commandUnavailableLabel()` and `start_engine_if_possible()`) actually correct?**
  _`load()` has 35 INFERRED edges - model-reasoned connections that need verification._
- **Are the 34 inferred relationships involving `load()` (e.g. with `start_engine_if_possible()` and `start_engine_for_pending_finder_install()`) actually correct?**
  _`load()` has 34 INFERRED edges - model-reasoned connections that need verification._
- **What connects `namespace`, `folder`, `file` to the rest of the system?**
  _82 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.03 - nodes in this community are weakly interconnected._