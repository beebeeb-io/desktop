# Graph Report - desktop  (2026-06-20)

## Corpus Check
- 108 files · ~278,759 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1689 nodes · 3277 edges · 22 communities detected
- Extraction: 84% EXTRACTED · 16% INFERRED · 0% AMBIGUOUS · INFERRED: 540 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 13|Community 13]]
- [[_COMMUNITY_Community 14|Community 14]]
- [[_COMMUNITY_Community 26|Community 26]]
- [[_COMMUNITY_Community 39|Community 39]]
- [[_COMMUNITY_Community 40|Community 40]]
- [[_COMMUNITY_Community 41|Community 41]]
- [[_COMMUNITY_Community 47|Community 47]]
- [[_COMMUNITY_Community 61|Community 61]]
- [[_COMMUNITY_Community 67|Community 67]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 61 edges
2. `load()` - 48 edges
3. `StateDb` - 46 edges
4. `command()` - 37 edges
5. `load()` - 36 edges
6. `EngineBridge` - 35 edges
7. `run()` - 24 edges
8. `commandUnavailableLabel()` - 22 edges
9. `api_client_from_session()` - 20 edges
10. `sync_tick()` - 19 edges

## Surprising Connections (you probably didn't know these)
- `get_known_folder_onboarding_seen()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/windows/views/ActivityView.tsx
- `mark_known_folder_onboarding_seen()` --calls--> `load()`  [INFERRED]
  src-tauri/src/lib.rs → src/windows/views/ActivityView.tsx
- `main()` --calls--> `run()`  [INFERRED]
  windows/src/main.rs → src-tauri/src/runner.rs
- `handle_connection()` --calls--> `load()`  [INFERRED]
  src-tauri/src/ipc_socket.rs → src/windows/views/ActivityView.tsx
- `run_known_folder_mirror()` --calls--> `load()`  [INFERRED]
  src-tauri/src/runner.rs → src/windows/views/ActivityView.tsx

## Communities

### Community 0 - "Community 0"
Cohesion: 0.02
Nodes (185): DomainControlTool, load(), ensure_directory(), ipc_socket_path(), serve_ipc(), legacy_platform_keychain_store(), migrate_legacy_keychain_to_account(), platform_keychain_store_for() (+177 more)

### Community 1 - "Community 1"
Cohesion: 0.03
Nodes (130): is_conflict(), is_text_file(), apply_metadata_file_row(), apply_shared_context(), apply_snapshot(), apply_sync_op(), bootstrap_from_snapshot(), CacheCleanupOutcome (+122 more)

### Community 2 - "Community 2"
Cohesion: 0.04
Nodes (40): AndroidKeyboard(), IOSKeyboard(), account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S> (+32 more)

### Community 3 - "Community 3"
Cohesion: 0.03
Nodes (92): AccountConfig, AccountId, AccountRuntime, active_account_after_synthesis_is_default_shape(), active_account_empty_registry_errs(), notification_preferences_parse_and_serialize_partial(), fixed_id(), synthesize_is_idempotent() (+84 more)

### Community 4 - "Community 4"
Cohesion: 0.04
Nodes (55): bandwidth_samples_insert_and_history(), bandwidth_samples_prune(), BandwidthSample, count_queue_groups(), decode_os_state(), decode_os_state_status_only_for_user_owned_states(), delete_file_subtree_of_a_plain_file_removes_only_that_row(), delete_file_subtree_prunes_descendants_when_folder_row_has_leading_slash() (+47 more)

### Community 5 - "Community 5"
Cohesion: 0.03
Nodes (77): account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity, AccountActivityEvent, AccountActivitySummary, AccountProfile (+69 more)

### Community 6 - "Community 6"
Cohesion: 0.04
Nodes (20): ApiClient, created_session_parses_minimal(), CreatedSession, DesktopUploadInitRequest, DesktopUploadInitResponse, header_secs(), header_secs_parses_numeric_and_rejects_dates(), HeartbeatBody (+12 more)

### Community 7 - "Community 7"
Cohesion: 0.04
Nodes (39): FileProviderEnumerator, FileProviderExtension, BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts (+31 more)

### Community 8 - "Community 8"
Cohesion: 0.04
Nodes (42): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), toggleFolder(), formatBytes() (+34 more)

### Community 9 - "Community 9"
Cohesion: 0.06
Nodes (47): accountActivity(), accountActivityFeed(), accountClientSessions(), accountDevices(), accountNotificationPreferences(), accountNotifications(), accountProfile(), accountRegion() (+39 more)

### Community 10 - "Community 10"
Cohesion: 0.06
Nodes (32): auto_resolution_deadline(), ConflictRecord, Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed(), v() (+24 more)

### Community 11 - "Community 11"
Cohesion: 0.06
Nodes (29): backup_dest_root(), backup_source_key_for_rel_path(), backup_source_key_for_this_device(), classify_copy(), classify_skip_when_size_and_mtime_match(), copy_preserving_mtime(), CopyDecision, dest_root_builds_backup_device_folder() (+21 more)

### Community 12 - "Community 12"
Cohesion: 0.13
Nodes (14): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), read_http_request(), Args, b64() (+6 more)

### Community 13 - "Community 13"
Cohesion: 0.17
Nodes (20): capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path(), FileProviderItemPayload, handle_connection() (+12 more)

### Community 14 - "Community 14"
Cohesion: 0.22
Nodes (3): ApiClient, parse_response(), MemoryStore

### Community 26 - "Community 26"
Cohesion: 0.33
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 39 - "Community 39"
Cohesion: 0.48
Nodes (6): buffer_to_string(), call_bridge(), install(), remove(), status(), visible_url()

### Community 40 - "Community 40"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 41 - "Community 41"
Cohesion: 0.33
Nodes (2): hasExcludedDescendant(), triStateFor()

### Community 47 - "Community 47"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

### Community 61 - "Community 61"
Cohesion: 0.67
Nodes (1): StatusUiClassFactory_Impl

### Community 67 - "Community 67"
Cohesion: 1.0
Nodes (1): StatusUiSourceFactory_Impl

## Knowledge Gaps
- **109 isolated node(s):** `namespace`, `folder`, `file`, `myFiles`, `sharedWithMe` (+104 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 41`** (7 nodes): `SelectiveSyncView.tsx`, `collectExcludedIds()`, `computeTotals()`, `hasExcludedDescendant()`, `setsEqual()`, `TriCheckbox()`, `triStateFor()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 47`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 61`** (3 nodes): `StatusUiClassFactory_Impl`, `.CreateInstance()`, `.LockServer()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 67`** (2 nodes): `StatusUiSourceFactory_Impl`, `.GetStatusUISource()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `run()` connect `Community 0` to `Community 1`, `Community 2`, `Community 3`, `Community 7`?**
  _High betweenness centrality (0.128) - this node is a cross-community bridge._
- **Why does `load()` connect `Community 0` to `Community 8`?**
  _High betweenness centrality (0.109) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 8` to `Community 0`, `Community 9`, `Community 7`?**
  _High betweenness centrality (0.109) - this node is a cross-community bridge._
- **Are the 47 inferred relationships involving `load()` (e.g. with `.upload_version()` and `.do_hydrate()`) actually correct?**
  _`load()` has 47 INFERRED edges - model-reasoned connections that need verification._
- **Are the 2 inferred relationships involving `command()` (e.g. with `handleInstall()` and `refresh()`) actually correct?**
  _`command()` has 2 INFERRED edges - model-reasoned connections that need verification._
- **Are the 35 inferred relationships involving `load()` (e.g. with `commandUnavailableLabel()` and `start_engine_if_possible()`) actually correct?**
  _`load()` has 35 INFERRED edges - model-reasoned connections that need verification._
- **What connects `namespace`, `folder`, `file` to the rest of the system?**
  _109 weakly-connected nodes found - possible documentation gaps or missing edges._