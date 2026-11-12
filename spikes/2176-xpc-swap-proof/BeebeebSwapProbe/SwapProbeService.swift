// spike(2176): the XPC face of the swap service. THROWAWAY device-proof code, never merged.
//
// Logging: one os.Logger, subsystem io.beebeeb.app.SwapProbe; every value is a closed code (result
// code, stage, errno or OSStatus, pid). Never a path or a file name, same rule as ExtensionLog.

import AppKit
import Darwin
import Foundation
import os
import XPC

/// Shared with the app's client (spikes/2176-xpc-swap-proof/client/Spike2176SwapProbeClient.m),
/// which declares the same selectors and block types in Objective-C.
@objc(BeebeebSwapProbeProtocol)
protocol BeebeebSwapProbeProtocol {
    /// "pid=..;uid=..;home_is_container=0|1;sandbox_env=0|1;version=..". No paths.
    @objc(pingWithReply:)
    func ping(withReply reply: @escaping (String) -> Void)

    /// P1 (SPIKE ONLY, delete before any production use): write a small file outside the app's
    /// container. Restricted to `<home>/beebeeb-spike-2176/<name>` so the probe cannot write anywhere.
    @objc(writeProbe:withReply:)
    func writeProbe(_ path: String, withReply reply: @escaping (String, String, Int32) -> Void)

    /// The swap: (code, stage, err). See SwapCore.performSwap.
    @objc(swap:destinationPath:withReply:)
    func swapBundle(_ stagedPath: String, destinationPath: String, withReply reply: @escaping (String, String, Int32) -> Void)

    /// P3 quit-first (SPIKE ONLY): reply ("ok", "armed", 0) at once, hold an XPC transaction, wait
    /// for the calling app to exit, then swap and launch the destination. Progress goes to
    /// `<home>/beebeeb-spike-2176/after-exit-<runID>.log`, because the client is gone by then.
    @objc(swapAfterClientExit:destinationPath:runID:withReply:)
    func swapAfterClientExit(_ stagedPath: String, destinationPath: String, runID: String,
                             withReply reply: @escaping (String, String, Int32) -> Void)

    /// Delete the sibling directories earlier swaps left (the previous bundle). err = how many.
    @objc(removeStaleUpdates:withReply:)
    func removeStaleUpdates(_ destinationPath: String, withReply reply: @escaping (String, String, Int32) -> Void)
}

/// One swap at a time in this process, whatever the number of connections.
private let swapLock = NSLock()
private let log = Logger(subsystem: BeebeebIdentity.serviceIdentifier, category: "swap")
/// Dispatch sources of armed quit-first swaps, kept alive until they finish.
private var armed: [DispatchSourceProcess] = []
private let armedQueue = DispatchQueue(label: "io.beebeeb.app.SwapProbe.after-exit")

final class SwapProbeService: NSObject, BeebeebSwapProbeProtocol {
    private let policy: SwapPolicy

    init(policy: SwapPolicy) {
        self.policy = policy
    }

    func ping(withReply reply: @escaping (String) -> Void) {
        let home = NSHomeDirectory()
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "unknown"
        let containerHome = home.contains("/Library/Containers/") ? 1 : 0
        let sandboxEnv = getenv("APP_SANDBOX_CONTAINER_ID") != nil ? 1 : 0
        log.info("ping pid=\(getpid(), privacy: .public) home_is_container=\(containerHome, privacy: .public)")
        reply("pid=\(getpid());uid=\(getuid());home_is_container=\(containerHome);sandbox_env=\(sandboxEnv);version=\(version)")
    }

    func writeProbe(_ path: String, withReply reply: @escaping (String, String, Int32) -> Void) {
        guard let dir = SpikeFiles.probeDirectory(), let parts = PathCheck.components(path),
              Array(parts.dropLast()) == dir, let name = parts.last, SpikeFiles.isPlainName(name)
        else {
            reply(SwapCode.destinationRefused.rawValue, SwapStage.paths.rawValue, 0)
            return
        }
        let text = "beebeeb spike 2176 P1 probe\nservice_pid=\(getpid())\nt=\(SpikeFiles.now())\n"
        let outcome = SpikeFiles.write(text, name: name, directory: dir, append: false)
        log.info("writeProbe code=\(outcome.code.rawValue, privacy: .public) err=\(outcome.err, privacy: .public)")
        reply(outcome.code.rawValue, outcome.stage.rawValue, outcome.err)
    }

    func swapBundle(_ stagedPath: String, destinationPath: String, withReply reply: @escaping (String, String, Int32) -> Void) {
        swapLock.lock()
        let outcome = SwapCore.performSwap(stagedPath: stagedPath, destinationPath: destinationPath, policy: policy)
        swapLock.unlock()
        log.info("swap code=\(outcome.code.rawValue, privacy: .public) stage=\(outcome.stage.rawValue, privacy: .public) err=\(outcome.err, privacy: .public)")
        reply(outcome.code.rawValue, outcome.stage.rawValue, outcome.err)
    }

