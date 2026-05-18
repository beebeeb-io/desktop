import FileProvider
import Foundation

enum BeebeebFileProviderDomain {
    static let identifier = NSFileProviderDomainIdentifier("io.beebeeb.app.domain")
    static let displayName = "Beebeeb"

    static var domain: NSFileProviderDomain {
        NSFileProviderDomain(identifier: identifier, displayName: displayName)
    }

    static func install(completionHandler: @escaping (Error?) -> Void) {
        NSFileProviderManager.add(domain, completionHandler: completionHandler)
    }

    static func remove(completionHandler: @escaping (Error?) -> Void) {
        NSFileProviderManager.remove(domain, completionHandler: completionHandler)
    }

    static func signalRootEnumerator(completionHandler: @escaping (Error?) -> Void) {
        guard let manager = NSFileProviderManager(for: domain) else {
            completionHandler(BeebeebIPCError.daemonUnavailable)
            return
        }
        manager.signalEnumerator(for: .rootContainer, completionHandler: completionHandler)
    }
}
