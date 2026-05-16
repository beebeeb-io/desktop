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

There is not yet an Xcode project or extension target in this repository, so
the local source-level check is:

```sh
xcrun swiftc -typecheck \
  -target arm64-apple-macos14.0 \
  BeebeebFileProvider/*.swift
```

When the Xcode target is added, include these files in a File Provider
Extension target with:

- extension point: `com.apple.fileprovider-nonui`;
- principal class: `$(PRODUCT_MODULE_NAME).FileProviderExtension`;
- bundle id: `io.beebeeb.desktop.FileProvider`;
- app-group entitlement shared with the containing Beebeeb app if the socket
  path moves out of `/tmp`.

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
