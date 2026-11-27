import CryptoKit
import FileProvider
import Foundation

// drive-macos-89: a create the system marks `.mayAlreadyExist` (a reimport or a re-enumeration
// replays creates for items the server may already have, dm-54).
//
// Owns: deciding whether the server's item under the same parent, name and kind IS the item being
// created. A folder is. A file is only when its bytes are the server's (size, then the SHA-256 the
// daemon puts in the content version, `{version}:{sha256 hex}`). A file that differs, or that
// cannot be shown equal, is uploaded as a new version of the server's item: an offline edit
// replayed as a create reaches the server, and the server's earlier version stays in its history.
// Must not: return the server's item for a file without comparing the bytes (that dropped the edit).
// Tests: BeebeebFileProviderTests/MayAlreadyExistTests.swift.

extension FileProviderExtension {
    enum MayAlreadyExistAction: Equatable {
        /// The server has no such item: create it as usual.
        case create
        /// The server's item is this one (a folder, or a file with the same bytes).
        case returnExisting
        /// The local file differs, or cannot be shown equal: upload it as a new version.
        case uploadNewVersion
    }

    /// dm-54: the item already under `parent` with this name and kind, if any.
    static func existingItem(
        parent: NSFileProviderItemIdentifier,
        filename: String,
        kind: BeebeebItemKind,
        list: (NSFileProviderItemIdentifier) throws -> [BeebeebProviderItem]
    ) throws -> BeebeebProviderItem? {
        try list(parent).first { $0.filename == filename && $0.kind == kind }
    }

    /// The SHA-256 (lower-case hex) a content version `{version}:{hash}` carries, if it carries one.
    static func contentHash(ofContentVersion version: String?) -> String? {
        guard let version else { return nil }
        let parts = version.split(separator: ":", omittingEmptySubsequences: false)
        guard parts.count == 2, Int64(parts[0]).map({ $0 > 0 }) == true else { return nil }
        let hash = parts[1].lowercased()
        guard hash.count == 64, hash.allSatisfy({ $0.isHexDigit }) else { return nil }
        return hash
    }

    /// Lower-case hex SHA-256 of the file at `url`, read in chunks and kept in memory only.
    static func sha256Hex(of url: URL) throws -> String {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var hasher = SHA256()
        while let chunk = try handle.read(upToCount: 1 << 20), !chunk.isEmpty {
            hasher.update(data: chunk)
        }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }

    static func mayAlreadyExistAction(
        existing: BeebeebProviderItem?,
        contents: URL?,
        hashFile: (URL) throws -> String = sha256Hex(of:)
    ) -> MayAlreadyExistAction {
        guard let existing else { return .create }
        if existing.kind == .folder { return .returnExisting }
        // No bytes handed over: nothing local to keep.
        guard let contents else { return .returnExisting }
        guard
            let size = (try? contents.resourceValues(forKeys: [.fileSizeKey]))?.fileSize,
            Int64(size) == existing.sizeBytes,
            let serverHash = contentHash(ofContentVersion: existing.contentVersion),
            let localHash = try? hashFile(contents)
        else { return .uploadNewVersion }
        return localHash.lowercased() == serverHash ? .returnExisting : .uploadNewVersion
    }

    /// What a `.mayAlreadyExist` create completes with, or nil when the server has no such item and
    /// the create goes ahead as usual. `queueNewVersion` queues the local file as a new version of
    /// the server's item (`queueModifyItem` on it, based on the content version it holds now).
    static func resolveMayAlreadyExist(
        parent: NSFileProviderItemIdentifier,
        filename: String,
        kind: BeebeebItemKind,
        contents: URL?,
        list: (NSFileProviderItemIdentifier) throws -> [BeebeebProviderItem],
        queueNewVersion: (BeebeebProviderItem) throws -> WriteQueueResult,
        hashFile: (URL) throws -> String = sha256Hex(of:)
    ) throws -> QueuedWriteCompletion? {
        let existing = try existingItem(parent: parent, filename: filename, kind: kind, list: list)
        switch mayAlreadyExistAction(existing: existing, contents: contents, hashFile: hashFile) {
        case .create:
            return nil
        case .returnExisting:
            return QueuedWriteCompletion(item: existing.map(FileProviderItem.init(model:)), shouldFetchContent: false, error: nil)
        case .uploadNewVersion:
            // `existing` is non-nil here: `.create` is the only answer for a missing item.
            return queuedWriteCompletion(try queueNewVersion(existing!))
        }
    }
}
