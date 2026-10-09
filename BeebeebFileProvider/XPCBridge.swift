import FileProvider
import Foundation

enum BeebeebIPCError: LocalizedError {
    case daemonUnavailable
    case invalidResponse(String)
    /// PR #100 review (Codex P2): the daemon (or this extension's own policy)
    /// actively REFUSED the request — the reply carried an explicit rejection
    /// ("Beebeeb cannot create items inside the Trash.", a permission/
    /// write-policy denial, ...). Definitive: restarting or unlocking the
    /// daemon cannot make the request valid, so it must NEVER ride the
    /// transient `serverUnreachable` class (Finder would retry forever and
    /// report an unrelated availability error). Daemon-side retryable
    /// conditions retry inside the daemon's own operation queue, so a
    /// definitive classification here does not strand transfers.
    case daemonRejected(String)
    case invalidIdentifier
    /// The daemon accepted the connection but sent nothing for `seconds`
    /// (task 1670 issue 3: previously a stall looked like "not valid JSON").
    case timedOut(seconds: Int)
    /// Finder cancelled the transfer.
    case cancelled
    /// The extension could not copy a write's contents into the App Group
    /// upload-staging directory (disk full, an I/O error, no container). The
    /// file is still on the user's disk, so this is TRANSIENT: the system
    /// retries the write instead of giving up on it. The payload is the
    /// error's domain and code only, never a path (see `UploadStaging`).
    case uploadStagingFailed(String)

    var errorDescription: String? {
        switch self {
        case .daemonUnavailable:
            return "Open Beebeeb and unlock your vault."
        case .invalidResponse(let message):
            return message
        case .daemonRejected(let message):
            return message
        case .invalidIdentifier:
            return "This item's identifier could not be used for a Finder operation."
        case .timedOut(let seconds):
            return "Beebeeb's sync engine did not answer within \(seconds) seconds. Open Beebeeb, make sure it is unlocked, and try again."
        case .cancelled:
            return "The transfer was cancelled."
        case .uploadStagingFailed(let reason):
            return "Beebeeb could not prepare this file for upload (\(reason)). It will try again."
        }
    }
}

/// Task 1670: without this, Swift's default `Error` -> `NSError` bridging
/// uses the TYPE's own module+name string as the error domain
/// ("BeebeebFileProvider.BeebeebIPCError") -- not a domain the File Provider
/// host process recognizes. Its own log made this exact failure explicit:
/// "[CRIT] Provider returned error 0 from domain
/// BeebeebFileProvider.BeebeebIPCError which is unsupported. Supported error
/// domains are NSCocoaErrorDomain, NSFileProviderErrorDomain." When that
/// happens the host can't relay the real error text to Finder at all, so
/// Finder falls back to its generic, unhelpful "Couldn't communicate with a
/// helper application" dialog -- masking whatever `errorDescription` above
/// actually says. `CustomNSError` conformance makes every `BeebeebIPCError`
/// bridge into a domain the host DOES support, so a real failure (the daemon
/// isn't running, a permission check rejected the request, etc.) surfaces as
/// an actual, specific message instead.
extension BeebeebIPCError: CustomNSError {
    static var errorDomain: String { NSFileProviderErrorDomain }

    var errorCode: Int {
        switch self {
        case .daemonUnavailable:
            // Semantically the closest fit: the daemon (this extension's only
            // "server") cannot be reached.
            return NSFileProviderError.serverUnreachable.rawValue
        case .invalidResponse:
            // Task 1698 (error truth): a transport glitch or a daemon-side
            // rejection is a TRANSIENT condition, not a definitive one.
            // `cannotSynchronize` told the system "stop retrying until an
            // OS/extension update" — a dropped connection or a busy daemon
            // would then brick every subsequent operation. `serverUnreachable`
            // keeps the system retrying (with backoff), which is the truth:
            // the daemon comes back when Beebeeb is running/unlocked again.
            return NSFileProviderError.serverUnreachable.rawValue
        case .invalidIdentifier:
            // Task 1670 round 2: the request is refused before it ever reaches
            // the daemon. Task 1698: deliberately stays DEFINITIVE — a
            // malformed identifier can never succeed on retry, so transient
            // classification would only churn the system.
            return NSFileProviderError.cannotSynchronize.rawValue
        case .daemonRejected:
            // PR #100 review (Codex P2): a deliberate refusal (trash-create
            // guard, permission/write-policy denial) — definitive, with the
            // daemon's own message as the user-facing text.
            return NSFileProviderError.cannotSynchronize.rawValue
        case .timedOut:
            // The daemon is there but not answering: same category as it
            // being unreachable.
            return NSFileProviderError.serverUnreachable.rawValue
        case .cancelled:
            return NSFileProviderError.cannotSynchronize.rawValue
        case .uploadStagingFailed:
            // The contents never reached the app, and the user's file is
            // intact: retry, like an unreachable daemon.
            return NSFileProviderError.serverUnreachable.rawValue
        }
    }

