# Graph Report - desktop  (2026-05-17)

## Corpus Check
- 81 files · ~128,862 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 876 nodes · 1367 edges · 17 communities detected
- Extraction: 86% EXTRACTED · 14% INFERRED · 0% AMBIGUOUS · INFERRED: 188 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 23|Community 23]]
- [[_COMMUNITY_Community 38|Community 38]]
- [[_COMMUNITY_Community 39|Community 39]]
- [[_COMMUNITY_Community 42|Community 42]]
- [[_COMMUNITY_Community 45|Community 45]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 33 edges
2. `EngineBridge` - 28 edges
3. `StateDb` - 25 edges
4. `load()` - 23 edges
5. `run()` - 16 edges
6. `commandUnavailableLabel()` - 16 edges
7. `now_secs()` - 15 edges
8. `test_bridge()` - 14 edges
9. `FileProviderExtension` - 12 edges
10. `XPCBridge` - 11 edges

## Surprising Connections (you probably didn't know these)
- `finder_location_state()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `get_selective_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `set_selective_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `run()` --calls--> `main()`  [INFERRED]
  src-tauri/src/runner.rs → windows/src/main.rs
- `start_engine_if_possible()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.04
Nodes (84): load(), default_sync_root_suggestion(), DesktopConfig, DesktopSettings, ensure_directory(), ipc_socket_path(), serve_ipc(), account_email() (+76 more)

### Community 1 - "Community 1"
Cohesion: 0.04
Nodes (60): apply_metadata_file_row(), apply_shared_context(), CacheCleanupOutcome, CachePolicy, classify_operation_error(), classify_review_operation(), ConflictDetected, decrypt_downloaded_chunk() (+52 more)

### Community 2 - "Community 2"
Cohesion: 0.05
Nodes (25): count_queue_groups(), ensure_column(), FileContractState, FileEntry, has_column(), ItemKind, Namespace, OperationKind (+17 more)

### Community 3 - "Community 3"
Cohesion: 0.06
Nodes (35): diagnostics(), lock(), runAction(), signOut(), toggleStartAtLogin(), unlock(), toggleFolder(), formatBytes() (+27 more)

### Community 4 - "Community 4"
Cohesion: 0.09
Nodes (8): ApiClient, DesktopUploadInitRequest, DesktopUploadInitResponse, ListFilesScope, test_chunk_url(), test_desktop_contract_urls_trim_base_and_encode_queries(), test_metadata_operation_urls(), UploadChunkResponse

### Community 5 - "Community 5"
Cohesion: 0.08
Nodes (14): AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S>, delete(), find_item(), load(), MacOsKeychainStore (+6 more)

### Community 6 - "Community 6"
Cohesion: 0.11
Nodes (18): BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts, myFiles, offline (+10 more)

### Community 7 - "Community 7"
Cohesion: 0.14
Nodes (13): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), Args, b64(), futures_task_noop_waker() (+5 more)

### Community 8 - "Community 8"
Cohesion: 0.13
Nodes (7): FileProviderEnumerator, FileProviderExtension, FileProviderItem, NSFileProviderEnumerator, NSFileProviderItem, NSFileProviderReplicatedExtension, NSObject

### Community 9 - "Community 9"
Cohesion: 0.17
Nodes (20): capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path(), FileProviderItemPayload, handle_connection() (+12 more)

### Community 10 - "Community 10"
Cohesion: 0.13
Nodes (11): Config, config_path(), load_config(), save_config(), debug_output_redacts_session_token_and_key_material(), install_and_clear_session_round_trips_through_store(), MemoryStore, unlock_rejects_wrong_sized_master_key_and_remains_locked() (+3 more)

### Community 11 - "Community 11"
Cohesion: 0.13
Nodes (13): auto_resolution_deadline(), ConflictRecord, is_conflict(), is_text_file(), Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed() (+5 more)

### Community 23 - "Community 23"
Cohesion: 0.56
Nodes (2): ApiClient, parse_response()

### Community 38 - "Community 38"
Cohesion: 0.38
Nodes (4): hostname_or_unknown(), LockContents, LockFile, pid_alive()

### Community 39 - "Community 39"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 42 - "Community 42"
Cohesion: 0.33
Nodes (2): fetch_data_callback(), bridge()

### Community 45 - "Community 45"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

## Knowledge Gaps
- **43 isolated node(s):** `daemonUnavailable`, `namespace`, `folder`, `file`, `myFiles` (+38 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 23`** (9 nodes): `ApiClient`, `.create_folder()`, `.list_files()`, `.require_auth()`, `.upload_encrypted()`, `.url()`, `.verify_session()`, `parse_response()`, `api.rs`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 42`** (6 nodes): `callbacks.rs`, `mod.rs`, `fetch_data_callback()`, `bridge()`, `register_sync_root()`, `set_bridge()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 45`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `load()` connect `Community 0` to `Community 3`?**
  _High betweenness centrality (0.076) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 8`, `Community 10`, `Community 5`?**
  _High betweenness centrality (0.073) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 3` to `Community 0`?**
  _High betweenness centrality (0.073) - this node is a cross-community bridge._
- **Are the 22 inferred relationships involving `load()` (e.g. with `start_engine_if_possible()` and `finder_location_state()`) actually correct?**
  _`load()` has 22 INFERRED edges - model-reasoned connections that need verification._
- **Are the 6 inferred relationships involving `run()` (e.g. with `.init()` and `.new()`) actually correct?**
  _`run()` has 6 INFERRED edges - model-reasoned connections that need verification._
- **What connects `daemonUnavailable`, `namespace`, `folder` to the rest of the system?**
  _43 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.04 - nodes in this community are weakly interconnected._