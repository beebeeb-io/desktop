// XPCBridge.swift
//
// Unix-domain-socket client used by the File Provider extension to talk
// to the Rust daemon (`repos/desktop/src-tauri/src/ipc_socket.rs`).
//
// "XPC" in the file name is historical — Apple's preferred IPC for
// extensions is XPC services, but those require entitlements that
// aren't trivial to set up across a Tauri shell + Swift extension. A
// plain Unix socket works on macOS, ships with no extra entitlements,
// and matches the path the daemon already binds (`{XDG_RUNTIME_DIR or
// /tmp}/beebeeb-daemon.sock`).
//
// Spec: docs/superpowers/plans/2026-05-07-desktop-sync-client.md Phase 2 Task 5.
//
// Each call opens a fresh connection — short and synchronous, well
// under Apple's "extension call timeout" budget. If the daemon is not
// running (no socket file, or `connect` returns non-zero), the
// completion callback fires with `nil` so the extension can render a
// "not synced" placeholder.

import Foundation

class XPCBridge {
    private let socketPath: String

    init() {
        // macOS doesn't set `XDG_RUNTIME_DIR`. The fallback to `/tmp`
        // matches `repos/desktop/src-tauri/src/ipc_socket.rs::ipc_socket_path`,
        // so daemon and extension agree on where to find each other
        // without extra config.
        let runtimeDir = ProcessInfo.processInfo.environment["XDG_RUNTIME_DIR"] ?? "/tmp"
        self.socketPath = "\(runtimeDir)/beebeeb-daemon.sock"
    }

    /// `GetFileStatus`: ask the daemon for the current state of a file.
    /// Returns the raw status string (`"local"`, `"cloud_only"`,
    /// `"downloading"`, `"uploading"`, `"conflict"`, `"error"`), the
    /// decrypted name (or `nil` if the daemon hasn't decoded it yet),
    /// and the plaintext size in bytes.
    func getFileStatus(fileId: String, completion: @escaping (String, String?, Int64) -> Void) {
        let req = ["type": "GetFileStatus", "file_id": fileId]
        sendRequest(req) { resp in
            let status = resp?["status"] as? String ?? "error"
            let name = resp?["name"] as? String
            let size = resp?["size_bytes"] as? Int64 ?? 0
            completion(status, name, size)
        }
    }

    /// `HydrateFile`: tell the daemon to download+decrypt the file and
    /// place plaintext at `destPath`. Completes with `nil` on success,
    /// or an `NSError` carrying the daemon's error message.
    func hydrateFile(fileId: String, destPath: String, completion: @escaping (Error?) -> Void) {
        let req: [String: Any] = ["type": "HydrateFile", "file_id": fileId, "dest_path": destPath]
        sendRequest(req) { resp in
            if resp?["type"] as? String == "Error" {
                let msg = resp?["message"] as? String ?? "hydration failed"
                completion(NSError(
                    domain: "Beebeeb",
                    code: 1,
                    userInfo: [NSLocalizedDescriptionKey: msg]
                ))
            } else {
                completion(nil)
            }
        }
    }

    /// Connect → write JSON request → read JSON response → close.
    /// Synchronous; the calling extension dispatch queue is whatever
    /// Apple delivered the underlying File Provider request on, which
    /// already isn't the main thread.
    private func sendRequest(
        _ request: [String: Any],
        completion: @escaping ([String: Any]?) -> Void
    ) {
        guard let data = try? JSONSerialization.data(withJSONObject: request) else {
            completion(nil); return
        }
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { completion(nil); return }
        defer { close(fd) }

        // sockaddr_un is a fixed-size struct; sun_path is a 108-byte
        // buffer. We rebind it to CChar so strncpy can write into it
        // without warning about C-string vs UInt8 type mismatch.
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        // strncpy returns the destination pointer; we don't use it.
        _ = socketPath.withCString { ptr in
            withUnsafeMutablePointer(to: &addr.sun_path) {
                $0.withMemoryRebound(to: CChar.self, capacity: 108) {
                    strncpy($0, ptr, 107)
                }
            }
        }
        let size = MemoryLayout<sockaddr_un>.size
        guard withUnsafePointer(to: &addr, {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(fd, $0, socklen_t(size)) == 0
            }
        }) else { completion(nil); return }

        _ = data.withUnsafeBytes { write(fd, $0.baseAddress, data.count) }

        var buf = Data(count: 65536)
        let n = buf.withUnsafeMutableBytes { read(fd, $0.baseAddress, 65536) }
        guard n > 0,
              let resp = try? JSONSerialization.jsonObject(with: buf.prefix(n)) as? [String: Any] else {
            completion(nil); return
        }
        completion(resp)
    }
}
