import FileProvider
import Foundation

enum BeebeebIPCError: LocalizedError {
    case daemonUnavailable
    case invalidResponse(String)

    var errorDescription: String? {
        switch self {
        case .daemonUnavailable:
            return "Open Beebeeb and unlock your vault."
        case .invalidResponse(let message):
            return message
        }
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
        if let groupContainer = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: Self.appGroupID
        ) {
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

    func hydrateFile(
        itemIdentifier: NSFileProviderItemIdentifier,
        destinationURL: URL
    ) throws {
        let response = try sendRequest([
            "HydrateFile": [
                "file_id": itemIdentifier.rawValue,
                "dest_path": destinationURL.path,
            ],
        ])
        if let error = response["Error"] as? [String: Any] {
            throw BeebeebIPCError.invalidResponse(error["message"] as? String ?? "hydration failed")
        }
    }

    func queueCreateItem(
        parentIdentifier: NSFileProviderItemIdentifier,
        filename: String,
        kind: BeebeebItemKind,
        contentsURL: URL?,
        contentType: String?
    ) throws -> WriteQueueResult {
        var payload: [String: Any] = [
            "parent_id": parentIdentifier.rawValue,
            "filename": filename,
            "kind": kind.rawValue,
        ]
        if let contentsURL {
            payload["contents_path"] = contentsURL.path
        }
        if let contentType {
            payload["content_type"] = contentType
        }
        return try decodeWriteResponse(sendRequest(["QueueFinderCreate": payload]))
    }

    func queueModifyItem(
        itemIdentifier: NSFileProviderItemIdentifier,
        parentIdentifier: NSFileProviderItemIdentifier,
        filename: String,
        kind: BeebeebItemKind,
        contentsURL: URL?,
        contentType: String?,
        baseVersionIdentifier: String?
    ) throws -> WriteQueueResult {
        var payload: [String: Any] = [
            "file_id": itemIdentifier.rawValue,
            "parent_id": parentIdentifier.rawValue,
            "filename": filename,
            "kind": kind.rawValue,
        ]
        if let contentsURL {
            payload["contents_path"] = contentsURL.path
        }
        if let contentType {
            payload["content_type"] = contentType
        }
        if let baseVersionIdentifier {
            payload["base_version_identifier"] = baseVersionIdentifier
        }
        return try decodeWriteResponse(sendRequest(["QueueFinderModify": payload]))
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

    private func sendRequest(_ request: [String: Any]) throws -> [String: Any] {
        guard let data = try? JSONSerialization.data(withJSONObject: request) else {
            throw BeebeebIPCError.invalidResponse("could not encode daemon request")
        }

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

        _ = data.withUnsafeBytes { write(fd, $0.baseAddress, data.count) }

        var buffer = Data(count: 65536)
        let count = buffer.withUnsafeMutableBytes { read(fd, $0.baseAddress, 65536) }
        guard count > 0,
              let response = try? JSONSerialization.jsonObject(with: buffer.prefix(count)) as? [String: Any] else {
            throw BeebeebIPCError.invalidResponse("daemon response was not valid JSON")
        }
        return response
    }
}

struct WriteQueueResult {
    let item: BeebeebProviderItem?
    let ignored: Bool
    let message: String
}
