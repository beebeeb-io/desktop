# Graph Report - beebeeb-desktop-0332-graph.2pYUii  (2026-05-16)

## Corpus Check
- 77 files · ~116,161 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 675 nodes · 886 edges · 14 communities detected
- Extraction: 88% EXTRACTED · 12% INFERRED · 0% AMBIGUOUS · INFERRED: 104 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 21|Community 21]]
- [[_COMMUNITY_Community 36|Community 36]]
- [[_COMMUNITY_Community 39|Community 39]]
- [[_COMMUNITY_Community 42|Community 42]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 33 edges
2. `StateDb` - 14 edges
3. `run()` - 13 edges
4. `load()` - 13 edges
5. `FileProviderExtension` - 12 edges
6. `AuthVault<S>` - 11 edges
7. `main()` - 10 edges
8. `FileProviderEnumerator` - 9 edges
9. `EngineBridge` - 9 edges
10. `run()` - 9 edges

## Surprising Connections (you probably didn't know these)
- `sync_status()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `get_selective_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `set_selective_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `resolve_conflict()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `run()` --calls--> `main()`  [INFERRED]
  src-tauri/src/runner.rs → windows/src/main.rs

## Communities

### Community 0 - "Community 0"
Cohesion: 0.06
Nodes (40): load(), default_sync_root_suggestion(), DesktopConfig, DesktopSettings, ensure_directory(), AppState, attach_tray_status_listener(), build_tray_menu() (+32 more)

### Community 1 - "Community 1"
Cohesion: 0.08
Nodes (14): AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S>, delete(), find_item(), load(), MacOsKeychainStore (+6 more)

### Community 2 - "Community 2"
Cohesion: 0.09
Nodes (8): ApiClient, DesktopUploadInitRequest, DesktopUploadInitResponse, ListFilesScope, test_chunk_url(), test_desktop_contract_urls_trim_base_and_encode_queries(), test_metadata_operation_urls(), UploadChunkResponse

### Community 3 - "Community 3"
Cohesion: 0.09
Nodes (16): resolve_conflict(), sync_status(), ensure_column(), FileContractState, FileEntry, has_column(), Namespace, OperationKind (+8 more)

### Community 4 - "Community 4"
Cohesion: 0.08
Nodes (19): auto_resolution_deadline(), ConflictRecord, is_conflict(), is_text_file(), Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed() (+11 more)

### Community 5 - "Community 5"
Cohesion: 0.08
Nodes (20): BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts, myFiles, offline (+12 more)

### Community 6 - "Community 6"
Cohesion: 0.1
Nodes (14): Inner, InodeMap, test_inode_assignment_stable(), Config, config_path(), load_config(), save_config(), debug_output_redacts_session_token_and_key_material() (+6 more)

### Community 7 - "Community 7"
Cohesion: 0.13
Nodes (13): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), Args, b64(), futures_task_noop_waker() (+5 more)

### Community 8 - "Community 8"
Cohesion: 0.13
Nodes (7): FileProviderEnumerator, FileProviderExtension, FileProviderItem, NSFileProviderEnumerator, NSFileProviderItem, NSFileProviderReplicatedExtension, NSObject

### Community 9 - "Community 9"
Cohesion: 0.2
Nodes (13): file_entry_payload(), file_status_string(), filename_from_path(), FileProviderItemPayload, handle_connection(), ipc_socket_path(), IpcRequest, IpcResponse (+5 more)

### Community 21 - "Community 21"
Cohesion: 0.56
Nodes (2): ApiClient, parse_response()

### Community 36 - "Community 36"
Cohesion: 0.38
Nodes (4): hostname_or_unknown(), LockContents, LockFile, pid_alive()

### Community 39 - "Community 39"
Cohesion: 0.33
Nodes (2): fetch_data_callback(), bridge()

### Community 42 - "Community 42"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

## Knowledge Gaps
- **30 isolated node(s):** `daemonUnavailable`, `namespace`, `folder`, `file`, `myFiles` (+25 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 21`** (9 nodes): `ApiClient`, `.create_folder()`, `.list_files()`, `.require_auth()`, `.upload_encrypted()`, `.url()`, `.verify_session()`, `parse_response()`, `api.rs`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 39`** (6 nodes): `callbacks.rs`, `mod.rs`, `fetch_data_callback()`, `bridge()`, `register_sync_root()`, `set_bridge()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 42`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `run()` connect `Community 0` to `Community 8`, `Community 1`, `Community 6`?**
  _High betweenness centrality (0.087) - this node is a cross-community bridge._
- **Why does `FileProviderItem` connect `Community 8` to `Community 5`?**
  _High betweenness centrality (0.061) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 3`, `Community 36`, `Community 6`, `Community 7`, `Community 9`?**
  _High betweenness centrality (0.058) - this node is a cross-community bridge._
- **Are the 6 inferred relationships involving `run()` (e.g. with `.fmt()` and `.init()`) actually correct?**
  _`run()` has 6 INFERRED edges - model-reasoned connections that need verification._
- **Are the 12 inferred relationships involving `load()` (e.g. with `desktop_login()` and `set_session()`) actually correct?**
  _`load()` has 12 INFERRED edges - model-reasoned connections that need verification._
- **What connects `daemonUnavailable`, `namespace`, `folder` to the rest of the system?**
  _30 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.06 - nodes in this community are weakly interconnected._