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

check("1694: namespace model grants .allowsAddingSubItems") {
    let namespace = BeebeebProviderItem.namespace(.myFiles)
    let mapped = FileProviderItem(model: namespace).capabilities
    try expect(
        mapped.contains(.allowsReading),
        "namespace must keep .allowsReading"
    )
    try expect(
        mapped.contains(.allowsAddingSubItems),
        "sidebar namespace roots must grant .allowsAddingSubItems (task 1694), got \(mapped.rawValue)"
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

check("1697: working-set filter — namespace roots are always materialized") {
    let materialized: Set<String> = ["some-unrelated-folder"]
    for parent in [
        NSFileProviderItemIdentifier.rootContainer.rawValue,
        BeebeebNamespace.myFiles.identifier.rawValue,
        BeebeebNamespace.sharedWithMe.identifier.rawValue,
        BeebeebNamespace.offline.identifier.rawValue,
        BeebeebNamespace.conflicts.identifier.rawValue,
    ] {
        try expect(
            WorkingSetStore.changeTouchesMaterialized(oldParent: nil, newParent: parent, materialized: materialized),
            "changes under \(parent) must always be reported"
        )
    }
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
            "next_anchor": "v1:9",
        ],
    ])
    try expect(page.changes.count == 2, "both changes decode")
    try expect(page.changes[0].item?.filename == "a.txt", "created rows carry the full item payload")
    try expect(page.changes[1].item == nil && page.changes[1].oldParentID == "dir-1", "deletions carry the old parent")
    try expect(page.nextAnchor == "v1:9", "the anchor to persist comes back")
    let empty = bridge.test_decodeChangesPayload([:])
    try expect(empty.changes.isEmpty && empty.nextAnchor == nil, "a response without FileProviderChanges reads as empty")
}

print("ipc-framing: \(passed) passed, \(failed) failed")
exit(failed == 0 ? 0 : 1)

// MARK: - Test hooks (task 1697)

// Exposes XPCBridge's change-page decoder so the harness can pin the wire
// shape without a socket. Not for production use.
extension XPCBridge {
    func test_decodeChangesPayload(_ dictionary: [String: Any]) -> ChangesPage {
        Self.decodeChanges(dictionary)
    }
}
