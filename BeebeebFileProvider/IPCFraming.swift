import CryptoKit
import Foundation

// Wire framing + exchange for the desktop daemon's Unix socket (task 1670
// issue 3). The contract is docs/IPC_PROTOCOL.md; the Rust half is
// src-tauri/src/ipc_frame.rs -- keep the two in step.
//
// Frame = ONE compact JSON value followed by a single "\n" (0x0A). Compact
// JSON escapes every control character inside a string, so a raw 0x0A can
// only be the delimiter.
//
// Why this exists: XPCBridge.sendRequest used to do ONE unlooped write() and
// ONE read() of at most 64 KiB, then JSON-parse whatever came back. On a
// SOCK_STREAM socket a single read can return a partial reply, and any reply
// over 64 KiB was always truncated. Separately the daemon answered a
// successful hydrate with the bare JSON string "Ok", which
// JSONSerialization rejects unless fragments are allowed -- so every
// successful hydrate surfaced as "daemon response was not valid JSON".
//
// Task 1684 adds the write-queue idempotency key (IPCWriteKey) and the request
// builders (IPCWriteRequest) that attach it. CryptoKit is system-provided on
// every macOS the extension runs on, so this file is still free of any
// FileProvider dependency.
//
// This file is deliberately pure Foundation (no FileProvider import): the
// macOS CI job compiles it together with BeebeebFileProviderTests/main.swift
// and runs the framing tests (scripts/test-ipc-framing.sh).

enum IPCFraming {
    static let delimiter: UInt8 = 0x0A

    /// Largest single reply the extension will buffer. A ListFileProviderItems
    /// reply is ~350 bytes per item, so 32 MiB is ~90k items in one folder --
    /// far beyond a usable Finder folder -- while still bounding the memory of
    /// an extension process that has a tight limit.
    static let maxReplyBytes = 32 * 1024 * 1024

    /// Metadata calls (list / item / queue*) are database work in the daemon
    /// and answer in milliseconds; 30 s is generous and still fails fast when
    /// the daemon is wedged.
    static let metadataTimeoutSeconds = 30

    /// Hydrate (download + decrypt) timeout. SO_RCVTIMEO is per read(), so
    /// with progress frames it is an IDLE timeout: every frame restarts it.
    /// The daemon's own HTTP client allows 30 s per request (api_client.rs), a
    /// hydrate makes one metadata request plus one per chunk, and an
    /// old daemon (no progress frames) stays silent for the whole download.
    /// 600 s is 20x the per-request ceiling and leaves room for the
    /// download_kbps_limit pacing sleeps; a stuck daemon still surfaces as an
    /// honest "did not answer" error after 10 minutes rather than hanging Finder.
    static let hydrateTimeoutSeconds = 600

    /// QueueFinderCreate / QueueFinderModify WITH file contents. Unlike the
    /// other metadata calls, the daemon copies the whole file into its
    /// staging area (`StagedPayload::copy`, a synchronous `std::fs::copy`)
    /// BEFORE it replies, and sends nothing in between. A timeout here is not
    /// harmless: the extension reports failure while the daemon carries on and
    /// queues the upload, and Finder's retry then queues it again under a
    /// fresh id (a duplicate). So the timeout must outlast any copy that will
    /// finish. It is bounded by disk, not network: 600 s covers 15 GB at a
    /// pessimistic 25 MB/s (spinning or USB disk; an APFS same-volume copy is
    /// a clone and near-instant), and it is the same 20x-metadata ceiling as
    /// hydrate. A wedged daemon still surfaces after 10 minutes, not never.
    /// A copy that would take longer still times out; task 1684 closes that:
    /// the same requests carry a stable `request_id` (see `IPCWriteKey`) and
    /// the daemon returns the first result for a repeat instead of queueing the
    /// upload again.
    static let stagedCopyTimeoutSeconds = 600

