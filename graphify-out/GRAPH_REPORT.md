# Graph Report - desktop-1521  (2026-09-25)

## Corpus Check
- 136 files · ~335,160 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 2275 nodes · 4736 edges · 29 communities detected
- Extraction: 84% EXTRACTED · 16% INFERRED · 0% AMBIGUOUS · INFERRED: 766 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 16|Community 16]]
- [[_COMMUNITY_Community 17|Community 17]]
- [[_COMMUNITY_Community 18|Community 18]]
- [[_COMMUNITY_Community 30|Community 30]]
- [[_COMMUNITY_Community 36|Community 36]]
- [[_COMMUNITY_Community 44|Community 44]]
- [[_COMMUNITY_Community 45|Community 45]]
- [[_COMMUNITY_Community 48|Community 48]]
- [[_COMMUNITY_Community 49|Community 49]]
- [[_COMMUNITY_Community 53|Community 53]]
- [[_COMMUNITY_Community 55|Community 55]]
- [[_COMMUNITY_Community 70|Community 70]]
- [[_COMMUNITY_Community 76|Community 76]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 75 edges
2. `load()` - 67 edges
3. `load()` - 67 edges
4. `EngineBridge` - 52 edges
5. `StateDb` - 50 edges
6. `command()` - 46 edges
7. `run()` - 30 edges
8. `test_bridge_with_api()` - 29 edges
9. `api_client_from_session()` - 25 edges
10. `commandUnavailableLabel()` - 24 edges

## Surprising Connections (you probably didn't know these)
- `emit()` --calls--> `openActivity()`  [INFERRED]
  src-tauri/src/browser_login.rs → src/WindowsTray.tsx
- `run()` --calls--> `main()`  [INFERRED]
  src-tauri/src/runner.rs → windows/src/main.rs
- `run_one_scan()` --calls--> `load()`  [INFERRED]
  src-tauri/src/watcher.rs → src/pages/SelectiveSync.tsx
- `run_one_scan()` --calls--> `load()`  [INFERRED]
  src-tauri/src/watcher.rs → src/windows/views/ActivityView.tsx
- `handle_settled_path()` --calls--> `guess_mime_type()`  [INFERRED]
  src-tauri/src/watcher.rs → windows/src/main.rs

## Communities

### Community 0 - "Community 0"
Cohesion: 0.01
Nodes (300): load(), ensure_directory(), linux_thumbnail_source_path_for_entry(), ipc_socket_path(), serve_ipc(), serve_ipc_at(), legacy_platform_keychain_store(), migrate_legacy_keychain_to_account() (+292 more)

### Community 1 - "Community 1"
Cohesion: 0.02
Nodes (195): is_conflict(), is_text_file(), apply_metadata_file_row(), apply_shared_context(), apply_shared_metadata_file_row(), apply_snapshot(), apply_sync_op(), audit_1244_apply_sync_op_trash_echo_preserves_local_trashing_owner() (+187 more)

### Community 2 - "Community 2"
Cohesion: 0.02
Nodes (134): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), toggleFolder(), openRoot() (+126 more)

### Community 3 - "Community 3"
Cohesion: 0.03
Nodes (46): AndroidKeyboard(), IOSKeyboard(), account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S> (+38 more)

### Community 4 - "Community 4"
Cohesion: 0.03
Nodes (63): bandwidth_samples_insert_and_history(), bandwidth_samples_prune(), BandwidthSample, count_queue_groups(), decode_os_state(), decode_os_state_status_only_for_user_owned_states(), delete_file_subtree_of_a_plain_file_removes_only_that_row(), delete_file_subtree_prunes_descendants_when_folder_row_has_leading_slash() (+55 more)

### Community 5 - "Community 5"
Cohesion: 0.03
Nodes (97): account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity, AccountActivityEvent, AccountActivitySummary, AccountProfile (+89 more)

### Community 6 - "Community 6"
Cohesion: 0.03
Nodes (95): AccountConfig, AccountId, AccountRuntime, active_account_after_synthesis_is_default_shape(), active_account_empty_registry_errs(), fixed_id(), synthesize_is_idempotent(), synthesize_single_account() (+87 more)

### Community 7 - "Community 7"
Cohesion: 0.03
Nodes (34): api_client_list_files_sends_provenance_headers_on_the_wire(), ApiClient, CreatedSession, DesktopUploadInitRequest, DesktopUploadInitResponse, header_secs(), header_secs_parses_numeric_and_rejects_dates(), HeaderMockServer (+26 more)

### Community 8 - "Community 8"
Cohesion: 0.03
Nodes (48): FileProviderEnumerator, FileProviderExtension, BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts (+40 more)

### Community 9 - "Community 9"
Cohesion: 0.05
Nodes (39): auto_resolution_deadline(), ConflictRecord, Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed(), v() (+31 more)

### Community 10 - "Community 10"
Cohesion: 0.06
Nodes (30): backup_dest_root(), backup_source_key_for_rel_path(), backup_source_key_for_this_device(), classify_copy(), classify_skip_when_size_and_mtime_match(), copy_preserving_mtime(), CopyDecision, dest_root_builds_backup_device_folder() (+22 more)

