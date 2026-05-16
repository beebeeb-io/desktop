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
        let destinationURL = FileManager.default.temporaryDirectory
            .appendingPathComponent(itemIdentifier.rawValue)

        do {
            try ipc.hydrateFile(itemIdentifier: itemIdentifier, destinationURL: destinationURL)
            let model = try ipc.item(identifier: itemIdentifier)
            completionHandler(destinationURL, FileProviderItem(model: model), nil)
        } catch {
            completionHandler(nil, nil, error)
        }

        progress.completedUnitCount = 1
        return progress
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
