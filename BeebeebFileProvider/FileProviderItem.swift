import FileProvider
import Foundation
import UniformTypeIdentifiers

enum BeebeebItemKind: String, Codable {
    case namespace
    case folder
    case file
}

enum BeebeebNamespace: String, CaseIterable {
    case myFiles = "my_files"
    case sharedWithMe = "shared_with_me"
    case offline
    case conflicts

    var identifier: NSFileProviderItemIdentifier {
        NSFileProviderItemIdentifier("namespace:\(rawValue)")
    }

    var displayName: String {
        switch self {
        case .myFiles: return "My files"
        case .sharedWithMe: return "Shared with me"
        case .offline: return "Offline"
        case .conflicts: return "Conflicts"
        }
    }
}

struct BeebeebProviderItem: Codable {
    let identifier: String
    let parentIdentifier: String
    let filename: String
    let kind: BeebeebItemKind
    let sizeBytes: Int64
    let contentType: String?
    let status: String
    let capabilities: Int
    let versionIdentifier: String?

    static let read = 1 << 0
    static let write = 1 << 1
    static let rename = 1 << 2
    static let delete = 1 << 3

    static func namespace(_ namespace: BeebeebNamespace) -> BeebeebProviderItem {
        BeebeebProviderItem(
            identifier: namespace.identifier.rawValue,
            parentIdentifier: NSFileProviderItemIdentifier.rootContainer.rawValue,
            filename: namespace.displayName,
            kind: .namespace,
            sizeBytes: 0,
            contentType: UTType.folder.identifier,
            status: "local",
            capabilities: read,
            versionIdentifier: nil
        )
    }
}

final class FileProviderItem: NSObject, NSFileProviderItem {
    let model: BeebeebProviderItem

    init(model: BeebeebProviderItem) {
        self.model = model
        super.init()
    }

    var itemIdentifier: NSFileProviderItemIdentifier {
        NSFileProviderItemIdentifier(model.identifier)
    }

    var parentItemIdentifier: NSFileProviderItemIdentifier {
        NSFileProviderItemIdentifier(model.parentIdentifier)
    }

    var filename: String {
        model.filename
    }

    var contentType: UTType {
        if model.kind == .namespace || model.kind == .folder {
            return .folder
        }
        if let raw = model.contentType, let type = UTType(raw) {
            return type
        }
        return .data
    }

    var documentSize: NSNumber? {
        model.kind == .file ? NSNumber(value: model.sizeBytes) : nil
    }

    var childItemCount: NSNumber? {
        model.kind == .file ? nil : NSNumber(value: 0)
    }

    var itemVersion: NSFileProviderItemVersion {
        let version = model.versionIdentifier ?? "\(model.status):\(model.sizeBytes)"
        let data = Data(version.utf8)
        return NSFileProviderItemVersion(contentVersion: data, metadataVersion: data)
    }

    var isUploaded: Bool {
        true
    }

    var isDownloaded: Bool {
        model.kind != .file || model.status == "local"
    }

    var isMostRecentVersionDownloaded: Bool {
        isDownloaded
    }

    var capabilities: NSFileProviderItemCapabilities {
        var result: NSFileProviderItemCapabilities = []
        if model.capabilities & BeebeebProviderItem.read != 0 {
            result.insert(.allowsReading)
        }
        if model.capabilities & BeebeebProviderItem.write != 0 {
            result.insert(.allowsWriting)
        }
        if model.capabilities & BeebeebProviderItem.rename != 0 {
            result.insert(.allowsRenaming)
        }
        if model.capabilities & BeebeebProviderItem.delete != 0 {
            result.insert(.allowsDeleting)
        }
        return result
    }
}