### Community 11 - "Community 11"
Cohesion: 0.11
Nodes (26): api_client_search_shard_urls_match_web_paths(), build_local_index(), compare_records_for_query(), DesktopSearchIndexState, DesktopSearchResponse, DesktopSearchResult, FixtureSearchStore, FixtureState (+18 more)

### Community 12 - "Community 12"
Cohesion: 0.12
Nodes (29): is_ignored_finder_name(), path_is_engine_internal(), relative_db_path(), assert_uploading_row(), debounce_loop(), dispatch_local_create(), engine_delete_suppress(), engine_delete_suppression_distinguishes_paths() (+21 more)

### Community 13 - "Community 13"
Cohesion: 0.11
Nodes (20): build_ws_request(), decrypt_payload(), device_code_recv_errors_immediately_on_closed_connection(), device_code_recv_returns_promptly_on_healthy_connection(), device_code_recv_times_out_when_server_stalls_after_connect(), emit(), HealthyStream, open_browser() (+12 more)

### Community 14 - "Community 14"
Cohesion: 0.15
Nodes (22): capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract(), file_status_string(), filename_from_path(), FileProviderItemPayload, handle_connection() (+14 more)

### Community 15 - "Community 15"
Cohesion: 0.13
Nodes (14): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), Session, Args, b64() (+6 more)

### Community 16 - "Community 16"
Cohesion: 0.17
Nodes (15): remove_linux_freedesktop_thumbnails_for_evicted_files(), encode_freedesktop_png(), file_uri(), fixture_png(), FreedesktopThumbnailSize, md5_hex(), png_output_is_rgba_resized_and_embeds_uri_and_mtime_text_chunks(), remove_freedesktop_thumbnails() (+7 more)

### Community 17 - "Community 17"
Cohesion: 0.25
Nodes (18): archive_legacy_state_dir_if_present(), beebeeb_state_dir_from_app_local_data(), copy_dir_all(), copy_dir_contents_no_clobber(), copy_file(), fresh_sync_root_uses_app_local_state_dir(), init_from_app(), migrate_legacy_state_dir() (+10 more)

### Community 18 - "Community 18"
Cohesion: 0.34
Nodes (13): commandForTauriCli(), decodeMinisignBase64Line(), decodeTauriBase64Text(), fail(), generateKeypair(), main(), parseTauriPubkey(), parseTauriSignature() (+5 more)

### Community 30 - "Community 30"
Cohesion: 0.33
Nodes (3): DomainControlTool, file_provider_installed(), install_file_provider_domain()

### Community 36 - "Community 36"
Cohesion: 0.39
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 44 - "Community 44"
Cohesion: 0.48
Nodes (6): buffer_to_string(), call_bridge(), install(), remove(), status(), visible_url()

### Community 45 - "Community 45"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 48 - "Community 48"
Cohesion: 0.33
Nodes (1): ApiClient

### Community 49 - "Community 49"
Cohesion: 0.47
Nodes (4): availableSettingsSections(), settingLabel(), shellIntegrationLabel(), withPlatformLabels()

### Community 53 - "Community 53"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

### Community 55 - "Community 55"
Cohesion: 0.7
Nodes (3): consumeStoredNav(), initialNav(), navIdFromString()

### Community 70 - "Community 70"
Cohesion: 0.67
Nodes (1): StatusUiClassFactory_Impl

### Community 76 - "Community 76"
Cohesion: 1.0
Nodes (1): StatusUiSourceFactory_Impl

## Knowledge Gaps
- **145 isolated node(s):** `daemonUnavailable`, `namespace`, `folder`, `file`, `myFiles` (+140 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 48`** (6 nodes): `ApiClient`, `.delete_shard()`, `.fetch_manifest()`, `.get_shard()`, `.master_key_bytes()`, `.put_shard()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 53`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 70`** (3 nodes): `StatusUiClassFactory_Impl`, `.CreateInstance()`, `.LockServer()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 76`** (2 nodes): `StatusUiSourceFactory_Impl`, `.GetStatusUISource()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `load()` connect `Community 0` to `Community 1`, `Community 2`, `Community 12`, `Community 14`?**
  _High betweenness centrality (0.131) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 2` to `Community 8`, `Community 0`?**
  _High betweenness centrality (0.109) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 1`, `Community 3`, `Community 5`, `Community 6`, `Community 8`, `Community 15`, `Community 17`?**
  _High betweenness centrality (0.072) - this node is a cross-community bridge._
- **Are the 66 inferred relationships involving `load()` (e.g. with `start_engine_if_possible()` and `clear_session_impl()`) actually correct?**
  _`load()` has 66 INFERRED edges - model-reasoned connections that need verification._
- **Are the 66 inferred relationships involving `load()` (e.g. with `start_engine_if_possible()` and `clear_session_impl()`) actually correct?**
  _`load()` has 66 INFERRED edges - model-reasoned connections that need verification._
- **What connects `daemonUnavailable`, `namespace`, `folder` to the rest of the system?**
  _145 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.01 - nodes in this community are weakly interconnected._