import Foundation

// Framing tests for BeebeebFileProvider/IPCFraming.swift (task 1670 issue 3).
//
// The File Provider extension target has no XCTest target, so this is a plain
// executable: scripts/test-ipc-framing.sh compiles IPCFraming.swift together
// with this file (pure Foundation, no FileProvider), runs it, and asserts the
// printed COUNT ("ipc-framing: N passed, 0 failed") -- a run that executed
// fewer tests than declared fails the script.

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

check("DELIBERATE RED PROOF (removed in the next commit)") {
    try expect(false, "deliberate failure to prove the count guard can go red")
}

print("ipc-framing: \(passed) passed, \(failed) failed")
exit(failed == 0 ? 0 : 1)
