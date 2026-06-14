# Graph Report - desktop  (2026-06-14)

## Corpus Check
- 89 files · ~155,309 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1007 nodes · 1689 edges · 19 communities detected
- Extraction: 84% EXTRACTED · 16% INFERRED · 0% AMBIGUOUS · INFERRED: 273 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 22|Community 22]]
- [[_COMMUNITY_Community 40|Community 40]]
- [[_COMMUNITY_Community 41|Community 41]]
- [[_COMMUNITY_Community 44|Community 44]]
- [[_COMMUNITY_Community 47|Community 47]]

## God Nodes (most connected - your core abstractions)
1. `load()` - 35 edges
2. `ApiClient` - 33 edges
3. `EngineBridge` - 28 edges
4. `StateDb` - 27 edges
5. `commandUnavailableLabel()` - 22 edges
6. `run()` - 17 edges
7. `now_secs()` - 15 edges
8. `test_bridge()` - 14 edges
9. `run()` - 13 edges
10. `FileProviderExtension` - 12 edges

## Surprising Connections (you probably didn't know these)
- `install_windows_shell_integration()` --calls--> `load()`  [INFERRED]
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
Cohesion: 0.03
Nodes (117): DomainControlTool, load(), default_sync_root_suggestion(), default_sync_root_uses_private_file_provider_state_folder(), default_sync_root_uses_user_facing_beebeeb_folder(), DesktopConfig, DesktopSettings, ensure_directory() (+109 more)

### Community 1 - "Community 1"
Cohesion: 0.04
Nodes (69): apply_metadata_file_row(), apply_shared_context(), CacheCleanupOutcome, CachePolicy, classify_operation_error(), classify_review_operation(), ConflictDetected, decrypt_downloaded_chunk() (+61 more)

### Community 2 - "Community 2"
Cohesion: 0.04
Nodes (43): diagnostics(), lock(), runAction(), signOut(), toggleStartAtLogin(), unlock(), toggleFolder(), formatBytes() (+35 more)

### Community 3 - "Community 3"
Cohesion: 0.05
Nodes (27): test_metadata_modify_does_not_queue_content_version(), count_queue_groups(), ensure_column(), FileContractState, FileEntry, has_column(), ItemKind, Namespace (+19 more)

### Community 4 - "Community 4"
Cohesion: 0.07
Nodes (16): AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S>, delete(), find_item(), load(), MacOsKeychainStore (+8 more)

### Community 5 - "Community 5"
Cohesion: 0.09
Nodes (8): ApiClient, DesktopUploadInitRequest, DesktopUploadInitResponse, ListFilesScope, test_chunk_url(), test_desktop_contract_urls_trim_base_and_encode_queries(), test_metadata_operation_urls(), UploadChunkResponse

### Community 6 - "Community 6"
Cohesion: 0.08
Nodes (19): auto_resolution_deadline(), ConflictRecord, is_conflict(), is_text_file(), Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed() (+11 more)

### Community 7 - "Community 7"
Cohesion: 0.11
Nodes (18): BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts, myFiles, offline (+10 more)

### Community 8 - "Community 8"
Cohesion: 0.12
Nodes (15): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), read_http_request(), Session, Args (+7 more)

### Community 9 - "Community 9"
Cohesion: 0.13
Nodes (7): FileProviderEnumerator, FileProviderExtension, FileProviderItem, NSFileProviderEnumerator, NSFileProviderItem, NSFileProviderReplicatedExtension, NSObject

### Community 10 - "Community 10"
Cohesion: 0.12
Nodes (12): Config, config_path(), load_config(), save_config(), debug_output_redacts_session_token_and_key_material(), install_and_clear_session_round_trips_through_store(), MemoryStore, unlock_rejects_wrong_sized_master_key_and_remains_locked() (+4 more)

### Community 11 - "Community 11"
Cohesion: 0.2
Nodes (19): capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path(), FileProviderItemPayload, handle_connection() (+11 more)

### Community 12 - "Community 12"
Cohesion: 0.17
Nodes (19): install_windows_shell_integration(), fail_transfer(), fetch_data_callback(), mark_in_sync(), normalized_path(), secure_remove_temp(), transfer_one(), transfer_range() (+11 more)

### Community 15 - "Community 15"
Cohesion: 0.31
Nodes (4): ApiClient, parse_response(), attach_tray_status_listener(), FileStatus

### Community 22 - "Community 22"
Cohesion: 0.42
Nodes (9): decrypt_payload(), emit(), open_browser(), recv_text(), run_handoff(), signin_log(), start_browser_login(), webpki_rustls_connector() (+1 more)

### Community 40 - "Community 40"
Cohesion: 0.48
Nodes (6): buffer_to_string(), call_bridge(), install(), remove(), status(), visible_url()

### Community 41 - "Community 41"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 44 - "Community 44"
Cohesion: 0.47
Nodes (4): hostname_or_unknown(), LockContents, LockFile, pid_alive()

### Community 47 - "Community 47"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

## Knowledge Gaps
- **47 isolated node(s):** `namespace`, `folder`, `file`, `myFiles`, `sharedWithMe` (+42 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 47`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `load()` connect `Community 0` to `Community 2`, `Community 12`, `Community 6`?**
  _High betweenness centrality (0.081) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 2` to `Community 0`?**
  _High betweenness centrality (0.071) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 4`, `Community 8`, `Community 9`, `Community 10`, `Community 15`?**
  _High betweenness centrality (0.063) - this node is a cross-community bridge._
- **Are the 34 inferred relationships involving `load()` (e.g. with `start_engine_if_possible()` and `start_engine_for_pending_finder_install()`) actually correct?**
  _`load()` has 34 INFERRED edges - model-reasoned connections that need verification._
- **Are the 21 inferred relationships involving `commandUnavailableLabel()` (e.g. with `toggle()` and `refresh()`) actually correct?**
  _`commandUnavailableLabel()` has 21 INFERRED edges - model-reasoned connections that need verification._
- **What connects `namespace`, `folder`, `file` to the rest of the system?**
  _47 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.03 - nodes in this community are weakly interconnected._