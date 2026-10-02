import FileProvider
import Foundation
import UniformTypeIdentifiers

final class FileProviderExtension: NSObject, NSFileProviderReplicatedExtension {
    private let ipc = XPCBridge()
    private let domain: NSFileProviderDomain
    /// Task 1698 (error truth): a transient failure reported to the system
    /// leaves the replica throttled until we signal the error resolved after a
    /// successful operation (see `TransientErrorTracker`).
    private let transientErrors = TransientErrorTracker()

    required init(domain: NSFileProviderDomain) {
        self.domain = domain
        super.init()
    }

    /// Task 1698: remember a transient failure so the next success can clear
    /// the system's throttling for it. Called from every completion path that
    /// reports an error to the system.
    private func noteFailure(_ error: Error) {
        let nsError = error as NSError
        if nsError.domain == NSFileProviderErrorDomain,
           nsError.code == NSFileProviderError.serverUnreachable.rawValue {
            transientErrors.record(nsError)
        }
    }

    /// Task 1698: after a successful operation, tell the system that the
    /// previously reported transient error (if any) is resolved. Best-effort:
    /// the signal only cancels throttling; a failure to send it is harmless.
    private func signalErrorResolvedIfPending() {
        guard let pending = transientErrors.takePending() else {
            return
        }
        NSFileProviderManager(for: domain)?.signalErrorResolved(pending) { error in
            if let error {
                NSLog("BeebeebFileProvider: signalErrorResolved failed: \(error)")
            }
        }
    }

