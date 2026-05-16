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
    private let socketPath: String

    init() {
        let runtimeDir = ProcessInfo.processInfo.environment["XDG_RUNTIME_DIR"] ?? "/tmp"
        self.socketPath = "\(runtimeDir)/beebeeb-daemon.sock"
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
