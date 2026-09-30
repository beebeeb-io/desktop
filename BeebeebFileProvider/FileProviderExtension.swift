import FileProvider
import Foundation
import UniformTypeIdentifiers

final class FileProviderExtension: NSObject, NSFileProviderReplicatedExtension {
    private let ipc = XPCBridge()
    private let domain: NSFileProviderDomain

    required init(domain: NSFileProviderDomain) {
        self.domain = domain
        super.init()
    }

    func item(
        for identifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        do {
            let model: BeebeebProviderItem
            if let namespace = BeebeebNamespace.allCases.first(where: { $0.identifier == identifier }) {
                model = .namespace(namespace)
            } else {
                model = try ipc.item(identifier: identifier)
            }
            completionHandler(FileProviderItem(model: model), nil)
        } catch {
            completionHandler(nil, error)
        }
        progress.completedUnitCount = 1
        return progress
    }

    func enumerator(
        for containerItemIdentifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest
    ) throws -> NSFileProviderEnumerator {
        FileProviderEnumerator(containerIdentifier: containerItemIdentifier, ipc: ipc)
    }

    func fetchContents(
        for itemIdentifier: NSFileProviderItemIdentifier,
        version requestedVersion: NSFileProviderItemVersion?,
        request: NSFileProviderRequest,
        completionHandler: @escaping (URL?, NSFileProviderItem?, Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)

        // Task 1670: MUST be the shared App Group container, not
        // `FileManager.default.temporaryDirectory` — see
        // `XPCBridge.hydrateDestinationURL(for:)`'s doc comment for why the
        // old destination could never pass the daemon's containment check.
        // The daemon still decrypts here — round 4 (below) only changes what
        // happens to this copy AFTER decryption, never where it lands.
        let destinationURL: URL
        switch XPCBridge.hydrateDestinationURL(for: itemIdentifier) {
        case .failure(let error):
            completionHandler(nil, nil, error)
            progress.completedUnitCount = 1
            return progress
        case .success(let url):
            destinationURL = url
        }

        // Task 1670 round 4 (Codex P1 on PR #75, review thread
        // PRRT_kwDOSLX6Xs6ncqQ7 on `ipc_socket.rs:462`): round 3 handed OUR
        // OWN App-Group staging URL straight to `fetchContents`'s
        // completionHandler and relied on a 2-minute TTL sweep
        // (`crate::ipc_socket::MACOS_HYDRATE_CACHE_TTL`) to eventually delete
        // it. Codex's finding, and it is correct: once `completionHandler` is
        // called, "the File Provider contract transfers control of that
        // local copy to the system; there is no documented maximum delay
        // before the system finishes consuming it" — so deleting that SAME
        // URL on any timer, however generous, can race a busy or suspended
        // `fileproviderd` and reopen this task's ORIGINAL "Couldn't
        // communicate with a helper application" bug.
        //
        // The fix: never hand our own staging URL to the system at all.
        // `copyToSystemTemporaryDirectory(stagedAt:)` below copies the
        // decrypted plaintext into `NSFileProviderManager(for:
        // domain).temporaryDirectoryURL()` — the system-managed, same-volume
        // directory Apple's own docs name specifically for this method (see
        // that function's doc comment for the exact quote) — under a fresh
        // name, and we delete OUR staging copy immediately afterward: we own
        // it, it was NEVER handed to anyone. `completionHandler` below is
        // called with the COPY's URL, never `destinationURL`; from that
        // point its lifetime belongs entirely to the system and this
        // extension never touches it again.
        //
        // The daemon's periodic hydrate-cache TTL sweep (`runner.rs`'s
        // `MACOS_HYDRATE_SWEEP_EVERY_N_TICKS`) and its sign-out/lock/startup
        // purge are UNCHANGED and still run — but with this fix, the ONLY
        // thing they can ever find in this directory is staging orphaned by
        // a crash between the decrypt above and the copy below (this
        // function never runs to completion), not a file the system is
        // relying on. They are now purely a crash backstop, not the primary
        // bound on plaintext lifetime that round 3 made them.
        func cleanupStagedPlaintext() {
            try? FileManager.default.removeItem(at: destinationURL)
        }

        do {
            try ipc.hydrateFile(itemIdentifier: itemIdentifier, destinationURL: destinationURL)
            let model = try ipc.item(identifier: itemIdentifier)

            // Copy the decrypted plaintext into the SYSTEM's own handoff
            // directory BEFORE we ever call completionHandler.
            let handoffURL = try copyToSystemTemporaryDirectory(stagedAt: destinationURL)

            // Our staging copy was never handed to the system — only
            // `handoffURL` is, below. Delete it now; there is nothing left
            // to race, because nothing outside this function has ever seen
            // `destinationURL`.
            cleanupStagedPlaintext()

            completionHandler(handoffURL, FileProviderItem(model: model), nil)
        } catch {
            // Nothing was ever handed to the system on this path —
            // `hydrateFile` failed, the item lookup failed, or the handoff
            // copy itself failed (which, on its own failure, already cleans
            // up any partial copy it made — see its doc comment). Either
            // way `completionHandler` below is called with `nil`, so nothing
            // else will ever clean up our staging copy at `destinationURL`.
            // Do it now; there is no race to lose here.
            cleanupStagedPlaintext()
            completionHandler(nil, nil, error)
        }

        progress.completedUnitCount = 1
        return progress
    }