    /// Task 1698: is this failure TRANSIENT (the system should retry after
    /// backoff) or definitive? Only the transport/unreachable family is
    /// transient; request-shape failures and user cancellations are not.
    var isTransient: Bool {
        switch self {
        case .daemonUnavailable, .invalidResponse, .timedOut, .uploadStagingFailed:
            return true
        case .daemonRejected, .invalidIdentifier, .cancelled:
            return false
        }
    }

    var errorUserInfo: [String: Any] {
        [NSLocalizedDescriptionKey: errorDescription ?? "Beebeeb couldn't complete this Finder operation."]
    }
}

final class XPCBridge {
    /// The App Group shared with the containing app — see
    /// `BeebeebFileProvider.entitlements`' `com.apple.security.application-
    /// groups` and `src-tauri/entitlements.plist`'s identical entry. Mirrors
    /// `crate::ipc_socket::MACOS_APP_GROUP_ID` on the Rust side (task 1524);
    /// keep both, plus the two entitlements plists, in sync if it ever
    /// changes.
    private static let appGroupID = "R8352WDJJR.io.beebeeb.app.fileprovider"

    /// Deliberately short — `sockaddr_un.sun_path` on macOS is 104 bytes
    /// including the NUL terminator, and the group-container prefix alone
    /// already uses most of that budget for a real username. Must match
    /// `crate::ipc_socket::MACOS_IPC_SOCKET_FILENAME`.
    private static let ipcSocketFileName = "ipc.sock"

    /// Task 1670: subdirectory name for staging hydrated plaintext, mirroring
    /// `crate::ipc_socket::MACOS_HYDRATE_CACHE_DIRNAME` on the Rust side —
    /// keep both in sync if it ever changes. See `hydrateDestinationURL(for:)`
    /// for why this has to be the App Group container and not
    /// `FileManager.default.temporaryDirectory`.
    private static let hydrateCacheDirectoryName = "hydrate-cache"

    private let socketPath: String

    /// Task 1524: this extension is sandboxed (`com.apple.security.app-
    /// sandbox`), so `/tmp` and `$XDG_RUNTIME_DIR` (which macOS doesn't set
    /// anyway — that env var is a Linux/systemd convention) are NOT
    /// reachable; only the app's own container and shared App Group
    /// containers are. `containerURL(forSecurityApplicationGroupIdentifier:)`
    /// is the real, sandbox-aware way to resolve that shared directory — the
    /// daemon (Rust side) resolves the identical path by construction
    /// (`ipc_socket::ipc_socket_path`) since a plain, unsigned `cargo test`
    /// binary can't call this API at all.
    init() {
        if let groupContainer = Self.resolveGroupContainer() {
            self.socketPath = groupContainer.appendingPathComponent(Self.ipcSocketFileName).path
        } else {
            // Should not happen in a properly provisioned build (both
            // targets declare the same app-group entitlement) — falling
            // back to the OLD, sandbox-unreachable path rather than crashing
            // means every IPC call below just fails closed with
            // `.daemonUnavailable`, which is the same failure shape as a
            // genuinely-not-running daemon, and is visible via this log.
            NSLog("BeebeebFileProvider: could not resolve app-group container for \(Self.appGroupID); IPC will not reach the daemon")
            let runtimeDir = ProcessInfo.processInfo.environment["XDG_RUNTIME_DIR"] ?? "/tmp"
            self.socketPath = "\(runtimeDir)/beebeeb-daemon.sock"
        }
    }

