# Graph Report - desktop-1202  (2026-07-25)

## Corpus Check
- 119 files · ~298,701 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1985 nodes · 4045 edges · 25 communities detected
- Extraction: 84% EXTRACTED · 16% INFERRED · 0% AMBIGUOUS · INFERRED: 658 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 15|Community 15]]
- [[_COMMUNITY_Community 27|Community 27]]
- [[_COMMUNITY_Community 33|Community 33]]
- [[_COMMUNITY_Community 41|Community 41]]
- [[_COMMUNITY_Community 42|Community 42]]
- [[_COMMUNITY_Community 43|Community 43]]
- [[_COMMUNITY_Community 49|Community 49]]
- [[_COMMUNITY_Community 50|Community 50]]
- [[_COMMUNITY_Community 66|Community 66]]
- [[_COMMUNITY_Community 71|Community 71]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 66 edges
2. `load()` - 63 edges
3. `StateDb` - 50 edges
4. `command()` - 44 edges
5. `EngineBridge` - 42 edges
6. `load()` - 36 edges
7. `run()` - 30 edges
8. `test_bridge_with_api()` - 25 edges
9. `api_client_from_session()` - 24 edges
10. `commandUnavailableLabel()` - 24 edges

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
Nodes (272): load(), ensure_directory(), ipc_socket_path(), serve_ipc(), legacy_platform_keychain_store(), migrate_legacy_keychain_to_account(), platform_keychain_store_for(), about_metadata() (+264 more)

### Community 1 - "Community 1"
Cohesion: 0.03
Nodes (145): is_conflict(), is_text_file(), apply_metadata_file_row(), apply_shared_context(), apply_snapshot(), apply_sync_op(), blurhash_for_source(), bootstrap_from_snapshot() (+137 more)

### Community 2 - "Community 2"
Cohesion: 0.04
Nodes (40): AndroidKeyboard(), IOSKeyboard(), account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S> (+32 more)

### Community 3 - "Community 3"
Cohesion: 0.03
Nodes (63): bandwidth_samples_insert_and_history(), bandwidth_samples_prune(), BandwidthSample, count_queue_groups(), decode_os_state(), decode_os_state_status_only_for_user_owned_states(), delete_file_subtree_of_a_plain_file_removes_only_that_row(), delete_file_subtree_prunes_descendants_when_folder_row_has_leading_slash() (+55 more)

### Community 4 - "Community 4"
Cohesion: 0.03
Nodes (95): account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity, AccountActivityEvent, AccountActivitySummary, AccountProfile (+87 more)

### Community 5 - "Community 5"
Cohesion: 0.03
Nodes (80): engine_internal_filters_state_dir_and_lock(), relative_db_path_is_slash_joined_without_leading_slash(), session_zeroizes_master_key_on_drop(), build_macos_file_provider_bridge(), main(), db_placeholder_path(), decide_size_action(), fail_transfer() (+72 more)

### Community 6 - "Community 6"
Cohesion: 0.04
Nodes (20): ApiClient, CreatedSession, DesktopUploadInitRequest, DesktopUploadInitResponse, header_secs(), header_secs_parses_numeric_and_rejects_dates(), heartbeat_body_minimal_serializes_status_only(), heartbeat_body_skips_none_optionals_but_keeps_zero() (+12 more)

### Community 7 - "Community 7"
Cohesion: 0.03
Nodes (60): AccountConfig, AccountId, AccountRuntime, active_account_after_synthesis_is_default_shape(), active_account_empty_registry_errs(), fixed_id(), synthesize_is_idempotent(), synthesize_single_account() (+52 more)

### Community 8 - "Community 8"
Cohesion: 0.03
Nodes (60): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), toggleFolder(), formatBytes() (+52 more)

### Community 9 - "Community 9"
Cohesion: 0.04
Nodes (30): FileProviderEnumerator, FileProviderExtension, BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts (+22 more)

### Community 10 - "Community 10"
Cohesion: 0.05
Nodes (54): accountActivity(), accountActivityFeed(), accountClientSessions(), accountDevices(), accountNotificationPreferences(), accountNotifications(), accountProfile(), accountRegion() (+46 more)