    /// FetchThumbnail (task 1699): one small encrypted thumbnail variant
    /// (a few KiB — the server stores three fixed-size variants, not the
    /// original image) downloaded, decrypted and atomically staged by the
    /// daemon before it replies. Not a whole-file download — 60 s is 2x the
    /// metadata ceiling and leaves room for a stalled network round trip,
    /// while a wedged daemon still surfaces quickly to every icon Finder is
    /// painting. Far below hydrate's 600 s ceiling.
    static let thumbnailTimeoutSeconds = 60

    /// Timeout for a write-queue request: long only when the daemon has file
    /// contents to copy; a folder create, a rename or a delete stays on the
    /// short metadata timeout so a wedged daemon still fails fast.
    static func writeQueueTimeoutSeconds(hasContents: Bool) -> Int {
        return hasContents ? stagedCopyTimeoutSeconds : metadataTimeoutSeconds
    }

    static let readChunkBytes = 64 * 1024

    /// Encode a request as one frame: compact JSON + "\n".
    static func encodeRequest(_ request: [String: Any]) throws -> Data {
        guard JSONSerialization.isValidJSONObject(request),
              var data = try? JSONSerialization.data(withJSONObject: request) else {
            throw IPCTransportError.encodingFailed
        }
        // Compact JSON cannot contain a raw newline; refuse rather than
        // corrupt the framing if that ever stops being true.
        if data.contains(delimiter) {
            throw IPCTransportError.encodingFailed
        }
        data.append(delimiter)
        return data
    }

    /// Decode one reply frame. The daemon's success reply is `{"Ok":{}}`, but
    /// an older daemon answers with the bare string `"Ok"` -- normalise that
    /// to `["Ok": [:]]` so callers only ever look at a dictionary.
    static func decodeReply(_ frame: Data) throws -> [String: Any] {
        guard let value = try? JSONSerialization.jsonObject(with: frame, options: [.fragmentsAllowed]) else {
            throw IPCTransportError.malformedReply
        }
        if let object = value as? [String: Any] {
            return object
        }
        if let name = value as? String {
            return [name: [String: Any]()]
        }
        throw IPCTransportError.malformedReply
    }

    /// `HydrateProgress` frames carry {"done": bytes, "total": bytes}.
    static func progressValues(in reply: [String: Any]) -> (done: Int64, total: Int64)? {
        guard let payload = reply["HydrateProgress"] as? [String: Any] else {
            return nil
        }
        let done = (payload["done"] as? NSNumber)?.int64Value ?? 0
        let total = (payload["total"] as? NSNumber)?.int64Value ?? 0
        return (done: done, total: total)
    }

    /// Write every byte, looping over partial writes and retrying EINTR.
    /// `write` returns bytes written (> 0) or a negative errno.
    static func writeAll(_ data: Data, using write: (UnsafeRawPointer, Int) -> Int) throws {
        try data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) -> Void in
            guard let base = raw.baseAddress else {
                return
            }
            var offset = 0
            while offset < raw.count {
                let written = write(base + offset, raw.count - offset)
                if written > 0 {
                    offset += written
                    continue
                }
                if written == 0 {
                    throw IPCTransportError.writeFailed(errnoCode: EPIPE)
                }
                let code = Int32(-written)
                if code == EINTR {
                    continue
                }
                if code == EAGAIN || code == EWOULDBLOCK {
                    throw IPCTransportError.timedOut
                }
                throw IPCTransportError.writeFailed(errnoCode: code)
            }
        }
    }
}

enum IPCTransportError: Error, Equatable {
    case encodingFailed
    case timedOut
    case cancelled
    case closedBeforeReply
    case replyTooLarge(limit: Int)
    case readFailed(errnoCode: Int32)
    case writeFailed(errnoCode: Int32)
    case malformedReply
}

/// One low-level read: returns bytes read (> 0), 0 at EOF, or a negative errno.
typealias IPCReadFunction = (UnsafeMutableRawPointer, Int) -> Int

