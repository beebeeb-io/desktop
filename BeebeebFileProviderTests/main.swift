import Foundation
import FileProvider

// Framing tests for BeebeebFileProvider/IPCFraming.swift (task 1670 issue 3).
//
// The File Provider extension target has no XCTest target, so this is a plain
// executable: scripts/test-ipc-framing.sh compiles IPCFraming.swift and
// FileProviderItem.swift (task 1694) together with this file, runs it, and
// asserts the printed COUNT ("ipc-framing: N passed, 0 failed") -- a run that
// executed fewer tests than declared fails the script.

setvbuf(stdout, nil, _IOLBF, 0)
signal(SIGPIPE, SIG_IGN)

var passed = 0
var failed = 0

struct TestFailure: Error, CustomStringConvertible {
    let description: String
}

func expect(_ condition: Bool, _ message: @autoclosure () -> String) throws {
    if !condition {
        throw TestFailure(description: message())
    }
}

func check(_ name: String, _ body: () throws -> Void) {
    do {
        try body()
        passed += 1
        print("ok   \(name)")
    } catch {
        failed += 1
        print("FAIL \(name): \(error)")
    }
}

func expectThrows(_ expected: IPCTransportError, _ body: () throws -> Void) throws {
    do {
        try body()
    } catch let error as IPCTransportError {
        try expect(error == expected, "expected \(expected), got \(error)")
        return
    }
    throw TestFailure(description: "expected \(expected), but nothing was thrown")
}

// MARK: - Scripted reads (no sockets)

enum Step {
    case bytes([UInt8])
    case err(Int32)
}

func scripted(_ steps: [Step]) -> IPCReadFunction {
    var index = 0
    return { pointer, capacity in
        guard index < steps.count else {
            return 0 // EOF
        }
        let step = steps[index]
        index += 1
        switch step {
        case .bytes(let bytes):
            let count = min(bytes.count, capacity)
            bytes.withUnsafeBufferPointer { source in
                pointer.copyMemory(from: source.baseAddress!, byteCount: count)
            }
            return count
        case .err(let code):
            return -Int(code)
        }
    }
}

func slices(_ bytes: [UInt8], every size: Int) -> [Step] {
    var steps = [Step]()
    var offset = 0
    while offset < bytes.count {
        let end = min(offset + size, bytes.count)
        steps.append(.bytes(Array(bytes[offset..<end])))
        offset = end
    }
    return steps
}

func utf8(_ text: String) -> [UInt8] {
    return Array(text.utf8)
}

func text(_ data: Data) -> String {
    return String(decoding: data, as: UTF8.self)
}

// MARK: - Socket helpers

func makePair() throws -> (client: Int32, server: Int32) {
    var fds: [Int32] = [0, 0]
    if socketpair(AF_UNIX, SOCK_STREAM, 0, &fds) != 0 {
        throw TestFailure(description: "socketpair failed (errno \(errno))")
    }
    return (client: fds[0], server: fds[1])
}

func sendAll(_ fd: Int32, _ bytes: [UInt8]) {
    bytes.withUnsafeBytes { raw in
        guard let base = raw.baseAddress else {
            return
        }
        var offset = 0
        while offset < raw.count {
            let written = Darwin.write(fd, base + offset, raw.count - offset)
            if written <= 0 {
                return
            }
            offset += written
        }
    }
}

/// Read bytes from `fd` up to and including the first "\n".
func readRequestLine(_ fd: Int32) -> String {
    var line = [UInt8]()
    var byte: UInt8 = 0
    while Darwin.read(fd, &byte, 1) == 1 {
        if byte == 0x0A {
            break
        }
        line.append(byte)
    }
    return String(decoding: line, as: UTF8.self)
}

/// Run `serverWork` on a background thread against the server end of a
/// socketpair while `clientWork` runs against the client end here.
func withServer<T>(_ serverWork: @escaping (Int32) -> Void, _ clientWork: (Int32) throws -> T) throws -> T {
    let pair = try makePair()
    let group = DispatchGroup()
    group.enter()
    DispatchQueue.global().async {
        serverWork(pair.server)
        close(pair.server)
        group.leave()
    }
    defer {
        close(pair.client)
        group.wait()
    }
    return try clientWork(pair.client)
}

// MARK: - Reader

check("a reply split into 3-byte slices is reassembled") {
    let payload = utf8("{\"FileStatus\":{\"identifier\":\"abc\"}}\n")
    let reader = IPCFrameReader(read: scripted(slices(payload, every: 3)))
    let frame = try reader.nextFrame()
    try expect(text(frame) == "{\"FileStatus\":{\"identifier\":\"abc\"}}", "got \(text(frame))")
}

check("a 200 KiB reply (over the old 64 KiB single read) arrives intact") {
    let items = (0..<600).map { "{\"i\":\($0),\"pad\":\"\(String(repeating: "x", count: 300))\"}" }
    let json = "{\"FileProviderItems\":{\"items\":[\(items.joined(separator: ","))]}}"
    let payload = utf8(json + "\n")
    try expect(payload.count > 65536, "fixture must exceed 64 KiB, got \(payload.count)")
    let reader = IPCFrameReader(read: scripted(slices(payload, every: 7000)))
    let frame = try reader.nextFrame()
    try expect(frame.count == payload.count - 1, "frame is \(frame.count) bytes, expected \(payload.count - 1)")
    let reply = try IPCFraming.decodeReply(frame)
    let inner = reply["FileProviderItems"] as? [String: Any]
    let decoded = inner?["items"] as? [[String: Any]]
    try expect(decoded?.count == 600, "decoded \(decoded?.count ?? -1) items, expected 600")
}

check("two frames in one read come out one at a time") {
    let reader = IPCFrameReader(read: scripted([.bytes(utf8("{\"a\":1}\n{\"b\":2}\n"))]))
    let first = try reader.nextFrame()
    let second = try reader.nextFrame()
    try expect(text(first) == "{\"a\":1}", "first was \(text(first))")
    try expect(text(second) == "{\"b\":2}", "second was \(text(second))")
}

check("delimiter search is linear: a 4 MiB reply in 64 KiB reads examines each byte once") {
    let body = String(repeating: "x", count: 4 * 1024 * 1024)
    let payload = utf8("{\"pad\":\"\(body)\"}\n")
    let reader = IPCFrameReader(read: scripted(slices(payload, every: 64 * 1024)))
    let frame = try reader.nextFrame()
    try expect(frame.count == payload.count - 1, "frame is \(frame.count) bytes, expected \(payload.count - 1)")
    // Rescanning the whole buffer after every read would examine roughly
    // 32x payload.count here (64 reads, ~half the buffer each time).
    try expect(reader.delimiterBytesExamined <= payload.count,
               "examined \(reader.delimiterBytesExamined) bytes for a \(payload.count)-byte reply")
}

check("frames straddling read boundaries survive the incremental delimiter scan") {
    // The first read ends inside frame 1's value (so the reader has already
    // scanned 19 bytes for a delimiter and found none); the second read
    // completes frame 1 and brings two whole frames after it, whose
    // delimiters sit BEFORE that stale scan position. The scan position must
    // restart at 0 for the leftover, or those frames are never found.
    // (A first read that already completed the JSON value would not do: the
    // reader returns a delimiter-less complete value at once, see the legacy
    // peer test below.)
    let reader = IPCFrameReader(read: scripted([
        .bytes(utf8("{\"key_long_enough\":")),
        .bytes(utf8("1}\n{}\n{\"c\":3}\n")),
    ]))
    var frames = [String]()
    for _ in 0..<3 {
        frames.append(text(try reader.nextFrame()))
    }
    try expect(frames == ["{\"key_long_enough\":1}", "{}", "{\"c\":3}"], "got \(frames)")
}

// This proves only the IPCFraming helper. XPCBridge.swift is not compiled into
// this harness, so whether the QueueFinderCreate/Modify and HydrateFile call
// sites actually pass these timeouts is guarded at source level by
// scripts/check-ipc-timeouts.py (CI: "IPC timeout call-site guard").
check("write-queue calls with contents get the long staged-copy timeout, others stay short") {
    try expect(IPCFraming.writeQueueTimeoutSeconds(hasContents: true) == IPCFraming.stagedCopyTimeoutSeconds,
               "with contents: \(IPCFraming.writeQueueTimeoutSeconds(hasContents: true))s")
    try expect(IPCFraming.stagedCopyTimeoutSeconds >= 600, "staged-copy timeout is only \(IPCFraming.stagedCopyTimeoutSeconds)s")
    try expect(IPCFraming.writeQueueTimeoutSeconds(hasContents: false) == IPCFraming.metadataTimeoutSeconds,
               "without contents: \(IPCFraming.writeQueueTimeoutSeconds(hasContents: false))s")
}

check("blank lines between frames are skipped") {
    let reader = IPCFrameReader(read: scripted([.bytes(utf8("\n\r\n{\"a\":1}\n"))]))
    let frame = try reader.nextFrame()
    try expect(text(frame) == "{\"a\":1}", "got \(text(frame))")
}

check("a complete unframed object (older daemon) is returned without a delimiter or EOF") {
    // The scripted source would return EOF after the one step; the reader
    // must return the object BEFORE asking for more (a real older daemon
    // keeps the connection open, so a second read would block).
    var reads = 0
    let read: IPCReadFunction = { pointer, capacity in
        reads += 1
        if reads > 1 {
            return -Int(EAGAIN)
        }
        let bytes = utf8("{\"SyncSummary\":{\"syncing\":0,\"cloud_only\":1,\"conflicts\":0}}")
        bytes.withUnsafeBufferPointer { pointer.copyMemory(from: $0.baseAddress!, byteCount: bytes.count) }
        return bytes.count
    }
    let frame = try IPCFrameReader(read: read).nextFrame()
    try expect(text(frame).hasPrefix("{\"SyncSummary\""), "got \(text(frame))")
    try expect(reads == 1, "reader asked for more data after a complete value (\(reads) reads)")
}

check("a bare \"Ok\" string with no delimiter (older daemon hydrate reply) is accepted") {
    var reads = 0
    let read: IPCReadFunction = { pointer, capacity in
        reads += 1
        if reads > 1 {
            return -Int(EAGAIN)
        }
        let bytes = utf8("\"Ok\"")
        bytes.withUnsafeBufferPointer { pointer.copyMemory(from: $0.baseAddress!, byteCount: bytes.count) }
        return bytes.count
    }
    let frame = try IPCFrameReader(read: read).nextFrame()
    let reply = try IPCFraming.decodeReply(frame)
    try expect(reply["Ok"] != nil, "bare \"Ok\" must normalise to an Ok key, got \(reply)")
}