### Community 11 - "Community 11"
Cohesion: 0.06
Nodes (30): backup_dest_root(), backup_source_key_for_rel_path(), backup_source_key_for_this_device(), classify_copy(), classify_skip_when_size_and_mtime_match(), copy_preserving_mtime(), CopyDecision, dest_root_builds_backup_device_folder() (+22 more)

### Community 12 - "Community 12"
Cohesion: 0.09
Nodes (32): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), Session, Args, b64() (+24 more)

### Community 13 - "Community 13"
Cohesion: 0.13
Nodes (28): is_ignored_finder_name(), path_is_engine_internal(), relative_db_path(), assert_uploading_row(), dispatch_local_create(), engine_delete_suppress(), engine_delete_suppression_distinguishes_paths(), engine_delete_suppression_is_consumed_once() (+20 more)

### Community 14 - "Community 14"
Cohesion: 0.11
Nodes (18): decrypt_payload(), device_code_recv_errors_immediately_on_closed_connection(), device_code_recv_returns_promptly_on_healthy_connection(), device_code_recv_times_out_when_server_stalls_after_connect(), emit(), HealthyStream, open_browser(), recv_text() (+10 more)

### Community 15 - "Community 15"
Cohesion: 0.17
Nodes (20): capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path(), FileProviderItemPayload, handle_connection() (+12 more)

### Community 27 - "Community 27"
Cohesion: 0.33
Nodes (3): DomainControlTool, install_file_provider_domain(), remove_file_provider_domain()

### Community 33 - "Community 33"
Cohesion: 0.39
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 41 - "Community 41"
Cohesion: 0.48
Nodes (6): buffer_to_string(), call_bridge(), install(), remove(), status(), visible_url()

### Community 42 - "Community 42"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 43 - "Community 43"
Cohesion: 0.33
Nodes (2): hasExcludedDescendant(), triStateFor()

### Community 49 - "Community 49"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

### Community 50 - "Community 50"
Cohesion: 0.7
Nodes (3): consumeStoredNav(), initialNav(), navIdFromString()

### Community 66 - "Community 66"
Cohesion: 0.67
Nodes (1): StatusUiClassFactory_Impl

### Community 71 - "Community 71"
Cohesion: 1.0
Nodes (1): StatusUiSourceFactory_Impl

## Knowledge Gaps
- **128 isolated node(s):** `namespace`, `folder`, `file`, `myFiles`, `sharedWithMe` (+123 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 43`** (7 nodes): `SelectiveSyncView.tsx`, `collectExcludedIds()`, `computeTotals()`, `hasExcludedDescendant()`, `setsEqual()`, `TriCheckbox()`, `triStateFor()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 49`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 66`** (3 nodes): `StatusUiClassFactory_Impl`, `.CreateInstance()`, `.LockServer()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 71`** (2 nodes): `StatusUiSourceFactory_Impl`, `.GetStatusUISource()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `run()` connect `Community 0` to `Community 1`, `Community 2`, `Community 4`, `Community 7`, `Community 9`, `Community 12`?**
  _High betweenness centrality (0.115) - this node is a cross-community bridge._
- **Why does `load()` connect `Community 0` to `Community 8`?**
  _High betweenness centrality (0.112) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 8` to `Community 0`, `Community 9`, `Community 10`?**
  _High betweenness centrality (0.107) - this node is a cross-community bridge._
- **Are the 62 inferred relationships involving `load()` (e.g. with `.do_upload_version()` and `.enforce_configured_cache_limit()`) actually correct?**
  _`load()` has 62 INFERRED edges - model-reasoned connections that need verification._
- **Are the 2 inferred relationships involving `command()` (e.g. with `handleInstall()` and `refresh()`) actually correct?**
  _`command()` has 2 INFERRED edges - model-reasoned connections that need verification._
- **What connects `namespace`, `folder`, `file` to the rest of the system?**
  _128 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.02 - nodes in this community are weakly interconnected._