/// Incrementally scans bytes and reports when they form exactly one complete
/// top-level JSON object/array/string. Only used to tolerate a peer that
/// omits the delimiter (an older daemon wrote its reply with no newline and
/// kept the connection open), so the reader need not wait for a "\n" or EOF
/// that never comes. Linear in the bytes fed -- never re-parses.
struct IPCJSONValueScanner {
    private var depth = 0
    private var inString = false
    private var escaped = false
    private var broken = false
    private(set) var isComplete = false

    mutating func reset() {
        depth = 0
        inString = false
        escaped = false
        broken = false
        isComplete = false
    }

    /// True when the bytes fed so far are exactly one complete value.
    var hasCompleteValue: Bool {
        return isComplete && !broken
    }

    mutating func feed(_ bytes: ArraySlice<UInt8>) {
        for byte in bytes {
            if isComplete {
                if !IPCJSONValueScanner.isWhitespace(byte) {
                    broken = true // something after the value: not a lone value
                }
                continue
            }
            if inString {
                if escaped {
                    escaped = false
                } else if byte == 0x5C { // backslash
                    escaped = true
                } else if byte == 0x22 { // quote
                    inString = false
                    if depth == 0 {
                        isComplete = true
                    }
                }
                continue
            }
            switch byte {
            case 0x22:
                inString = true
            case 0x7B, 0x5B: // { [
                depth += 1
            case 0x7D, 0x5D: // } ]
                depth -= 1
                if depth == 0 {
                    isComplete = true
                } else if depth < 0 {
                    broken = true
                    depth = 0
                }
            default:
                break
            }
        }
    }

    private static func isWhitespace(_ byte: UInt8) -> Bool {
        return byte == 0x20 || byte == 0x09 || byte == 0x0A || byte == 0x0D
    }
}

/// Reads delimiter-terminated frames from a stream, however the bytes are
/// split across read() calls.
final class IPCFrameReader {
    private let readFunction: IPCReadFunction
    private let maxFrameBytes: Int
    private var buffer = [UInt8]()
    private var scanner = IPCJSONValueScanner()
    /// `buffer[0..<scannedThrough]` is known to contain no delimiter, so each
    /// pass looks only at bytes appended since the last one. Rescanning the
    /// whole buffer after every 64 KiB read is quadratic on a large reply.
    private var scannedThrough = 0
    /// Total bytes examined while hunting for delimiters. Test-visible: the
    /// framing tests assert it stays linear in the reply size.
    private(set) var delimiterBytesExamined = 0

    init(maxFrameBytes: Int = IPCFraming.maxReplyBytes, read: @escaping IPCReadFunction) {
        self.maxFrameBytes = maxFrameBytes
        self.readFunction = read
    }

    /// The next non-empty frame, without its delimiter.
    func nextFrame() throws -> Data {
        var chunk = [UInt8](repeating: 0, count: IPCFraming.readChunkBytes)
        while true {
            // 1. A complete delimited frame already buffered (frames often
            //    arrive several to a read).
            let found = buffer[scannedThrough..<buffer.count].firstIndex(of: IPCFraming.delimiter)
            delimiterBytesExamined += (found.map { $0 + 1 } ?? buffer.count) - scannedThrough
            if let index = found {
                let frame = Array(buffer[0..<index])
                buffer.removeSubrange(0...index)
                scannedThrough = 0
                scanner.reset()
                scanner.feed(buffer[0..<buffer.count])
                if IPCFrameReader.isBlank(frame) {
                    continue // tolerate stray blank lines
                }
                return Data(frame)
            }
            scannedThrough = buffer.count
            // 2. Legacy peer: one complete value, no delimiter.
            if scanner.hasCompleteValue {
                let frame = buffer
                buffer.removeAll()
                scannedThrough = 0
                scanner.reset()
                return Data(frame)
            }
            // 3. Bounded memory.
            if buffer.count > maxFrameBytes {
                throw IPCTransportError.replyTooLarge(limit: maxFrameBytes)
            }
            // 4. Read more.
            let count = chunk.withUnsafeMutableBytes { (raw: UnsafeMutableRawBufferPointer) -> Int in
                guard let base = raw.baseAddress else {
                    return 0
                }
                return readFunction(base, raw.count)
            }
            if count > 0 {
                let start = buffer.count
                buffer.append(contentsOf: chunk[0..<count])
                scanner.feed(buffer[start..<buffer.count])
                continue
            }
            if count == 0 {
                // EOF: a final unterminated value is still a reply.
                if IPCFrameReader.isBlank(buffer) {
                    throw IPCTransportError.closedBeforeReply
                }
                let frame = buffer
                buffer.removeAll()
                scannedThrough = 0
                scanner.reset()
                return Data(frame)
            }
            let code = Int32(-count)
            if code == EINTR {
                continue
            }
            if code == EAGAIN || code == EWOULDBLOCK {
                throw IPCTransportError.timedOut
            }
            throw IPCTransportError.readFailed(errnoCode: code)
        }
    }

