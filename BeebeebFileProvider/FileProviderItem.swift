// FileProviderItem.swift
//
// SCAFFOLD STATUS: this file is the plan's exact code (per spec
// 2026-05-07-desktop-sync-client.md Phase 2 Task 5). Outside an Xcode
// extension target it is NOT expected to compile — the legacy
// `NSFileProviderItem` protocol's `typeIdentifier` and `isTrashed`
// properties were removed/renamed (`contentType: UTType`,
// `isTrashed -> presence in trash domain`) in newer macOS SDKs (12+).
// Whoever finalises the Xcode target setup should pick one of:
//
//   1. Set the extension's deployment target to macOS 11 (Big Sur) or
//      lower, where the legacy properties still exist.
//   2. Migrate to the modern `NSFileProviderReplicatedExtension` API +
//      `contentType: UTType` — preferred long-term, ~30 LOC of changes.
//
// For Phase 2 Task 5 ("scaffold"), this captures the design intent:
// per-file metadata wrapper, capabilities split by isLocal status,
// shared/uploaded flags reserved for future sharing UX. Refactor to the
// real SDK once the .xcodeproj is wired up and the deployment target is
// chosen.
//
// Per-file metadata wrapper for the macOS File Provider extension.
// Each `NSFileProviderItem` describes one entry the system might
// surface in Finder — the filename Finder displays, the size shown
// in Get-Info, the capabilities (can the user write? rename? delete?),
// and the badge state (uploading / downloaded / shared).
//
// Spec: docs/superpowers/plans/2026-05-07-desktop-sync-client.md
// Phase 2 Task 5.
//
// ## Capabilities policy
//
// - **Cloud-only files** (`isLocal == false`): read-only. Finder shows
//   the placeholder; double-clicking triggers `fetchContents` on the
//   extension which calls into the Rust daemon to hydrate.
// - **Local files** (`isLocal == true`): full read+write+rename+delete.
//   The Rust daemon owns the upload-on-change logic; the extension
//   just hands modified content back.
//
// Forward-compat fields (`isShared`, etc.) are pinned to constants for
// the v1 scaffold. They become dynamic when sharing UX lands.

import FileProvider

class FileProviderItem: NSObject, NSFileProviderItem {
    let itemIdentifier: NSFileProviderItemIdentifier
    let filename: String
    let typeIdentifier: String
    let capabilities: NSFileProviderItemCapabilities
    let documentSize: NSNumber?
    let isTrashed: Bool = false
    let isUploaded: Bool
    let isDownloaded: Bool
    let isShared: Bool = false

    /// `fileId`  — server-side UUID. Maps 1:1 to `files.file_id` in the
    ///             Rust state DB.
    /// `name`    — decrypted filename. Already decoded by the daemon
    ///             before being handed to the extension; the extension
    ///             never sees ciphertext.
    /// `size`    — plaintext byte count. Finder uses this for the size
    ///             column even before hydration.
    /// `isLocal` — `true` once `state == "local"` in the state DB; the
    ///             daemon flips it after a successful download or
    ///             upload completion.
    /// `mimeType`— used to derive a UTI for Finder. Pass `nil` to fall
    ///             back to `public.data` (a generic binary blob); the
    ///             daemon should pass `application/pdf` etc. when known.
    init(fileId: String, name: String, size: Int64, isLocal: Bool, mimeType: String?) {
        self.itemIdentifier = NSFileProviderItemIdentifier(fileId)
        self.filename = name
        self.typeIdentifier = mimeType ?? "public.data"
        self.documentSize = NSNumber(value: size)
        self.isUploaded = true       // file exists on the server
        self.isDownloaded = isLocal  // file exists locally
        self.capabilities = isLocal
            ? [.allowsReading, .allowsWriting, .allowsDeleting, .allowsRenaming]
            : [.allowsReading]
        super.init()
    }
}
