import FileProvider
import Foundation

@main
struct DomainControlTool {
    static let identifier = NSFileProviderDomainIdentifier("io.beebeeb.app.domain")
    static let displayName = "Beebeeb"

    static var domain: NSFileProviderDomain {
        NSFileProviderDomain(identifier: identifier, displayName: displayName)
    }

    static func main() {
        let command = CommandLine.arguments.dropFirst().first ?? "status"
        switch command {
        case "status":
            status()
        case "install":
            install()
        case "remove":
            remove()
        case "signal-root":
            signalRoot()
        default:
            fputs("unknown command: \(command)\n", stderr)
            exit(2)
        }
    }

    static func status() {
        let semaphore = DispatchSemaphore(value: 0)
        var exitCode: Int32 = 1

        NSFileProviderManager.getDomainsWithCompletionHandler { domains, error in
            if let error {
                fputs("\(error.localizedDescription)\n", stderr)
            } else if domains.contains(where: { $0.identifier == identifier }) {
                print("installed")
                exitCode = 0
            } else {
                print("missing")
                exitCode = 0
            }
            semaphore.signal()
        }

        semaphore.wait()
        exit(exitCode)
    }

    static func install() {
        let semaphore = DispatchSemaphore(value: 0)
        var exitCode: Int32 = 1

        NSFileProviderManager.add(domain) { error in
            if let error {
                fputs("\(error.localizedDescription)\n", stderr)
            } else {
                print("installed")
                exitCode = 0
            }
            semaphore.signal()
        }

        semaphore.wait()
        exit(exitCode)
    }

    static func remove() {
        let semaphore = DispatchSemaphore(value: 0)
        var exitCode: Int32 = 1

        NSFileProviderManager.remove(domain) { error in
            if let error {
                fputs("\(error.localizedDescription)\n", stderr)
            } else {
                print("removed")
                exitCode = 0
            }
            semaphore.signal()
        }

        semaphore.wait()
        exit(exitCode)
    }

    static func signalRoot() {
        guard let manager = NSFileProviderManager(for: domain) else {
            fputs("File Provider domain is not installed\n", stderr)
            exit(1)
        }

        let semaphore = DispatchSemaphore(value: 0)
        var exitCode: Int32 = 1
        manager.signalEnumerator(for: .rootContainer) { error in
            if let error {
                fputs("\(error.localizedDescription)\n", stderr)
            } else {
                print("signaled")
                exitCode = 0
            }
            semaphore.signal()
        }

        semaphore.wait()
        exit(exitCode)
    }
}