    private static func isBlank(_ bytes: [UInt8]) -> Bool {
        for byte in bytes where !(byte == 0x20 || byte == 0x09 || byte == 0x0A || byte == 0x0D) {
            return false
        }
        return true
    }
}

/// Lets another thread (Finder cancelling the Progress) interrupt a blocking
/// exchange. `cancel()` shuts the socket down, which makes the blocked read()
/// return; the lock guarantees it can never touch an fd that has already been
/// closed and reused.
final class IPCCancellation {
    private let lock = NSLock()
    private var fd: Int32 = -1
    private var cancelled = false

    var isCancelled: Bool {
        lock.lock()
        defer { lock.unlock() }
        return cancelled
    }

    /// Returns false if cancellation already happened (do not start).
    func attach(fd: Int32) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        if cancelled {
            return false
        }
        self.fd = fd
        return true
    }

    func detach() {
        lock.lock()
        fd = -1
        lock.unlock()
    }

    func cancel() {
        lock.lock()
        cancelled = true
        if fd >= 0 {
            _ = shutdown(fd, SHUT_RDWR)
        }
        lock.unlock()
    }
}

enum IPCExchange {
    /// Send one framed request on a CONNECTED socket and return the final reply.
    ///
    /// `timeoutSeconds` is applied as SO_RCVTIMEO / SO_SNDTIMEO, i.e. per
    /// read/write call. `HydrateProgress` frames are handed to `onProgress`
    /// and never returned. Errors are transport-level only; the caller maps
    /// them to user-facing text (never raw reply bytes).
    static func perform(
        fd: Int32,
        request: [String: Any],
        timeoutSeconds: Int,
        maxReplyBytes: Int = IPCFraming.maxReplyBytes,
        cancellation: IPCCancellation? = nil,
        onProgress: ((Int64, Int64) -> Void)? = nil
    ) throws -> [String: Any] {
        let framed = try IPCFraming.encodeRequest(request)

        if let cancellation = cancellation {
            if !cancellation.attach(fd: fd) {
                throw IPCTransportError.cancelled
            }
        }
        defer { cancellation?.detach() }

        // A write to a peer that has gone away must fail with EPIPE, not kill
        // the extension process with SIGPIPE.
        var noSigPipe: Int32 = 1
        _ = setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &noSigPipe, socklen_t(MemoryLayout<Int32>.size))

