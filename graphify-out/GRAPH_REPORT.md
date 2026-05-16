# Graph Report - desktop  (2026-05-16)

## Corpus Check
- 76 files · ~115,739 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 628 nodes · 809 edges · 13 communities detected
- Extraction: 88% EXTRACTED · 12% INFERRED · 0% AMBIGUOUS · INFERRED: 96 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 4|Community 4]]
- [[_COMMUNITY_Community 5|Community 5]]
- [[_COMMUNITY_Community 6|Community 6]]
- [[_COMMUNITY_Community 15|Community 15]]
- [[_COMMUNITY_Community 19|Community 19]]
- [[_COMMUNITY_Community 34|Community 34]]
- [[_COMMUNITY_Community 35|Community 35]]
- [[_COMMUNITY_Community 36|Community 36]]
- [[_COMMUNITY_Community 39|Community 39]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 33 edges
2. `StateDb` - 14 edges
3. `run()` - 13 edges
4. `load()` - 13 edges
5. `AuthVault<S>` - 11 edges
6. `main()` - 10 edges
7. `EngineBridge` - 9 edges
8. `run()` - 9 edges
9. `MacOsKeychainStore` - 9 edges
10. `BeebeebFS` - 8 edges

## Surprising Connections (you probably didn't know these)
- `sync_status()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `get_selective_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `set_selective_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `resolve_conflict()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `run()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.05
Nodes (39): load(), default_sync_root_suggestion(), DesktopConfig, DesktopSettings, ensure_directory(), handle_connection(), ipc_socket_path(), IpcRequest (+31 more)

### Community 1 - "Community 1"
Cohesion: 0.09
Nodes (8): ApiClient, DesktopUploadInitRequest, DesktopUploadInitResponse, ListFilesScope, test_chunk_url(), test_desktop_contract_urls_trim_base_and_encode_queries(), test_metadata_operation_urls(), UploadChunkResponse

### Community 2 - "Community 2"
Cohesion: 0.08
Nodes (14): AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S>, delete(), find_item(), load(), MacOsKeychainStore (+6 more)

### Community 3 - "Community 3"
Cohesion: 0.09
Nodes (25): Config, config_path(), load_config(), save_config(), AppState, attach_tray_status_listener(), open_onboarding_window(), open_onboarding_window_impl() (+17 more)

### Community 4 - "Community 4"
Cohesion: 0.09
Nodes (16): resolve_conflict(), sync_status(), ensure_column(), FileContractState, FileEntry, has_column(), Namespace, OperationKind (+8 more)

### Community 5 - "Community 5"
Cohesion: 0.08
Nodes (19): auto_resolution_deadline(), ConflictRecord, is_conflict(), is_text_file(), Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed() (+11 more)

### Community 6 - "Community 6"
Cohesion: 0.18
Nodes (6): XPCBridge, BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice()

### Community 15 - "Community 15"
Cohesion: 0.22
Nodes (5): FileProviderExtension, FileProviderItem, NSFileProviderExtension, NSFileProviderItem, NSObject

### Community 19 - "Community 19"
Cohesion: 0.56
Nodes (2): ApiClient, parse_response()

### Community 34 - "Community 34"
Cohesion: 0.29
Nodes (1): MemoryStore

### Community 35 - "Community 35"
Cohesion: 0.38
Nodes (4): hostname_or_unknown(), LockContents, LockFile, pid_alive()

### Community 36 - "Community 36"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 39 - "Community 39"
Cohesion: 0.33
Nodes (2): fetch_data_callback(), bridge()

## Knowledge Gaps
- **21 isolated node(s):** `VersionInfo`, `ConflictRecord`, `Resolution`, `FileEntry`, `PendingOperation` (+16 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 19`** (9 nodes): `ApiClient`, `.create_folder()`, `.list_files()`, `.require_auth()`, `.upload_encrypted()`, `.url()`, `.verify_session()`, `parse_response()`, `api.rs`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 34`** (7 nodes): `MemoryStore`, `.delete_session_token()`, `.delete_wrapped_master_key()`, `.load_session_token()`, `.load_wrapped_master_key()`, `.save_session_token()`, `.save_wrapped_master_key()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 39`** (6 nodes): `callbacks.rs`, `mod.rs`, `fetch_data_callback()`, `bridge()`, `register_sync_root()`, `set_bridge()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `run()` connect `Community 0` to `Community 3`, `Community 35`, `Community 4`?**
  _High betweenness centrality (0.058) - this node is a cross-community bridge._
- **Why does `sweep_auto_resolutions()` connect `Community 5` to `Community 0`?**
  _High betweenness centrality (0.050) - this node is a cross-community bridge._
- **Are the 6 inferred relationships involving `run()` (e.g. with `.fmt()` and `.init()`) actually correct?**
  _`run()` has 6 INFERRED edges - model-reasoned connections that need verification._
- **Are the 12 inferred relationships involving `load()` (e.g. with `desktop_login()` and `set_session()`) actually correct?**
  _`load()` has 12 INFERRED edges - model-reasoned connections that need verification._
- **What connects `VersionInfo`, `ConflictRecord`, `Resolution` to the rest of the system?**
  _21 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.05 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.09 - nodes in this community are weakly interconnected._