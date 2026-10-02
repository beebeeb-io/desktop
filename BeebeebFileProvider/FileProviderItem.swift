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
    // Task 1697: optional metadata from the daemon (absent = not reported;
    // `decodeItem` defaults everything so an older daemon stays compatible).
    let createdAt: Date?
    let modifiedAt: Date?
    /// Real child count for folders; `nil` for files and namespaces.
    let childItemCount: Int64?
    /// contentVersion: changes only when the content identity changes.
    let contentVersion: String?
    /// metadataVersion: changes on rename/move/status — no re-download.
    let metadataVersion: String?

    static let read = 1 << 0
    static let write = 1 << 1
    static let rename = 1 << 2
    static let delete = 1 << 3
    /// .allowsAddingSubItems — task 1694. Bit 4, identical numbering to the
    /// Rust payload builder's CAP_ADD_SUBITEMS (src-tauri/src/ipc_socket.rs);
    /// the payload crosses the XPC bridge as this plain Int.
    static let addSubItems = 1 << 4
    /// .allowsReparenting — task 1697. Bit 5, same numbering as Rust's
    /// CAP_REPARENT. Without it Finder blocks dragging an item OUT (a move).
    static let reparent = 1 << 5
    /// .allowsTrashing — task 1697. Bit 6, same numbering as Rust's CAP_TRASH.
    /// Trash SEMANTICS (trashContainer handling) are task 1698's decision.
    static let trash = 1 << 6

    /// The memberwise constructor with defaults for the 1697 fields, so
    /// existing call sites (and tests) stay unchanged.
    init(
        identifier: String,
        parentIdentifier: String,
        filename: String,
        kind: BeebeebItemKind,
        sizeBytes: Int64,
        contentType: String?,
        status: String,
        capabilities: Int,
        versionIdentifier: String?,
        createdAt: Date? = nil,
        modifiedAt: Date? = nil,
        childItemCount: Int64? = nil,
        contentVersion: String? = nil,
        metadataVersion: String? = nil
    ) {
        self.identifier = identifier
        self.parentIdentifier = parentIdentifier
        self.filename = filename
        self.kind = kind
        self.sizeBytes = sizeBytes
        self.contentType = contentType
        self.status = status
        self.capabilities = capabilities
        self.versionIdentifier = versionIdentifier
        self.createdAt = createdAt
        self.modifiedAt = modifiedAt
        self.childItemCount = childItemCount
        self.contentVersion = contentVersion
        self.metadataVersion = metadataVersion
    }

    static func namespace(_ namespace: BeebeebNamespace) -> BeebeebProviderItem {
        BeebeebProviderItem(
            identifier: namespace.identifier.rawValue,
            parentIdentifier: NSFileProviderItemIdentifier.rootContainer.rawValue,
            filename: namespace.displayName,
            kind: .namespace,
            sizeBytes: 0,
            contentType: UTType.folder.identifier,
            status: "local",
            capabilities: read | addSubItems,
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

    /// Task 1697: the real child count from the daemon (informational;
    /// Finder renders it in list views). The daemon sends it only for
    /// folders; a file's count stays nil.
    var childItemCount: NSNumber? {
        model.kind == .file ? nil : model.childItemCount.flatMap { NSNumber(value: $0) }
    }

    var creationDate: Date? {
        model.createdAt ?? model.modifiedAt
    }

    var contentModificationDate: Date? {
        model.modifiedAt
    }

    /// Task 1697: the version SPLIT. `contentVersion` must change ONLY when
    /// the content identity changes — a change forces a re-download and
    /// invalidates the thumbnail cache. `metadataVersion` covers
    /// mtime/size/parent/name so a rename no longer fakes a content change.
    /// The contentVersion also doubles as the write-key base version (the
    /// Rust write path parses its numeric prefix — see `parse_base_version_
    /// number` in engine_bridge.rs), so it stays `versionIdentifier`'s
    /// format and never becomes a bare hash.
    var itemVersion: NSFileProviderItemVersion {
        let content = model.contentVersion ?? model.versionIdentifier ?? "\(model.status):\(model.sizeBytes)"
        let metadata = model.metadataVersion ?? "\(model.status):\(model.sizeBytes)"
        return NSFileProviderItemVersion(contentVersion: Data(content.utf8), metadataVersion: Data(metadata.utf8))
    }

    var isUploaded: Bool {
        switch model.status {
        case "uploading", "conflict", "error":
            return false
        default:
            return true
        }
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
        if model.capabilities & BeebeebProviderItem.addSubItems != 0 {
            // Task 1694: without this Finder refuses every drop INTO the item
            // (blocked icon) — it is the only flag that governs adding
            // sub-items.
            result.insert(.allowsAddingSubItems)
        }
        if model.capabilities & BeebeebProviderItem.reparent != 0 {
            // Task 1697: without this Finder blocks dragging an item OUT of
            // its parent (a move). Same bridge numbering as Rust CAP_REPARENT.
            result.insert(.allowsReparenting)
        }
        if model.capabilities & BeebeebProviderItem.trash != 0 {
            // Task 1697: without this Finder blocks drag-to-Trash. Trash
            // SEMANTICS (trashContainer handling) are task 1698's decision.
            result.insert(.allowsTrashing)
        }
        return result
    }
}