    func swapAfterClientExit(_ stagedPath: String, destinationPath: String, runID: String,
                             withReply reply: @escaping (String, String, Int32) -> Void) {
        guard SpikeFiles.isRunID(runID), let pid = NSXPCConnection.current()?.processIdentifier, pid > 0 else {
            reply(SwapCode.ioError.rawValue, SwapStage.paths.rawValue, EINVAL)
            return
        }
        let policy = self.policy
        let marker = "after-exit-\(runID).log"
        func note(_ line: String) {
            if let dir = SpikeFiles.probeDirectory() {
                _ = SpikeFiles.write("\(SpikeFiles.now()) \(line)\n", name: marker, directory: dir, append: true)
            }
        }
        // Keep this process alive while the work is outstanding. Whether launchd honours that once
        // the app that owns this service has exited is exactly what P3 asks.
        xpc_transaction_begin()
        note("armed client_pid=\(pid) service_pid=\(getpid())")
        log.info("after-exit armed client_pid=\(pid, privacy: .public)")
        reply(SwapCode.ok.rawValue, "armed", 0)

        let source = DispatchSource.makeProcessSource(identifier: pid, eventMask: .exit, queue: armedQueue)
        var finished = false
        let finish: (String) -> Void = { reason in
            guard !finished else { return }
            finished = true
            source.cancel()
            note(reason)
            guard reason == "client_exited" else {
                note("done")
                xpc_transaction_end()
                return
            }
            // Give the exiting app a moment to let go of its files, then swap and relaunch.
            armedQueue.asyncAfter(deadline: .now() + 1) {
                swapLock.lock()
                let outcome = SwapCore.performSwap(stagedPath: stagedPath, destinationPath: destinationPath, policy: policy)
                swapLock.unlock()
                note("swap code=\(outcome.code.rawValue) stage=\(outcome.stage.rawValue) err=\(outcome.err)")
                log.info("after-exit swap code=\(outcome.code.rawValue, privacy: .public)")
                let configuration = NSWorkspace.OpenConfiguration()
                configuration.activates = true
                NSWorkspace.shared.openApplication(at: URL(fileURLWithPath: destinationPath, isDirectory: true),
                                                   configuration: configuration) { app, error in
                    if let error = error as NSError? {
                        note("launch failed domain=\(error.domain) code=\(error.code)")
                    } else {
                        note("launch ok pid=\(app?.processIdentifier ?? -1)")
                    }
                    note("done")
                    xpc_transaction_end()
                }
            }
        }
        source.setEventHandler { finish("client_exited") }
        armedQueue.sync {
            armed.append(source)
            source.resume()
            // The app may already be gone before the source was armed.
            if kill(pid, 0) != 0 && errno == ESRCH { finish("client_exited") }
        }
        armedQueue.asyncAfter(deadline: .now() + 120) { finish("client_did_not_exit_within_120s") }
    }

    func removeStaleUpdates(_ destinationPath: String, withReply reply: @escaping (String, String, Int32) -> Void) {
        swapLock.lock()
        let outcome = SwapCore.removeStaleUpdates(destinationPath: destinationPath, policy: policy)
        swapLock.unlock()
        log.info("cleanup code=\(outcome.code.rawValue, privacy: .public) removed=\(outcome.err, privacy: .public)")
        reply(outcome.code.rawValue, outcome.stage.rawValue, outcome.err)
    }
}

/// The spike's own files outside the container: `<home>/beebeeb-spike-2176/` (made 0700 if absent).
enum SpikeFiles {
    static func probeDirectory() -> [String]? {
        guard let pw = getpwuid(getuid()), let homeDir = pw.pointee.pw_dir,
              let home = PathCheck.components(String(cString: homeDir))
        else { return nil }
        let dir = home + ["beebeeb-spike-2176"]
        _ = mkdir(PathCheck.join(dir), 0o700)
        return dir
    }

    static func isPlainName(_ name: String) -> Bool {
        !name.hasPrefix(".") && (1...64).contains(name.count)
            && name.unicodeScalars.allSatisfy { CharacterSet.alphanumerics.contains($0) && $0.isASCII || "._-".unicodeScalars.contains($0) }
    }

    static func isRunID(_ id: String) -> Bool {
        (1...40).contains(id.count)
            && id.unicodeScalars.allSatisfy { ($0.isASCII && CharacterSet.alphanumerics.contains($0)) || $0 == "-" }
    }

    static func now() -> String {
        ISO8601DateFormatter().string(from: Date())
    }

    /// Writes `text` to `<directory>/<name>` without following a symlink at any step.
    static func write(_ text: String, name: String, directory: [String], append: Bool) -> SwapOutcome {
        let dirFD = open(PathCheck.join(directory), O_RDONLY | O_DIRECTORY | O_NOFOLLOW_ANY | O_CLOEXEC)
        guard dirFD >= 0 else {
            let e = errno
            return SwapOutcome(e == ELOOP ? .symlinkRefused : .ioError, .paths, e)
        }
        defer { close(dirFD) }
        let flags = O_WRONLY | O_CREAT | O_NOFOLLOW | O_CLOEXEC | (append ? O_APPEND : O_TRUNC)
        let fd = openat(dirFD, name, flags, 0o600)
        guard fd >= 0 else {
            let e = errno
            return SwapOutcome(e == ELOOP ? .symlinkRefused : .ioError, .copy, e)
        }
        defer { close(fd) }
        let bytes = Array(text.utf8)
        let written = bytes.withUnsafeBytes { Darwin.write(fd, $0.baseAddress, $0.count) }
        return written == bytes.count ? SwapOutcome(.ok, .done) : SwapOutcome(.ioError, .copy, errno)
    }
}