check("a final unterminated value at EOF is returned") {
    let reader = IPCFrameReader(read: scripted([.bytes(utf8("{\"Error\":{\"message\":\"nope\"}"))]))
    let frame = try reader.nextFrame()
    try expect(text(frame) == "{\"Error\":{\"message\":\"nope\"}", "got \(text(frame))")
}

check("EOF with nothing buffered is closedBeforeReply") {
    try expectThrows(.closedBeforeReply) {
        _ = try IPCFrameReader(read: scripted([])).nextFrame()
    }
}

check("a reply over the limit is refused instead of buffered forever") {
    let payload = utf8(String(repeating: "x", count: 5000))
    try expectThrows(.replyTooLarge(limit: 1000)) {
        _ = try IPCFrameReader(maxFrameBytes: 1000, read: scripted(slices(payload, every: 500))).nextFrame()
    }
}

check("EINTR is retried, not surfaced") {
    let reader = IPCFrameReader(read: scripted([.err(EINTR), .bytes(utf8("{\"a\":1}\n"))]))
    let frame = try reader.nextFrame()
    try expect(text(frame) == "{\"a\":1}", "got \(text(frame))")
}

check("EAGAIN (SO_RCVTIMEO expiry) is timedOut") {
    try expectThrows(.timedOut) {
        _ = try IPCFrameReader(read: scripted([.bytes(utf8("{\"a\":")), .err(EAGAIN)])).nextFrame()
    }
}

// MARK: - Encode / decode

check("decodeReply normalises the legacy bare string \"Ok\" and accepts {\"Ok\":{}}") {
    let legacy = try IPCFraming.decodeReply(Data(utf8("\"Ok\"")))
    try expect(legacy["Ok"] != nil, "legacy: \(legacy)")
    let modern = try IPCFraming.decodeReply(Data(utf8("{\"Ok\":{}}")))
    try expect(modern["Ok"] != nil, "modern: \(modern)")
}

check("decodeReply rejects non-JSON and non-object replies without echoing them") {
    try expectThrows(.malformedReply) { _ = try IPCFraming.decodeReply(Data(utf8("not json at all"))) }
    try expectThrows(.malformedReply) { _ = try IPCFraming.decodeReply(Data(utf8("42"))) }
}

check("encodeRequest is compact JSON plus exactly one trailing newline, even for strings containing newlines") {
    let data = try IPCFraming.encodeRequest(["HydrateFile": ["file_id": "a\nb", "dest_path": "/tmp/x y"]])
    try expect(data.last == 0x0A, "must end with a newline")
    let newlines = data.filter { $0 == 0x0A }.count
    try expect(newlines == 1, "expected exactly 1 newline byte, found \(newlines)")
    let roundTrip = try IPCFraming.decodeReply(data.dropLast())
    let payload = roundTrip["HydrateFile"] as? [String: Any]
    try expect(payload?["file_id"] as? String == "a\nb", "the newline inside the string must survive")
}

// MARK: - writeAll

check("writeAll loops over partial writes and EINTR until every byte is sent") {
    let message = Data(utf8("{\"GetSyncSummary\":null}\n"))
    var received = [UInt8]()
    var calls = 0
    try IPCFraming.writeAll(message) { pointer, count in
        calls += 1
        if calls == 2 {
            return -Int(EINTR)
        }
        let take = min(3, count)
        received.append(contentsOf: UnsafeBufferPointer(start: pointer.assumingMemoryBound(to: UInt8.self), count: take))
        return take
    }
    try expect(received == Array(message), "received \(received.count) of \(message.count) bytes")
    try expect(calls > 3, "expected several partial writes, saw \(calls) calls")
}

check("writeAll surfaces EPIPE as writeFailed") {
    try expectThrows(.writeFailed(errnoCode: EPIPE)) {
        try IPCFraming.writeAll(Data(utf8("x"))) { _, _ in -Int(EPIPE) }
    }
}

// MARK: - Exchange over a real socketpair

check("exchange: progress frames arrive first, then the final reply; request is one framed line") {
    var seenRequest = ""
    let requestLock = NSLock()
    var progress = [(Int64, Int64)]()
    let reply = try withServer({ fd in
        let line = readRequestLine(fd)
        requestLock.lock()
        seenRequest = line
        requestLock.unlock()
        sendAll(fd, utf8("{\"HydrateProgress\":{\"done\":0,\"total\":30}}\n{\"HydrateProgress\":{\"done\":10,\"total\":30}}\n{\"Hyd"))
        Thread.sleep(forTimeInterval: 0.05)
        sendAll(fd, utf8("rateProgress\":{\"done\":30,\"total\":30}}\n{\"Ok\":{}}\n"))
    }, { fd in
        try IPCExchange.perform(
            fd: fd,
            request: ["HydrateFile": ["file_id": "f", "dest_path": "/tmp/f", "progress": true]],
            timeoutSeconds: 10,
            onProgress: { done, total in progress.append((done, total)) }
        )
    })
    try expect(reply["Ok"] != nil, "final reply was \(reply)")
    try expect(progress.count == 3, "expected 3 progress callbacks, got \(progress.count)")
    try expect(progress[0] == (0, 30) && progress[1] == (10, 30) && progress[2] == (30, 30), "progress was \(progress)")
    requestLock.lock()
    let request = seenRequest
    requestLock.unlock()
    try expect(request.contains("\"HydrateFile\"") && request.contains("\"progress\":true"), "server saw: \(request)")
}

check("exchange: a 300 KiB reply over a real socket is received whole") {
    let items = (0..<900).map { "{\"i\":\($0),\"pad\":\"\(String(repeating: "y", count: 300))\"}" }
    let json = "{\"FileProviderItems\":{\"items\":[\(items.joined(separator: ","))]}}\n"
    let reply = try withServer({ fd in
        _ = readRequestLine(fd)
        sendAll(fd, utf8(json))
    }, { fd in
        try IPCExchange.perform(fd: fd, request: ["ListFileProviderItems": ["container_id": "x"]], timeoutSeconds: 10)
    })
    let inner = reply["FileProviderItems"] as? [String: Any]
    let decoded = inner?["items"] as? [[String: Any]]
    try expect(decoded?.count == 900, "decoded \(decoded?.count ?? -1) items, expected 900")
}

check("exchange: a daemon that never answers times out with timedOut (not a JSON error)") {
    // Measured inside the client closure: withServer also waits for the
    // (deliberately slow) server thread before returning.
    var elapsed = 0.0
    try expectThrows(.timedOut) {
        _ = try withServer({ fd in
            _ = readRequestLine(fd)
            Thread.sleep(forTimeInterval: 3.0)
        }, { fd in
            let started = Date()
            defer { elapsed = Date().timeIntervalSince(started) }
            return try IPCExchange.perform(fd: fd, request: ["GetFileStatus": ["file_id": "x"]], timeoutSeconds: 1)
        })
    }
    try expect(elapsed >= 0.9 && elapsed < 2.5, "timed out after \(elapsed)s, expected about 1s")
}

check("exchange: cancellation from another thread interrupts a blocked read with cancelled") {
    let cancellation = IPCCancellation()
    var elapsed = 0.0
    try expectThrows(.cancelled) {
        _ = try withServer({ fd in
            _ = readRequestLine(fd)
            Thread.sleep(forTimeInterval: 3.0)
        }, { fd in
            let started = Date()
            defer { elapsed = Date().timeIntervalSince(started) }
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.3) {
                cancellation.cancel()
            }
            return try IPCExchange.perform(
                fd: fd,
                request: ["HydrateFile": ["file_id": "x", "dest_path": "/tmp/x"]],
                timeoutSeconds: 30,
                cancellation: cancellation
            )
        })
    }
    try expect(elapsed < 2.5, "cancel took \(elapsed)s to interrupt the read")
}

check("exchange: cancelling before the request starts never sends it") {
    let cancellation = IPCCancellation()
    cancellation.cancel()
    try expectThrows(.cancelled) {
        _ = try withServer({ _ in }, { fd in
            try IPCExchange.perform(fd: fd, request: ["GetSyncSummary": [String: Any]()], timeoutSeconds: 5, cancellation: cancellation)
        })
    }
}

check("exchange: a peer that hangs up before replying is closedBeforeReply") {
    try expectThrows(.closedBeforeReply) {
        _ = try withServer({ fd in
            _ = readRequestLine(fd)
        }, { fd in
            try IPCExchange.perform(fd: fd, request: ["GetSyncSummary": [String: Any]()], timeoutSeconds: 5)
        })
    }
}

// MARK: - Write-queue idempotency key (task 1684)

let sampleContents = IPCContentFingerprint(sizeBytes: 16_106_127_360, modifiedNanos: 1_788_000_000_123_456_789)

func sampleCreateKey(
    parent: String = "NSFileProviderRootContainerItemIdentifier",
    filename: String = "video.mov",
    kind: String = "file",
    contentType: String? = "public.movie",
    contents: IPCContentFingerprint = sampleContents
) -> String {
    return IPCWriteKey.create(
        parentIdentifier: parent,
        filename: filename,
        kind: kind,
        contentType: contentType,
        contents: contents
    )
}

func sampleModifyKey(
    item: String = "3f2a9c1e-0000-4000-8000-000000000001",
    parent: String = "NSFileProviderRootContainerItemIdentifier",
    filename: String = "notes.txt",
    kind: String = "file",
    contentType: String? = "public.plain-text",
    base: String? = "7",
    changedFields: UInt64 = 0b1010,
    contents: IPCContentFingerprint = IPCContentFingerprint(sizeBytes: 1234, modifiedNanos: 1_788_000_000_000_000_001)
) -> String {
    return IPCWriteKey.modify(
        itemIdentifier: item,
        parentIdentifier: parent,
        filename: filename,
        kind: kind,
        contentType: contentType,
        baseVersionIdentifier: base,
        changedFields: changedFields,
        contents: contents
    )
}

func requestPayload(_ request: [String: Any], _ op: String) throws -> [String: Any] {
    guard let payload = request[op] as? [String: Any] else {
        throw TestFailure(description: "request has no \(op) payload: \(request)")
    }
    return payload
}

check("write key: identical create inputs give the identical key, a 64-character lowercase hex SHA-256") {
    let first = sampleCreateKey()
    let second = sampleCreateKey()
    try expect(first == second, "same inputs must give the same key (a per-call random id would fail this): \(first) vs \(second)")
    try expect(first.count == 64, "expected 64 hex characters, got \(first.count)")
    try expect(first.allSatisfy { "0123456789abcdef".contains($0) }, "key must be lowercase hex: \(first)")
}

