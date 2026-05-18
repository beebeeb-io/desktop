# Beebeeb File Provider

Modern macOS File Provider foundation for the Finder drive surface.

## Scope

This target is intentionally thin:

- registers a `Beebeeb` File Provider domain from the containing app;
- enumerates top-level namespaces: `My files`, `Shared with me`, `Offline`, `Conflicts`;
- forwards file enumeration and hydration to the Rust daemon over the local Unix socket;
- maps daemon permission bits to Finder read/write/rename/delete capabilities.

The Rust daemon owns auth, unlock state, encrypted names, uploads, downloads,
version anchors, permissions, cache state, and queue persistence.

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

If this later moves to a full Xcode target, keep the same contract:

- extension point: `com.apple.fileprovider-nonui`;
- principal class: `$(PRODUCT_MODULE_NAME).FileProviderExtension`;
- bundle id: `io.beebeeb.desktop.FileProvider`;
- document group: `R8352WDJJR.io.beebeeb.desktop.fileprovider`;
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
extension target. Source type-checking alone does not install a domain.

## Production Domain Contract

The domain identifier is stable release state:

```text
io.beebeeb.desktop.domain
```

Do not change it after public release without an explicit migration plan. A
different identifier creates a new Finder location and can orphan user state
from the old domain.

Release verification for domain survival:

1. Install the signed and notarized DMG into a clean macOS user.
2. Launch Beebeeb, sign in, unlock, and call `BeebeebFileProviderDomain.install`.
3. Confirm Finder shows the `Beebeeb` location and top-level namespaces.
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

Safe order:

1. Pause sync and surface pending uploads.
2. Remove the File Provider domain with `BeebeebFileProviderDomain.remove`.
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