    func item(
        for identifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        do {
            // Task 1701: no synthetic namespace lookup any more — every
            // identifier is either the root (answered locally by
            // `XPCBridge.item`), the trash container (a SYSTEM container,
            // answered locally — task 1698), or a real item served by the
            // daemon. A stale cached namespace identifier from a pre-1701
            // replica now fails the daemon lookup honestly instead of
            // resurrecting a folder that no longer exists.
            let model = try ipc.item(identifier: identifier)
            completionHandler(FileProviderItem(model: model), nil)
            signalErrorResolvedIfPending()
        } catch {
            noteFailure(error)
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
        // Task 1670 issue 3: this used to be `Progress(totalUnitCount: 1)`
        // that was never updated -- and the whole download ran synchronously
        // BEFORE `fetchContents` returned, so Finder never even received the
        // Progress object. Now the work runs on a background queue, the
        // Progress is returned immediately, its units are plaintext BYTES
        // (switched over as soon as the daemon reports the real size), and
        // cancelling it cancels the daemon request.
        //
        // It starts INDETERMINATE: Apple documents `Progress.isIndeterminate`
        // as true when `totalUnitCount` or `completedUnitCount` is less than
        // zero (or both are zero), so `totalUnitCount = -1` is the documented
        // way to say "size not known yet". A placeholder of 1 would render as
        // a determinate 0% bar for as long as the size stays unknown (an old
        // daemon sends no progress frames at all). Apple documents no
        // Finder-specific rule beyond "the system observes this progress
        // object", so whether Finder draws a spinner or a bar is only
        // provable on a Mac (still open, see task 1670 Notes).
        let progress = Progress(totalUnitCount: -1)
        progress.kind = .file
        progress.setUserInfoObject(Progress.FileOperationKind.downloading, forKey: .fileOperationKindKey)
        progress.isCancellable = true
        progress.isPausable = false
        let cancellation = IPCCancellation()
        progress.cancellationHandler = {
            cancellation.cancel()
        }

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
            Self.finish(progress)
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

        DispatchQueue.global(qos: .userInitiated).async { [self] in
            do {
                try ipc.hydrateFile(
                    itemIdentifier: itemIdentifier,
                    destinationURL: destinationURL,
                    cancellation: cancellation,
                    onProgress: { done, total in
                        // Real progress from the daemon's download + decrypt.
                        // `total` is 0 when the size is unknown: leave the
                        // Progress at its indeterminate -1 rather than show a
                        // wrong bar.
                        if total > 0 {
                            progress.totalUnitCount = total
                            progress.completedUnitCount = min(done, total)
                        }
                    }
                )
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
                signalErrorResolvedIfPending()
            } catch {
                // Nothing was ever handed to the system on this path —
                // `hydrateFile` failed, the item lookup failed, or the handoff
                // copy itself failed (which, on its own failure, already cleans
                // up any partial copy it made — see its doc comment). Either
                // way `completionHandler` below is called with `nil`, so nothing
                // else will ever clean up our staging copy at `destinationURL`.
                // Do it now; there is no race to lose here.
                cleanupStagedPlaintext()
                if cancellation.isCancelled {
                    // Finder asked for this; report it as a user cancellation
                    // rather than a failure.
                    completionHandler(nil, nil, NSError(domain: NSCocoaErrorDomain, code: NSUserCancelledError, userInfo: nil))
                } else {
                    noteFailure(error)
                    completionHandler(nil, nil, error)
                }
            }
            Self.finish(progress)
        }

        return progress
    }

    /// Mark a fetchContents Progress complete. An indeterminate Progress
    /// (`totalUnitCount == -1`, size never reported) must become a real
    /// 1-of-1 first: `completedUnitCount = totalUnitCount` would set -1 and
    /// leave it indeterminate instead of finished.
    private static func finish(_ progress: Progress) {
        if progress.totalUnitCount < 1 {
            progress.totalUnitCount = 1
        }
        progress.completedUnitCount = progress.totalUnitCount
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

    // MARK: - 1698 trash routing (ruling: full sync)

    /// Where a user-initiated MOVE (reparent) must land. The trash container
    /// is special: the Finder-side trash IS a reparent to
    /// `.trashContainer`, and it maps to the server's EXISTING trash
    /// operation (`TrashFile` → `DELETE /files/{id}` sets `is_trashed=TRUE`)
    /// — never a metadata move (the server has no trash-container parent).
    static func modifyRoute(newParent: NSFileProviderItemIdentifier) -> ModifyRoute {
        newParent == .trashContainer ? .serverTrash : .metadataUpdate
    }

    enum ModifyRoute: Equatable {
        case metadataUpdate
        case serverTrash
    }

    /// The `deleteItem` disposition. Apple's Replicated contract:
    /// "This is called when the user deletes an item that was already in the
    /// Trash... Unless NSFileProviderDeleteItemRecursive is passed, the
    /// deletion of a directory should be non-recursive. If the deletion
    /// targets a non-empty directory, the extension must reject with
    /// NSFileProviderErrorDirectoryNotEmpty." A child count the daemon did
    /// not send (older daemon) fails OPEN — the ruling's reject rule needs a
    /// count to be provable.
    static func deleteDisposition(
        isFolder: Bool,
        childItemCount: Int64?,
        recursive: Bool
    ) -> DeleteDisposition {
        if isFolder, !recursive, let count = childItemCount, count > 0 {
            return .directoryNotEmpty
        }
        return .queueServerTrash
    }

    enum DeleteDisposition: Equatable {
        case queueServerTrash
        case directoryNotEmpty
    }

    /// The server trash / permanent-delete path for one item. The DAEMON
    /// decides the outcome: an unknown item is an idempotent `Ignored`
    /// (ruling: "unknown items report success"), a known item queues the
    /// `TrashFile` op (for an in-trash item the server-side re-trash is an
    /// idempotent no-op — see the task's deviation on delete-forever).
    private func queueServerTrash(
        identifier: NSFileProviderItemIdentifier,
        baseVersion: NSFileProviderItemVersion,
        completionHandler: @escaping (Error?) -> Void
    ) {
        do {
            _ = try ipc.queueDeleteItem(
                itemIdentifier: identifier,
                baseVersionIdentifier: Self.versionIdentifier(baseVersion)
            )
            completionHandler(nil)
            signalErrorResolvedIfPending()
        } catch {
            noteFailure(error)
            completionHandler(error)
        }
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
            // Task 1698: the trash container is a SYSTEM container — creating
            // inside it is not a server operation (an "undelete" would be a
            // reparent back OUT, never a create). Refuse honestly.
            if itemTemplate.parentItemIdentifier == .trashContainer {
                throw BeebeebIPCError.invalidResponse(
                    "Beebeeb cannot create items inside the Trash."
                )
            }
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
            signalErrorResolvedIfPending()
        } catch {
            noteFailure(error)
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
            // Task 1698 (trash ruling): a reparent INTO the trash container
            // is the Finder-side TRASH — map it to the server's existing
            // trash operation. The daemon parks the row `Trashing`, so the
            // change feed immediately presents the item under the trash
            // container (matching what the system just did in its own
            // replica), and the queued op converges the server side.
            if Self.modifyRoute(newParent: item.parentItemIdentifier) == .serverTrash
                && changedFields.rawValue & NSFileProviderItemFields.parentItemIdentifier.rawValue != 0
            {
                queueServerTrash(
                    identifier: item.itemIdentifier,
                    baseVersion: version
                ) { error in
                    if let error {
                        completionHandler(nil, [], false, error)
                    } else {
                        completionHandler(nil, [], true, nil)
                    }
                }
                progress.completedUnitCount = 1
                return progress
            }
            let contentType = item.contentType
            let kind: BeebeebItemKind = contentType?.conforms(to: .folder) == true ? .folder : .file
            let result = try ipc.queueModifyItem(
                itemIdentifier: item.itemIdentifier,
                parentIdentifier: item.parentItemIdentifier,
                filename: item.filename,
                kind: kind,
                contentsURL: newContents,
                contentType: kind == .file ? contentType?.identifier : nil,
                baseVersionIdentifier: Self.versionIdentifier(version),
                changedFields: changedFields
            )
            if result.ignored {
                completionHandler(nil, [], false, nil)
            } else if let model = result.item {
                completionHandler(FileProviderItem(model: model), [], true, nil)
            } else {
                completionHandler(nil, [], true, nil)
            }
            signalErrorResolvedIfPending()
        } catch {
            noteFailure(error)
            completionHandler(nil, [], false, error)
        }
        progress.completedUnitCount = 1
        return progress
    }

    /// Async variant of the server-trash queue for the modifyItem path (the
    /// work runs off the callback queue; the Progress completes immediately).
    func deleteItem(
        identifier: NSFileProviderItemIdentifier,
        baseVersion version: NSFileProviderItemVersion,
        options: NSFileProviderDeleteItemOptions = [],
        request: NSFileProviderRequest,
        completionHandler: @escaping (Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        DispatchQueue.global(qos: .userInitiated).async { [self] in
            // Apple's contract: deleteItem fires for items ALREADY in the
            // Trash ("delete forever"). The lookup only feeds the
            // directoryNotEmpty proof; every actual outcome is the daemon's
            // call: an unknown item is an idempotent `Ignored` (ruling:
            // success), a known item queues the server trash op (an
            // in-trash item's re-trash is an idempotent server no-op — the
            // "delete forever" gap is recorded as task 1698's deviation 2).
            var childItemCount: Int64?
            var isFolder = false
            if let model = try? ipc.item(identifier: identifier) {
                isFolder = model.kind == .folder
                childItemCount = model.childItemCount
            }
            guard Self.deleteDisposition(
                isFolder: isFolder,
                childItemCount: childItemCount,
                recursive: options.contains(.recursive)
            ) != .directoryNotEmpty else {
                completionHandler(NSError(
                    domain: NSFileProviderErrorDomain,
                    code: NSFileProviderError.directoryNotEmpty.rawValue,
                    userInfo: [NSLocalizedDescriptionKey: "This folder still has items inside. Delete them first, or empty the Trash."]
                ))
                progress.completedUnitCount = 1
                return
            }
            queueServerTrash(
                identifier: identifier,
                baseVersion: version,
                completionHandler: completionHandler
            )
            progress.completedUnitCount = 1
        }
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

    /// The change observer's suggested batch size, defaulting when the system
    /// declines to suggest (it is an optional protocol property, imported as
    /// `Int?`). Apple caps the system at 100× the suggestion; our default of
    /// 100 matches the daemon's page ceiling.
    private static func batchSize(_ observer: NSFileProviderChangeObserver) -> Int {
        let suggested = observer.suggestedBatchSize ?? 0
        return suggested > 0 ? min(suggested, 10_000) : 100
    }

    /// The enumeration observer's suggested page size, same optional-property
    /// handling.
    private static func pageSize(_ observer: NSFileProviderEnumerationObserver) -> Int {
        let suggested = observer.suggestedPageSize ?? 0
        return suggested > 0 ? min(suggested, 10_000) : 100
    }

    // MARK: enumerateItems — paged, stable ordering

    /// Task 1697: paged enumeration. The daemon returns the container's full
    /// listing; we slice it into pages of the SYSTEM-SUGGESTED size, sorted
    /// by name (stable across pages: the sort happens over the WHOLE list
    /// before slicing, so a page never reshuffles), and hand the system a
    /// ≤500-byte resume token naming the next offset. Apple caps page data
    /// at 500 bytes — a decimal offset trivially fits.
    func enumerateItems(
        for observer: NSFileProviderEnumerationObserver,
        startingAt page: NSFileProviderPage
    ) {
        DispatchQueue.global(qos: .userInitiated).async { [self] in
            do {
                let models = try Self.listModels(
                    containerIdentifier: containerIdentifier,
                    ipc: ipc
                )
                // Stable ordering: sort the FULL list once. Slicing a sorted
                // list keeps every page's ordering stable across re-paging.
                let sorted = models.sorted { lhs, rhs in
                    if lhs.filename.caseInsensitiveCompare(rhs.filename) != .orderedSame {
                        return lhs.filename.caseInsensitiveCompare(rhs.filename) == .orderedAscending
                    }
                    return lhs.identifier < rhs.identifier
                }
                let start = Self.pageOffset(page)
                let pageSize = Self.pageSize(observer)
                let end = min(start + pageSize, sorted.count)
                guard start < sorted.count else {
                    observer.finishEnumerating(upTo: nil)
                    return
                }
                observer.didEnumerate(sorted[start..<end].map(FileProviderItem.init(model:)))
                if end < sorted.count {
                    let nextPage = Data(Self.resumeTokenPrefix.utf8) + Data(String(end).utf8)
                    observer.finishEnumerating(upTo: NSFileProviderPage(nextPage))
                } else {
                    observer.finishEnumerating(upTo: nil)
                }
            } catch BeebeebIPCError.daemonUnavailable where containerIdentifier == .rootContainer {
                // Task 1701: the old fallback enumerated the synthetic
                // namespace roots so the sidebar kept its shape while the
                // daemon was down. There are no synthetic roots any more —
                // an unreachable daemon means an honestly EMPTY listing, not
                // a fake one.
                observer.finishEnumerating(upTo: nil)
            } catch {
                observer.finishEnumeratingWithError(error)
            }
        }
    }

    private static let resumeTokenPrefix = "beebeeb-page:"

    /// Decode a resume token. System-provided initial pages
    /// (`InitialPageSortedByName`/`...ByDate`/empty) decode to offset 0; a
    /// token we minted decodes to its offset; anything else (corrupt) also
    /// restarts at 0 — restarting can only re-deliver already-seen items,
    /// which is idempotent for the system, while failing the enumeration
    /// would leave the folder permanently blank.
    static func pageOffset(_ page: NSFileProviderPage) -> Int {
        let text = String(decoding: Data(page as NSData), as: UTF8.self)
        guard text.hasPrefix(resumeTokenPrefix), let offset = Int(text.dropFirst(resumeTokenPrefix.count)) else {
            return 0
        }
        return max(0, offset)
    }

    /// Full listing for a container. The daemon routes containers inside
    /// `list_file_provider_items` (root = the real top-level tree; real
    /// folders = children by the contract's parent).
    private static func listModels(
        containerIdentifier: NSFileProviderItemIdentifier,
        ipc: XPCBridge
    ) throws -> [BeebeebProviderItem] {
        try ipc.enumerate(containerIdentifier: containerIdentifier)
    }

    // MARK: enumerateChanges — the working-set machinery (task 1697)

    /// Task 1697 — the core gap. Pull the daemon's change log since the
    /// system's anchor, report updates/deletes, and finish with the REAL
    /// anchor. Apple (`Enum.h:170-202`): these are "really required. System
    /// performance will be severely degraded if they are not implemented."
    func enumerateChanges(
        for observer: NSFileProviderChangeObserver,
        from syncAnchor: NSFileProviderSyncAnchor
    ) {
        DispatchQueue.global(qos: .userInitiated).async { [self] in
            do {
                // An unparseable anchor means the log no longer contains that
                // cursor: the system must drop its caches and rescan
                // (Apple's documented expiry handling), never continue.
                guard let since = WorkingSetStore.decodeAnchor(Data(syncAnchor as NSData)) else {
                    throw NSError(
                        domain: NSFileProviderErrorDomain,
                        code: NSFileProviderError.syncAnchorExpired.rawValue,
                        userInfo: [NSLocalizedDescriptionKey: "The sync anchor is no longer valid; starting over."]
                    )
                }

                let materialized: Set<String>? = Self.materializedContainersForFiltering()
                let batch = Self.batchSize(observer)
                var updates: [FileProviderItem] = []
                var deletions: [NSFileProviderItemIdentifier] = []
                var deliveredAny = false
                var nextAnchor: String?

                // Page through the daemon's log. Each reply's `next_anchor` is
                // the resume token; the loop stops when the daemon reports
                // the batch complete (its anchor stops advancing).
                var cursor: String? = since == 0 ? nil : String(since)
                while true {
                    let response = try ipc.listChanges(sinceAnchor: cursor, limit: batch)
                    for change in response.changes {
                        guard Self.changeBelongsToContainer(
                            change,
                            container: containerIdentifier,
                            materialized: materialized
                        ) else {
                            continue
                        }
                        switch change.kind {
                        case "deleted":
                            deletions.append(NSFileProviderItemIdentifier(change.fileID))
                        default:
                            // created/modified/reparented carry the full item
                            // payload; a row that vanished between the change
                            // and this poll has no payload — skip it (its
                            // later `deleted` row, if any, reports the exit).
                            if let item = change.item {
                                updates.append(FileProviderItem(model: item))
                            }
                        }
                    }
                    deliveredAny = deliveredAny || !response.changes.isEmpty
                    if let anchor = response.nextAnchor {
                        nextAnchor = anchor
                        // Stop when the daemon's anchor stops advancing (the
                        // batch completed the window).
                        if anchor == cursor {
                            break
                        }
                        cursor = anchor
                    } else {
                        // Empty log: nothing was ever recorded.
                        break
                    }
                }

                if !updates.isEmpty {
                    observer.didUpdate(updates)
                }
                if !deletions.isEmpty {
                    observer.didDeleteItems(withIdentifiers: deletions)
                }

                if let anchor = nextAnchor {
                    // Task 1697 review fix: the daemon's anchor is a raw
                    // decimal rowid; the system's wire anchor must be the
                    // encoded `v1:` form. A raw decimal handed to the system
                    // comes back through `enumerateChanges(from:)`, fails
                    // `decodeAnchor`, and answers `syncAnchorExpired` — a
                    // full rescan after EVERY batch.
                    if let wire = WorkingSetStore.encodeDaemonAnchor(anchor) {
                        WorkingSetStore.recordAnchor(wire)
                        observer.finishEnumeratingChanges(
                            upTo: NSFileProviderSyncAnchor(Data(wire.utf8)),
                            moreComing: false
                        )
                    } else {
                        // Unparseable daemon anchor: do not advance the
                        // system's cursor past what we can decode. Report
                        // up-to-date from the starting anchor — the next
                        // enumeration re-derives from it safely.
                        observer.finishEnumeratingChanges(
                            upTo: NSFileProviderSyncAnchor(Data(syncAnchor as NSData)),
                            moreComing: false
                        )
                    }
                } else if deliveredAny || nextAnchor == nil {
                    // An empty log has no anchor yet: finish with the starting
                    // anchor so the system's cursor does not regress, and
                    // report up-to-date.
                    observer.finishEnumeratingChanges(
                        upTo: NSFileProviderSyncAnchor(Data(syncAnchor as NSData)),
                        moreComing: false
                    )
                }
            } catch {
                observer.finishEnumeratingWithError(error)
            }
        }
    }

    /// The persisted materialized set, or nil when it has never been
    /// populated (fail-open: Apple's documented "working set is the entire
    /// dataset" fallback — report everything until the first
    /// materialization event arrives).
    private static func materializedContainersForFiltering() -> Set<String>? {
        let state = WorkingSetStore.loadState()
        return state.materializedContainers.isEmpty ? nil : state.materializedContainers
    }

    /// Filtering rules per container:
    /// - `.workingSet` (the ONLY container Replicated honors): Apple's
    ///   materialized-set filter — report when old-or-new parent is
    ///   materialized (the root container always is; task 1701 removed the
    ///   synthetic namespace roots that used to be always-materialized too).
    /// - Any other container: only items whose old-or-new parent IS that
    ///   container (the system also drives per-container change
    ///   enumerations; delivering unrelated items would confuse it).
    static func changeBelongsToContainer(
        _ change: XPCBridge.DaemonChange,
        container: NSFileProviderItemIdentifier,
        materialized: Set<String>?
    ) -> Bool {
        if container == .workingSet {
            return WorkingSetStore.changeTouchesMaterialized(
                oldParent: change.oldParentID,
                newParent: change.newParentID ?? change.item?.parentIdentifier,
                materialized: materialized
            )
        }
        let containerID = container.rawValue
        if change.newParentID == containerID || change.item?.parentIdentifier == containerID {
            return true
        }
        // A reparent OUT of this container must also be reported so the
        // container's view drops the item.
        return change.oldParentID == containerID
    }

    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        DispatchQueue.global(qos: .userInitiated).async { [self] in
            // Daemon-sourced first (its persistent cursor is the truth); the
            // App Group copy is the crash-recovery fallback when the daemon
            // is unreachable — this call must never block on a dead daemon
            // for long (sendRequest times out at metadataTimeoutSeconds).
            var anchor: String?
            if let daemonAnchor = try? ipc.syncAnchor() {
                // Task 1697 review fix: encode the daemon's raw decimal rowid
                // into the wire form BEFORE handing it to the system or
                // persisting it — a raw decimal fails decodeAnchor on the
                // next enumerateChanges (syncAnchorExpired → full rescan).
                anchor = WorkingSetStore.encodeDaemonAnchor(daemonAnchor)
                if let anchor {
                    WorkingSetStore.recordAnchor(anchor)
                }
            }
            if anchor == nil {
                anchor = WorkingSetStore.loadState().lastAnchor
            }
            if let anchor {
                completionHandler(NSFileProviderSyncAnchor(Data(anchor.utf8)))
            } else {
                completionHandler(nil)
            }
        }
    }
}

/// Task 1697: `materializedItemsDidChange` — the system tells this extension
/// that the set of materialized items changed. Per Apple's Replicated
/// contract we (a) record the materialized DIRECTORIES in the App Group store
/// so the working-set filter survives process death, (b) publish the set to
/// the daemon (`ReportMaterialized`) so ITS signal filtering can use it, and
/// (c) signal the working set ourselves so pending changes for the newly
/// materialized subtree are delivered promptly.
///
/// The heavy work runs off the callback; the completion handler fires when
/// the bookkeeping is done (Apple budgets "a few seconds").
extension FileProviderExtension {
    func materializedItemsDidChange(completionHandler: @escaping () -> Void) {
        DispatchQueue.global(qos: .utility).async { [self] in
            defer { completionHandler() }
            guard let manager = NSFileProviderManager(for: domain) else {
                return
            }
            // The roles are REVERSED on this enumerator (Apple,
            // NSFileProviderManager.h): WE call enumerateItems on the system's
            // materialized-set enumerator and observe what it reports. Start
            // from `NSData()` per the header (the sort-page constants do not
            // apply).
            let collector = MaterializedSetCollector()
            let enumerator = manager.enumeratorForMaterializedItems()
            var page: NSFileProviderPage = NSFileProviderPage(Data())
            var safetyPages = 0
            while safetyPages < 100 {
                safetyPages += 1
                // Task 1697 review fix: page completion is signalled by the
                // COLLECTOR when the system's finish callback lands — NOT
                // when `enumerateItems(for:startingAt:)` returns. Delivery
                // happens via didEnumerate/finishEnumerating ASYNCHRONOUSLY;
                // waiting on the method's return woke this loop early,
                // `collector.nextPage` was still nil, the loop broke, and an
                // EMPTY materialized set was persisted (filtering never
                // became effective).
                collector.beginPage()
                let pageToSend = page
                DispatchQueue.global(qos: .utility).async {
                    enumerator.enumerateItems(for: collector, startingAt: pageToSend)
                }
                // Bounded wait: a silent/hung enumerator must not block the
                // completion handler forever (Apple budgets "a few seconds"
                // for this callback). A timeout breaks pagination instead of
                // racing a still-in-flight page with the next request.
                if collector.waitPageCompletion(timeout: .seconds(5)) == .timedOut {
                    break
                }
                guard let next = collector.nextPage, !collector.finished else {
                    break
                }
                page = next
            }
            // The materialized set enumerates ITEMS (files and folders);
            // the filter needs the DIRECTORIES (containers).
            let directories = collector.collectedFolderIdentifiers()
            var state = WorkingSetStore.loadState()
            let changed = state.materializedContainers != directories
            state.materializedContainers = directories
            WorkingSetStore.persist(state)
            ipc.reportMaterializedContainers(Array(directories))
            if changed {
                // New materialized containers may have pending changes that
                // were filtered out before; nudge the system to re-consult
                // the working set. Best-effort.
                manager.signalEnumerator(for: .workingSet) { _ in }
            }
        }
    }
}

/// The observer we pass to the SYSTEM's materialized-set enumerator. Collects
/// identifiers; folders are what the filter tracks.
final class MaterializedSetCollector: NSObject, NSFileProviderEnumerationObserver {
    private(set) var itemIdentifiers: [String] = []
    private(set) var folderIdentifiers: Set<String> = []
    private(set) var nextPage: NSFileProviderPage?
    private(set) var finished = false

    // Task 1697 review fix: the page-completion channel the pagination loop
    // waits on. A FRESH semaphore per page: a stale signal from a page whose
    // wait timed out can never release a later page's wait.
    private var pageCompletion: DispatchSemaphore?

    /// Arms the wait for the CURRENT page. Call before dispatching
    /// `enumerateItems(for:startingAt:)`.
    func beginPage() {
        pageCompletion = DispatchSemaphore(value: 0)
    }

    /// Blocks until the system reports the page complete
    /// (`finishEnumerating(upTo:)` / `finishEnumeratingWithError`) or the
    /// timeout elapses. Without a begun page this reports `.timedOut`.
    func waitPageCompletion(timeout: DispatchTimeInterval) -> DispatchTimeoutResult {
        pageCompletion?.wait(timeout: .now() + timeout) ?? .timedOut
    }

    /// Deliverer-side hook: state is published FIRST so the woken waiter
    /// observes a consistent page result (the signal/wait edge provides the
    /// memory ordering).
    private func signalPageCompletion() {
        pageCompletion?.signal()
    }

    func didEnumerate(_ items: [NSFileProviderItem]) {
        for item in items {
            let id = item.itemIdentifier.rawValue
            itemIdentifiers.append(id)
            if item.contentType?.conforms(to: .folder) == true {
                folderIdentifiers.insert(id)
            }
        }
    }

    func finishEnumerating(upTo nextPage: NSFileProviderPage?) {
        self.nextPage = nextPage
        finished = nextPage == nil
        signalPageCompletion()
    }

    func finishEnumeratingWithError(_ error: Error) {
        // A failed materialized-set enumeration degrades to the PREVIOUS
        // in-memory set (fail-open filtering) rather than an empty one —
        // an empty set would over-report, which is safe, but persisting an
        // empty set over a good one could under-report later.
        finished = true
        nextPage = nil
        if error is BeebeebIPCError {
            // Not expected from the system enumerator; ignore.
        }
        _ = error
        signalPageCompletion()
    }

    func collectedFolderIdentifiers() -> Set<String> {
        folderIdentifiers
    }
}
