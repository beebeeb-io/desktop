# Graph Report - desktop  (2026-06-15)

## Corpus Check
- 91 files · ~166,654 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1064 nodes · 1850 edges · 19 communities detected
- Extraction: 84% EXTRACTED · 16% INFERRED · 0% AMBIGUOUS · INFERRED: 303 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 15|Community 15]]
- [[_COMMUNITY_Community 31|Community 31]]
- [[_COMMUNITY_Community 40|Community 40]]
- [[_COMMUNITY_Community 41|Community 41]]
- [[_COMMUNITY_Community 42|Community 42]]
- [[_COMMUNITY_Community 47|Community 47]]

## God Nodes (most connected - your core abstractions)
1. `load()` - 35 edges
2. `ApiClient` - 33 edges
3. `StateDb` - 30 edges
4. `EngineBridge` - 29 edges
5. `commandUnavailableLabel()` - 22 edges
6. `run()` - 19 edges
7. `now_secs()` - 15 edges
8. `test_bridge()` - 14 edges
9. `AuthVault<S>` - 13 edges
10. `run()` - 13 edges

## Surprising Connections (you probably didn't know these)
- `list_version_conflict_center()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `tray_pause_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `tray_resume_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `set_sync_mode()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `get_selective_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.04
Nodes (118): load(), DesktopConfig, ensure_directory(), ipc_socket_path(), serve_ipc(), platform_keychain_store(), account_email(), apply_session() (+110 more)

### Community 1 - "Community 1"
Cohesion: 0.04
Nodes (68): is_conflict(), is_text_file(), apply_shared_context(), CacheCleanupOutcome, CachePolicy, classify_operation_error(), classify_review_operation(), ConflictDetected (+60 more)

### Community 2 - "Community 2"
Cohesion: 0.04
Nodes (35): apply_metadata_file_row(), resolve_relative_path(), test_metadata_file_row_classifies_folder_rows(), test_metadata_file_row_composes_nested_path_under_parent(), test_metadata_file_row_decrypts_encrypted_name_into_path(), test_metadata_file_row_falls_back_when_name_undecryptable(), test_metadata_file_row_persists_contract_for_own_file(), test_metadata_modify_does_not_queue_content_version() (+27 more)

### Community 3 - "Community 3"
Cohesion: 0.06
Nodes (24): account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S>, clear_session_removes_account_email(), delete() (+16 more)

### Community 4 - "Community 4"
Cohesion: 0.04
Nodes (44): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), toggleFolder(), formatBytes() (+36 more)

### Community 5 - "Community 5"
Cohesion: 0.07
Nodes (30): BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts, myFiles, offline (+22 more)

### Community 6 - "Community 6"
Cohesion: 0.09
Nodes (8): ApiClient, DesktopUploadInitRequest, DesktopUploadInitResponse, ListFilesScope, test_chunk_url(), test_desktop_contract_urls_trim_base_and_encode_queries(), test_metadata_operation_urls(), UploadChunkResponse

### Community 7 - "Community 7"
Cohesion: 0.08
Nodes (29): Config, config_path(), default_sync_root_suggestion(), default_sync_root_uses_private_file_provider_state_folder(), default_sync_root_uses_user_facing_beebeeb_folder(), DesktopSettings, load_config(), save_config() (+21 more)

### Community 8 - "Community 8"
Cohesion: 0.08
Nodes (21): auto_resolution_deadline(), ConflictRecord, Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed(), v() (+13 more)

### Community 9 - "Community 9"
Cohesion: 0.14
Nodes (22): fail_transfer(), fetch_data_callback(), mark_in_sync(), normalized_path(), secure_remove_temp(), transfer_one(), transfer_range(), wide_to_vec() (+14 more)

### Community 10 - "Community 10"
Cohesion: 0.13
Nodes (7): FileProviderEnumerator, FileProviderExtension, FileProviderItem, NSFileProviderEnumerator, NSFileProviderItem, NSFileProviderReplicatedExtension, NSObject

### Community 11 - "Community 11"
Cohesion: 0.2
Nodes (19): capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path(), FileProviderItemPayload, handle_connection() (+11 more)

### Community 12 - "Community 12"
Cohesion: 0.25
Nodes (6): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), read_http_request()

### Community 15 - "Community 15"
Cohesion: 0.31
Nodes (4): ApiClient, parse_response(), attach_tray_status_listener(), FileStatus

### Community 31 - "Community 31"
Cohesion: 0.39
Nodes (2): DomainControlTool, install_file_provider_domain()

### Community 40 - "Community 40"
Cohesion: 0.29
Nodes (1): MemoryStore

### Community 41 - "Community 41"
Cohesion: 0.48
Nodes (6): buffer_to_string(), call_bridge(), install(), remove(), status(), visible_url()

### Community 42 - "Community 42"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 47 - "Community 47"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

## Knowledge Gaps
- **47 isolated node(s):** `namespace`, `folder`, `file`, `myFiles`, `sharedWithMe` (+42 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 31`** (8 nodes): `DomainControlTool`, `.install()`, `.main()`, `.remove()`, `.signalRoot()`, `.status()`, `DomainControlTool.swift`, `install_file_provider_domain()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 40`** (7 nodes): `MemoryStore`, `.delete_session_token()`, `.delete_wrapped_master_key()`, `.load_session_token()`, `.load_wrapped_master_key()`, `.save_session_token()`, `.save_wrapped_master_key()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 47`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `run()` connect `Community 0` to `Community 10`, `Community 3`, `Community 5`, `Community 15`?**
  _High betweenness centrality (0.084) - this node is a cross-community bridge._
- **Why does `load()` connect `Community 0` to `Community 8`, `Community 1`, `Community 4`?**
  _High betweenness centrality (0.069) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 4` to `Community 0`, `Community 5`?**
  _High betweenness centrality (0.065) - this node is a cross-community bridge._
- **Are the 34 inferred relationships involving `load()` (e.g. with `start_engine_if_possible()` and `start_engine_for_pending_finder_install()`) actually correct?**
  _`load()` has 34 INFERRED edges - model-reasoned connections that need verification._
- **Are the 21 inferred relationships involving `commandUnavailableLabel()` (e.g. with `toggle()` and `refresh()`) actually correct?**
  _`commandUnavailableLabel()` has 21 INFERRED edges - model-reasoned connections that need verification._
- **What connects `namespace`, `folder`, `file` to the rest of the system?**
  _47 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.04 - nodes in this community are weakly interconnected._