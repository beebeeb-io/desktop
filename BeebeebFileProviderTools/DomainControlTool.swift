import FileProvider
import Foundation

@main
struct DomainControlTool {
    static let identifier = NSFileProviderDomainIdentifier("io.beebeeb.app.domain")
    static let displayName = "Drive"

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

    /// Task 1882 (spec docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md): keeps the
    /// files that never reached the server, like every removal in the app. Prints `removed`, then
    /// `preserved: <folder>` when macOS kept any (this is a developer tool, so the path goes to
    /// its own stdout). Round 2 (device K-F2): macOS reports a folder even when it kept nothing,
    /// so `preserved:` is printed only when that folder exists, by the app's own rule. Round 3
    /// (re-review D1): an existing folder counts whether it is empty or not, and a missing one is
    /// looked at again for about a second first.
    static func remove() {
        let semaphore = DispatchSemaphore(value: 0)
        var exitCode: Int32 = 1

        NSFileProviderManager.remove(domain, mode: .preserveDirtyUserData) { preservedLocation, error in
            if let error {
                fputs("\(error.localizedDescription)\n", stderr)
            } else {
                print("removed")
                let path = preservedLocation?.path ?? ""
                switch settledKeptFolder(preservedLocation) {
                case .hasEntries, .empty, .unchecked:
                    print("preserved: \(path)")
                case .missing:
                    print("nothing kept: macOS reported \(path), which is missing")
                case .noPath:
                    print("removed; macOS reported kept files without a folder")
                case .notReported:
                    break
                }
                exitCode = 0
            }
            semaphore.signal()
        }

        semaphore.wait()
        exit(exitCode)
    }

    /// What the folder macOS reported holds. The same rule as `BeebeebKeptFolderState` in
    /// src-tauri/macos/FileProviderBridge.m (spec §4, §5): it looks through the URL's own scope,
    /// stops at "something is there", and never opens a file.
    enum KeptFolder {
        case notReported, missing, empty, hasEntries, unchecked, noPath
    }

    /// The app's `settle_kept_state` (src-tauri/src/finder_removal.rs, re-review D1): a folder that
    /// reads as missing is looked at again, on the same URL. The constants are the app's;
    /// a test pins them equal.
    static let keptMissingRechecks = 4
    static let keptMissingRecheckInterval: TimeInterval = 0.25

    static func settledKeptFolder(_ location: URL?) -> KeptFolder {
        var state = keptFolder(location)
        guard state == .missing else { return state }
        for _ in 0..<keptMissingRechecks {
            Thread.sleep(forTimeInterval: keptMissingRecheckInterval)
            state = keptFolder(location)
            if state != .missing { return state }
        }
        return state
    }

    static func keptFolder(_ location: URL?) -> KeptFolder {
        guard let location else { return .notReported }
        let path = location.path
        guard !path.isEmpty else { return .noPath }
        let scoped = location.startAccessingSecurityScopedResource()
        defer {
            if scoped { location.stopAccessingSecurityScopedResource() }
        }
        var info = stat()
        guard lstat(path, &info) == 0 else {
            return errno == ENOENT || errno == ENOTDIR ? .missing : .unchecked
        }
        guard info.st_mode & S_IFMT == S_IFDIR else { return .hasEntries }
        guard let entries = try? FileManager.default.contentsOfDirectory(atPath: path) else {
            return .unchecked
        }
        return entries.isEmpty ? .empty : .hasEntries
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
