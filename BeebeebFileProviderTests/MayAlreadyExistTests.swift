import FileProvider
import Foundation

// drive-macos-89: a `.mayAlreadyExist` create keeps an offline edit (MayAlreadyExist.swift).
//
// Owns: the folder / equal file / edited file outcomes and the content-version hash parse.
// Tests: called from FeatureChecks.swift's runner.

func runMayAlreadyExistChecks() {
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("dm89-\(UUID().uuidString)")
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let serverBytes = Data(repeating: 0x61, count: 120) // item1697's sizeBytes
    let serverHash = (try? FileProviderExtension.sha256Hex(of: write(serverBytes, "server.txt", in: dir))) ?? ""
    let server = item1697Versions(content: "7:\(serverHash)", metadata: "m1")
    let parent = NSFileProviderItemIdentifier("namespace:my_files")

    // Replays one create of report.txt; returns the completion and the items it queued as a new version.
    func replay(_ contents: URL?, kind: BeebeebItemKind = .file, listed: [BeebeebProviderItem]) throws
        -> (FileProviderExtension.QueuedWriteCompletion?, [String])
    {
        var queued: [String] = []
        let completion = try FileProviderExtension.resolveMayAlreadyExist(
            parent: parent, filename: "report.txt", kind: kind, contents: contents,
            list: { _ in listed },
            queueNewVersion: { existing in
                queued.append(existing.identifier)
                return WriteQueueResult(item: item1697Versions(content: "8:local", metadata: "m2"), ignored: false, message: "queued")
            }
        )
        return (completion, queued)
    }

    check("dm89: an offline-edited file replayed with mayAlreadyExist is uploaded as a new version, never dropped") {
        // Same size, different bytes: only the hash tells them apart.
        let edited = try write(Data(repeating: 0x62, count: 120), "edited.txt", in: dir)
        let (completion, queued) = try replay(edited, listed: [server])
        try expect(queued == ["1697-item"], "the edit must be queued as a new version of the server item, queued \(queued)")
        let version = completion?.item.map { String(decoding: $0.itemVersion!.contentVersion, as: UTF8.self) }
        try expect(version == "8:local" && completion?.error == nil, "the queued version is returned, got \(String(describing: version))")
        try expect(completion?.shouldFetchContent == false, "the server's bytes must never be fetched over the edit")
        try expect(try Data(contentsOf: edited) == Data(repeating: 0x62, count: 120), "the local bytes stay as they were")
        // A different size, and a server version without a hash, also upload.
        let longer = try write(Data(repeating: 0x61, count: 121), "longer.txt", in: dir)
        try expect(try replay(longer, listed: [server]).1 == ["1697-item"], "a different size is an edit")
        let unhashed = item1697Versions(content: "7", metadata: "m1")
        let same = try write(serverBytes, "same.txt", in: dir)
        try expect(try replay(same, listed: [unhashed]).1 == ["1697-item"], "bytes that cannot be shown equal are uploaded")
    }

    check("dm89: an equal file returns the server item; a folder matches as before; no match creates") {
        let same = try write(serverBytes, "same.txt", in: dir)
        let (equal, queued) = try replay(same, listed: [server])
        try expect(equal?.item?.itemIdentifier.rawValue == "1697-item" && queued.isEmpty, "an equal file is the server item")
        let folder = item1697(capabilities: 0, kind: .folder)
        let (matched, folderQueued) = try replay(nil, kind: .folder, listed: [folder])
        try expect(matched?.item?.itemIdentifier.rawValue == "1697-item" && folderQueued.isEmpty, "a folder matches by name and kind")
        let (none, noneQueued) = try replay(same, listed: [])
        try expect(none == nil && noneQueued.isEmpty, "no server item: the create goes ahead")
    }

    check("dm89: the content-version hash parse and the local SHA-256") {
        let hex = String(repeating: "ab", count: 32)
        try expect(FileProviderExtension.contentHash(ofContentVersion: "7:\(hex.uppercased())") == hex, "{v}:{hash} carries the hash")
        for bad in [nil, "7", "0:\(hex)", "7:abc", "7:1700000100:120", "x:\(hex)"] as [String?] {
            try expect(FileProviderExtension.contentHash(ofContentVersion: bad) == nil, "no hash in \(String(describing: bad))")
        }
        let abc = try write(Data("abc".utf8), "abc.txt", in: dir)
        try expect(
            try FileProviderExtension.sha256Hex(of: abc) == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "SHA-256 of \"abc\""
        )
    }
}

private func write(_ data: Data, _ name: String, in dir: URL) throws -> URL {
    let url = dir.appendingPathComponent(name)
    try data.write(to: url)
    return url
}
