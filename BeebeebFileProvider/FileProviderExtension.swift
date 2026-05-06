// FileProviderExtension.swift
//
// SCAFFOLD STATUS: see FileProviderItem.swift. Same caveat applies —
// `NSFileProviderExtension` is deprecated in macOS 11+; modern code
// should subclass `NSFileProviderReplicatedExtension`. Plan code kept
// verbatim so the design intent is on disk; Xcode-target setup will
// either pin the deployment target to macOS 10.15 (where this compiles
// as-is) or migrate to the replicated extension API.
//
// Apple-side entry point of the BeebeebFileProvider extension. Registered
// via Info.plist's `NSExtensionPrincipalClass = $(PRODUCT_MODULE_NAME).FileProviderExtension`.
//
// Spec: docs/superpowers/plans/2026-05-07-desktop-sync-client.md
// Phase 2 Task 5.
//
// ## Lifecycle
//
// macOS spawns this process when Finder needs metadata or content for a
// file in the Beebeeb domain. We don't own a long-running connection —
// `XPCBridge.sendRequest` opens a new Unix socket connection to the Rust
// daemon for each call. The daemon owns the durable state (state.db,
// the actual encrypted blobs); the extension is a thin shim that
// translates Apple File Provider callbacks into JSON requests.
//
// ## Threading
//
// The completion handlers must be called on whatever queue Apple
// delivered the request on. We rely on `XPCBridge` doing its socket
// I/O synchronously — that's fine for a v1 scaffold; if hydration takes
// minutes, Apple will simply mark the file as "still loading" without
// blocking Finder's UI thread.

import FileProvider

class FileProviderExtension: NSFileProviderExtension {
    private let ipc = XPCBridge()

    /// Apple asks "tell me about this item" — usually before showing it
    /// in Finder, populating Get-Info, or deciding whether the user can
    /// drag it.
    override func item(
        for identifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, Error?) -> Void
    ) {
        ipc.getFileStatus(fileId: identifier.rawValue) { status, name, size in
            let isLocal = status == "local"
            let item = FileProviderItem(
                fileId: identifier.rawValue,
                name: name ?? identifier.rawValue,
                size: size,
                isLocal: isLocal,
                mimeType: nil
            )
            completionHandler(item, nil)
        }
    }

    /// Apple asks "deliver the file's contents now" — triggered when a
    /// user double-clicks a cloud-only placeholder, or when Spotlight
    /// indexes a file. We forward to the daemon's hydrate path; the
    /// daemon writes plaintext to the temporary URL we picked, then we
    /// hand that URL back to Apple along with refreshed metadata
    /// reflecting the new `local` status.
    override func fetchContents(
        for itemVersion: NSFileProviderItemVersion,
        request: NSFileProviderRequest,
        completionHandler: @escaping (URL?, NSFileProviderItem?, Error?) -> Void
    ) {
        let fileId = itemVersion.itemIdentifier.rawValue
        let destURL = NSFileProviderManager.default.temporaryDirectoryURL()
            .appendingPathComponent(fileId)

        ipc.hydrateFile(fileId: fileId, destPath: destURL.path) { error in
            if let error = error {
                completionHandler(nil, nil, error)
                return
            }
            self.ipc.getFileStatus(fileId: fileId) { status, name, size in
                _ = status // suppress unused — we know it's "local" or the hydrate failed
                let item = FileProviderItem(
                    fileId: fileId,
                    name: name ?? fileId,
                    size: size,
                    isLocal: true,
                    mimeType: nil
                )
                completionHandler(destURL, item, nil)
            }
        }
    }
}