    private static func resolveGroupContainer() -> URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroupID)
    }

    /// Task 1670 root cause: `fetchContents` used to build its destination
    /// from `FileManager.default.temporaryDirectory`, which for a SANDBOXED
    /// process (this extension has `com.apple.security.app-sandbox`) resolves
    /// INSIDE that process's own per-bundle-ID container
    /// (`io.beebeeb.app.FileProvider`). The daemon that validates the
    /// destination is a DIFFERENT sandboxed process
    /// (`io.beebeeb.app`) with its OWN, different private temp dir
    /// (`crate::ipc_socket`'s `macos_hydrate_cache_dir` doc comment has the
    /// full writeup) — so the destination this extension built could never be
    /// inside any root the daemon was willing to write to. Every real
    /// `fetchContents` call failed with "hydrate destination is not within an
    /// allowed root" (confirmed via the unified log), and because
    /// `BeebeebIPCError` didn't bridge to a supported `NSError` domain either,
    /// Finder showed the generic "Couldn't communicate with a helper
    /// application" instead of any real message.
    ///
    /// The shared App Group container is the one directory BOTH sandboxes are
    /// actually entitled to and already use, for the IPC socket itself
    /// (`socketPath` above) — this reuses that exact, already-proven-working
    /// mechanism for hydration staging instead of each process's own private
    /// temp dir.
    ///
    /// **Round 2 (task 1670): considered and rejected `NSFileProviderManager
    /// .temporaryDirectoryURL()`.** Apple's own docs for it: "Returns the URL
    /// of a directory that **the File Provider extension** can use to
    /// temporarily store files before passing them to the system" — framed
    /// around the EXTENSION process handing content to the framework
    /// (`createItem`/`modifyItem`/`fetchContents`, all `NSFileProviderReplic
    /// atedExtension` methods this extension implements), not around a
    /// SEPARATE companion daemon process writing there too. There is no
    /// documented guarantee that calling it from the containing app
    /// (`io.beebeeb.app`, where the daemon that does the actual decrypt
    /// runs — see `crate::ipc_socket`) resolves to the SAME literal directory
    /// as calling it from this extension; the App Group container, by
    /// contrast, is explicitly and symmetrically keyed by the group id both
    /// entitlements files declare, which is exactly the cross-process sharing
    /// contract this two-process architecture needs. Verifying the
    /// alternative empirically would require a signed build against a real,
    /// registered domain, which this round's restrictions rule out (no
    /// touching Guus's real domain, no second daemon instance). Kept the
    /// group-container location; see the task file for the full writeup with
    /// sources.
    ///
    /// **Round 3 (task 1670, lead review of round 2): the caller stopped
    /// deleting the returned file immediately on success** — Apple's own
    /// `fetchContents` docs say only "After you call the completion handler,
    /// the system takes complete control over the local copy" and that the
    /// system "can clone it", never that the clone happens synchronously
    /// inside the `completionHandler` call itself, so round 2's unconditional
    /// post-success delete could race an undocumented-timing clone.
    ///
    /// **Round 4 (task 1670, Codex P1 on PR #75): the URL returned by THIS
    /// function is no longer what `fetchContents` hands to
    /// `completionHandler` at all.** Codex's review of round 3 made the
    /// sharper point: once `completionHandler` IS called with a URL, "the
    /// File Provider contract transfers control of that local copy to the
    /// system; there is no documented maximum delay before the system
    /// finishes consuming it" — so deleting THAT SAME URL later, on any
    /// timer, can race a busy or suspended `fileproviderd`, however
    /// generous the timer. `FileProviderExtension.fetchContents` now copies
    /// the file this function stages into `NSFileProviderManager(for:
    /// domain).temporaryDirectoryURL()` — a SYSTEM-managed directory, not
    /// this one — and hands `completionHandler` THAT copy's URL instead;
    /// see `FileProviderExtension.copyToSystemTemporaryDirectory(stagedAt:)`
    /// for the full mechanism. The file this function returns a destination
    /// for is deleted by the caller immediately after that copy succeeds —
    /// we own it end to end, it is never handed to anyone. The per-request
    /// random suffix below still matters: it is what keeps two hydrations of
    /// the SAME item, moments apart (two Finder windows, a retry racing the
    /// original), from ever sharing a leaf name in EITHER this directory or
    /// the system's temp directory the caller copies into (it reuses this
    /// exact leaf name there too).
    ///
    /// `nil` only in the same group-container-unavailable case `init()`
    /// already falls back from, or when `itemIdentifier` fails
    /// `sanitizedHydrateFilename` — see that function's doc comment.
    static func hydrateDestinationURL(for itemIdentifier: NSFileProviderItemIdentifier) -> Result<URL, BeebeebIPCError> {
        guard let safeName = sanitizedHydrateFilename(for: itemIdentifier) else {
            return .failure(.invalidIdentifier)
        }
        guard let groupContainer = resolveGroupContainer() else {
            return .failure(.daemonUnavailable)
        }
        let dir = groupContainer.appendingPathComponent(hydrateCacheDirectoryName, isDirectory: true)
        do {
            // Task 1670 round 2: owner-only (0o700) — this directory only
            // ever holds decrypted plaintext. Mirrors
            // `crate::ipc_socket::macos_ensure_hydrate_cache_dir`, which also
            // forces the mode down on an already-existing dir and carries the
            // tested contract (this target has no XCTest target).
            try FileManager.default.createDirectory(
                at: dir,
                withIntermediateDirectories: true,
                attributes: [.posixPermissions: 0o700]
            )
        } catch {
            NSLog("BeebeebFileProvider: could not create hydrate-cache dir: \(error)")
            return .failure(.daemonUnavailable)
        }

        // Task 1670 round 2: belt-and-suspenders backup exclusion via the
        // modern Foundation resource-key API. The Rust daemon sets the SAME
        // underlying xattr directly on every hydration
        // (`crate::ipc_socket::macos_exclude_from_backups`, which is what
        // actually has a test) — this covers the case where THIS extension
        // creates the directory before the daemon has run this session.
        // Best-effort: a failure here must not block hydration.
        var excludedDir = dir
        var resourceValues = URLResourceValues()
        resourceValues.isExcludedFromBackup = true
        try? excludedDir.setResourceValues(resourceValues)

        // Task 1670 round 2: a random per-request suffix so two concurrent
        // fetches of the SAME item (two Finder windows opening the same file,
        // a retry racing the original attempt) get DIFFERENT destination
        // files — never sharing a leaf name, so neither can collide with or
        // observe the other mid-write.
        let unique = "\(safeName).\(UUID().uuidString.prefix(8))"
        return .success(dir.appendingPathComponent(unique))
    }

    /// Task 1670 round 2: reject a raw item identifier before it is ever used
    /// as a path component — no `/` or `\` (also covers a leading `/`
    /// absolute path), no embedded `..`, not empty, no embedded NUL. Mirrors
    /// `crate::ipc_socket::macos_validate_hydrate_item_identifier` (Rust,
    /// `ipc_socket.rs`) — THAT function carries the tested contract (this
    /// repo's File Provider extension target has no XCTest target, so this
    /// Swift copy cannot be unit-tested the same way); keep both in sync if
    /// the rule ever changes.
    private static func sanitizedHydrateFilename(for itemIdentifier: NSFileProviderItemIdentifier) -> String? {
        let raw = itemIdentifier.rawValue
        if raw.isEmpty || raw.contains("/") || raw.contains("\\") || raw.contains("..") || raw.contains("\0") {
            return nil
        }
        return raw
    }

    func enumerate(containerIdentifier: NSFileProviderItemIdentifier) throws -> [BeebeebProviderItem] {
        let response = try sendRequest([
            "ListFileProviderItems": [
                "container_id": containerIdentifier.rawValue,
            ],
        ])

        if let error = response["Error"] as? [String: Any] {
            throw BeebeebIPCError.daemonRejected(error["message"] as? String ?? "daemon returned an error")
        }
        guard let payload = response["FileProviderItems"] as? [String: Any],
              let rawItems = payload["items"] as? [[String: Any]] else {
            throw BeebeebIPCError.invalidResponse("daemon response did not include FileProviderItems")
        }

        return rawItems.compactMap(Self.decodeItem)
    }

    func item(identifier: NSFileProviderItemIdentifier) throws -> BeebeebProviderItem {
        if identifier == .rootContainer {
            // Task 1701: the single "Beebeeb" root (no synthetic namespaces).
            return .root()
        }
        if identifier == .trashContainer {
            // Task 1698: the SYSTEM trash container — a local answer, never
            // a daemon lookup (the daemon has no row for it).
            return .trashContainer()
        }

        let response = try sendRequest([
            "GetFileStatus": [
                "file_id": identifier.rawValue,
            ],
        ])

        if let error = response["Error"] as? [String: Any] {
            throw BeebeebIPCError.daemonRejected(error["message"] as? String ?? "item lookup failed")
        }
        guard let payload = response["FileStatus"] as? [String: Any],
              let item = Self.decodeItem(payload) else {
            throw BeebeebIPCError.invalidResponse("daemon response did not include FileStatus")
        }
        return item
    }

    /// Ask the daemon to download + decrypt `itemIdentifier` into
    /// `destinationURL`. Blocks the calling thread until the daemon answers,
    /// so call it off the File Provider callback queue.
    ///
    /// `onProgress(done, total)` is called (on the calling thread) with
    /// plaintext byte counts as the daemon works; `cancellation.cancel()` from
    /// another thread aborts the request (and the daemon stops downloading).
    func hydrateFile(
        itemIdentifier: NSFileProviderItemIdentifier,
        destinationURL: URL,
        cancellation: IPCCancellation? = nil,
        onProgress: ((Int64, Int64) -> Void)? = nil
    ) throws {
        var payload: [String: Any] = [
            "file_id": itemIdentifier.rawValue,
            "dest_path": destinationURL.path,
        ]
        if onProgress != nil {
            // Opt in: an older daemon ignores the field and sends no progress.
            payload["progress"] = true
        }
        let response = try sendRequest(
            ["HydrateFile": payload],
            timeoutSeconds: IPCFraming.hydrateTimeoutSeconds,
            cancellation: cancellation,
            onProgress: onProgress
        )
        if let error = response["Error"] as? [String: Any] {
            throw BeebeebIPCError.daemonRejected(error["message"] as? String ?? "hydration failed")
        }
    }

    /// Task 1684: the request is built by `IPCWriteRequest` (IPCFraming.swift),
    /// which derives a STABLE `request_id` from the operation's inputs and the
    /// staged file's size + mtime, so the system's retry of a createItem that
    /// timed out here is recognised by the daemon instead of queueing the same
    /// upload again. Do not hand-build this payload: scripts/check-ipc-
    /// timeouts.py fails if this call site stops going through the builder.
    func queueCreateItem(
        parentIdentifier: NSFileProviderItemIdentifier,
        filename: String,
        kind: BeebeebItemKind,
        contentsURL: URL?,
        contentType: String?
    ) throws -> WriteQueueResult {
        // The app cannot open the system's contents URL (it is outside the
        // app's sandbox), so it gets an App Group copy instead; see
        // `UploadStaging`. The key is still derived from the system's file:
        // that is what stays the same across the system's retries.
        let stagedContents = try stageUploadContents(contentsURL, kind: kind)
        defer {
            if let stagedContents {
                UploadStaging.discard(stagedContents)
            }
        }
        let request = IPCWriteRequest.create(
            parentIdentifier: parentIdentifier.rawValue,
            filename: filename,
            kind: kind.rawValue,
            contentsPath: stagedContents?.path,
            contentType: contentType,
            contents: contentsURL.flatMap { IPCContentFingerprint.ofFile(at: $0) }
        )
        Self.logRequestID(of: request, operation: "create")
        return try decodeWriteResponse(sendRequest(
            request,
            timeoutSeconds: IPCFraming.writeQueueTimeoutSeconds(hasContents: contentsURL != nil)
        ))
    }

    /// Task 1684: see `queueCreateItem`; the modify key also folds in the base
    /// version and the changed-fields mask.
    func queueModifyItem(
        itemIdentifier: NSFileProviderItemIdentifier,
        parentIdentifier: NSFileProviderItemIdentifier,
        filename: String,
        kind: BeebeebItemKind,
        contentsURL: URL?,
        contentType: String?,
        baseVersionIdentifier: String?,
        changedFields: NSFileProviderItemFields
    ) throws -> WriteQueueResult {
        // See `queueCreateItem`: the app gets an App Group copy.
        let stagedContents = try stageUploadContents(contentsURL, kind: kind)
        defer {
            if let stagedContents {
                UploadStaging.discard(stagedContents)
            }
        }
        let request = IPCWriteRequest.modify(
            itemIdentifier: itemIdentifier.rawValue,
            parentIdentifier: parentIdentifier.rawValue,
            filename: filename,
            kind: kind.rawValue,
            contentsPath: stagedContents?.path,
            contentType: contentType,
            baseVersionIdentifier: baseVersionIdentifier,
            changedFields: UInt64(truncatingIfNeeded: changedFields.rawValue),
            contents: contentsURL.flatMap { IPCContentFingerprint.ofFile(at: $0) }
        )
        Self.logRequestID(of: request, operation: "modify")
        return try decodeWriteResponse(sendRequest(
            request,
            timeoutSeconds: IPCFraming.writeQueueTimeoutSeconds(hasContents: contentsURL != nil)
        ))
    }

    /// Task 1684 device rung: log the first 12 characters of the write key (a
    /// SHA-256, not secret, no file name) so the unified log shows whether the
    /// system's retry of a timed-out write recomputed the SAME key.
    private static func logRequestID(of request: [String: Any], operation: String) {
        for payload in request.values {
            if let fields = payload as? [String: Any], let id = fields["request_id"] as? String {
                NSLog("BeebeebFileProvider: \(operation) request_id=\(id.prefix(12))")
            }
        }
    }

    /// Copy a FILE write's contents into the App Group upload-staging
    /// directory for the app. A folder has no contents to hand over. Throws
    /// the transient `uploadStagingFailed`; the caller deletes the copy once
    /// the exchange is over, whatever its outcome.
    private func stageUploadContents(_ contentsURL: URL?, kind: BeebeebItemKind) throws -> URL? {
        guard kind == .file, let contentsURL else {
            return nil
        }
        do {
            guard let groupContainer = Self.resolveGroupContainer() else {
                throw BeebeebIPCError.uploadStagingFailed("App Group container unavailable")
            }
            return try UploadStaging.stage(contentsOf: contentsURL, in: groupContainer)
        } catch {
            // Category only: the reason carries no path or file name.
            NSLog("BeebeebFileProvider: upload staging failed: \(error.localizedDescription)")
            throw error
        }
    }

    func queueDeleteItem(
        itemIdentifier: NSFileProviderItemIdentifier,
        baseVersionIdentifier: String?
    ) throws -> WriteQueueResult {
        var payload: [String: Any] = [
            "file_id": itemIdentifier.rawValue,
        ]
        if let baseVersionIdentifier {
            payload["base_version_identifier"] = baseVersionIdentifier
        }
        return try decodeWriteResponse(sendRequest(["QueueFinderDelete": payload]))
    }

    // MARK: - Thumbnails (task 1699)

    /// The FetchThumbnail request shape. The daemon downloads the encrypted
    /// variant matching `maxDimension` (small/medium/large — see
    /// `FileProviderExtension.thumbnailVariant`, mirroring the Rust bucket
    /// picker), decrypts it and atomically stages the PLAINTEXT at
    /// `destinationPath` (must be under the daemon's allowed roots — the
    /// hydrate-cache directory qualifies), then replies
    /// `{"ThumbnailWritten":{"size_bytes":N}}`. The extension reads the file
    /// back, deletes it, and hands the Data to the system. Mirrors hydrate's
    /// staging handoff (docs/IPC_PROTOCOL.md, "FetchThumbnail").
    static func thumbnailRequest(fileID: String, destinationPath: String, maxDimension: UInt32) -> [String: Any] {
        return ["FetchThumbnail": [
            "file_id": fileID,
            "dest_path": destinationPath,
            "max_dimension": maxDimension,
        ]]
    }

    /// The `ThumbnailWritten` reply decoder, static so the framing harness
    /// can pin the wire shape without a socket (mirror of `decodeChanges`).
    /// An `Error` reply is a deliberate daemon-side refusal/failure (the
    /// thumbnail could not be fetched or staged) — definitive per the
    /// daemonRejected contract.
    static func decodeThumbnailReply(_ response: [String: Any]) throws -> Int64 {
        if let error = response["Error"] as? [String: Any] {
            throw BeebeebIPCError.daemonRejected(error["message"] as? String ?? "thumbnail fetch failed")
        }
        guard let payload = response["ThumbnailWritten"] as? [String: Any],
              let size = (payload["size_bytes"] as? NSNumber)?.int64Value else {
            throw BeebeebIPCError.invalidResponse("daemon response did not include ThumbnailWritten")
        }
        return size
    }

    /// Fetch one thumbnail: the daemon decrypts it into `destinationURL`
    /// (the App-Group hydrate-cache staging area, like hydrate), and the
    /// byte count is returned (informational). Blocks the calling thread
    /// until the daemon answers, so call it off the File Provider callback
    /// queue. The caller owns deleting the staged file.
    func fetchThumbnail(
        itemIdentifier: NSFileProviderItemIdentifier,
        destinationURL: URL,
        maxDimension: UInt32,
        cancellation: IPCCancellation? = nil
    ) throws -> Int64 {
        let response = try sendRequest(
            Self.thumbnailRequest(
                fileID: itemIdentifier.rawValue,
                destinationPath: destinationURL.path,
                maxDimension: maxDimension
            ),
            timeoutSeconds: IPCFraming.thumbnailTimeoutSeconds,
            cancellation: cancellation
        )
        return try Self.decodeThumbnailReply(response)
    }

    // MARK: - Change feed + sync anchor (task 1697)

    /// One daemon change-log row, as the replica's `enumerateChanges` consumes
    /// it. `item` is the FULL item payload for created/modified/reparented
    /// rows (so the enumerator can `didUpdateItems` without a second lookup)
    /// and nil for deletions.
    struct DaemonChange {
        let fileID: String
        /// created | modified | deleted | reparented
        let kind: String
        let oldParentID: String?
        let newParentID: String?
        let item: BeebeebProviderItem?
    }

    struct ChangesPage {
        let changes: [DaemonChange]
        /// The anchor to persist + finish with. `nil` only when the log is
        /// empty (nothing was ever recorded).
        let nextAnchor: String?
    }

    /// One page of the daemon's change log, ordered oldest-first, resuming
    /// after `sinceAnchor` (nil = from the retained beginning). The reply
    /// carries the anchor to persist; paging stops when the reply's anchor
    /// equals the request's.
    func listChanges(sinceAnchor: String?, limit: Int) throws -> ChangesPage {
        var payload: [String: Any] = ["limit": limit]
        if let sinceAnchor {
            payload["since_anchor"] = sinceAnchor
        }
        let response = try sendRequest(["ListChanges": payload])
        if let error = response["Error"] as? [String: Any] {
            throw BeebeebIPCError.daemonRejected(error["message"] as? String ?? "change enumeration failed")
        }
        return Self.decodeChanges(response)
    }

    /// The `FileProviderChanges` payload decoder, internal so the framing
    /// harness can pin the wire shape without a socket (mirror via the test
    /// extension in BeebeebFileProviderTests/main.swift).
    static func decodeChanges(_ response: [String: Any]) -> ChangesPage {
        guard let changes = response["FileProviderChanges"] as? [String: Any] else {
            return ChangesPage(changes: [], nextAnchor: nil)
        }
        let rawChanges = changes["changes"] as? [[String: Any]] ?? []
        let decoded = rawChanges.map { raw -> DaemonChange in
            let item = (raw["item"] as? [String: Any]).flatMap(Self.decodeItem)
            return DaemonChange(
                fileID: raw["file_id"] as? String ?? "",
                kind: raw["kind"] as? String ?? "",
                oldParentID: raw["old_parent_id"] as? String,
                newParentID: raw["new_parent_id"] as? String,
                item: item
            )
        }
        return ChangesPage(changes: decoded, nextAnchor: changes["next_anchor"] as? String)
    }

    /// The daemon's persistent change-log cursor — what `currentSyncAnchor`
    /// reports after extension process death. `nil` when nothing ever changed.
    func syncAnchor() throws -> String? {
        let response = try sendRequest(["GetSyncAnchor": [String: Any]()])
        if let error = response["Error"] as? [String: Any] {
            throw BeebeebIPCError.daemonRejected(error["message"] as? String ?? "anchor lookup failed")
        }
        guard let payload = response["FileProviderSyncAnchor"] as? [String: Any] else {
            throw BeebeebIPCError.invalidResponse("daemon response did not include FileProviderSyncAnchor")
        }
        return payload["anchor"] as? String
    }

    /// Publish the materialized container set the system reported, so the
    /// daemon can filter working-set signals to materialized parents.
    /// Best-effort: a failure is swallowed (the signal is a performance hint,
    /// not a correctness requirement).
    func reportMaterializedContainers(_ containerIDs: [String]) {
        _ = try? sendRequest(["ReportMaterialized": ["container_ids": containerIDs]])
    }

    /// Decode one `FileProviderItemPayload` from the daemon's wire JSON.
    /// `static` (not `private`) so the framing harness can pin the wire shape
    /// without a socket (BeebeebFileProviderTests).
    static func decodeItem(_ dictionary: [String: Any]) -> BeebeebProviderItem? {
        guard let identifier = dictionary["identifier"] as? String,
              let parentIdentifier = dictionary["parent_identifier"] as? String,
              let filename = dictionary["filename"] as? String,
              let kindRaw = dictionary["kind"] as? String,
              let kind = BeebeebItemKind(rawValue: kindRaw),
              let status = dictionary["status"] as? String else {
            return nil
        }

        // Task 1697: all new fields default when absent (older daemons), and
        // zero timestamps mean "the daemon doesn't know" -> no date.
        func date(_ key: String) -> Date? {
            guard let seconds = (dictionary[key] as? NSNumber)?.doubleValue, seconds > 0 else {
                return nil
            }
            return Date(timeIntervalSince1970: seconds)
        }

        return BeebeebProviderItem(
            identifier: identifier,
            parentIdentifier: parentIdentifier,
            filename: filename,
            kind: kind,
            sizeBytes: (dictionary["size_bytes"] as? NSNumber)?.int64Value ?? 0,
            contentType: dictionary["content_type"] as? String,
            status: status,
            capabilities: (dictionary["capabilities"] as? NSNumber)?.intValue ?? BeebeebProviderItem.read,
            versionIdentifier: dictionary["version_identifier"] as? String,
            createdAt: date("created_at"),
            modifiedAt: date("modified_at"),
            childItemCount: (dictionary["child_item_count"] as? NSNumber)?.int64Value,
            contentVersion: dictionary["content_version"] as? String,
            metadataVersion: dictionary["metadata_version"] as? String,
            pinned: (dictionary["pinned"] as? NSNumber)?.boolValue ?? false
        )
    }

    private func decodeWriteResponse(_ response: [String: Any]) throws -> WriteQueueResult {
        if let error = response["Error"] as? [String: Any] {
            throw BeebeebIPCError.daemonRejected(error["message"] as? String ?? "Finder write failed")
        }
        guard let payload = response["WriteQueued"] as? [String: Any] else {
            throw BeebeebIPCError.invalidResponse("daemon response did not include WriteQueued")
        }
        let item: BeebeebProviderItem?
        if let rawItem = payload["item"] as? [String: Any] {
            item = Self.decodeItem(rawItem)
        } else {
            item = nil
        }
        return WriteQueueResult(
            item: item,
            ignored: (payload["ignored"] as? NSNumber)?.boolValue ?? false,
            message: payload["message"] as? String ?? ""
        )
    }

    /// One framed request/reply exchange with the daemon (see
    /// `IPCFraming.swift` and docs/IPC_PROTOCOL.md). Every failure is a
    /// `BeebeebIPCError` with fixed, human-readable text -- raw reply bytes
    /// are never put in an error message.
    private func sendRequest(
        _ request: [String: Any],
        timeoutSeconds: Int = IPCFraming.metadataTimeoutSeconds,
        cancellation: IPCCancellation? = nil,
        onProgress: ((Int64, Int64) -> Void)? = nil
    ) throws -> [String: Any] {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw BeebeebIPCError.daemonUnavailable }
        defer { close(fd) }

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        _ = socketPath.withCString { ptr in
            withUnsafeMutablePointer(to: &addr.sun_path) {
                $0.withMemoryRebound(to: CChar.self, capacity: 108) {
                    strncpy($0, ptr, 107)
                }
            }
        }

        let size = MemoryLayout<sockaddr_un>.size
        let connected = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(fd, $0, socklen_t(size)) == 0
            }
        }
        guard connected else { throw BeebeebIPCError.daemonUnavailable }

        do {
            return try IPCExchange.perform(
                fd: fd,
                request: request,
                timeoutSeconds: timeoutSeconds,
                cancellation: cancellation,
                onProgress: onProgress
            )
        } catch let error as IPCTransportError {
            throw Self.map(error, timeoutSeconds: timeoutSeconds)
        }
    }

    private static func map(_ error: IPCTransportError, timeoutSeconds: Int) -> BeebeebIPCError {
        switch error {
        case .timedOut:
            return .timedOut(seconds: timeoutSeconds)
        case .cancelled:
            return .cancelled
        case .encodingFailed:
            return .invalidResponse("Could not encode the request for Beebeeb's sync engine.")
        case .closedBeforeReply:
            return .invalidResponse("Beebeeb's sync engine closed the connection before answering. Reopen Beebeeb and try again.")
        case .replyTooLarge(let limit):
            return .invalidResponse("Beebeeb's sync engine sent a reply larger than Finder can accept (\(limit / (1024 * 1024)) MB limit).")
        case .malformedReply:
            return .invalidResponse("Beebeeb's sync engine sent a reply the Finder extension could not read. Update Beebeeb and try again.")
        case .readFailed(let code), .writeFailed(let code):
            if code == EPIPE || code == ECONNRESET {
                return .daemonUnavailable
            }
            return .invalidResponse("Lost the connection to Beebeeb's sync engine (error \(code)).")
        }
    }
}