        var timeout = timeval(tv_sec: timeoutSeconds, tv_usec: 0)
        _ = setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
        _ = setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))

        do {
            try IPCFraming.writeAll(framed) { pointer, count in
                let written = Darwin.write(fd, pointer, count)
                return written >= 0 ? written : -Int(errno)
            }

            let reader = IPCFrameReader(maxFrameBytes: maxReplyBytes) { pointer, count in
                let bytesRead = Darwin.read(fd, pointer, count)
                return bytesRead >= 0 ? bytesRead : -Int(errno)
            }

            while true {
                let frame = try reader.nextFrame()
                let reply = try IPCFraming.decodeReply(frame)
                if let progress = IPCFraming.progressValues(in: reply) {
                    onProgress?(progress.done, progress.total)
                    continue
                }
                return reply
            }
        } catch {
            // A shutdown() from cancel() surfaces as EOF or a read error;
            // report it as what it was.
            if cancellation?.isCancelled == true {
                throw IPCTransportError.cancelled
            }
            throw error
        }
    }
}


// MARK: - Write-queue idempotency (task 1684)

/// Size and modification time of the file the system staged for a write
/// (`createItem` / `modifyItem` `contents`). These are the content-derived
/// inputs of `IPCWriteKey`.
///
/// The inode is deliberately NOT part of this: the system may hand a retry a
/// re-staged copy of the same content at a new path (and so a new inode), and a
/// key that changed with the inode would never match across retries, which is
/// the one thing it exists to do. Size + nanosecond mtime is preserved by a
/// clone/copy that keeps metadata; if a retry's staged file carries a different
/// mtime the key simply differs and the daemon falls back to its pre-1684
/// behaviour (one extra upload), which is the same as no key at all.
struct IPCContentFingerprint: Equatable {
    let sizeBytes: Int64
    let modifiedNanos: Int64

    /// `nil` if the file cannot be stat'ed (the request is then sent with no
    /// key and behaves as it did before 1684).
    static func ofFile(at url: URL) -> IPCContentFingerprint? {
        var info = stat()
        guard stat(url.path, &info) == 0 else {
            return nil
        }
        let nanos = Int64(info.st_mtimespec.tv_sec) * 1_000_000_000 + Int64(info.st_mtimespec.tv_nsec)
        return IPCContentFingerprint(sizeBytes: Int64(info.st_size), modifiedNanos: nanos)
    }
}

/// The idempotency key sent as `request_id` on `QueueFinderCreate` /
/// `QueueFinderModify` requests that carry file contents.
///
/// **It must be STABLE across the system's retries, so it is derived, never
/// random.** A retry after an IPC timeout is a NEW `createItem` / `modifyItem`
/// invocation, so a fresh `UUID()` per call would be different every time and
/// dedup nothing. The key is SHA-256 (lowercase hex, 64 characters) over a
/// canonical, length-prefixed encoding of the inputs that identify the logical
/// operation:
///
/// * create: parent item identifier, filename, kind, content type, content size,
///   content modification time;
/// * modify: item identifier, base version identifier, parent, filename, kind,
///   content type, changed-fields bitmask, content size, content modification
///   time.
///
/// **False-dedup risk (two genuinely different operations producing one key).**
/// Two creates collide only if parent, name, size and nanosecond mtime are all
/// equal -- i.e. the same file into the same folder, which would be a name
/// collision anyway. The realistic hazard is the SAME inputs recurring as a new
/// operation inside the daemon's 30 minute window (create `a.txt`, delete it,
/// copy the identical file back with its mtime preserved). The daemon guards
/// that, not this file: it returns a remembered result only while the row it
/// created still exists under the same name, and it never merges a key across a
/// different operation/parent-or-item/filename/kind. A modify additionally folds
/// in the base version and the changed-fields mask, which change when the item
/// does. A sha256 hash collision is not a concern.
enum IPCWriteKey {
    /// Bumping this changes every key; never reuse a version for a different encoding.
    static let version = "bb-write-v1"

    private static func field(_ value: String?) -> String {
        guard let value = value else {
            return "n;"
        }
        // Length-prefixed so ("ab", "c") and ("a", "bc") cannot produce the same bytes.
        return "s\(value.utf8.count):\(value);"
    }

    private static func field(_ value: Int64) -> String {
        return "i\(value);"
    }

