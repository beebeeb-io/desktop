# Graph Report - desktop-flow7-6  (2026-09-26)

## Corpus Check
- 150 files · ~361,658 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 2424 nodes · 5118 edges · 29 communities detected
- Extraction: 84% EXTRACTED · 16% INFERRED · 0% AMBIGUOUS · INFERRED: 803 edges (avg confidence: 0.8)
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
- [[_COMMUNITY_Community 28|Community 28]]
- [[_COMMUNITY_Community 34|Community 34]]
- [[_COMMUNITY_Community 42|Community 42]]
- [[_COMMUNITY_Community 43|Community 43]]
- [[_COMMUNITY_Community 46|Community 46]]
- [[_COMMUNITY_Community 47|Community 47]]
- [[_COMMUNITY_Community 48|Community 48]]
- [[_COMMUNITY_Community 52|Community 52]]
- [[_COMMUNITY_Community 53|Community 53]]
- [[_COMMUNITY_Community 55|Community 55]]
- [[_COMMUNITY_Community 70|Community 70]]
- [[_COMMUNITY_Community 79|Community 79]]

## God Nodes (most connected - your core abstractions)
1. `ApiClient` - 76 edges
2. `load()` - 70 edges
3. `load()` - 67 edges
4. `EngineBridge` - 62 edges
5. `StateDb` - 56 edges
6. `command()` - 47 edges
7. `test_bridge_with_api()` - 41 edges
8. `run()` - 30 edges
9. `api_client_from_session()` - 25 edges
10. `commandUnavailableLabel()` - 25 edges

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
Nodes (325): load(), ensure_directory(), linux_thumbnail_source_path_for_entry(), shared_metadata_uses_placeholder_for_authenticated_traversal_name(), serve_ipc(), serve_ipc_at(), legacy_platform_keychain_store(), migrate_legacy_keychain_to_account() (+317 more)

### Community 1 - "Community 1"
Cohesion: 0.02
Nodes (218): is_text_file(), apply_metadata_file_row(), apply_shared_context(), apply_shared_metadata_file_row(), apply_snapshot(), apply_sync_op(), assert_rename_then_update_in_one_tick_applies_update(), audit_1244_apply_sync_op_trash_echo_preserves_local_trashing_owner() (+210 more)

### Community 2 - "Community 2"
Cohesion: 0.02
Nodes (133): diagnostics(), lock(), refresh(), runAction(), toggleStartAtLogin(), unlock(), toggleFolder(), openRoot() (+125 more)

### Community 3 - "Community 3"
Cohesion: 0.03
Nodes (46): AndroidKeyboard(), IOSKeyboard(), account_email_absent_returns_none(), account_email_round_trips(), AuthSecretStore, AuthStoreError, AuthVault, AuthVault<S> (+38 more)

### Community 4 - "Community 4"
Cohesion: 0.03
Nodes (69): bandwidth_samples_insert_and_history(), bandwidth_samples_prune(), BandwidthSample, count_queue_groups(), decode_os_state(), decode_os_state_status_only_for_user_owned_states(), delete_file_subtree_of_a_plain_file_removes_only_that_row(), delete_file_subtree_prunes_descendants_when_folder_row_has_leading_slash() (+61 more)

### Community 5 - "Community 5"
Cohesion: 0.03
Nodes (106): account_activity_parses_events_and_summary(), account_profile_parses_full_shape(), account_profile_parses_minimal_shape(), account_sessions_parse(), AccountActivity, AccountActivityEvent, AccountActivitySummary, AccountProfile (+98 more)

### Community 6 - "Community 6"
Cohesion: 0.03
Nodes (88): AccountConfig, AccountId, AccountRuntime, active_account_after_synthesis_is_default_shape(), active_account_empty_registry_errs(), fixed_id(), synthesize_is_idempotent(), synthesize_single_account() (+80 more)

### Community 7 - "Community 7"
Cohesion: 0.03
Nodes (35): api_client_list_files_sends_provenance_headers_on_the_wire(), api_client_zeroizes_master_key_on_drop(), ApiClient, CreatedSession, DesktopUploadInitRequest, DesktopUploadInitResponse, header_secs(), header_secs_parses_numeric_and_rejects_dates() (+27 more)

### Community 8 - "Community 8"
Cohesion: 0.04
Nodes (87): is_ignored_finder_name(), path_is_engine_internal(), relative_db_path(), assert_uploading_row(), debounce_loop(), dispatch_local_create(), engine_delete_suppress(), engine_delete_suppression_distinguishes_paths() (+79 more)

### Community 9 - "Community 9"
Cohesion: 0.03
Nodes (50): FileProviderEnumerator, FileProviderExtension, BeebeebItemKind, file, folder, namespace, BeebeebNamespace, conflicts (+42 more)

### Community 10 - "Community 10"
Cohesion: 0.04
Nodes (56): resolve(), engine_internal_filters_state_dir_and_lock(), relative_db_path_is_slash_joined_without_leading_slash(), bind_ipc_listener(), capabilities_for_status(), file_entry_payload(), file_entry_payload_for_db(), file_entry_payload_without_contract() (+48 more)

### Community 11 - "Community 11"
Cohesion: 0.08
Nodes (33): BeebeebFS, dir_attr(), file_attr(), mount(), reply_read_slice(), Session, Args, b64() (+25 more)

### Community 12 - "Community 12"
Cohesion: 0.11
Nodes (26): api_client_search_shard_urls_match_web_paths(), build_local_index(), compare_records_for_query(), DesktopSearchIndexState, DesktopSearchResponse, DesktopSearchResult, FixtureSearchStore, FixtureState (+18 more)

