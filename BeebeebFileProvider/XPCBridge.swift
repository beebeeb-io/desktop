import FileProvider
import Foundation

enum BeebeebIPCError: LocalizedError {
    case daemonUnavailable
    case invalidResponse(String)
    case invalidIdentifier
    /// The daemon accepted the connection but sent nothing for `seconds`
    /// (task 1670 issue 3: previously a stall looked like "not valid JSON").
    case timedOut(seconds: Int)
    /// Finder cancelled the transfer.
    case cancelled

    var errorDescription: String? {
        switch self {
        case .daemonUnavailable:
            return "Open Beebeeb and unlock your vault."
        case .invalidResponse(let message):
            return message
        case .invalidIdentifier:
            return "This item's identifier could not be used for a Finder operation."
        case .timedOut(let seconds):
            return "Beebeeb's sync engine did not answer within \(seconds) seconds. Open Beebeeb, make sure it is unlocked, and try again."
        case .cancelled:
            return "The transfer was cancelled."
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
            // Covers the daemon's own rejections (e.g. a hydration
            // destination outside an allowed root, task 1670's actual root
            // cause) and any other explicit error message it returned.
            return NSFileProviderError.cannotSynchronize.rawValue
        case .invalidIdentifier:
            // Task 1670 round 2: same category as `.invalidResponse` — the
            // request is refused before it ever reaches the daemon.
            return NSFileProviderError.cannotSynchronize.rawValue
        case .timedOut:
            // The daemon is there but not answering: same category as it
            // being unreachable.
            return NSFileProviderError.serverUnreachable.rawValue
        case .cancelled:
            return NSFileProviderError.cannotSynchronize.rawValue
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
            throw BeebeebIPCError.invalidResponse(error["message"] as? String ?? "daemon returned an error")
        }
        guard let payload = response["FileProviderItems"] as? [String: Any],
              let rawItems = payload["items"] as? [[String: Any]] else {
            throw BeebeebIPCError.invalidResponse("daemon response did not include FileProviderItems")
        }

        return rawItems.compactMap(Self.decodeItem)
    }

    func item(identifier: NSFileProviderItemIdentifier) throws -> BeebeebProviderItem {
        if identifier == .rootContainer {
            return BeebeebProviderItem(
                identifier: identifier.rawValue,
                parentIdentifier: identifier.rawValue,
                filename: "Beebeeb",
                kind: .folder,
                sizeBytes: 0,
                contentType: nil,
                status: "local",
                capabilities: BeebeebProviderItem.read,
                versionIdentifier: nil
            )
        }

        let response = try sendRequest([
            "GetFileStatus": [
                "file_id": identifier.rawValue,
            ],
        ])

        if let error = response["Error"] as? [String: Any] {
            throw BeebeebIPCError.invalidResponse(error["message"] as? String ?? "item lookup failed")
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
            throw BeebeebIPCError.invalidResponse(error["message"] as? String ?? "hydration failed")
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
        let request = IPCWriteRequest.create(
            parentIdentifier: parentIdentifier.rawValue,
            filename: filename,
            kind: kind.rawValue,
            contentsPath: contentsURL?.path,
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
        let request = IPCWriteRequest.modify(
            itemIdentifier: itemIdentifier.rawValue,
            parentIdentifier: parentIdentifier.rawValue,
            filename: filename,
            kind: kind.rawValue,
            contentsPath: contentsURL?.path,
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

    private static func decodeItem(_ dictionary: [String: Any]) -> BeebeebProviderItem? {
        guard let identifier = dictionary["identifier"] as? String,
              let parentIdentifier = dictionary["parent_identifier"] as? String,
              let filename = dictionary["filename"] as? String,
              let kindRaw = dictionary["kind"] as? String,
              let kind = BeebeebItemKind(rawValue: kindRaw),
              let status = dictionary["status"] as? String else {
            return nil
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
            versionIdentifier: dictionary["version_identifier"] as? String
        )
    }

    private func decodeWriteResponse(_ response: [String: Any]) throws -> WriteQueueResult {
        if let error = response["Error"] as? [String: Any] {
            throw BeebeebIPCError.invalidResponse(error["message"] as? String ?? "Finder write failed")
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
