# Beebeeb File Provider

Modern macOS File Provider foundation for the Finder drive surface.

## Scope

This target is intentionally thin:

- registers a `Beebeeb` File Provider domain from the containing app;
- presents ONE root folder — `Beebeeb`, the user's own files (task 1701 ruling:
  no synthetic `My files` / `Shared with me` / `Offline` / `Conflicts` split;
  shared files stay in the webapp for now);
- forwards file enumeration and hydration to the Rust daemon over the local Unix socket;
- maps daemon permission bits to Finder read/write/rename/delete capabilities.

The Rust daemon owns auth, unlock state, encrypted names, uploads, downloads,
version anchors, permissions, cache state, and queue persistence.

## Current Release Contract

Current macOS identifiers:

- containing app: `io.beebeeb.app`
- extension: `io.beebeeb.app.FileProvider`
- domain: `io.beebeeb.app.domain`
- document/app group: `R8352WDJJR.io.beebeeb.app.fileprovider`

The visible Finder integration must be the File Provider domain, not a fake
local folder root. The user's local path is only cache/offline state controlled
by the daemon.

Known blocker: Finder installation times out when the app or extension is not
signed with provisioning profiles that include the exact document/app group
above. Treat entitlement rejection from `trustd`, `taskgated`, or File Provider
logs as a signing/provisioning issue, not a daemon protocol issue.

## Local Build Check

The local bundle build is:

```sh
scripts/build-fileprovider-extension.sh
```

It creates:

```text
src-tauri/target/fileprovider/BeebeebFileProvider.appex
src-tauri/target/fileprovider/BeebeebFileProviderCtl
```

Tauri embeds that generated bundle into
`Beebeeb.app/Contents/PlugIns/BeebeebFileProvider.appex` during macOS bundle
creation.

After a local app build, sign the nested extension before the parent app:

```sh
scripts/sign-local-macos-app.sh \
  src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Beebeeb.app
```

The source-level check is still useful while editing Swift files:

```sh
xcrun swiftc -typecheck \
  -target arm64-apple-macos14.0 \
  BeebeebFileProvider/*.swift
```

## Extension sources

| File | Role |
| --- | --- |
| `FileProviderExtension.swift` | `NSFileProviderReplicatedExtension` + `NSFileProviderThumbnailing` (task 1699), enumerator, item/change/pending-item callbacks, trash routing, hydrate staging. |
| `FileProviderItem.swift` | `BeebeebProviderItem` wire model (Codable) + `FileProviderItem` system item, incl. `NSFileProviderItemDecorating` badge mapping (task 1699). |
| `XPCBridge.swift` | Typed daemon IPC client (enumerate, item, hydrate, write queue, changes, anchor, thumbnails), error mapping to `NSFileProviderErrorDomain`. |
| `IPCFraming.swift` | Line-delimited JSON framing, timeouts, cancellation, write-request builder (checked by `scripts/check-ipc-timeouts.py`). |
| `WorkingSetStore.swift` | App-Group persistence for sync anchors, materialized containers, change filtering. |
| `Info.plist` | Extension declaration, pipeline depths, `NSFileProviderDecorations` + badge UTI declarations, domain usage string. |
| `Resources/badge-*.png` | Badge icons referenced by `UTImportedTypeDeclarations`; generated deterministically by `scripts/gen-badge-pngs.py` and copied into `Contents/Resources` by the build script. |

Behavioural changes to this target land RED-first through the headless harness
(`BeebeebFileProviderTests/main.swift`, run by `scripts/test-ipc-framing.sh`;
the pass count is enforced by `EXPECTED_TESTS` in that script).

## Sync badges and thumbnails (task 1699)

- Sync badges: macOS renders at most the FIRST badge per item in the `Badge`
  category, so `FileProviderItem` maps each sync status to exactly one
  decoration, priority error > conflict > trashing > uploading > downloading.
  Identifiers (`io.beebeeb.app.decoration.*`) and badge UTIs
  (`io.beebeeb.app.badge.*`) are declared in `Info.plist`.
- Thumbnails: the extension stages decrypted plaintext via the daemon
  `FetchThumbnail` RPC into the App-Group hydrate cache, reads it back, and
  deletes the staging file (wire contract: `docs/IPC_PROTOCOL.md`). Thumbnails
  never bump `contentVersion` — the system keys its thumbnail cache on it.
- `NSExtensionFileProviderAppliesChangesAtomically` is deliberately NOT
  declared; the rationale is a comment in `Info.plist`.

## BeebeebFileProviderCtl

`BeebeebFileProviderCtl` is our own manual diagnostic tool (source:
`BeebeebFileProviderTools/DomainControlTool.swift`, built to
`src-tauri/target/fileprovider/BeebeebFileProviderCtl`). It is NOT used by the
app or the extension; it exists for humans debugging domain state. It is
unrelated to Apple's system `/usr/bin/fileproviderctl`.

Verbs (default `status`):

| Verb | Effect | Exit codes |
| --- | --- | --- |
| `status` | Print `installed` or `missing` for domain `io.beebeeb.app.domain` ("Drive"). | 0 always (informational); 1 on FileProvider error |
| `install` | `NSFileProviderManager.add` for the domain. | 0 installed; 1 error |
| `remove` | `NSFileProviderManager.remove(_:mode: .preserveDirtyUserData)` for the domain: files that never reached the server are kept (task 1882). Prints `removed`, then `preserved: <folder>` only when that folder holds something (round 2: macOS reports a folder even when it kept nothing); otherwise `nothing kept: macOS reported <folder>, which is missing` (or `… empty`). | 0 removed; 1 error |
| `signal-root` | `signalEnumerator(.rootContainer)` for the installed domain. | 0 signaled; 1 not installed or error |