    /// Task 1670 round 4: copy `sourceURL` (our own App-Group hydrate-cache
    /// staging file, decrypted moments ago) into the SYSTEM-managed directory
    /// `NSFileProviderManager(for: domain).temporaryDirectoryURL()` names
    /// specifically for `fetchContents`, under a fresh unique leaf name, and
    /// return that copy's URL. See `fetchContents`'s doc comment above for
    /// WHY this exists (Codex's P1 on PR #75: never hand our own staging URL
    /// to the system and then delete it on a timer).
    ///
    /// Apple's own docs for `temporaryDirectoryURL()` (developer.apple.com/
    /// documentation/fileprovider/nsfileprovidermanager/temporarydirectoryurl())
    /// — quoted verbatim: "The system guarantees that the temporary URL
    /// refers to a directory on the same volume as the user-visible URL so
    /// that the system can automatically clone or move files between the
    /// temporary URL and the user-visible URL... When you implement your
    /// File Provider extension's `fetchContents(...)` method, the URL you
    /// pass to the completion handler must be on the same volume as the
    /// temporary directory, so the system can clone it to provide the
    /// content for the dataless item." This is the API this class already
    /// holds everything it needs to call: `domain` is the SAME
    /// `NSFileProviderDomain` this extension was initialized with (`init(domain:)`
    /// above) — the one whose enumerator/item calls this whole class already
    /// serves.
    ///
    /// `FileManager.copyItem(at:to:)` is used rather than a raw
    /// `clonefile(2)` call: on APFS — the only filesystem either directory
    /// lives on here, both being under this Mac's boot volume — it is well
    /// known (WWDC "What's New in File Provider", not Apple's public
    /// `copyItem` API reference, which documents no such guarantee) to
    /// perform a copy-on-write clone rather than a byte-for-byte duplicate.
    /// Correctness here never depends on which one actually happens: the
    /// only property this function relies on is that an independent file
    /// exists at `destinationURL` before `fetchContents` removes
    /// `sourceURL`, which `copyItem` guarantees either way.
    ///
    /// On ANY failure — `temporaryDirectoryURL()` throwing, the copy itself
    /// failing, or the permissions fix-up failing — removes whatever partial
    /// copy it may have already created at `destinationURL` before
    /// rethrowing, so a broken or unreadable file never lingers in the
    /// system's own temp directory. Throws a `BeebeebIPCError`, which
    /// already bridges to `NSFileProviderErrorDomain` (see that type's
    /// `CustomNSError` conformance above) — a proper, supported
    /// `NSFileProviderError`, not the ad-hoc unsupported-domain error this
    /// whole task started from.
    ///
    /// **Round 5 (Codex P1 on PR #75, review thread `PRRT_kwDOSLX6Xs6ndhN1`
    /// on `ipc_socket.rs:468`): touches `sourceURL`'s mtime immediately
    /// before the copy, narrowing (not eliminating — see below) a real race
    /// with the daemon's mtime-based TTL sweep.** The daemon's sweep
    /// (`crate::ipc_socket::macos_sweep_stale_hydrate_cache_entries`) has no
    /// cross-process signal for "this file is mid-handoff" — it only ever
    /// looks at mtime. Codex's finding: if this extension is suspended by
    /// the system between `ipc.hydrateFile` finishing and this function
    /// actually running `copyItem`, or if `copyItem` itself takes longer
    /// than the TTL for a very large file, the daemon's sweep can delete
    /// `sourceURL` out from under an in-flight copy, reopening this task's
    /// original bug. Touching the mtime right here means a SLOW `copyItem`
    /// (Codex's second, fully-addressed trigger) gets the FULL TTL as
    /// headroom regardless of file size, and narrows the FIRST trigger
    /// (suspension before this line runs) to the width of ordinary thread
    /// scheduling between `ipc.hydrateFile`/`ipc.item` returning and this
    /// statement — not the width of an entire `copyItem` call. It does NOT
    /// theoretically eliminate an unboundedly long OS suspension landing in
    /// that narrower window; closing that completely would need a
    /// lease/heartbeat protocol between this extension and the daemon
    /// (flagged as a follow-up, out of scope for this patch). `try?`:
    /// best-effort — a failed touch should not block the real copy below.
    private func copyToSystemTemporaryDirectory(stagedAt sourceURL: URL) throws -> URL {
        guard let manager = NSFileProviderManager(for: domain) else {
            throw BeebeebIPCError.daemonUnavailable
        }
        let systemTempDir: URL
        do {
            systemTempDir = try manager.temporaryDirectoryURL()
        } catch {
            throw BeebeebIPCError.invalidResponse(
                "could not resolve the system handoff directory: \(error.localizedDescription)"
            )
        }

        // Reuse `sourceURL`'s leaf name: `XPCBridge.hydrateDestinationURL(for:)`
        // already validated it (`sanitizedHydrateFilename`) and gave it a
        // random per-request suffix, so it is both safe and already unique —
        // no need to derive a second name here. `isDirectory: false`: this
        // MUST be a path to a FILE, not a directory, per `fetchContents`'s
        // completionHandler contract.
        let destinationURL = systemTempDir.appendingPathComponent(sourceURL.lastPathComponent, isDirectory: false)

        // Round 5: see this function's doc comment above — refresh the
        // source's mtime immediately before the copy so the daemon's TTL
        // sweep measures age from "copy about to start", not "originally
        // decrypted", giving a slow copy the full TTL as headroom.
        try? FileManager.default.setAttributes([.modificationDate: Date()], ofItemAtPath: sourceURL.path)

        do {
            try FileManager.default.copyItem(at: sourceURL, to: destinationURL)
        } catch {
            // Round 5 (Codex P2 on PR #75, review thread `PRRT_kwDOSLX6Xs6ndhN-`
            // on this file): `copyItem` can create `destinationURL` and THEN
            // fail partway (ENOSPC, an I/O error mid-copy) — nothing else
            // ever cleans the SYSTEM's temp directory (the daemon's purge/
            // sweep only ever touches the App-Group hydrate-cache dir), so a
            // partial copy left here would linger indefinitely. Remove it,
            // exactly as the permissions-fixup catch below already does.
            try? FileManager.default.removeItem(at: destinationURL)
            throw BeebeebIPCError.invalidResponse("could not stage file for Finder: \(error.localizedDescription)")
        }

        do {
            // Owner-only: this directory is system-managed, not ours, so we
            // cannot assume its default permissions — set them explicitly,
            // same posture as the App-Group staging copy we just deleted.
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: destinationURL.path)
        } catch {
            try? FileManager.default.removeItem(at: destinationURL)
            throw BeebeebIPCError.invalidResponse("could not secure staged file for Finder: \(error.localizedDescription)")
        }

