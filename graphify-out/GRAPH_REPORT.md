# Graph Report - desktop  (2026-05-16)

## Corpus Check
- 80 files · ~118,096 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 729 nodes · 1006 edges · 17 communities detected
- Extraction: 87% EXTRACTED · 13% INFERRED · 0% AMBIGUOUS · INFERRED: 127 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 22|Community 22]]
- [[_COMMUNITY_Community 37|Community 37]]
- [[_COMMUNITY_Community 38|Community 38]]
- [[_COMMUNITY_Community 39|Community 39]]
- [[_COMMUNITY_Community 42|Community 42]]
- [[_COMMUNITY_Community 45|Community 45]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 33 edges
2. `StateDb` - 14 edges
3. `load()` - 14 edges
4. `run()` - 13 edges
5. `EngineBridge` - 13 edges
6. `FileProviderExtension` - 12 edges
7. `XPCBridge` - 11 edges
8. `AuthVault<S>` - 11 edges
9. `commandUnavailableLabel()` - 11 edges
10. `main()` - 10 edges

## Surprising Connections (you probably didn't know these)
- `get_selective_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `set_selective_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `resolve_conflict()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `run()` --calls--> `main()`  [INFERRED]
  src-tauri/src/runner.rs → windows/src/main.rs
- `desktop_login()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.05
Nodes (52): load(), Config, config_path(), default_sync_root_suggestion(), DesktopConfig, DesktopSettings, ensure_directory(), load_config() (+44 more)

### Community 1 - "Community 1"
Cohesion: 0.08
Nodes (25): is_conflict(), is_text_file(), ConflictDetected, default_finder_staging_root(), encrypted_metadata_for_name(), EngineBridge, FileEvent, FileSM (+17 more)

### Community 2 - "Community 2"
Cohesion: 0.08
Nodes (14): AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S>, delete(), find_item(), load(), MacOsKeychainStore (+6 more)

### Community 3 - "Community 3"
Cohesion: 0.09
Nodes (8): ApiClient, DesktopUploadInitRequest, DesktopUploadInitResponse, ListFilesScope, test_chunk_url(), test_desktop_contract_urls_trim_base_and_encode_queries(), test_metadata_operation_urls(), UploadChunkResponse

### Community 4 - "Community 4"
Cohesion: 0.07
Nodes (21): diagnostics(), lock(), runAction(), signOut(), toggleStartAtLogin(), unlock(), toggleFolder(), refresh() (+13 more)

### Community 5 - "Community 5"
Cohesion: 0.09
Nodes (15): resolve_conflict(), ensure_column(), FileContractState, FileEntry, has_column(), Namespace, OperationKind, PendingOperation (+7 more)

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
Cohesion: 0.22
Nodes (15): file_entry_payload(), file_status_string(), filename_from_path(), FileProviderItemPayload, handle_connection(), ipc_socket_path(), IpcRequest, IpcResponse (+7 more)

### Community 10 - "Community 10"
Cohesion: 0.17
Nodes (10): auto_resolution_deadline(), ConflictRecord, Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed(), v() (+2 more)

### Community 22 - "Community 22"
Cohesion: 0.56
Nodes (2): ApiClient, parse_response()

### Community 37 - "Community 37"
Cohesion: 0.38
Nodes (4): hostname_or_unknown(), LockContents, LockFile, pid_alive()

### Community 38 - "Community 38"
Cohesion: 0.29
Nodes (1): MemoryStore

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
- **33 isolated node(s):** `daemonUnavailable`, `namespace`, `folder`, `file`, `myFiles` (+28 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 22`** (9 nodes): `ApiClient`, `.create_folder()`, `.list_files()`, `.require_auth()`, `.upload_encrypted()`, `.url()`, `.verify_session()`, `parse_response()`, `api.rs`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 38`** (7 nodes): `MemoryStore`, `.delete_session_token()`, `.delete_wrapped_master_key()`, `.load_session_token()`, `.load_wrapped_master_key()`, `.save_session_token()`, `.save_wrapped_master_key()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 42`** (6 nodes): `callbacks.rs`, `mod.rs`, `fetch_data_callback()`, `bridge()`, `register_sync_root()`, `set_bridge()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 45`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `run()` connect `Community 0` to `Community 8`, `Community 2`?**
  _High betweenness centrality (0.085) - this node is a cross-community bridge._
- **Why does `load()` connect `Community 0` to `Community 4`, `Community 5`?**
  _High betweenness centrality (0.053) - this node is a cross-community bridge._
- **Are the 13 inferred relationships involving `load()` (e.g. with `desktop_login()` and `set_session()`) actually correct?**
  _`load()` has 13 INFERRED edges - model-reasoned connections that need verification._
- **Are the 6 inferred relationships involving `run()` (e.g. with `.fmt()` and `.init()`) actually correct?**
  _`run()` has 6 INFERRED edges - model-reasoned connections that need verification._
- **What connects `daemonUnavailable`, `namespace`, `folder` to the rest of the system?**
  _33 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.05 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.08 - nodes in this community are weakly interconnected._