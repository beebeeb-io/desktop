# Graph Report - desktop  (2026-06-15)

## Corpus Check
- 92 files · ~177,247 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1170 nodes · 2055 edges · 17 communities detected
- Extraction: 83% EXTRACTED · 17% INFERRED · 0% AMBIGUOUS · INFERRED: 341 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 39|Community 39]]
- [[_COMMUNITY_Community 40|Community 40]]
- [[_COMMUNITY_Community 45|Community 45]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 50 edges
2. `load()` - 36 edges
3. `EngineBridge` - 31 edges
4. `StateDb` - 30 edges
5. `commandUnavailableLabel()` - 22 edges
6. `run()` - 19 edges
7. `api_client_from_session()` - 17 edges
8. `now_secs()` - 15 edges
9. `test_bridge()` - 14 edges
10. `AuthVault<S>` - 13 edges

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
Cohesion: 0.02
Nodes (160): DomainControlTool, load(), decrypt_payload(), emit(), open_browser(), recv_text(), run_handoff(), signin_log() (+152 more)

### Community 1 - "Community 1"
Cohesion: 0.04
Nodes (69): is_conflict(), is_text_file(), apply_shared_context(), CacheCleanupOutcome, CachePolicy, classify_operation_error(), classify_review_operation(), ConflictDetected (+61 more)

### Community 2 - "Community 2"
Cohesion: 0.04
Nodes (33): apply_metadata_file_row(), resolve_relative_path(), test_metadata_file_row_classifies_folder_rows(), test_metadata_file_row_composes_nested_path_under_parent(), test_metadata_file_row_decrypts_encrypted_name_into_path(), test_metadata_file_row_falls_back_when_name_undecryptable(), test_metadata_file_row_persists_contract_for_own_file(), count_queue_groups() (+25 more)

### Community 3 - "Community 3"
Cohesion: 0.06
Nodes (24): account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S>, clear_session_removes_account_email(), delete() (+16 more)

### Community 4 - "Community 4"
Cohesion: 0.04
Nodes (44): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), toggleFolder(), formatBytes() (+36 more)

### Community 5 - "Community 5"
Cohesion: 0.05
Nodes (28): FileProviderEnumerator, FileProviderExtension, BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts (+20 more)

### Community 6 - "Community 6"
Cohesion: 0.07
Nodes (8): ApiClient, DesktopUploadInitRequest, DesktopUploadInitResponse, ListFilesScope, test_chunk_url(), test_desktop_contract_urls_trim_base_and_encode_queries(), test_metadata_operation_urls(), UploadChunkResponse

### Community 7 - "Community 7"
Cohesion: 0.05
Nodes (49): account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity, AccountActivityEvent, AccountActivitySummary, AccountProfile (+41 more)

### Community 8 - "Community 8"
Cohesion: 0.09
Nodes (30): is_ignored_finder_name(), path_is_engine_internal(), relative_db_path(), Args, b64(), futures_task_noop_waker(), guess_mime_type(), is_ignored_file() (+22 more)

### Community 9 - "Community 9"
Cohesion: 0.08
Nodes (21): auto_resolution_deadline(), ConflictRecord, Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed(), v() (+13 more)

### Community 10 - "Community 10"
Cohesion: 0.14
Nodes (26): fail_transfer(), fetch_data_callback(), mark_in_sync(), normalized_path(), notify_delete_completion_callback(), notify_file_close_completion_callback(), notify_full_path(), notify_rename_completion_callback() (+18 more)

### Community 11 - "Community 11"
Cohesion: 0.2
Nodes (19): capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path(), FileProviderItemPayload, handle_connection() (+11 more)

### Community 12 - "Community 12"
Cohesion: 0.25
Nodes (6): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), read_http_request()

### Community 15 - "Community 15"
Cohesion: 0.31
Nodes (4): ApiClient, parse_response(), attach_tray_status_listener(), FileStatus

### Community 39 - "Community 39"
Cohesion: 0.48
Nodes (6): buffer_to_string(), call_bridge(), install(), remove(), status(), visible_url()

### Community 40 - "Community 40"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 45 - "Community 45"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

## Knowledge Gaps
- **74 isolated node(s):** `namespace`, `folder`, `file`, `myFiles`, `sharedWithMe` (+69 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 45`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `run()` connect `Community 0` to `Community 8`, `Community 5`, `Community 15`?**
  _High betweenness centrality (0.082) - this node is a cross-community bridge._
- **Why does `load()` connect `Community 0` to `Community 1`, `Community 4`, `Community 9`, `Community 7`?**
  _High betweenness centrality (0.073) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 4` to `Community 0`, `Community 5`?**
  _High betweenness centrality (0.069) - this node is a cross-community bridge._
- **Are the 35 inferred relationships involving `load()` (e.g. with `start_engine_if_possible()` and `start_engine_for_pending_finder_install()`) actually correct?**
  _`load()` has 35 INFERRED edges - model-reasoned connections that need verification._
- **Are the 21 inferred relationships involving `commandUnavailableLabel()` (e.g. with `toggle()` and `refresh()`) actually correct?**
  _`commandUnavailableLabel()` has 21 INFERRED edges - model-reasoned connections that need verification._
- **What connects `namespace`, `folder`, `file` to the rest of the system?**
  _74 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.02 - nodes in this community are weakly interconnected._