Unknown verbs exit 2. `scripts/qa-fileprovider.sh` wraps the SYSTEM
`fileproviderctl` (dump/diagnose/evaluate) plus `pluginkit` for a health
report; this ctl is the domain-level counterpart.

If this later moves to a full Xcode target, keep the same contract:

- extension point: `com.apple.fileprovider-nonui`;
- principal class: `$(PRODUCT_MODULE_NAME).FileProviderExtension`;
- bundle id: `io.beebeeb.app.FileProvider`;
- document group: `R8352WDJJR.io.beebeeb.app.fileprovider`;
- app-group entitlement shared with the containing Beebeeb app;
- the same Apple Team ID and provisioning profile entitlement values as the
  containing Beebeeb app;
- matching keychain-access-group entitlement only if the extension ever reads
  shared Keychain items. The current design keeps secrets in the daemon.

The extension binary must be a Mach-O executable, not a shared library. The
`main.swift` entrypoint calls `NSExtensionMain`; without that, macOS cannot
attach sandbox/app-group entitlements to the launched provider process.

## Install / Remount Flow

Finder registration must be called by the containing app, not manually from
the extension process:

> Task 1697 note: the `BeebeebFileProviderDomain` helper (DomainRegistration.swift)
> was retired. The same operations are available today via `BeebeebFileProviderCtl`
> (see above) — `install`, `remove`, `signal-root` — driven by the containing app
> or by hand during development. The Swift snippets below are the historical
> in-app flow.

```swift
BeebeebFileProviderDomain.install { error in
    // surface error in the control center
}
```

To force Finder to re-enumerate after daemon metadata changes:

```swift
BeebeebFileProviderDomain.signalRootEnumerator { error in
    // best effort; Finder will also re-query on next open
}
```

For local remount during development:

1. Quit Beebeeb.
2. Remove the existing domain with `BeebeebFileProviderDomain.remove`.
3. Build and run the signed containing app with the File Provider extension
   embedded.
4. Call `BeebeebFileProviderDomain.install`.
5. Open Finder and select the Beebeeb location.

Full Finder verification requires a signed containing app and an embedded
extension target with matching app-group entitlements. Source type-checking
alone does not install a domain.

## Production Domain Contract

The domain identifier is stable release state:

```text
io.beebeeb.app.domain
```

Do not change it after public release without an explicit migration plan. A
different identifier creates a new Finder location and can orphan user state
from the old domain.

Release verification for domain survival:

1. Install the signed and notarized DMG into a clean macOS user.
2. Launch Beebeeb, sign in, unlock, and call `BeebeebFileProviderDomain.install`.
3. Confirm Finder shows the `Beebeeb` location with the user's real top-level
   files at its root (single root, no synthetic subfolders — task 1701).
4. Reboot the Mac and confirm the same domain remains visible.
5. Install a newer signed build over the old build and confirm the domain does
   not duplicate.
6. Lock/unlock the vault and confirm enumeration remains available while
   hydration/write operations require unlock.

Useful inspection commands:

```sh
pluginkit -m -v -p com.apple.fileprovider-nonui | grep -A3 -i Beebeeb
log show --predicate 'subsystem CONTAINS "fileprovider" OR process CONTAINS "Beebeeb"' \
  --last 20m --style compact
```

## Uninstall / Sign-out Expectations

Uninstall removes local integration state only. It must never delete remote
vault data.

**Every domain removal keeps what has not synced (task 1882, P0).** Sign-out,
Repair, the Add-to-Finder rollback, the app-start sweep of other domains and
`BeebeebFileProviderCtl remove` all call
`removeDomain:mode:NSFileProviderDomainRemovalModePreserveDirtyUserData`
(macOS 12+; the app requires 14), never the plain `removeDomain:` (which deleted
Finder-created files that had not uploaded). macOS reports a folder, even when it
kept nothing, so the bridge checks it first (it must exist and hold at least one
entry; spec §5). Only then does the app show the person that folder: an alert
after a sign-out, the add rollback or the app-start sweep, and a row in Settings →
Sync that stays until the person dismisses it (the latest kept folder is saved in
`desktop.toml` as `kept_unsynced_folder`). Logs say only that files were kept,
never the path. `src-tauri/src/finder_removal.rs` pins every removal in the repo to this
mode. Limit: macOS counts a create or modify as synced once the extension has
handed it to the app's upload queue, and sign-out purges that queue, so a file
still queued at sign-out is not kept. Spec:
`docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md`.

Safe order:

1. Pause sync and surface pending uploads.
2. Remove the File Provider domain with `BeebeebFileProviderDomain.remove`
   (today: the preserving removal above).
3. Disable the login item in the containing app.
4. Stop the daemon/control center.
5. Remove stale IPC socket and lock files.
6. Remove local cache/state only after pending upload handling is explicit.
7. Delete Keychain session/unlock material only on sign-out, not simple app
   removal.

Until a signed uninstall helper exists, avoid documenting one-line destructive
`rm -rf` commands for app-support directories. The daemon must own that cleanup
because it can tell the difference between disposable cache and unsynced local
work.
