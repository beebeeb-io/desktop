# Graph Report - desktop  (2026-05-06)

## Corpus Check
- 70 files · ~99,107 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 462 nodes · 505 edges · 11 communities detected
- Extraction: 92% EXTRACTED · 8% INFERRED · 0% AMBIGUOUS · INFERRED: 40 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 4|Community 4]]
- [[_COMMUNITY_Community 13|Community 13]]
- [[_COMMUNITY_Community 14|Community 14]]
- [[_COMMUNITY_Community 18|Community 18]]
- [[_COMMUNITY_Community 33|Community 33]]
- [[_COMMUNITY_Community 34|Community 34]]
- [[_COMMUNITY_Community 37|Community 37]]

## God Nodes (most connected - your core abstractions)
1. `run()` - 9 edges
2. `run()` - 9 edges
3. `StateDb` - 8 edges
4. `ApiClient` - 8 edges
5. `main()` - 8 edges
6. `BeebeebFS` - 8 edges
7. `ApiClient` - 8 edges
8. `FileProviderItem` - 6 edges
9. `EngineBridge` - 6 edges
10. `parse_response()` - 6 edges

## Surprising Connections (you probably didn't know these)
- `main()` --calls--> `run()`  [INFERRED]
  windows/src/main.rs → src-tauri/src/runner.rs
- `pick_sync_root()` --calls--> `ensure_directory()`  [INFERRED]
  src-tauri/src/lib.rs → src-tauri/src/config.rs
- `pick_sync_root()` --calls--> `default_sync_root_suggestion()`  [INFERRED]
  src-tauri/src/lib.rs → src-tauri/src/config.rs
- `default_sync_root()` --calls--> `default_sync_root_suggestion()`  [INFERRED]
  src-tauri/src/lib.rs → src-tauri/src/config.rs
- `run()` --calls--> `serve_ipc()`  [INFERRED]
  src-tauri/src/runner.rs → src-tauri/src/ipc_socket.rs

## Communities

### Community 0 - "Community 0"
Cohesion: 0.08
Nodes (27): default_sync_root_suggestion(), DesktopConfig, ensure_directory(), handle_connection(), ipc_socket_path(), IpcRequest, IpcResponse, serve_ipc() (+19 more)

### Community 1 - "Community 1"
Cohesion: 0.15
Nodes (13): Config, config_path(), load_config(), save_config(), Args, b64(), futures_task_noop_waker(), is_ignored_file() (+5 more)

### Community 2 - "Community 2"
Cohesion: 0.18
Nodes (6): XPCBridge, BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice()

### Community 3 - "Community 3"
Cohesion: 0.23
Nodes (5): FileEntry, FileStatus, StateDb, test_list_by_status(), test_upsert_and_get_file()

### Community 4 - "Community 4"
Cohesion: 0.19
Nodes (6): EngineBridge, FileEvent, FileSM, sync_tick(), test_file_state_machine_transitions(), test_invalid_transition_rejected()

### Community 13 - "Community 13"
Cohesion: 0.22
Nodes (5): FileProviderExtension, FileProviderItem, NSFileProviderExtension, NSFileProviderItem, NSObject

### Community 14 - "Community 14"
Cohesion: 0.27
Nodes (2): ApiClient, test_chunk_url()

### Community 18 - "Community 18"
Cohesion: 0.56
Nodes (2): ApiClient, parse_response()

### Community 33 - "Community 33"
Cohesion: 0.38
Nodes (4): hostname_or_unknown(), LockContents, LockFile, pid_alive()

### Community 34 - "Community 34"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 37 - "Community 37"
Cohesion: 0.33
Nodes (2): fetch_data_callback(), bridge()

## Knowledge Gaps
- **9 isolated node(s):** `FileEntry`, `LockContents`, `Session`, `AppState`, `FileEvent` (+4 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 14`** (10 nodes): `ApiClient`, `.chunk_url()`, `.download_chunk()`, `.get_file()`, `.list_files()`, `.master_key()`, `.new()`, `.upload_chunk()`, `test_chunk_url()`, `api_client.rs`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 18`** (9 nodes): `ApiClient`, `.create_folder()`, `.list_files()`, `.require_auth()`, `.upload_encrypted()`, `.url()`, `.verify_session()`, `parse_response()`, `api.rs`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 37`** (6 nodes): `callbacks.rs`, `mod.rs`, `fetch_data_callback()`, `bridge()`, `register_sync_root()`, `set_bridge()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `run()` connect `Community 0` to `Community 1`, `Community 13`?**
  _High betweenness centrality (0.020) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 33`, `Community 3`, `Community 1`?**
  _High betweenness centrality (0.017) - this node is a cross-community bridge._
- **Why does `main()` connect `Community 1` to `Community 0`?**
  _High betweenness centrality (0.012) - this node is a cross-community bridge._
- **Are the 4 inferred relationships involving `run()` (e.g. with `.default()` and `.init()`) actually correct?**
  _`run()` has 4 INFERRED edges - model-reasoned connections that need verification._
- **Are the 5 inferred relationships involving `run()` (e.g. with `.acquire()` and `.open()`) actually correct?**
  _`run()` has 5 INFERRED edges - model-reasoned connections that need verification._
- **Are the 3 inferred relationships involving `main()` (e.g. with `run()` and `.new()`) actually correct?**
  _`main()` has 3 INFERRED edges - model-reasoned connections that need verification._
- **What connects `FileEntry`, `LockContents`, `Session` to the rest of the system?**
  _9 weakly-connected nodes found - possible documentation gaps or missing edges._