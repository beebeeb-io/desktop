// spike(2176): entry point of BeebeebSwapProbe.xpc, a bundled XPC service in
// Beebeeb.app/Contents/XPCServices. It is NOT sandboxed: it is signed with the hardened runtime and
// no entitlements at all (the absence of com.apple.security.app-sandbox is what lets it write
// /Applications). THROWAWAY device-proof code, never merged.

import Foundation

final class ListenerDelegate: NSObject, NSXPCListenerDelegate {
    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection) -> Bool {
        guard let policy = SwapPolicy.production() else { return false }
        // macOS 13+: every incoming message is checked against this requirement, and the
        // connection is invalidated when the sender does not satisfy it. Only Beebeeb itself,
        // Developer ID signed by team R8352WDJJR, may talk to this service.
        connection.setCodeSigningRequirement(BeebeebIdentity.developerIDRequirement(identifier: BeebeebIdentity.appIdentifier))
        connection.exportedInterface = NSXPCInterface(with: BeebeebSwapProbeProtocol.self)
        connection.exportedObject = SwapProbeService(policy: policy)
        connection.resume()
        return true
    }
}

let delegate = ListenerDelegate()
let listener = NSXPCListener.service()
listener.delegate = delegate
listener.resume() // never returns