check("write key: pinned vectors match an independent implementation of the encoding (create and modify)") {
    // Computed with Python hashlib over the same canonical encoding
    // (docs/IPC_PROTOCOL.md, "Write-queue idempotency"); pins the byte format so
    // an accidental change to it -- which would break dedup against keys
    // remembered by a daemon mid-update -- fails here.
    try expect(
        sampleCreateKey() == "24d844f42d9574efec2950af17915c65f62b2999556a0ca2199bd891b49e1803",
        "create vector mismatch: \(sampleCreateKey())"
    )
    try expect(
        sampleModifyKey() == "a6d2f8a36a80d3a1bf19c8063ef90634a820753f4c041ad06e5e4a861f72626e",
        "modify vector mismatch: \(sampleModifyKey())"
    )
}

check("write key: changing ANY create input changes the key") {
    let base = sampleCreateKey()
    let variants: [(String, String)] = [
        ("parent", sampleCreateKey(parent: "other-folder")),
        ("filename", sampleCreateKey(filename: "video2.mov")),
        ("kind", sampleCreateKey(kind: "folder")),
        ("contentType", sampleCreateKey(contentType: "public.data")),
        ("contentType nil", sampleCreateKey(contentType: nil)),
        ("size", sampleCreateKey(contents: IPCContentFingerprint(sizeBytes: sampleContents.sizeBytes + 1, modifiedNanos: sampleContents.modifiedNanos))),
        ("mtime", sampleCreateKey(contents: IPCContentFingerprint(sizeBytes: sampleContents.sizeBytes, modifiedNanos: sampleContents.modifiedNanos + 1))),
    ]
    for (name, key) in variants {
        try expect(key != base, "changing \(name) must change the create key")
    }
    try expect(Set(variants.map { $0.1 }).count == variants.count, "every variant must also differ from each other")
}

check("write key: changing ANY modify input changes the key") {
    let base = sampleModifyKey()
    let variants: [(String, String)] = [
        ("item", sampleModifyKey(item: "3f2a9c1e-0000-4000-8000-000000000002")),
        ("parent", sampleModifyKey(parent: "other-folder")),
        ("filename", sampleModifyKey(filename: "renamed.txt")),
        ("kind", sampleModifyKey(kind: "folder")),
        ("contentType", sampleModifyKey(contentType: "public.data")),
        ("base version", sampleModifyKey(base: "8")),
        ("base version nil", sampleModifyKey(base: nil)),
        ("changed fields", sampleModifyKey(changedFields: 0b1011)),
        ("size", sampleModifyKey(contents: IPCContentFingerprint(sizeBytes: 1235, modifiedNanos: 1_788_000_000_000_000_001))),
        ("mtime", sampleModifyKey(contents: IPCContentFingerprint(sizeBytes: 1234, modifiedNanos: 1_788_000_000_000_000_002))),
    ]
    for (name, key) in variants {
        try expect(key != base, "changing \(name) must change the modify key")
    }
}

check("write key: field boundaries are unambiguous and create never collides with modify") {
    try expect(
        sampleCreateKey(parent: "ab", filename: "c") != sampleCreateKey(parent: "a", filename: "bc"),
        "(ab, c) and (a, bc) must not share a key"
    )
    try expect(
        sampleCreateKey(contentType: "") != sampleCreateKey(contentType: nil),
        "an empty content type and an absent one must not share a key"
    )
    let sameValues = IPCContentFingerprint(sizeBytes: 5, modifiedNanos: 5)
    let create = IPCWriteKey.create(parentIdentifier: "p", filename: "f", kind: "file", contentType: nil, contents: sameValues)
    let modify = IPCWriteKey.modify(
        itemIdentifier: "p", parentIdentifier: "p", filename: "f", kind: "file", contentType: nil,
        baseVersionIdentifier: nil, changedFields: 0, contents: sameValues
    )
    try expect(create != modify, "a create and a modify must never share a key")
}

func sampleCreateRequest(contentsPath: String? = "/tmp/staged", contents: IPCContentFingerprint? = sampleContents) -> [String: Any] {
    return IPCWriteRequest.create(
        parentIdentifier: "NSFileProviderRootContainerItemIdentifier",
        filename: "video.mov",
        kind: "file",
        contentsPath: contentsPath,
        contentType: "public.movie",
        contents: contents
    )
}

func sampleModifyRequest(contentsPath: String? = "/tmp/staged", contents: IPCContentFingerprint? = IPCContentFingerprint(sizeBytes: 1234, modifiedNanos: 1_788_000_000_000_000_001)) -> [String: Any] {
    return IPCWriteRequest.modify(
        itemIdentifier: "3f2a9c1e-0000-4000-8000-000000000001",
        parentIdentifier: "NSFileProviderRootContainerItemIdentifier",
        filename: "notes.txt",
        kind: "file",
        contentsPath: contentsPath,
        contentType: "public.plain-text",
        baseVersionIdentifier: "7",
        changedFields: 0b1010,
        contents: contents
    )
}

check("write request: QueueFinderCreate with contents carries the derived request_id (and the same one on every build)") {
    let payload = try requestPayload(sampleCreateRequest(), "QueueFinderCreate")
    let id = payload["request_id"] as? String
    try expect(id == sampleCreateKey(), "request_id must be the derived create key, got \(String(describing: id))")
    try expect(payload["contents_path"] as? String == "/tmp/staged", "contents_path must be kept")
    try expect(payload["filename"] as? String == "video.mov" && payload["kind"] as? String == "file", "other fields must be kept")
    let again = try requestPayload(sampleCreateRequest(contentsPath: "/tmp/a-different-staged-path"), "QueueFinderCreate")
    try expect(
        again["request_id"] as? String == id,
        "a retry whose staged path differs must still get the same request_id (the path is not a key input)"
    )
}

check("write request: QueueFinderModify with contents carries the derived request_id") {
    let payload = try requestPayload(sampleModifyRequest(), "QueueFinderModify")
    let id = payload["request_id"] as? String
    try expect(id == sampleModifyKey(), "request_id must be the derived modify key, got \(String(describing: id))")
    try expect(payload["file_id"] as? String == "3f2a9c1e-0000-4000-8000-000000000001", "file_id must be kept")
    try expect(payload["base_version_identifier"] as? String == "7", "base version must be kept")
}

check("write request: no contents, or contents that could not be fingerprinted, sends no request_id") {
    let noContentsCreate = try requestPayload(sampleCreateRequest(contentsPath: nil), "QueueFinderCreate")
    try expect(noContentsCreate["request_id"] == nil, "a folder create is fast and is not deduplicated")
    let noContentsModify = try requestPayload(sampleModifyRequest(contentsPath: nil), "QueueFinderModify")
    try expect(noContentsModify["request_id"] == nil, "a rename/move is fast and is not deduplicated")
    let unreadableCreate = try requestPayload(sampleCreateRequest(contents: nil), "QueueFinderCreate")
    try expect(unreadableCreate["request_id"] == nil, "no fingerprint means no key, never a made-up one")
    let unreadableModify = try requestPayload(sampleModifyRequest(contents: nil), "QueueFinderModify")
    try expect(unreadableModify["request_id"] == nil, "no fingerprint means no key, never a made-up one")
}

check("write request: request_id survives framing on the wire for both shapes") {
    for (op, request) in [("QueueFinderCreate", sampleCreateRequest()), ("QueueFinderModify", sampleModifyRequest())] {
        let frame = try IPCFraming.encodeRequest(request)
        let json = try JSONSerialization.jsonObject(with: frame.dropLast()) as? [String: Any]
        let payload = json?[op] as? [String: Any]
        let id = payload?["request_id"] as? String
        try expect(id?.count == 64, "\(op) frame must carry a 64-character request_id, got \(String(describing: id))")
    }
}

check("content fingerprint: stable for an untouched file, changes with size and with mtime, nil for a missing file") {
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("ipc-fingerprint-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let file = dir.appendingPathComponent("staged.bin")
    try Data("hello".utf8).write(to: file)
    guard let first = IPCContentFingerprint.ofFile(at: file), let second = IPCContentFingerprint.ofFile(at: file) else {
        throw TestFailure(description: "a readable file must fingerprint")
    }
    try expect(first == second, "an untouched file must fingerprint identically twice")
    try expect(first.sizeBytes == 5, "size must be the byte length, got \(first.sizeBytes)")

    try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: 1_700_000_000)], ofItemAtPath: file.path)
    guard let touched = IPCContentFingerprint.ofFile(at: file) else {
        throw TestFailure(description: "touched file must fingerprint")
    }
    try expect(touched.modifiedNanos != first.modifiedNanos, "a different mtime must change the fingerprint")
    try expect(touched.modifiedNanos == 1_700_000_000 * 1_000_000_000, "mtime must be reported in nanoseconds, got \(touched.modifiedNanos)")

    try Data("hello world".utf8).write(to: file)
    guard let grown = IPCContentFingerprint.ofFile(at: file) else {
        throw TestFailure(description: "grown file must fingerprint")
    }
    try expect(grown.sizeBytes == 11 && grown != touched, "a different size must change the fingerprint")

    try expect(IPCContentFingerprint.ofFile(at: dir.appendingPathComponent("missing.bin")) == nil, "a missing file has no fingerprint")
}

// MARK: - Task 1694: add-subitems capability mapping

// Finder refuses a drop into an item whose NSFileProviderItemCapabilities
// lacks .allowsAddingSubItems. The daemon payload crosses the XPC bridge as
// a plain Int with bit 4 = addSubItems (same numbering as Rust's
// CAP_ADD_SUBITEMS); FileProviderItem.capabilities must map it.
//
// These tests compile against BeebeebFileProvider/FileProviderItem.swift
// (added to the harness compile line by scripts/test-ipc-framing.sh,
// task 1694). On unmodified code the mapping never emits
// .allowsAddingSubItems, so the two positive tests go red at runtime.

check("1694: addSubItems bit (1 << 4) maps to .allowsAddingSubItems") {
    let withBit = BeebeebProviderItem(
        identifier: "1694-folder",
        parentIdentifier: "namespace:my_files",
        filename: "Folder",
        kind: .folder,
        sizeBytes: 0,
        contentType: nil,
        status: "local",
        capabilities: 1 << 4,
        versionIdentifier: nil
    )
    let mapped = FileProviderItem(model: withBit).capabilities
    try expect(
        mapped.contains(.allowsAddingSubItems),
        "capability bit 1 << 4 must map to .allowsAddingSubItems, got \(mapped.rawValue)"
    )
    try expect(
        !mapped.contains(.allowsReading),
        "bit 1 << 4 must not gain .allowsReading, got \(mapped.rawValue)"
    )
    // NOTE: .allowsWriting cannot be negatively asserted here — on this
    // SDK .allowsWriting and .allowsAddingSubItems share rawValue 2, so
    // contains(.allowsWriting) is true whenever .allowsAddingSubItems is
    // set. The read-only-file half below pins the direction that matters.

    let withoutBit = BeebeebProviderItem(
        identifier: "1694-file",
        parentIdentifier: "namespace:my_files",
        filename: "file.txt",
        kind: .file,
        sizeBytes: 0,
        contentType: nil,
        status: "local",
        capabilities: BeebeebProviderItem.read,
        versionIdentifier: nil
    )
    let fileMapped = FileProviderItem(model: withoutBit).capabilities
    try expect(
        fileMapped.contains(.allowsReading),
        "read bit must still map to .allowsReading"
    )
    try expect(
        !fileMapped.contains(.allowsAddingSubItems),
        "a read-only file payload must not gain .allowsAddingSubItems, got \(fileMapped.rawValue)"
    )
}