### Community 13 - "Community 13"
Cohesion: 0.11
Nodes (20): build_ws_request(), decrypt_payload(), device_code_recv_errors_immediately_on_closed_connection(), device_code_recv_returns_promptly_on_healthy_connection(), device_code_recv_times_out_when_server_stalls_after_connect(), emit(), HealthyStream, open_browser() (+12 more)

### Community 14 - "Community 14"
Cohesion: 0.21
Nodes (13): encode_freedesktop_png(), file_uri(), fixture_png(), FreedesktopThumbnailSize, md5_hex(), png_output_is_rgba_resized_and_embeds_uri_and_mtime_text_chunks(), remove_freedesktop_thumbnails(), resize_rgba_to_fit() (+5 more)

### Community 15 - "Community 15"
Cohesion: 0.16
Nodes (11): auto_resolution_deadline(), ConflictRecord, is_conflict(), Resolution, test_conflict_detected_when_both_sides_changed(), test_no_conflict_when_both_sides_landed_on_same_hash(), test_no_conflict_when_only_local_changed(), test_no_conflict_when_only_remote_changed() (+3 more)

### Community 16 - "Community 16"
Cohesion: 0.34
Nodes (13): commandForTauriCli(), decodeMinisignBase64Line(), decodeTauriBase64Text(), fail(), generateKeypair(), main(), parseTauriPubkey(), parseTauriSignature() (+5 more)

### Community 28 - "Community 28"
Cohesion: 0.33
Nodes (3): DomainControlTool, file_provider_installed(), install_file_provider_domain()

### Community 34 - "Community 34"
Cohesion: 0.39
Nodes (6): dayLabel(), displayTitle(), humanizeType(), looksLikeId(), parseDate(), relativeTime()

### Community 42 - "Community 42"
Cohesion: 0.48
Nodes (6): buffer_to_string(), call_bridge(), install(), remove(), status(), visible_url()

### Community 43 - "Community 43"
Cohesion: 0.38
Nodes (3): Inner, InodeMap, test_inode_assignment_stable()

### Community 46 - "Community 46"
Cohesion: 0.33
Nodes (1): ApiClient

### Community 47 - "Community 47"
Cohesion: 0.4
Nodes (2): localDayIndex(), planRenewalCopy()

### Community 48 - "Community 48"
Cohesion: 0.47
Nodes (4): availableSettingsSections(), settingLabel(), shellIntegrationLabel(), withPlatformLabels()

### Community 52 - "Community 52"
Cohesion: 0.5
Nodes (2): findColorViolations(), toKebabCase()

### Community 53 - "Community 53"
Cohesion: 0.4
Nodes (1): BeebeebFileProviderDomain

### Community 55 - "Community 55"
Cohesion: 0.7
Nodes (3): consumeStoredNav(), initialNav(), navIdFromString()

### Community 70 - "Community 70"
Cohesion: 0.67
Nodes (1): StatusUiClassFactory_Impl

### Community 79 - "Community 79"
Cohesion: 1.0
Nodes (1): StatusUiSourceFactory_Impl

## Knowledge Gaps
- **156 isolated node(s):** `daemonUnavailable`, `namespace`, `folder`, `file`, `myFiles` (+151 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **Thin community `Community 46`** (6 nodes): `ApiClient`, `.delete_shard()`, `.fetch_manifest()`, `.get_shard()`, `.master_key_bytes()`, `.put_shard()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 47`** (6 nodes): `localDayIndex()`, `planRenewalCopy()`, `planStatusTone()`, `quotaPercent()`, `titleCasePlan()`, `planPresentation.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 52`** (5 nodes): `findColorViolations()`, `isCommentLine()`, `listSourceFiles()`, `toKebabCase()`, `noHardcodedThemeColors.test.ts`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 53`** (5 nodes): `BeebeebFileProviderDomain`, `.install()`, `.remove()`, `.signalRootEnumerator()`, `DomainRegistration.swift`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 70`** (3 nodes): `StatusUiClassFactory_Impl`, `.CreateInstance()`, `.LockServer()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.
- **Thin community `Community 79`** (2 nodes): `StatusUiSourceFactory_Impl`, `.GetStatusUISource()`
  Too small to be a meaningful cluster - may be noise or needs more connections extracted.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `load()` connect `Community 0` to `Community 8`, `Community 1`, `Community 2`, `Community 10`?**
  _High betweenness centrality (0.161) - this node is a cross-community bridge._
- **Why does `commandUnavailableLabel()` connect `Community 2` to `Community 0`, `Community 9`?**
  _High betweenness centrality (0.148) - this node is a cross-community bridge._
- **Why does `run()` connect `Community 0` to `Community 1`, `Community 3`, `Community 5`, `Community 6`, `Community 9`, `Community 11`?**
  _High betweenness centrality (0.065) - this node is a cross-community bridge._
- **Are the 69 inferred relationships involving `load()` (e.g. with `start_engine_if_possible()` and `clear_session_impl()`) actually correct?**
  _`load()` has 69 INFERRED edges - model-reasoned connections that need verification._
- **Are the 66 inferred relationships involving `load()` (e.g. with `start_engine_if_possible()` and `clear_session_impl()`) actually correct?**
  _`load()` has 66 INFERRED edges - model-reasoned connections that need verification._
- **What connects `daemonUnavailable`, `namespace`, `folder` to the rest of the system?**
  _156 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.01 - nodes in this community are weakly interconnected._