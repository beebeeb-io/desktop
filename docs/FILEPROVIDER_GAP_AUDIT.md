# macOS File Provider gap audit — our extension vs the Apple contract

Date: 2026-10-01 · Codebase state: desktop main `f21d279` · Sources: macOS 15.4 + 27.0 SDK headers
(`FileProvider.framework/Headers`), Apple developer docs, WWDC21 session 10182 transcript, live probes
(`fileproviderctl`, `pluginkit`, compile tests), and a full codebase inventory of
`BeebeebFileProvider/` + `src-tauri/src/ipc_socket.rs` + `macos_file_provider.rs`.

Purpose: every File Provider surface point, what Apple's contract says, what we do today, and the gap.
Feeds tasks 1697 (P0), 1698 (P1), 1699 (P2). Cross-refs: 1694 (done), 1695, 1696.

## 1. Architecture verdict

We implement **`NSFileProviderReplicatedExtension`** (`FileProviderExtension.swift:5`) — the FPFS
architecture (macOS 11+): the system replicates a local replica on disk; items are created dataless and
content/children are fetched on first access. This is the correct and only macOS architecture
(`NSFileProviderExtension` itself is iOS-only — proven by compile error on both SDKs). The transport
below it (Unix socket + peer-cred + framed JSON + idempotent write queue) is sound.

**The verdict: the foundation is right, but the replica's change machinery is stubbed.** Apple's
enumerator contract states `enumerateChanges`/`currentSyncAnchor` are "marked optional for historical
reasons, but are really required. System performance will be severely degraded if they are not
implemented" (`Enum.h:170–202`). Ours return nothing. There is no daemon→extension change signal at all.
Under Replicated, the ONLY working signal channel is `.workingSet` — signaling any other container
"is ignored" (`Mgr.h:85–90`). Our dead `DomainRegistration.swift:25` signals `.rootContainer`, which
Replicated ignores by design.

## 2. Root causes of the observed behavior (Guus hand test, 0.8.7 alpha)

| Observation | Root cause (audit refs) |
|---|---|
| New domain volume empty; 8195 cloud-only files never materialized | Enumerator returns one full listing per container with no paging/anchor; once the system's initial import consumes it, nothing ever updates it (G1/G2). Verify the `namespace:my_files` data path returns real rows (G5 repro gate). |
| Opened file worked but "slow, no progress" | The opened file was one of 4 already-local rows — no `fetchContents` ran, so no Progress existed to render. On a cloud-only file, our `fetchContents` Progress (indeterminate → streaming byte counts, cancellable) already matches Apple's contract — Finder renders the returned NSProgress (WWDC21). 1695's fix gate: open a CLOUD-ONLY file. |
| Changes never appear until re-open | No `enumerateChanges`, constant empty anchor, no `.workingSet` signal path (G1). |
| 🚫 when dropping files into folders | Missing add-subitems capability — fixed in 1694 (merged `662184b`). |
| Move/rename between folders, trash — also blocked | `allowsReparenting` and `allowsTrashing` never mapped (G5). |
| Zombie "Beebeeb-Drive" volume + sidebar shows "Beebeeb" while we register "Drive" | Stale domain `io.beebeeb.desktop.FileProvider` in fileproviderd's DB from the pre-rename bundle id (1696); displayName mismatch is its residue (G8). |

## 3. Conformance table