check("1701: the root container item grants .allowsReading + .allowsAddingSubItems") {
    // Migrated from "1694: namespace model grants .allowsAddingSubItems":
    // the synthetic namespace roots are gone (1701 ruling) and the single
    // root — "Beebeeb" — inherits the capability contract: drops into the
    // root must keep working (1694 semantics).
    let root = BeebeebProviderItem.root()
    let mapped = FileProviderItem(model: root).capabilities
    try expect(
        mapped.contains(.allowsReading),
        "the root must keep .allowsReading"
    )
    try expect(
        mapped.contains(.allowsAddingSubItems),
        "the Beebeeb root must grant .allowsAddingSubItems (task 1694 semantics), got \(mapped.rawValue)"
    )
    try expect(
        root.filename == "Beebeeb",
        "the single root is named Beebeeb (1701 ruling), got \(root.filename)"
    )
    try expect(
        root.identifier == NSFileProviderItemIdentifier.rootContainer.rawValue
            && root.parentIdentifier == NSFileProviderItemIdentifier.rootContainer.rawValue,
        "the root item is the root container, parented on itself"
    )
}

// MARK: - Task 1697: capabilities completion, item metadata, enumerator state

// The two new capability bits cross the bridge with the SAME numbering as the
// Rust payload builder (CAP_REPARENT = 1 << 5, CAP_TRASH = 1 << 6 in
// src-tauri/src/ipc_socket.rs). Finder blocks drag-MOVE and drag-to-Trash on
// items whose mapped NSFileProviderItemCapabilities lacks them.

func item1697(capabilities: Int, kind: BeebeebItemKind = .file) -> BeebeebProviderItem {
    BeebeebProviderItem(
        identifier: "1697-item",
        parentIdentifier: "namespace:my_files",
        filename: "report.txt",
        kind: kind,
        sizeBytes: 120,
        contentType: nil,
        status: "local",
        capabilities: capabilities,
        versionIdentifier: "7:1700000100:120"
    )
}

check("1697: reparent bit (1 << 5) maps to .allowsReparenting, trash bit (1 << 6) to .allowsTrashing") {
    let live = FileProviderItem(model: item1697(capabilities: BeebeebProviderItem.read | BeebeebProviderItem.reparent | BeebeebProviderItem.trash))
    let mapped = live.capabilities
    try expect(mapped.contains(.allowsReparenting), "bit 1 << 5 must map to .allowsReparenting, got \(mapped.rawValue)")
    try expect(mapped.contains(.allowsTrashing), "bit 1 << 6 must map to .allowsTrashing, got \(mapped.rawValue)")
    try expect(mapped.contains(.allowsReading), "the read bit must still map")
}

check("1697: a read-only item gains neither reparenting nor trashing") {
    let readOnly = FileProviderItem(model: item1697(capabilities: BeebeebProviderItem.read))
    let mapped = readOnly.capabilities
    try expect(!mapped.contains(.allowsReparenting), "read-only items cannot be moved, got \(mapped.rawValue)")
    try expect(!mapped.contains(.allowsTrashing), "read-only items cannot be trashed, got \(mapped.rawValue)")
    try expect(mapped.contains(.allowsReading), "the read bit must still map")
}

func item1697Versions(content: String?, metadata: String?) -> BeebeebProviderItem {
    var model = item1697(capabilities: BeebeebProviderItem.read)
    // Rebuild with the version fields (the memberwise init takes them).
    return BeebeebProviderItem(
        identifier: model.identifier,
        parentIdentifier: model.parentIdentifier,
        filename: model.filename,
        kind: model.kind,
        sizeBytes: model.sizeBytes,
        contentType: model.contentType,
        status: model.status,
        capabilities: model.capabilities,
        versionIdentifier: model.versionIdentifier,
        createdAt: nil,
        modifiedAt: nil,
        childItemCount: nil,
        contentVersion: content,
        metadataVersion: metadata
    )
}

check("1697: itemVersion splits contentVersion from metadataVersion") {
    let item = FileProviderItem(model: item1697Versions(content: "7:abc123", metadata: "1700000100:120:parent-1:report.txt:local"))
    let version = item.itemVersion
    let content = String(decoding: version.contentVersion, as: UTF8.self)
    let metadata = String(decoding: version.metadataVersion, as: UTF8.self)
    try expect(content == "7:abc123", "contentVersion must come from the content_version field, got \(content)")
    try expect(metadata == "1700000100:120:parent-1:report.txt:local", "metadataVersion must come from the metadata_version field, got \(metadata)")
    try expect(
        version.contentVersion != version.metadataVersion,
        "the two versions are distinct identities (before 1697 they were the same bytes)"
    )
}

check("1697: a rename changes metadataVersion but never contentVersion") {
    let before = FileProviderItem(model: item1697Versions(content: "7:abc123", metadata: "1700000100:120:parent-1:report.txt:local"))
    var model = item1697Versions(content: "7:abc123", metadata: "1700000100:120:parent-1:renamed.txt:local")
    model = BeebeebProviderItem(
        identifier: model.identifier,
        parentIdentifier: model.parentIdentifier,
        filename: "renamed.txt",
        kind: model.kind,
        sizeBytes: model.sizeBytes,
        contentType: model.contentType,
        status: model.status,
        capabilities: model.capabilities,
        versionIdentifier: model.versionIdentifier,
        createdAt: nil,
        modifiedAt: nil,
        childItemCount: nil,
        contentVersion: model.contentVersion,
        metadataVersion: model.metadataVersion
    )
    let after = FileProviderItem(model: model)
    try expect(
        before.itemVersion.contentVersion == after.itemVersion.contentVersion,
        "a rename must not change the contentVersion (it would force a re-download)"
    )
    try expect(
        before.itemVersion.metadataVersion != after.itemVersion.metadataVersion,
        "a rename must change the metadataVersion"
    )
}

check("1697: itemVersion falls back to the daemon's versionIdentifier for old payloads") {
    let legacy = FileProviderItem(model: item1697Versions(content: nil, metadata: nil))
    let version = legacy.itemVersion
    let content = String(decoding: version.contentVersion, as: UTF8.self)
    try expect(content == "7:1700000100:120", "contentVersion falls back to the pre-1697 versionIdentifier, got \(content)")
    let metadata = String(decoding: version.metadataVersion, as: UTF8.self)
    try expect(metadata == "local:120", "metadataVersion falls back to the legacy status:size synthesis, got \(metadata)")
}

func item1697Dates(createdAt: Date?, modifiedAt: Date?) -> FileProviderItem {
    let model = BeebeebProviderItem(
        identifier: "1697-dates",
        parentIdentifier: "namespace:my_files",
        filename: "report.txt",
        kind: .file,
        sizeBytes: 120,
        contentType: nil,
        status: "local",
        capabilities: BeebeebProviderItem.read,
        versionIdentifier: nil,
        createdAt: createdAt,
        modifiedAt: modifiedAt,
        childItemCount: nil,
        contentVersion: nil,
        metadataVersion: nil
    )
    return FileProviderItem(model: model)
}

check("1697: contentModificationDate comes from the payload, creationDate falls back to it") {
    let modified = Date(timeIntervalSince1970: 1_700_000_100)
    let item = item1697Dates(createdAt: nil, modifiedAt: modified)
    try expect(item.contentModificationDate == modified, "contentModificationDate must surface the payload mtime")
    try expect(item.creationDate == modified, "a missing creationDate falls back to the mtime (Apple: never leave dates blank)")
    let blank = item1697Dates(createdAt: nil, modifiedAt: nil)
    try expect(blank.contentModificationDate == nil && blank.creationDate == nil, "an old daemon's payload yields no dates")
}

check("1697: childItemCount surfaces the real count for folders, nil for files") {
    let folder = FileProviderItem(model: BeebeebProviderItem(
        identifier: "1697-folder",
        parentIdentifier: "namespace:my_files",
        filename: "Folder",
        kind: .folder,
        sizeBytes: 0,
        contentType: nil,
        status: "local",
        capabilities: BeebeebProviderItem.read,
        versionIdentifier: nil,
        createdAt: nil,
        modifiedAt: nil,
        childItemCount: 7,
        contentVersion: nil,
        metadataVersion: nil
    ))
    try expect(folder.childItemCount == 7, "the folder's real child count must surface, got \(String(describing: folder.childItemCount))")
    let file = FileProviderItem(model: BeebeebProviderItem(
        identifier: "1697-file",
        parentIdentifier: "folder-1697",
        filename: "f.txt",
        kind: .file,
        sizeBytes: 1,
        contentType: nil,
        status: "local",
        capabilities: BeebeebProviderItem.read,
        versionIdentifier: nil,
        createdAt: nil,
        modifiedAt: nil,
        childItemCount: 7,
        contentVersion: nil,
        metadataVersion: nil
    ))
    try expect(file.childItemCount == nil, "files have no child count")
    let legacyFolder = FileProviderItem(model: BeebeebProviderItem(
        identifier: "1697-legacy",
        parentIdentifier: "namespace:my_files",
        filename: "Old",
        kind: .folder,
        sizeBytes: 0,
        contentType: nil,
        status: "local",
        capabilities: BeebeebProviderItem.read,
        versionIdentifier: nil
    ))
    try expect(legacyFolder.childItemCount == nil, "an old daemon reports no count (not a fake 0)")
}

// MARK: WorkingSetStore: anchor codec + persistence + materialized filter

check("1697: anchor codec round-trips, empty is genesis, garbage is expired") {
    let encoded = WorkingSetStore.encodeAnchor(42)
    try expect(WorkingSetStore.decodeAnchor(encoded) == 42, "round-trip failed")
    try expect(encoded.count <= 500, "Apple caps the anchor at 500 bytes, got \(encoded.count)")
    try expect(WorkingSetStore.decodeAnchor(Data()) == 0, "an empty anchor is the beginning of the log")
    try expect(WorkingSetStore.decodeAnchor(Data("garbage-anchor".utf8)) == nil, "an unparseable anchor must be EXPIRED, not silently restart at 0")
}