struct WriteQueueResult {
    let item: BeebeebProviderItem?
    let ignored: Bool
    let message: String
}

/// Task 1698 (error truth): remembers the most recent TRANSIENT error this
/// extension reported to the system, so a later successful operation can call
/// `-[NSFileProviderManager signalErrorResolved:completionHandler:]` with the
/// SAME error. Apple (Mgr.h): the call "causes the system to cancel throttling
/// on every item which has been throttled due to the given error" — without
/// it, one `serverUnreachable` blip leaves the replica throttled even after
/// the daemon comes back, until an OS-driven retry window happens to open.
///
/// Thread-safe: the File Provider callbacks arrive on arbitrary queues.
final class TransientErrorTracker: @unchecked Sendable {
    private let lock = NSLock()
    private var pending: NSError?

    /// Record a failure. Only transient errors (per `BeebeebIPCError
    /// .isTransient`) arm the resolved-signal; a definitive failure must never
    /// tell the system "an earlier transient problem is resolved".
    func record(_ error: NSError) {
        lock.lock()
        defer { lock.unlock() }
        guard error.domain == NSFileProviderErrorDomain,
              error.code == NSFileProviderError.serverUnreachable.rawValue else {
            return
        }
        pending = error
    }

    var hasPending: Bool {
        lock.lock()
        defer { lock.unlock() }
        return pending != nil
    }

    /// Take (and clear) the pending transient error for a
    /// `signalErrorResolved` call after a successful operation.
    func takePending() -> NSError? {
        lock.lock()
        defer { lock.unlock() }
        let error = pending
        pending = nil
        return error
    }
}