    private static func hexSHA256(_ material: String) -> String {
        let digest = SHA256.hash(data: Data(material.utf8))
        return digest.map { String(format: "%02x", $0) }.joined()
    }

    static func create(
        parentIdentifier: String,
        filename: String,
        kind: String,
        contentType: String?,
        contents: IPCContentFingerprint
    ) -> String {
        var material = field(version) + field("create")
        material += field(parentIdentifier)
        material += field(filename)
        material += field(kind)
        material += field(contentType)
        material += field(contents.sizeBytes)
        material += field(contents.modifiedNanos)
        return hexSHA256(material)
    }

    static func modify(
        itemIdentifier: String,
        parentIdentifier: String,
        filename: String,
        kind: String,
        contentType: String?,
        baseVersionIdentifier: String?,
        changedFields: UInt64,
        contents: IPCContentFingerprint
    ) -> String {
        var material = field(version) + field("modify")
        material += field(itemIdentifier)
        material += field(baseVersionIdentifier)
        material += field(parentIdentifier)
        material += field(filename)
        material += field(kind)
        material += field(contentType)
        material += field(Int64(bitPattern: changedFields))
        material += field(contents.sizeBytes)
        material += field(contents.modifiedNanos)
        return hexSHA256(material)
    }
}

/// Builds the two write-queue requests that can make the daemon copy a file.
/// Both attach `request_id` whenever they carry contents AND the contents could
/// be fingerprinted; a request with no contents (a folder, a rename, a move) is
/// answered from the database in milliseconds under the short timeout, so it
/// has nothing to deduplicate and is sent without one.
///
/// XPCBridge must build these requests through here (scripts/check-ipc-
/// timeouts.py fails if a call site stops doing so) so the key cannot be
/// dropped by a refactor, and BeebeebFileProviderTests pins that both shapes
/// carry it.
enum IPCWriteRequest {
    static func create(
        parentIdentifier: String,
        filename: String,
        kind: String,
        contentsPath: String?,
        contentType: String?,
        contents: IPCContentFingerprint?
    ) -> [String: Any] {
        var payload: [String: Any] = [
            "parent_id": parentIdentifier,
            "filename": filename,
            "kind": kind,
        ]
        if let contentsPath = contentsPath {
            payload["contents_path"] = contentsPath
        }
        if let contentType = contentType {
            payload["content_type"] = contentType
        }
        if contentsPath != nil, let contents = contents {
            payload["request_id"] = IPCWriteKey.create(
                parentIdentifier: parentIdentifier,
                filename: filename,
                kind: kind,
                contentType: contentType,
                contents: contents
            )
        }
        return ["QueueFinderCreate": payload]
    }

    static func modify(
        itemIdentifier: String,
        parentIdentifier: String,
        filename: String,
        kind: String,
        contentsPath: String?,
        contentType: String?,
        baseVersionIdentifier: String?,
        changedFields: UInt64,
        contents: IPCContentFingerprint?
    ) -> [String: Any] {
        var payload: [String: Any] = [
            "file_id": itemIdentifier,
            "parent_id": parentIdentifier,
            "filename": filename,
            "kind": kind,
        ]
        if let contentsPath = contentsPath {
            payload["contents_path"] = contentsPath
        }
        if let contentType = contentType {
            payload["content_type"] = contentType
        }
        if let baseVersionIdentifier = baseVersionIdentifier {
            payload["base_version_identifier"] = baseVersionIdentifier
        }
        if contentsPath != nil, let contents = contents {
            payload["request_id"] = IPCWriteKey.modify(
                itemIdentifier: itemIdentifier,
                parentIdentifier: parentIdentifier,
                filename: filename,
                kind: kind,
                contentType: contentType,
                baseVersionIdentifier: baseVersionIdentifier,
                changedFields: changedFields,
                contents: contents
            )
        }
        return ["QueueFinderModify": payload]
    }
}