check("1697: anchor acceptance is strictly monotonic") {
    try expect(WorkingSetStore.anchorMonotonic(next: 5, current: nil), "the first anchor is always accepted")
    try expect(!WorkingSetStore.anchorMonotonic(next: 5, current: 5), "an equal anchor is a no-op")
    try expect(!WorkingSetStore.anchorMonotonic(next: 3, current: 5), "an older anchor must never rewind the replica")
    try expect(WorkingSetStore.anchorMonotonic(next: 6, current: 5), "a later anchor advances")
}

let stateDir = FileManager.default.temporaryDirectory.appendingPathComponent("1697-state-\(UUID().uuidString)")
try? FileManager.default.createDirectory(at: stateDir, withIntermediateDirectories: true)
let stateURL = stateDir.appendingPathComponent(WorkingSetStore.fileName)
defer { try? FileManager.default.removeItem(at: stateDir) }

check("1697: state store persists anchor + materialized containers and survives corruption") {
    try expect(WorkingSetStore.save(WorkingSetStore.State(
        lastAnchor: WorkingSetStore.encodeAnchor(9).base64EncodedString(),
        materializedContainers: ["dir-b", "dir-a"]
    ), to: stateURL), "the save must succeed in a temp dir")
    let loaded = WorkingSetStore.load(from: stateURL)
    try expect(loaded.materializedContainers == ["dir-a", "dir-b"], "containers must round-trip, got \(loaded.materializedContainers)")
    try expect(loaded.lastAnchor != nil, "the anchor must round-trip")
    try expect(WorkingSetStore.decodeAnchor(Data((loaded.lastAnchor ?? "").utf8)) == nil, "anchors persist as opaque strings; the codec parses the WIRE form")
    // A torn/corrupt file degrades to an empty state (fail-open), never a crash.
    try Data("not json".utf8).write(to: stateURL)
    let corrupt = WorkingSetStore.load(from: stateURL)
    try expect(corrupt.materializedContainers.isEmpty && corrupt.lastAnchor == nil, "corruption must read as an empty state")
}

check("1697: recordAnchor is monotonic on disk (injectable path — the real App Group container is never touched)") {
    try expect(WorkingSetStore.save(WorkingSetStore.State(
        lastAnchor: "v1:5",
        materializedContainers: []
    ), to: stateURL), "seed save")
    try expect(!WorkingSetStore.recordAnchor("v1:3", to: stateURL), "an older anchor must be refused")
    try expect(WorkingSetStore.load(from: stateURL).lastAnchor == "v1:5", "the persisted anchor must not rewind")
    try expect(WorkingSetStore.recordAnchor("v1:6", to: stateURL), "a later anchor is recorded")
    try expect(WorkingSetStore.load(from: stateURL).lastAnchor == "v1:6", "the persisted anchor advanced")
    try expect(!WorkingSetStore.recordAnchor("garbage", to: stateURL), "an unparseable anchor is refused outright")
}

// Task 1697 review fix (T1): the daemon speaks RAW decimal rowids ("1", "41")
// — review thread on FileProviderExtension.swift:598. `finishEnumeratingChanges`
// and `currentSyncAnchor` handed those raw decimals to the system; the next
// `enumerateChanges(from:)` decodes them via `decodeAnchor`, which only accepts
// the `v1:`-prefixed wire form → `syncAnchorExpired` → a FULL rescan after
// EVERY batch. The boundary helper encodes the daemon value before it is
// handed to (or persisted for) the system.
check("1697-T1: the daemon-anchor boundary encodes the decimal rowid into the wire form") {
    try expect(WorkingSetStore.encodeDaemonAnchor("1") == "v1:1", "the daemon's raw decimal '1' must reach the system as the 'v1:' wire form")
    try expect(WorkingSetStore.decodeAnchor(Data((WorkingSetStore.encodeDaemonAnchor("41") ?? "").utf8)) == 41, "daemon decimal → encode → decode must round-trip (the next enumerateChanges resumes from it)")
}

check("1697-T1: a raw daemon decimal fails decodeAnchor — the bug this fixes — and garbage encodes to nil") {
    try expect(WorkingSetStore.decodeAnchor(Data("1".utf8)) == nil, "raw '1' is NOT the wire form: handed to the system it comes back and fails decodeAnchor → syncAnchorExpired → full rescan after every batch")
    try expect(WorkingSetStore.encodeDaemonAnchor("garbage-anchor") == nil, "an unparseable daemon anchor must never be handed to the system (callers keep the starting anchor)")
}

check("1701: working-set filter — the root container is always materialized; namespace ids are gone") {
    let materialized: Set<String> = ["some-unrelated-folder"]
    try expect(
        WorkingSetStore.changeTouchesMaterialized(oldParent: nil, newParent: NSFileProviderItemIdentifier.rootContainer.rawValue, materialized: materialized),
        "changes under the root container must always be reported"
    )
    // Task 1701: the synthetic namespace ids are no longer a Finder surface
    // (one "Beebeeb" root). A change parented to one must be dropped.
    for ns in ["namespace:my_files", "namespace:shared_with_me", "namespace:offline", "namespace:conflicts"] {
        try expect(
            !WorkingSetStore.changeTouchesMaterialized(oldParent: nil, newParent: ns, materialized: materialized),
            "\(ns) is no longer enumerated (1701): its changes must be dropped by the working-set filter"
        )
    }
}

check("1701: a top-level row with no parent on either side is a root-child change — always reported") {
    let materialized: Set<String> = ["some-unrelated-folder"]
    try expect(
        WorkingSetStore.changeTouchesMaterialized(oldParent: nil, newParent: nil, materialized: materialized),
        "files.parent_id is NULL only at the vault root: a both-parents-nil row is a root-child change (a top-level create/modify, or a deletion whose parents are both gone), and the root is always materialized"
    )
}

check("1697: working-set filter — materialized parents pass, unknown fail, empty set fails open") {
    let materialized: Set<String> = ["dir-1"]
    try expect(WorkingSetStore.changeTouchesMaterialized(oldParent: nil, newParent: "dir-1", materialized: materialized), "a materialized parent passes")
    try expect(!WorkingSetStore.changeTouchesMaterialized(oldParent: nil, newParent: "dir-9", materialized: materialized), "an unknown parent is dropped")
    try expect(WorkingSetStore.changeTouchesMaterialized(oldParent: "dir-1", newParent: "dir-9", materialized: materialized), "Apple: old OR new parent for reparents")
    try expect(WorkingSetStore.changeTouchesMaterialized(oldParent: "dir-9", newParent: "dir-1", materialized: materialized), "a move INTO a materialized folder passes")
    try expect(WorkingSetStore.changeTouchesMaterialized(oldParent: nil, newParent: "dir-9", materialized: nil), "no tracked set = the whole dataset (fail open)")
    try expect(WorkingSetStore.changeTouchesMaterialized(oldParent: nil, newParent: "dir-9", materialized: []), "an empty set is fail-open too")
}

check("1697: changeBelongsToContainer — per-container filtering (old-or-new parent)") {
    func change(kind: String, itemParent: String?, oldParent: String?, newParent: String?) -> XPCBridge.DaemonChange {
        XPCBridge.DaemonChange(
            fileID: "moved-1",
            kind: kind,
            oldParentID: oldParent,
            newParentID: newParent,
            item: itemParent.map { parent in
                BeebeebProviderItem(
                    identifier: "moved-1",
                    parentIdentifier: parent,
                    filename: "f.txt",
                    kind: .file,
                    sizeBytes: 1,
                    contentType: nil,
                    status: "local",
                    capabilities: BeebeebProviderItem.read,
                    versionIdentifier: nil
                )
            }
        )
    }
    let materialized: Set<String> = ["dir-1"]
    try expect(
        FileProviderEnumerator.changeBelongsToContainer(
            change(kind: "modified", itemParent: "dir-1", oldParent: nil, newParent: nil),
            container: .workingSet,
            materialized: materialized
        ),
        "workingSet: item payload's parent is materialized"
    )
    try expect(
        !FileProviderEnumerator.changeBelongsToContainer(
            change(kind: "modified", itemParent: "dir-9", oldParent: nil, newParent: nil),
            container: .workingSet,
            materialized: materialized
        ),
        "workingSet: unrelated parent filtered"
    )
    try expect(
        FileProviderEnumerator.changeBelongsToContainer(
            change(kind: "modified", itemParent: "dir-1", oldParent: nil, newParent: nil),
            container: NSFileProviderItemIdentifier("dir-1"),
            materialized: materialized
        ),
        "container: the item's parent IS the container"
    )
    try expect(
        !FileProviderEnumerator.changeBelongsToContainer(
            change(kind: "modified", itemParent: "dir-9", oldParent: nil, newParent: nil),
            container: NSFileProviderItemIdentifier("dir-1"),
            materialized: materialized
        ),
        "container: unrelated items filtered"
    )
    try expect(
        FileProviderEnumerator.changeBelongsToContainer(
            change(kind: "reparented", itemParent: "dir-9", oldParent: "dir-1", newParent: "dir-9"),
            container: NSFileProviderItemIdentifier("dir-1"),
            materialized: materialized
        ),
        "container: a reparent OUT of the container is reported so its view drops the item"
    )
}

check("1697: page tokens — system initial pages decode to offset 0, minted tokens resume, garbage restarts") {
    try expect(FileProviderEnumerator.pageOffset(NSFileProviderPage(Data(NSFileProviderPage.initialPageSortedByName as NSData))) == 0, "InitialPageSortedByName is a fresh start")
    try expect(FileProviderEnumerator.pageOffset(NSFileProviderPage(Data(NSFileProviderPage.initialPageSortedByDate as NSData))) == 0, "InitialPageSortedByDate is a fresh start")
    try expect(FileProviderEnumerator.pageOffset(NSFileProviderPage(Data())) == 0, "an empty page is a fresh start")
    let token = Data("beebeeb-page:120".utf8)
    try expect(FileProviderEnumerator.pageOffset(NSFileProviderPage(token)) == 120, "a minted resume token names its offset")
    try expect(FileProviderEnumerator.pageOffset(NSFileProviderPage(Data("corrupt".utf8))) == 0, "corrupt tokens restart (idempotent), never fail")
}

