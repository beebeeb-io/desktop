# Graph Report - desktop  (2026-06-15)

## Corpus Check
- 93 files · ~182,834 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1195 nodes · 2126 edges · 22 communities detected
- Extraction: 83% EXTRACTED · 17% INFERRED · 0% AMBIGUOUS · INFERRED: 365 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 17|Community 17]]
- [[_COMMUNITY_Community 33|Community 33]]
- [[_COMMUNITY_Community 42|Community 42]]
- [[_COMMUNITY_Community 43|Community 43]]
- [[_COMMUNITY_Community 44|Community 44]]
- [[_COMMUNITY_Community 47|Community 47]]
- [[_COMMUNITY_Community 50|Community 50]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 50 edges
2. `load()` - 36 edges
3. `EngineBridge` - 31 edges
4. `StateDb` - 30 edges
5. `open()` - 24 edges
6. `commandUnavailableLabel()` - 23 edges
7. `command()` - 20 edges
8. `run()` - 19 edges
9. `api_client_from_session()` - 17 edges
10. `now_secs()` - 15 edges

## Surprising Connections (you probably didn't know these)
- `test_bridge()` --calls--> `open()`  [INFERRED]
  src-tauri/src/engine_bridge.rs → src/WindowsApp.tsx
- `test_bridge_with_api()` --calls--> `open()`  [INFERRED]
  src-tauri/src/engine_bridge.rs → src/WindowsApp.tsx
- `desktop_config()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `tray_pause_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `tray_resume_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.03
Nodes (144): load(), ensure_directory(), apply_metadata_file_row(), test_metadata_file_row_classifies_folder_rows(), test_metadata_file_row_composes_nested_path_under_parent(), test_metadata_file_row_decrypts_encrypted_name_into_path(), test_metadata_file_row_falls_back_when_name_undecryptable(), test_metadata_file_row_persists_contract_for_own_file() (+136 more)

### Community 1 - "Community 1"
Cohesion: 0.04
Nodes (69): is_conflict(), is_text_file(), apply_shared_context(), CacheCleanupOutcome, CachePolicy, classify_operation_error(), classify_review_operation(), ConflictDetected (+61 more)

### Community 2 - "Community 2"
Cohesion: 0.04
Nodes (60): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), toggleFolder(), formatBytes() (+52 more)

### Community 3 - "Community 3"
Cohesion: 0.06
Nodes (23): account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S>, clear_session_removes_account_email(), delete() (+15 more)

### Community 4 - "Community 4"
Cohesion: 0.05
Nodes (26): count_queue_groups(), ensure_column(), FileContractState, FileEntry, get_file_by_path_matches_across_leading_slash(), has_column(), ItemKind, Namespace (+18 more)

### Community 5 - "Community 5"
Cohesion: 0.07
Nodes (8): ApiClient, DesktopUploadInitRequest, DesktopUploadInitResponse, ListFilesScope, test_chunk_url(), test_desktop_contract_urls_trim_base_and_encode_queries(), test_metadata_operation_urls(), UploadChunkResponse

### Community 6 - "Community 6"
Cohesion: 0.05
Nodes (48): account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity, AccountActivityEvent, AccountActivitySummary, AccountProfile (+40 more)

### Community 7 - "Community 7"
Cohesion: 0.07
Nodes (30): BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts, myFiles, offline (+22 more)

### Community 8 - "Community 8"
Cohesion: 0.08
Nodes (33): is_ignored_finder_name(), path_is_engine_internal(), relative_db_path(), Session, Args, b64(), futures_task_noop_waker(), guess_mime_type() (+25 more)

### Community 9 - "Community 9"
Cohesion: 0.15
Nodes (25): fail_transfer(), fetch_data_callback(), mark_in_sync(), normalized_path(), notify_delete_completion_callback(), notify_file_close_completion_callback(), notify_full_path(), notify_rename_completion_callback() (+17 more)

### Community 10 - "Community 10"
Cohesion: 0.1
Nodes (16): auto_resolution_deadline(), ConflictRecord, Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed(), v() (+8 more)

### Community 11 - "Community 11"
Cohesion: 0.1
Nodes (17): Config, config_path(), default_sync_root_suggestion(), default_sync_root_uses_private_file_provider_state_folder(), default_sync_root_uses_user_facing_beebeeb_folder(), DesktopConfig, DesktopSettings, load_config() (+9 more)

### Community 12 - "Community 12"
Cohesion: 0.13
Nodes (7): FileProviderEnumerator, FileProviderExtension, FileProviderItem, NSFileProviderEnumerator, NSFileProviderItem, NSFileProviderReplicatedExtension, NSObject

### Community 13 - "Community 13"
Cohesion: 0.2
Nodes (19): capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path(), FileProviderItemPayload, handle_connection() (+11 more)

### Community 14 - "Community 14"
Cohesion: 0.25
Nodes (6): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), read_http_request()

### Community 17 - "Community 17"
Cohesion: 0.31
Nodes (4): ApiClient, parse_response(), attach_tray_status_listener(), FileStatus

### Community 33 - "Community 33"
Cohesion: 0.39
Nodes (2): DomainControlTool, install_file_provider_domain()

### Community 42 - "Community 42"
Cohesion: 0.29
Nodes (1): MemoryStore

### Community 43 - "Community 43"
Cohesion: 0.48
Nodes (6): buffer_to_string(), call_bridge(), install(), remove(), status(), visible_url()

### Community 44 - "Community 44"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 47 - "Community 47"
Cohesion: 0.47
Nodes (4): hostname_or_unknown(), LockContents, LockFile, pid_alive()

### Community 50 - "Community 50"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

## Knowledge Gaps
- **74 isolated node(s):** `namespace`, `folder`, `file`, `myFiles`, `sharedWithMe` (+69 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 33`** (8 nodes): `DomainControlTool`, `.install()`, `.main()`, `.remove()`, `.signalRoot()`, `.status()`, `DomainControlTool.swift`, `install_file_provider_domain()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 42`** (7 nodes): `MemoryStore`, `.delete_session_token()`, `.delete_wrapped_master_key()`, `.load_session_token()`, `.load_wrapped_master_key()`, `.save_session_token()`, `.save_wrapped_master_key()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 50`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `commandUnavailableLabel()` connect `Community 2` to `Community 0`, `Community 7`?**
  _High betweenness centrality (0.100) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 8`, `Community 17`, `Community 3`, `Community 12`?**
  _High betweenness centrality (0.078) - this node is a cross-community bridge._
- **Are the 35 inferred relationships involving `load()` (e.g. with `start_engine_if_possible()` and `start_engine_for_pending_finder_install()`) actually correct?**
  _`load()` has 35 INFERRED edges - model-reasoned connections that need verification._
- **Are the 23 inferred relationships involving `open()` (e.g. with `.upload_version()` and `test_bridge()`) actually correct?**
  _`open()` has 23 INFERRED edges - model-reasoned connections that need verification._
- **What connects `namespace`, `folder`, `file` to the rest of the system?**
  _74 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.03 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.04 - nodes in this community are weakly interconnected._