        return destinationURL
    }

    func createItem(
        basedOn itemTemplate: NSFileProviderItem,
        fields: NSFileProviderItemFields,
        contents url: URL?,
        options: NSFileProviderCreateItemOptions = [],
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        do {
            let contentType = itemTemplate.contentType
            let kind: BeebeebItemKind = contentType?.conforms(to: .folder) == true ? .folder : .file
            let result = try ipc.queueCreateItem(
                parentIdentifier: itemTemplate.parentItemIdentifier,
                filename: itemTemplate.filename,
                kind: kind,
                contentsURL: url,
                contentType: kind == .file ? contentType?.identifier : nil
            )
            if result.ignored {
                completionHandler(nil, [], false, nil)
            } else if let model = result.item {
                completionHandler(FileProviderItem(model: model), [], true, nil)
            } else {
                completionHandler(nil, [], true, nil)
            }
        } catch {
            completionHandler(nil, [], false, error)
        }
        progress.completedUnitCount = 1
        return progress
    }

    func modifyItem(
        _ item: NSFileProviderItem,
        baseVersion version: NSFileProviderItemVersion,
        changedFields: NSFileProviderItemFields,
        contents newContents: URL?,
        options: NSFileProviderModifyItemOptions = [],
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        do {
            let contentType = item.contentType
            let kind: BeebeebItemKind = contentType?.conforms(to: .folder) == true ? .folder : .file
            let result = try ipc.queueModifyItem(
                itemIdentifier: item.itemIdentifier,
                parentIdentifier: item.parentItemIdentifier,
                filename: item.filename,
                kind: kind,
                contentsURL: newContents,
                contentType: kind == .file ? contentType?.identifier : nil,
                baseVersionIdentifier: Self.versionIdentifier(version)
            )
            if result.ignored {
                completionHandler(nil, [], false, nil)
            } else if let model = result.item {
                completionHandler(FileProviderItem(model: model), [], true, nil)
            } else {
                completionHandler(nil, [], true, nil)
            }
        } catch {
            completionHandler(nil, [], false, error)
        }
        progress.completedUnitCount = 1
        return progress
    }

    func deleteItem(
        identifier: NSFileProviderItemIdentifier,
        baseVersion version: NSFileProviderItemVersion,
        options: NSFileProviderDeleteItemOptions = [],
        request: NSFileProviderRequest,
        completionHandler: @escaping (Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        do {
            _ = try ipc.queueDeleteItem(
                itemIdentifier: identifier,
                baseVersionIdentifier: Self.versionIdentifier(version)
            )
            completionHandler(nil)
        } catch {
            completionHandler(error)
        }
        progress.completedUnitCount = 1
        return progress
    }

    func invalidate() {
        _ = domain
    }

    private static func versionIdentifier(_ version: NSFileProviderItemVersion) -> String? {
        String(data: version.contentVersion, encoding: .utf8)
    }
}