check("1697: XPCBridge decodes the change page (anchor + kinds + items)") {
    let bridge = XPCBridge()
    let page = bridge.test_decodeChangesPayload([
        "FileProviderChanges": [
            "changes": [
                ["file_id": "f-1", "kind": "created", "new_parent_id": "dir-1",
                 "item": ["identifier": "f-1", "parent_identifier": "dir-1", "filename": "a.txt",
                          "kind": "file", "size_bytes": 3, "status": "cloud_only", "capabilities": 1]],
                ["file_id": "f-2", "kind": "deleted", "old_parent_id": "dir-1"],
            ],
            "next_anchor": "9",
        ],
    ])
    try expect(page.changes.count == 2, "both changes decode")
    try expect(page.changes[0].item?.filename == "a.txt", "created rows carry the full item payload")
    try expect(page.changes[1].item == nil && page.changes[1].oldParentID == "dir-1", "deletions carry the old parent")
    try expect(page.nextAnchor == "9", "the anchor to persist comes back — the daemon's RAW decimal rowid (task 1697-T1: the extension encodes it at the boundary)")
    let empty = bridge.test_decodeChangesPayload([:])
    try expect(empty.changes.isEmpty && empty.nextAnchor == nil, "a response without FileProviderChanges reads as empty")
}

// MARK: - Task 1697 review fix (T4): materialized-set page completion
//
// `materializedItemsDidChange` used to leave a DispatchGroup right after
// `enumerator.enumerateItems(for:startingAt:)` RETURNED — but delivery
// happens via didEnumerate/finishEnumerating callbacks ASYNCHRONOUSLY. The
// 5s wait woke early, `collector.nextPage` was still nil → loop broke → an
// EMPTY materialized set was persisted and filtering never became effective.
// The collector now signals page completion itself and the loop waits on it.
// Pure Foundation/Darwin: the collector class compiles into this harness,
// and the finish callbacks can be driven by hand exactly the way the system
// delivers them.

check("1697-T4: the collector's page wait is released by the ASYNC finish callback, not by enumerateItems returning") {
    let collector = MaterializedSetCollector()
    collector.beginPage()
    // Simulate the system exactly: the enumerateItems CALL returns
    // immediately (nothing to do here), the finish callback lands later.
    DispatchQueue.global().asyncAfter(deadline: .now() + 0.05) {
        collector.didEnumerate([])
        collector.finishEnumerating(upTo: nil)
    }
    let outcome = collector.waitPageCompletion(timeout: .seconds(5))
    try expect(outcome == .success, "the finish callback must release the page wait (a return-based wait would have woken BEFORE this)")
    try expect(collector.finished, "finishEnumerating(upTo: nil) marks the page complete")
    try expect(collector.nextPage == nil, "a nil next page ends pagination")
}

check("1697-T4: a paginated page delivers nextPage through the completion wait") {
    let collector = MaterializedSetCollector()
    collector.beginPage()
    DispatchQueue.global().asyncAfter(deadline: .now() + 0.05) {
        collector.didEnumerate([])
        collector.finishEnumerating(upTo: NSFileProviderPage(Data("beebeeb-page:7".utf8)))
    }
    let outcome = collector.waitPageCompletion(timeout: .seconds(5))
    try expect(outcome == .success, "the finish callback releases the wait")
    try expect(!collector.finished && collector.nextPage != nil, "a minted next page keeps pagination going")
}

check("1697-T4: the page wait is bounded (5s budget) and a stale signal never releases the NEXT page") {
    let collector = MaterializedSetCollector()
    collector.beginPage()
    let started = Date()
    let timedOut = collector.waitPageCompletion(timeout: .milliseconds(150))
    try expect(timedOut == .timedOut, "a silent enumerator must time out, not block the completion handler forever")
    try expect(Date().timeIntervalSince(started) >= 0.1, "the wait actually waited (bounded, not instant)")
    // The system's late finish callback for the TIMED-OUT page arrives now;
    // it must NOT release the wait of the page that follows.
    collector.finishEnumerating(upTo: NSFileProviderPage(Data("beebeeb-page:9".utf8)))
    collector.beginPage()
    let second = collector.waitPageCompletion(timeout: .milliseconds(150))
    try expect(second == .timedOut, "a fresh semaphore per page isolates stale signals from later waits")
}

// MARK: - 1698 part 1: contentPolicy mapping

check("1698-C1: the ROOT item reports .downloadLazily (lazy materialization of the tree)") {
    let item = FileProviderItem(model: .root())
    try expect(item.contentPolicy == .downloadLazily, "root contentPolicy must be .downloadLazily, got \(item.contentPolicy.rawValue)")
}

check("1698-C2: an effectively-PINNED file reports .downloadEagerlyAndKeepDownloaded") {
    var model = item1697(capabilities: BeebeebProviderItem.read)
    model = BeebeebProviderItem(
        identifier: model.identifier,
        parentIdentifier: model.parentIdentifier,
        filename: model.filename,
        kind: model.kind,
        sizeBytes: model.sizeBytes,
        contentType: model.contentType,
        status: model.status,
        capabilities: model.capabilities,
        versionIdentifier: model.versionIdentifier,
        pinned: true
    )
    let item = FileProviderItem(model: model)
    try expect(item.contentPolicy == .downloadEagerlyAndKeepDownloaded, "pinned file contentPolicy got \(item.contentPolicy.rawValue)")
}

check("1698-C3: a PINNED FOLDER also reports .downloadEagerlyAndKeepDownloaded (children inherit)") {
    var model = item1697(capabilities: BeebeebProviderItem.read)
    model = BeebeebProviderItem(
        identifier: model.identifier,
        parentIdentifier: model.parentIdentifier,
        filename: model.filename,
        kind: .folder,
        sizeBytes: model.sizeBytes,
        contentType: model.contentType,
        status: model.status,
        capabilities: model.capabilities,
        versionIdentifier: model.versionIdentifier,
        pinned: true
    )
    let item = FileProviderItem(model: model)
    try expect(item.contentPolicy == .downloadEagerlyAndKeepDownloaded, "pinned folder contentPolicy got \(item.contentPolicy.rawValue)")
}

check("1698-C4: an UNPINNED item reports .inherited (root default governs)") {
    var model = item1697(capabilities: BeebeebProviderItem.read)
    model = BeebeebProviderItem(
        identifier: model.identifier,
        parentIdentifier: model.parentIdentifier,
        filename: model.filename,
        kind: model.kind,
        sizeBytes: model.sizeBytes,
        contentType: model.contentType,
        status: model.status,
        capabilities: model.capabilities,
        versionIdentifier: model.versionIdentifier,
        pinned: false
    )
    let item = FileProviderItem(model: model)
    try expect(item.contentPolicy == .inherited, "unpinned contentPolicy got \(item.contentPolicy.rawValue)")
}

check("1698-C5: decodeItem reads the daemon's `pinned` field (absent = false, older daemons)") {
    let bridge = XPCBridge()
    let decoder = { (dict: [String: Any]) -> BeebeebProviderItem? in
        // XPCBridge.decodeItem is private; reach it through the same
        // item-payload path the framing harness already uses: a one-item
        // ListFileProviderItems reply.
        try? bridge.test_decodeItemsPayload(["FileProviderItems": ["items": [dict]]]).first
    }
    let base: [String: Any] = [
        "identifier": "f-1",
        "parent_identifier": "NSFileProviderRootContainerItemIdentifier",
        "filename": "a.txt",
        "kind": "file",
        "status": "cloud_only",
    ]
    let unpinned = decoder(base)
    try expect(unpinned?.pinned == false, "absent pinned defaults to false")
    let pinned = decoder(base.merging(["pinned": true]) { _, new in new })
    try expect(pinned?.pinned == true, "pinned=true decodes")
}

// MARK: - 1698 part 2: trash semantics (ruling: full sync)

check("1698-T1: item(for: trashContainer) is answered locally as a system folder (no IPC)") {
    // The trash container is a SYSTEM container: always materialized,
    // never a daemon lookup. This must not touch the IPC socket (which the
    // test binary cannot reach) — a successful local answer proves it.
    let item = try XPCBridge().item(identifier: .trashContainer)
    try expect(item.identifier == NSFileProviderItemIdentifier.trashContainer.rawValue, "the trash container identifies itself")
    try expect(item.kind == .folder, "the trash container is a folder")
    try expect(item.capabilities == BeebeebProviderItem.read, "the system trash is read-only to us, got \(item.capabilities)")
}

check("1698-T2: deleteDisposition rejects a non-recursive NON-EMPTY folder with directoryNotEmpty") {
    try expect(
        FileProviderExtension.deleteDisposition(isFolder: true, childItemCount: 2, recursive: false) == .directoryNotEmpty,
        "a non-recursive delete of a folder with children must be rejected"
    )
    try expect(
        FileProviderExtension.deleteDisposition(isFolder: true, childItemCount: 0, recursive: false) == .queueServerTrash,
        "an EMPTY folder deletes fine without the recursive option"
    )
    try expect(
        FileProviderExtension.deleteDisposition(isFolder: true, childItemCount: 2, recursive: true) == .queueServerTrash,
        "the recursive option accepts a non-empty folder"
    )
    try expect(
        FileProviderExtension.deleteDisposition(isFolder: false, childItemCount: nil, recursive: false) == .queueServerTrash,
        "a file deletes without the recursive option"
    )
    try expect(
        FileProviderExtension.deleteDisposition(isFolder: true, childItemCount: nil, recursive: false) == .queueServerTrash,
        "an unknown child count fails OPEN (the daemon always sends one for folders)"
    )
}

check("1698-T3: the trash container is ALWAYS materialized for the working-set filter") {
    let always = WorkingSetStore.alwaysMaterializedIdentifiers()
    try expect(always.contains(NSFileProviderItemIdentifier.trashContainer.rawValue), "the trash container is a system container")
    try expect(
        WorkingSetStore.changeTouchesMaterialized(
            oldParent: nil,
            newParent: NSFileProviderItemIdentifier.trashContainer.rawValue,
            materialized: ["some-unrelated-folder"]
        ),
        "a change parented at the trash container is reported even with an unrelated materialized set"
    )
}

check("1698-T4: a modify whose NEW parent is the trash container routes to the server trash") {
    try expect(
        FileProviderExtension.modifyRoute(newParent: .trashContainer) == .serverTrash,
        "the Finder-side trash is a reparent to the trash container → the server's existing trash op"
    )
    try expect(
        FileProviderExtension.modifyRoute(newParent: .rootContainer) == .metadataUpdate,
        "a reparent under the root is an ordinary move"
    )
    try expect(
        FileProviderExtension.modifyRoute(newParent: NSFileProviderItemIdentifier("dir-9")) == .metadataUpdate,
        "a reparent into a folder is an ordinary move"
    )
}

// MARK: - Test hooks (task 1697)

// Exposes XPCBridge's change-page decoder so the harness can pin the wire
// shape without a socket. Not for production use.
extension XPCBridge {
    func test_decodeChangesPayload(_ dictionary: [String: Any]) -> ChangesPage {
        Self.decodeChanges(dictionary)
    }

