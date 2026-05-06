# Graph Report - desktop  (2026-05-07)

## Corpus Check
- 72 files · ~105,431 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 499 nodes · 568 edges · 11 communities detected
- Extraction: 90% EXTRACTED · 10% INFERRED · 0% AMBIGUOUS · INFERRED: 56 edges (avg confidence: 0.8)
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
1. `run()` - 10 edges
2. `run()` - 9 edges
3. `main()` - 9 edges
4. `StateDb` - 8 edges
5. `ApiClient` - 8 edges
6. `BeebeebFS` - 8 edges
7. `ApiClient` - 8 edges
8. `EngineBridge` - 7 edges
9. `FileProviderItem` - 6 edges
10. `DesktopConfig` - 6 edges

## Surprising Connections (you probably didn't know these)
- `run()` --calls--> `main()`  [INFERRED]
  src-tauri/src/runner.rs → windows/src/main.rs
- `sync_tick()` --calls--> `is_conflict()`  [INFERRED]
  src-tauri/src/engine_bridge.rs → src-tauri/src/conflict.rs
- `sync_tick()` --calls--> `is_text_file()`  [INFERRED]
  src-tauri/src/engine_bridge.rs → src-tauri/src/conflict.rs
- `ensure_directory()` --calls--> `pick_sync_root()`  [INFERRED]
  src-tauri/src/config.rs → src-tauri/src/lib.rs
- `sweep_auto_resolutions()` --calls--> `auto_resolution_deadline()`  [INFERRED]
  src-tauri/src/runner.rs → src-tauri/src/conflict.rs

## Communities

### Community 0 - "Community 0"
Cohesion: 0.06
Nodes (33): default_sync_root_suggestion(), DesktopConfig, DesktopSettings, ensure_directory(), handle_connection(), ipc_socket_path(), IpcRequest, IpcResponse (+25 more)

### Community 1 - "Community 1"
Cohesion: 0.08
Nodes (19): auto_resolution_deadline(), ConflictRecord, is_conflict(), is_text_file(), Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed() (+11 more)

### Community 2 - "Community 2"
Cohesion: 0.15
Nodes (13): Config, config_path(), load_config(), save_config(), Args, b64(), futures_task_noop_waker(), is_ignored_file() (+5 more)

### Community 3 - "Community 3"
Cohesion: 0.18
Nodes (6): XPCBridge, BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice()

### Community 4 - "Community 4"
Cohesion: 0.19
Nodes (7): attach_tray_status_listener(), sync_status(), FileEntry, FileStatus, StateDb, test_list_by_status(), test_upsert_and_get_file()

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
- **12 isolated node(s):** `VersionInfo`, `ConflictRecord`, `Resolution`, `FileEntry`, `LockContents` (+7 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 14`** (10 nodes): `ApiClient`, `.chunk_url()`, `.download_chunk()`, `.get_file()`, `.list_files()`, `.master_key()`, `.new()`, `.upload_chunk()`, `test_chunk_url()`, `api_client.rs`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 18`** (9 nodes): `ApiClient`, `.create_folder()`, `.list_files()`, `.require_auth()`, `.upload_encrypted()`, `.url()`, `.verify_session()`, `parse_response()`, `api.rs`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 37`** (6 nodes): `callbacks.rs`, `mod.rs`, `fetch_data_callback()`, `bridge()`, `register_sync_root()`, `set_bridge()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `run()` connect `Community 0` to `Community 33`, `Community 2`, `Community 4`?**
  _High betweenness centrality (0.042) - this node is a cross-community bridge._
- **Why does `sweep_auto_resolutions()` connect `Community 1` to `Community 0`?**
  _High betweenness centrality (0.039) - this node is a cross-community bridge._
- **Are the 4 inferred relationships involving `run()` (e.g. with `.init()` and `.new()`) actually correct?**
  _`run()` has 4 INFERRED edges - model-reasoned connections that need verification._
- **Are the 5 inferred relationships involving `run()` (e.g. with `.acquire()` and `.open()`) actually correct?**
  _`run()` has 5 INFERRED edges - model-reasoned connections that need verification._
- **Are the 4 inferred relationships involving `main()` (e.g. with `run()` and `.new()`) actually correct?**
  _`main()` has 4 INFERRED edges - model-reasoned connections that need verification._
- **What connects `VersionInfo`, `ConflictRecord`, `Resolution` to the rest of the system?**
  _12 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.06 - nodes in this community are weakly interconnected._