final class FileProviderEnumerator: NSObject, NSFileProviderEnumerator {
    private let containerIdentifier: NSFileProviderItemIdentifier
    private let ipc: XPCBridge

    init(containerIdentifier: NSFileProviderItemIdentifier, ipc: XPCBridge) {
        self.containerIdentifier = containerIdentifier
        self.ipc = ipc
        super.init()
    }

    func invalidate() {}

    func enumerateItems(
        for observer: NSFileProviderEnumerationObserver,
        startingAt page: NSFileProviderPage
    ) {
        do {
            let models: [BeebeebProviderItem]
            if containerIdentifier == .rootContainer {
                models = try ipc.enumerate(containerIdentifier: containerIdentifier)
            } else if BeebeebNamespace.allCases.contains(where: { $0.identifier == containerIdentifier }) {
                models = try ipc.enumerate(containerIdentifier: containerIdentifier)
            } else {
                models = try ipc.enumerate(containerIdentifier: containerIdentifier)
            }

            observer.didEnumerate(models.map(FileProviderItem.init(model:)))
            observer.finishEnumerating(upTo: nil)
        } catch BeebeebIPCError.daemonUnavailable where containerIdentifier == .rootContainer {
            observer.didEnumerate(BeebeebNamespace.allCases.map { FileProviderItem(model: .namespace($0)) })
            observer.finishEnumerating(upTo: nil)
        } catch {
            observer.finishEnumeratingWithError(error)
        }
    }

    func enumerateChanges(
        for observer: NSFileProviderChangeObserver,
        from syncAnchor: NSFileProviderSyncAnchor
    ) {
        observer.finishEnumeratingChanges(upTo: NSFileProviderSyncAnchor(Data()), moreComing: false)
    }

    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        completionHandler(NSFileProviderSyncAnchor(Data()))
    }
}