    func test_decodeItemsPayload(_ dictionary: [String: Any]) -> [BeebeebProviderItem] {
        guard let payload = dictionary["FileProviderItems"] as? [String: Any],
              let rawItems = payload["items"] as? [[String: Any]] else {
            return []
        }
        return rawItems.compactMap(Self.decodeItem)
    }
}

// MARK: - 1698 part 4: error truth (transient vs definitive)

check("1698-E1: invalidResponse maps to TRANSIENT serverUnreachable, NOT cannotSynchronize") {
    let err = BeebeebIPCError.invalidResponse("Beebeeb's sync engine closed the connection before answering.") as NSError
    try expect(err.domain == NSFileProviderErrorDomain, "bridges into NSFileProviderErrorDomain, got \(err.domain)")
    try expect(
        err.code == NSFileProviderError.serverUnreachable.rawValue,
        "invalidResponse must be transient (serverUnreachable = -1004), got code \(err.code)"
    )
}

check("1698-E2: timedOut maps to serverUnreachable (transient)") {
    let err = BeebeebIPCError.timedOut(seconds: 30) as NSError
    try expect(err.code == NSFileProviderError.serverUnreachable.rawValue, "got code \(err.code)")
}

check("1698-E3: daemonUnavailable maps to serverUnreachable (transient)") {
    let err = BeebeebIPCError.daemonUnavailable as NSError
    try expect(err.code == NSFileProviderError.serverUnreachable.rawValue, "got code \(err.code)")
}

check("1698-E4: invalidIdentifier stays DEFINITIVE cannotSynchronize") {
    // A malformed identifier is a request-shape failure, not a transport
    // glitch: retrying can never fix it, so it must stay definitive.
    let err = BeebeebIPCError.invalidIdentifier as NSError
    try expect(err.code == NSFileProviderError.cannotSynchronize.rawValue, "got code \(err.code)")
}

check("1698-E5: the transient classification used by the resolved-signal bookkeeping") {
    try expect(BeebeebIPCError.invalidResponse("x").isTransient, "invalidResponse is transient")
    try expect(BeebeebIPCError.timedOut(seconds: 1).isTransient, "timedOut is transient")
    try expect(BeebeebIPCError.daemonUnavailable.isTransient, "daemonUnavailable is transient")
    try expect(!BeebeebIPCError.invalidIdentifier.isTransient, "invalidIdentifier is definitive")
    try expect(!BeebeebIPCError.cancelled.isTransient, "cancelled is a user action, not an unreachable")
}

check("1698-E6: TransientErrorTracker arms on transient, ignores definitive, clears on success") {
    let tracker = TransientErrorTracker()
    try expect(!tracker.hasPending, "a fresh tracker has nothing pending")
    tracker.record(BeebeebIPCError.invalidIdentifier as NSError)
    try expect(!tracker.hasPending, "a definitive error must never arm the resolved-signal")
    tracker.record(BeebeebIPCError.invalidResponse("sync engine unreachable") as NSError)
    try expect(tracker.hasPending, "a transient error arms the resolved-signal")
    let pending = tracker.takePending()
    try expect(pending?.code == NSFileProviderError.serverUnreachable.rawValue, "the pending error is the reported one")
    try expect(!tracker.hasPending, "take clears the pending state")
}

check("1698-E7: the daemon's real message still reaches the system (error truth)") {
    let err = BeebeebIPCError.invalidResponse("hydrate destination is not within an allowed root") as NSError
    let message = err.userInfo[NSLocalizedDescriptionKey] as? String
    try expect(message?.contains("allowed root") == true, "the real message must survive the mapping, got \(message ?? "nil")")
}

check("1698-E8: daemonRejected maps to DEFINITIVE cannotSynchronize (not retried)") {
    // PR #100 review (Codex P2): the daemon (or this extension's own policy)
    // actively REFUSED the request — restarting or unlocking the daemon
    // cannot make the request valid — so it must ride the definitive class,
    // never the transient serverUnreachable one Finder would retry forever.
    let err = BeebeebIPCError.daemonRejected("Beebeeb cannot create items inside the Trash.") as NSError
    try expect(err.domain == NSFileProviderErrorDomain, "bridges into NSFileProviderErrorDomain, got \(err.domain)")
    try expect(
        err.code == NSFileProviderError.cannotSynchronize.rawValue,
        "a daemon rejection is definitive (cannotSynchronize), got code \(err.code)"
    )
    try expect(!BeebeebIPCError.daemonRejected("x").isTransient, "a daemon rejection must never arm the resolved-signal")
}

check("1698-E9: the daemon's rejection text is the user-facing message") {
    let err = BeebeebIPCError.daemonRejected("Beebeeb cannot create items inside the Trash.")
    try expect(
        err.errorDescription == "Beebeeb cannot create items inside the Trash.",
        "the real message must survive the mapping, got \(err.errorDescription ?? "nil")"
    )
}

// MARK: - Task 1699: sync-state badges (NSFileProviderItemDecorating)

// The daemon's per-item `status` (state_db.rs FileStatus::as_str: cloud_only,
// downloading, local, uploading, conflict, error, trashing) crosses the bridge
// as a plain string. macOS renders only the FIRST decoration in a category
// (Badge), so the mapping is intentionally AT MOST ONE identifier per item;
// if statuses ever compose, the documented priority is
// error > conflict > trashing > uploading > downloading.
func item1699(status: String, kind: BeebeebItemKind = .file, contentType: String? = nil) -> BeebeebProviderItem {
    BeebeebProviderItem(
        identifier: "1699-item",
        parentIdentifier: "NSFileProviderRootContainerItemIdentifier",
        filename: "photo.jpg",
        kind: kind,
        sizeBytes: 2048,
        contentType: contentType,
        status: status,
        capabilities: BeebeebProviderItem.read,
        versionIdentifier: nil
    )
}

check("1699-D1: each sync state maps to exactly one badge identifier; plain states map none") {
    try expect(
        BeebeebProviderItem.decorationIdentifier(forStatus: "error") == BeebeebProviderItem.decorationError,
        "error → error badge, got \(String(describing: BeebeebProviderItem.decorationIdentifier(forStatus: "error")))"
    )
    try expect(
        BeebeebProviderItem.decorationIdentifier(forStatus: "conflict") == BeebeebProviderItem.decorationConflict,
        "conflict → conflict badge"
    )
    try expect(
        BeebeebProviderItem.decorationIdentifier(forStatus: "trashing") == BeebeebProviderItem.decorationTrashing,
        "trashing → trashing badge"
    )
    try expect(
        BeebeebProviderItem.decorationIdentifier(forStatus: "uploading") == BeebeebProviderItem.decorationUploading,
        "uploading → uploading badge"
    )
    try expect(
        BeebeebProviderItem.decorationIdentifier(forStatus: "downloading") == BeebeebProviderItem.decorationDownloading,
        "downloading → downloading badge"
    )
    for plain in ["local", "cloud_only", "nonsense"] {
        try expect(
            BeebeebProviderItem.decorationIdentifier(forStatus: plain) == nil,
            "\(plain) must carry NO badge, got \(String(describing: BeebeebProviderItem.decorationIdentifier(forStatus: plain)))"
        )
    }
}

check("1699-D2: FileProviderItem.decorations surfaces the model's badge; a plain local item has none") {
    let uploading = FileProviderItem(model: item1699(status: "uploading"))
    try expect(
        uploading.decorations == [NSFileProviderItemDecorationIdentifier(BeebeebProviderItem.decorationUploading)],
        "an uploading item must carry exactly the uploading badge, got \(String(describing: uploading.decorations))"
    )
    let local = FileProviderItem(model: item1699(status: "local"))
    try expect(local.decorations == nil, "a fully-synced local item must carry no badge, got \(String(describing: local.decorations))")
}

check("1699-D3: the five badge identifiers are distinct and share the io.beebeeb.app.decoration namespace") {
    let ids = [
        BeebeebProviderItem.decorationError,
        BeebeebProviderItem.decorationConflict,
        BeebeebProviderItem.decorationTrashing,
        BeebeebProviderItem.decorationUploading,
        BeebeebProviderItem.decorationDownloading,
    ]
    try expect(Set(ids).count == 5, "five distinct identifiers, got \(ids)")
    for id in ids {
        try expect(id.hasPrefix("io.beebeeb.app.decoration."), "identifier \(id) must live in the extension's declared namespace (Info.plist NSFileProviderDecorations)")
    }
}

// MARK: - Task 1699: thumbnails (NSFileProviderThumbnailing)

// The pure decision helpers the (untestable-headless) fetchThumbnails method
// is built from: variant bucket, pixel budget and eligibility. The daemon's
// bucket picker (ipc_socket.rs thumbnail_variant, mirroring the Windows
// provider) is ≤96 "small", ≤256 "medium", else "large".

check("1699-T1: variant buckets mirror the daemon's picker (≤96 small, ≤256 medium, else large)") {
    try expect(FileProviderExtension.thumbnailVariant(maxDimension: 1) == "small", "1 → small")
    try expect(FileProviderExtension.thumbnailVariant(maxDimension: 96) == "small", "96 → small (bucket edge)")
    try expect(FileProviderExtension.thumbnailVariant(maxDimension: 97) == "medium", "97 → medium")
    try expect(FileProviderExtension.thumbnailVariant(maxDimension: 256) == "medium", "256 → medium (bucket edge)")
    try expect(FileProviderExtension.thumbnailVariant(maxDimension: 257) == "large", "257 → large")
    try expect(FileProviderExtension.thumbnailVariant(maxDimension: UInt32.max) == "large", "huge request → large")
}

check("1699-T2: requestedSize points → a 2x pixel budget, larger side governs, clamped, deterministic") {
    try expect(FileProviderExtension.thumbnailMaxDimension(requestedSize: CGSize(width: 22, height: 22)) == 44, "22pt @2x = 44px")
    try expect(FileProviderExtension.thumbnailMaxDimension(requestedSize: CGSize(width: 200, height: 100)) == 400, "the LARGER side governs (200pt @2x = 400px)")
    try expect(FileProviderExtension.thumbnailMaxDimension(requestedSize: CGSize(width: 9999, height: 9999)) == 1024, "an absurd request clamps to the 1024px budget")
    try expect(FileProviderExtension.thumbnailMaxDimension(requestedSize: .zero) == 256, "a sizeless request falls back to the 256px default (never 0)")
}

