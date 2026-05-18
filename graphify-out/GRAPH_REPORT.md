# Graph Report - desktop  (2026-05-18)

## Corpus Check
- 82 files · ~130,011 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 895 nodes · 1412 edges · 19 communities detected
- Extraction: 86% EXTRACTED · 14% INFERRED · 0% AMBIGUOUS · INFERRED: 194 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 13|Community 13]]
- [[_COMMUNITY_Community 14|Community 14]]
- [[_COMMUNITY_Community 38|Community 38]]
- [[_COMMUNITY_Community 39|Community 39]]
- [[_COMMUNITY_Community 40|Community 40]]
- [[_COMMUNITY_Community 41|Community 41]]
- [[_COMMUNITY_Community 44|Community 44]]
- [[_COMMUNITY_Community 47|Community 47]]

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
- `get_selective_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `set_selective_sync()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `run()` --calls--> `main()`  [INFERRED]
  src-tauri/src/runner.rs → windows/src/main.rs
- `start_engine_if_possible()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx
- `desktop_login()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/pages/SelectiveSync.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.05
Nodes (85): load(), default_sync_root_suggestion(), DesktopConfig, DesktopSettings, ensure_directory(), ipc_socket_path(), serve_ipc(), account_email() (+77 more)

### Community 1 - "Community 1"
Cohesion: 0.04
Nodes (66): apply_metadata_file_row(), apply_shared_context(), CacheCleanupOutcome, CachePolicy, classify_operation_error(), classify_review_operation(), ConflictDetected, decrypt_downloaded_chunk() (+58 more)

### Community 2 - "Community 2"
Cohesion: 0.05
Nodes (26): test_metadata_modify_does_not_queue_content_version(), count_queue_groups(), ensure_column(), FileContractState, FileEntry, has_column(), ItemKind, Namespace (+18 more)

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
Cohesion: 0.08
Nodes (19): auto_resolution_deadline(), ConflictRecord, is_conflict(), is_text_file(), Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed() (+11 more)

### Community 8 - "Community 8"
Cohesion: 0.13
Nodes (7): FileProviderEnumerator, FileProviderExtension, FileProviderItem, NSFileProviderEnumerator, NSFileProviderItem, NSFileProviderReplicatedExtension, NSObject

### Community 9 - "Community 9"
Cohesion: 0.2
Nodes (19): capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path(), FileProviderItemPayload, handle_connection() (+11 more)

### Community 10 - "Community 10"
Cohesion: 0.16
Nodes (13): Config, config_path(), load_config(), save_config(), Args, b64(), futures_task_noop_waker(), guess_mime_type() (+5 more)

### Community 13 - "Community 13"
Cohesion: 0.31
Nodes (4): ApiClient, parse_response(), attach_tray_status_listener(), FileStatus

### Community 14 - "Community 14"
Cohesion: 0.28
Nodes (5): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice()

### Community 38 - "Community 38"
Cohesion: 0.29
Nodes (1): MemoryStore

### Community 39 - "Community 39"
Cohesion: 0.38
Nodes (4): hostname_or_unknown(), LockContents, LockFile, pid_alive()

### Community 40 - "Community 40"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 41 - "Community 41"
Cohesion: 0.48
Nodes (1): DomainControlTool

### Community 44 - "Community 44"
Cohesion: 0.33
Nodes (2): fetch_data_callback(), bridge()

### Community 47 - "Community 47"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

## Knowledge Gaps
- **44 isolated node(s):** `daemonUnavailable`, `namespace`, `folder`, `file`, `myFiles` (+39 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 38`** (7 nodes): `MemoryStore`, `.delete_session_token()`, `.delete_wrapped_master_key()`, `.load_session_token()`, `.load_wrapped_master_key()`, `.save_session_token()`, `.save_wrapped_master_key()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 41`** (7 nodes): `DomainControlTool`, `.install()`, `.main()`, `.remove()`, `.signalRoot()`, `.status()`, `DomainControlTool.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 44`** (6 nodes): `callbacks.rs`, `mod.rs`, `fetch_data_callback()`, `bridge()`, `register_sync_root()`, `set_bridge()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 47`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `load()` connect `Community 0` to `Community 3`?**
  _High betweenness centrality (0.071) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 8`, `Community 10`, `Community 13`?**
  _High betweenness centrality (0.070) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 3` to `Community 0`?**
  _High betweenness centrality (0.069) - this node is a cross-community bridge._
- **Are the 22 inferred relationships involving `load()` (e.g. with `start_engine_if_possible()` and `finder_location_state()`) actually correct?**
  _`load()` has 22 INFERRED edges - model-reasoned connections that need verification._
- **Are the 6 inferred relationships involving `run()` (e.g. with `.init()` and `.new()`) actually correct?**
  _`run()` has 6 INFERRED edges - model-reasoned connections that need verification._
- **What connects `daemonUnavailable`, `namespace`, `folder` to the rest of the system?**
  _44 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.05 - nodes in this community are weakly interconnected._