Legend: ✅ conforms · 🟡 partial · ❌ missing/stub · ⚫ not applicable (API doesn't exist — see §5)

| Surface point | Apple contract | Our state | Gap |
|---|---|---|---|
| Principal class | `NSFileProviderReplicatedExtension` (macOS 11+) | ✅ `FileProviderExtension.swift:5` | — |
| Domain | `NSFileProviderDomain(identifier:displayName:)` (replicated init); `pathRelativeToDocumentStorage` init is iOS-only ⚫ | ✅ Rust FFI `FileProviderBridge.m` installs; displayName "Drive" | G8: stale domains not cleaned; displayName truth |
| `item(for:request:)` | metadata lookup, seconds budget; unknown ⇒ `noSuchItem` | ✅ via `GetFileStatus`; trivial Progress(1) | fine |
| Item identity | stable identifiers mandatory | ✅ file_id based | — |
| `contentType` | **required on macOS**; folders `UTType.folder` | ✅ mapped | — |
| `capabilities` | gates every Finder action; aliases: addingSubItems==writing, contentEnumerating==reading | 🟡 5 bits after 1694 | G5: `allowsReparenting`, `allowsTrashing` unmapped; eviction via contentPolicy (G6) |
| `documentSize` | logical document size | 🟡 files only | fine (folders use childItemCount) |
| `childItemCount` | informational | ❌ hardcoded 0 | G4 |
| `creationDate`/`contentModificationDate` | item dates Finder renders | ❌ absent from payload + item | G4 |
| `isUploaded`/`isUploading` | **gates eviction** ("system will only evict a file that you report as uploaded"); `uploadingError` for quota/network | 🟡 status-string driven | G6 (error fields missing) |
| `isDownloaded`/`isDownloading` | **ignored under Replicated** (inferred from fetchContents) | 🟡 status-string driven | cosmetic only — document, don't fix |
| `isMostRecentVersionDownloaded` | stale-vs-server signal | 🟡 mirrors isDownloaded | G1 (fold into version model) |
| `itemVersion` | contentVersion change ⇒ re-download + thumbnail cache invalidation; metadataVersion stored-but-ignored | 🟡 content==metadata, synthesized | G1/G4 (split when change log lands) |
| `contentPolicy` (macOS 13+) | `.inherited` / `.downloadLazily` (root default) / `.downloadLazilyAndEvictOnRemoteUpdate` / `.downloadEagerlyAndKeepDownloaded` (pinning) | ❌ absent | G6: native offline pinning — replaces our unused `SetRecursivePin` RPC concept |
| `userInfo`, `lastUsedDate`, `tagData`, sharing fields | working-set/Spotlight inputs | ❌ absent | P2 |
| `enumerator(for:)` | return enumerator; system keeps them open for presented items | ✅ fresh per container | — |
| `enumerateItems(from:startingAt:)` | paged; page token ≤500 bytes; **stable sort across pages**; honor `suggestedPageSize` (100× clamp) | ❌ token ignored, single full listing, no ordering | G2 |
| `enumerateChanges(for:from:)` + `currentSyncAnchor` | **"really required"; anchor = cursor into provider change log; expiry ⇒ system drops caches + full rescan ("very expensive")** | ❌ stub: empty anchor, no changes ever | G1 — the core gap |
| `signalEnumerator` | Replicated: **only `.workingSet` is honored**; system propagates to UI; call on every remote change | ❌ none effective (dead `.rootContainer` call; daemon `file-provider-invalidate` Tauri event has zero consumers) | G1 |
| `materializedItemsDidChange…` + `enumeratorForMaterializedItems` | track materialized dirs; filter working-set updates — "If the extension doesn't keep track of the materialized set… the working set is the entire dataset" | ❌ absent | G3 (mandatory with 8195 files) |
| `pendingItemsDidChange…` (11.3+) | pending-set refresh | ❌ absent | P2 |
| `fetchContents(for:version:request:…)` | return NSProgress (Finder renders it); complete only after bytes on disk; same-volume temp staging; `requestedVersion` "currently always nil" | ✅ rich: App Group staging → system temp copy, cancellable, byte-count units (1670) | version arg ignored — acceptable per Apple |
| `createItem(basedOn:fields:contents:options:…)` | idempotent replay via template identifier; `mayAlreadyExist`/`deletionConflicted` options; stillPendingFields; progress covers upload | 🟡 QueueFinderCreate + dedup table; options ignored | 🟡 acceptable now; revisit on conflicts (G7) |
| `modifyItem(_:baseVersion:changedFields:…)` | baseVersion conflict detection; return surviving id on merge | 🟡 QueueFinderModify; baseVersion folded into write key | 🟡 acceptable now |
| `deleteItem(identifier:baseVersion:options:…)` | **only called for items already in Trash**; `DirectoryNotEmpty` on non-recursive; unknown ⇒ report success | 🟡 queues server `TrashFile` for any delete | G7: trash semantics decision (`supportsSyncingTrash` default YES on 13+) |
| Trash | macOS 13+: system reparents trashed items to `trashContainer` when `supportsSyncingTrash=YES` (default) | ❌ no trashContainer handling | G7 |
| Thumbnails (`NSFileProviderThumbnailing`) | `fetchThumbnails(for:requestedSize:…)`, system cache keyed on contentVersion | ❌ absent (server HAS thumbnails — 1692) | G11 (P2) |
| Custom actions / decorations / UserInteractions | Info.plist-driven badges & pre-flight alerts | ❌ absent | G12/G14 (P2) |
| Errors | domain must be `NSFileProviderErrorDomain`/Cocoa; `serverUnreachable` = transient+backoff; `cannotSynchronize` = **definitive, no retry**; `signalErrorResolved` clears throttle | 🟡 framing maps `invalidResponse`→`cannotSynchronize` | G9: transient transport glitches must not be definitive |
| Info.plist | documented keys: DocumentGroup, SupportsEnumeration, Download/UploadPipelineDepth, Actions; header keys: AppliesChangesAtomically, AllowsUserControlledEviction, … | 🟡 depths 8/8 set; SupportsEnumeration=true | G14: consider AppliesChangesAtomically (our staging is atomic) |
| Testing | `testingModes` + `listAvailableTestingOperations` (needs testing-mode entitlement, **irreversible**); `fileproviderctl dump/diagnose/evaluate/check`; `waitForStabilization` | ❌ none | G15 (P2, QA only) |
| Dead code | — | `DomainRegistration.swift`, `SetFileStatus` no-op stub, `RecordOpenedFile`/`EnforceSmartCache`/`SetRecursivePin` RPCs (no caller), CTL unused by app | G13 (P2) |

## 4. Phased plan

### P0 — make the replica actually sync (task 1697)
1. **Daemon change log + anchor RPC** (Rust): monotonic change cursor per domain (file_id, kind
   created/modified/deleted, parent, timestamp) persisted in state.db; new IPC
   `ListChanges { since_anchor } → { changes[], next_anchor }`; anchor = cursor id (opaque bytes ≤500).
2. **Swift enumerator conformance**: implement `currentSyncAnchor` (read from daemon, persist in App
   Group for crash-recovery), `enumerateChanges(for:from:)` (drive from `ListChanges`; deletions via
   `didDeleteItems`; `finishEnumeratingChanges(upTo:moreComing:)` with real anchors), paged
   `enumerateItems` (server-ordered pages, ≤500-byte resume tokens, honor `suggestedPageSize`,
   stable `sortedByName` ordering).
3. **Signal path**: daemon (app process, via existing FFI bridge) calls
   `signalEnumerator(.workingSet)` after each applied operation batch — the only honored container on
   Replicated. Retire `file-provider-invalidate` Tauri event + dead `DomainRegistration`.
4. **Materialized-set tracking**: `materializedItemsDidChange` + `enumeratorForMaterializedItems`;
   daemon tracks materialized container set (App Group DB); working-set signals filtered to changes
   whose old-or-new parent is materialized.
5. **Item metadata**: dates (creation/modification from state.db), real `childItemCount` for folders,
   split `itemVersion` content vs metadata (content = version hash; metadata = mtime/size/parent/name).
6. **Capabilities completion**: map `allowsReparenting` + `allowsTrashing` (Rust `CAP_REPARENT/CAP_TRASH`
   bits + Swift mapping, same pattern as 1694).
7. **Repro gates (RED-first tests)**: empty-volume repro (8195-file tree enumerates into Finder);
   cloud-only open → fetchContents → visible progress; Finder reflects a daemon-side rename/create/
   delete without re-open; move between folders; trash works.

### P1 — correctness & UX (task 1698)
1. **contentPolicy integration** (macOS 13+; our minimumSystemVersion is 14.0): pinned/offline folders =
   `.downloadEagerlyAndKeepDownloaded`; root `.downloadLazily`; wire the Offline namespace to the pin
   set; retirement or repurposing of `SetRecursivePin`.
2. **Trash semantics**: decide `supportsSyncingTrash` (default YES) → implement trashContainer
   reparenting + `deleteItem` per contract (empty-trash path), or explicitly opt out. Server already
   has a trash model — align.
3. **Stale domain cleanup** (1696): at app start, `getDomains` + `remove` any domain whose identifier
   isn't ours; assert displayName truth afterwards.
4. **Error truth**: `invalidResponse`/transport glitches → transient (`serverUnreachable`) not
   definitive `cannotSynchronize`; add `signalErrorResolved` after reconnect; `uploadingError`/
   `downloadingError` surfaces for quota/network states.

### P2 — polish (task 1699)
1. Thumbnails via `NSFileProviderThumbnailing` (server thumbnails exist since 1692; system cache keys
   on contentVersion — pairs with the P0 version split).
2. Sync-state badges (`NSFileProviderDecorations` + `ItemDecorating`), pending-set notifications.
3. Dead-code removal (G13 list), Info.plist `AppliesChangesAtomically`, QA harness (`fileproviderctl
   evaluate/check` in scripts; testingModes only if entitlement approved — irreversible).

## 5. Explicit "does not exist" ledger (verified 0-hit across SDK 15.4 + 27.0, docs, GitHub)

Do not chase these in future work: `NSFileProviderExtension` on macOS (iOS-only); `usedSpaceBytes`;
`parentItemIdentifiers` (plural); `isExcludedFromSync` (the real mechanism is `allowsExcludingFromSync`
capability + `excludedFromSync` error); `providesFileModificationDates`; `mostRecentEditorName` (only
`…NameComponents`); `versionIdentifier` on macOS (use `itemVersion`); old-style contentPolicy cases
(`publicContent`… — real API is the `NSFileProviderContentPolicy` enum, macOS 13+);
`NSFileProviderEvictionPolicy`; capabilities `allowsSharing`/`allowsTagging`/`allowsSettingAttributes`/
`allowsSubItemsRenaming`/`allowsSubItemsDeleting`/`allowsDownloading`/`allowsUploading`;
`NSFileProviderEnumerator.registerObservers`; `NSFileProviderManager.getUserEnabled` (it's the domain's
`userEnabled` property); `importUsingCoreServices` (it's `importDomain(fromDirectoryAtURL:)`);
`NSFileProviderError.staleVersion` (real codes: `syncAnchorExpired`, `versionNoLongerAvailable`);
`testingAlwaysAllowed` (real: `testingModes`); Info.plist `NSExtensionFileProviderActionTypes` /
`NSExtensionFileProviderIsPrimaryFileProvider` / `NSExtensionFileProviderDocumentTypes`;
`fileproviderctl list/enumerate/evict` verbs (real verbs: dump, diagnose, evaluate, check, repair,
obfuscate).