check("1699-T3: only files with an image/video/movie content type are thumbnail-eligible") {
    try expect(!FileProviderExtension.thumbnailEligible(kind: .folder, contentType: "public.folder"), "folders are never thumbnail-eligible")
    try expect(FileProviderExtension.thumbnailEligible(kind: .file, contentType: "public.jpeg"), "images are eligible")
    try expect(FileProviderExtension.thumbnailEligible(kind: .file, contentType: "public.movie"), "movies are eligible")
    try expect(!FileProviderExtension.thumbnailEligible(kind: .file, contentType: "public.plain-text"), "documents are not")
    try expect(!FileProviderExtension.thumbnailEligible(kind: .file, contentType: nil), "an unknown content type is not (the daemon only has image/video thumbnails)")
    try expect(!FileProviderExtension.thumbnailEligible(kind: .file, contentType: "not a uti"), "an unparseable content type is not")
    // Task 1699 review (PR #103, thread PRRT_kwDOSLX6Xs6oeNfK, P1): the sync
    // pipeline stores MIME content types (`image/png`, engine_bridge.rs
    // guess_mime_type), not UTIs. `UTType(raw)` only parses UTI identifiers,
    // so the old guard made EVERY synced media ineligible and thumbnails
    // never fetched. MIME must convert via `UTType(mimeType:)`, with the
    // UTI parse retained as the fallback.
    try expect(FileProviderExtension.thumbnailEligible(kind: .file, contentType: "image/png"), "MIME image content types (what the sync pipeline stores) are eligible")
    try expect(FileProviderExtension.thumbnailEligible(kind: .file, contentType: "video/mp4"), "MIME video content types are eligible")
    try expect(FileProviderExtension.thumbnailEligible(kind: .file, contentType: "public.png"), "UTI identifiers still parse (fallback retained)")
}

check("1699-T4: the FetchThumbnail request carries file_id + dest_path + max_dimension") {
    let payload = try requestPayload(
        XPCBridge.thumbnailRequest(fileID: "abc-123", destinationPath: "/staging/thumb.bin", maxDimension: 128),
        "FetchThumbnail"
    )
    try expect(payload["file_id"] as? String == "abc-123", "file_id must ride the payload, got \(String(describing: payload["file_id"]))")
    try expect(payload["dest_path"] as? String == "/staging/thumb.bin", "dest_path must ride the payload")
    try expect((payload["max_dimension"] as? NSNumber)?.uint32Value == 128, "max_dimension must ride the payload")
}

check("1699-T5: ThumbnailWritten decodes the byte count; Error is daemonRejected; a bare reply is invalidResponse") {
    let size = try XPCBridge.decodeThumbnailReply(["ThumbnailWritten": ["size_bytes": 1234]])
    try expect(size == 1234, "size_bytes must decode, got \(size)")
    do {
        _ = try XPCBridge.decodeThumbnailReply(["Error": ["message": "thumbnail fetch failed: 404"]])
        throw TestFailure(description: "an Error reply must throw daemonRejected")
    } catch let error as BeebeebIPCError {
        guard case .daemonRejected = error else {
            throw TestFailure(description: "an Error reply must be daemonRejected, got \(error)")
        }
    }
    do {
        _ = try XPCBridge.decodeThumbnailReply(["Ok": [String: Any]()])
        throw TestFailure(description: "a reply without ThumbnailWritten must throw invalidResponse")
    } catch let error as BeebeebIPCError {
        guard case .invalidResponse = error else {
            throw TestFailure(description: "a bare reply must be invalidResponse, got \(error)")
        }
    }
}

check("1699-T6: the thumbnail timeout stays between the metadata and hydrate ceilings") {
    try expect(IPCFraming.thumbnailTimeoutSeconds == 60, "pinned at 60s, got \(IPCFraming.thumbnailTimeoutSeconds)")
    try expect(
        IPCFraming.thumbnailTimeoutSeconds > IPCFraming.metadataTimeoutSeconds
            && IPCFraming.thumbnailTimeoutSeconds < IPCFraming.hydrateTimeoutSeconds,
        "a thumbnail is more than a metadata lookup but nowhere near a whole-file download"
    )
}

check("1699-P1: pendingItemsDidChange hands control back promptly (a system→extension callback)") {
    // macOS 11.3+ (V3_1): the SYSTEM calls this when its pending set
    // refreshes — it tracks pending items itself (enumeratorForPendingItems).
    // The honest implementation logs and completes immediately; this test
    // pins that the completion handler is always invoked (a hung callback
    // would stall the system's pending-set bookkeeping).
    let domain = NSFileProviderDomain(identifier: NSFileProviderDomainIdentifier("1699-pending"), displayName: "Beebeeb")
    let extension1699 = FileProviderExtension(domain: domain)
    let group = DispatchGroup()
    group.enter()
    var calledBack = false
    extension1699.pendingItemsDidChange {
        calledBack = true
        group.leave()
    }
    try expect(group.wait(timeout: .now() + 2) == .success, "pendingItemsDidChange must call its completion handler promptly")
    try expect(calledBack, "the completion handler must have run")
}

// MARK: - Upload staging (write contents handed to the app via the App Group)
//
// The app cannot read the system's contents URL, so the extension copies a
// write's contents into <App Group>/upload-staging/ and sends that copy's
// path. Each test uses a throwaway directory as the "container": never the
// real App Group container.

func withScratchContainer(_ body: (URL) throws -> Void) throws {
    let container = FileManager.default.temporaryDirectory
        .appendingPathComponent("upload-staging-test-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: container, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: container) }
    try body(container)
}

func posixMode(_ url: URL) throws -> Int {
    let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
    return ((attributes[.posixPermissions] as? NSNumber)?.intValue ?? -1) & 0o777
}

func stagingEntries(_ container: URL) -> [String] {
    let dir = container.appendingPathComponent(UploadStaging.directoryName, isDirectory: true)
    return (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []
}

check("upload-staging-S1: the directory is <container>/upload-staging, owner-only even if it existed looser") {
    try withScratchContainer { container in
        let expected = container.appendingPathComponent("upload-staging", isDirectory: true)
        try FileManager.default.createDirectory(
            at: expected, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o755]
        )
        let dir = try UploadStaging.directory(in: container)
        try expect(dir.standardizedFileURL.path == expected.standardizedFileURL.path, "got \(dir.path)")
        var isDirectory: ObjCBool = false
        try expect(FileManager.default.fileExists(atPath: dir.path, isDirectory: &isDirectory) && isDirectory.boolValue,
                   "the directory must exist")
        let mode = try posixMode(dir)
        try expect(mode == 0o700, "upload-staging must be 0700, got \(String(mode, radix: 8))")
    }
}

check("upload-staging-S2: each copy gets a fresh UUID name, never the user's file name") {
    try withScratchContainer { container in
        let source = container.appendingPathComponent("Quarterly report.pdf")
        try Data("x".utf8).write(to: source)
        let first = try UploadStaging.stage(contentsOf: source, in: container)
        let second = try UploadStaging.stage(contentsOf: source, in: container)
        try expect(first != second, "two stagings must never share a copy")
        for copy in [first, second] {
            try expect(UUID(uuidString: copy.lastPathComponent) != nil, "copy name must be a UUID, got \(copy.lastPathComponent)")
            try expect(!copy.lastPathComponent.contains("Quarterly"), "copy name must not carry the user's file name")
            try expect(copy.deletingLastPathComponent().lastPathComponent == "upload-staging",
                       "the copy must sit directly in upload-staging, got \(copy.path)")
        }
    }
}

check("upload-staging-S3: the copy holds the exact bytes, is owner-only (0600) and stamped now, the source untouched") {
    try withScratchContainer { container in
        let source = container.appendingPathComponent("photo.jpg")
        let bytes = Data((0..<4096).map { UInt8($0 % 251) })
        try bytes.write(to: source)
        try FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: source.path)
        let old = Date(timeIntervalSince1970: 1_000_000_000) // 2001: a copy that kept this would look orphaned
        try FileManager.default.setAttributes([.modificationDate: old], ofItemAtPath: source.path)
        let copy = try UploadStaging.stage(contentsOf: source, in: container)
        try expect(copy != source, "the app must get a copy, not the system's URL")
        try expect(try Data(contentsOf: copy) == bytes, "the copy must hold the exact bytes")
        let mode = try posixMode(copy)
        try expect(mode == 0o600, "the copy must be 0600, got \(String(mode, radix: 8))")
        let stamped = try FileManager.default.attributesOfItem(atPath: copy.path)[.modificationDate] as? Date
        try expect(stamped.map { abs($0.timeIntervalSinceNow) < 60 } == true,
                   "the copy's mtime must be the staging time, got \(String(describing: stamped))")
        try expect(try Data(contentsOf: source) == bytes, "the system's file must be left as it was")
        let sourceDate = try FileManager.default.attributesOfItem(atPath: source.path)[.modificationDate] as? Date
        try expect(sourceDate == old, "the system's file must keep its own mtime")
    }
}

check("upload-staging-S4: a failed copy is a TRANSIENT serverUnreachable error and leaves nothing behind") {
    try withScratchContainer { container in
        let missing = container.appendingPathComponent("secret-name.txt") // never created
        do {
            _ = try UploadStaging.stage(contentsOf: missing, in: container)
            throw TestFailure(description: "staging a missing file must throw")
        } catch let error as BeebeebIPCError {
            guard case .uploadStagingFailed = error else {
                throw TestFailure(description: "expected uploadStagingFailed, got \(error)")
            }
            let ns = error as NSError
            try expect(ns.domain == NSFileProviderErrorDomain, "domain \(ns.domain)")
            try expect(ns.code == NSFileProviderError.serverUnreachable.rawValue,
                       "a staging failure must be transient serverUnreachable (-1004), got \(ns.code)")
            try expect(error.isTransient, "a staging failure must arm the resolved-signal bookkeeping")
            try expect(!ns.localizedDescription.contains("secret-name"),
                       "the error text must not carry the file name: \(ns.localizedDescription)")
        }
        try expect(stagingEntries(container).isEmpty, "a failed copy must leave nothing: \(stagingEntries(container))")
    }
}

check("upload-staging-S5: discard deletes the copy and tolerates one the app already deleted") {
    try withScratchContainer { container in
        let source = container.appendingPathComponent("a.txt")
        try Data("a".utf8).write(to: source)
        let copy = try UploadStaging.stage(contentsOf: source, in: container)
        try expect(FileManager.default.fileExists(atPath: copy.path), "staged")
        UploadStaging.discard(copy)
        try expect(!FileManager.default.fileExists(atPath: copy.path), "discard must delete the copy")
        UploadStaging.discard(copy) // the app deleted it first: must not crash
        try expect(FileManager.default.fileExists(atPath: source.path), "discard never touches the system's file")
    }
}

print("ipc-framing: \(passed) passed, \(failed) failed")
exit(failed == 0 ? 0 : 1)
