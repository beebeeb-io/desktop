import Foundation
import FileProvider

/// Task 1697: the replica's crash-recovery state, in the shared App Group
/// container. Two things must survive extension process death:
///
/// 1. `last_anchor` — the sync anchor the enumerator last DELIVERED to the
///    system. `currentSyncAnchor` reads it when the daemon is unreachable;
///    daemon-sourced anchors overwrite it after every change batch.
/// 2. `materialized_containers` — the directory identifiers the system
///    reported as materialized (via `materializedItemsDidChange`). The
///    working-set enumerator filters changes against this set (Apple's
///    Replicated contract: "the filtering by parentItemIdentifier is better
///    done in the extension"), and without it the working set is the entire
///    dataset.
///
/// The file is a single JSON document written atomically (temp + rename) by
/// the extension only; the daemon never writes it. All pure decision helpers
/// (`anchorMonotonic`, `isParentMaterialized`, `mergeMaterialized`) are
/// static so the framing harness can test them without a real app group.
enum WorkingSetStore {
    struct State {
        var lastAnchor: String?
        var materializedContainers: Set<String>
    }

    static let fileName = "fileprovider-state.json"
    /// Version prefix of anchor/page-token payloads; mismatched versions are
    /// treated as unknown (fall back to a full enumeration) rather than
    /// guessed at.
    static let anchorVersionPrefix = "v1:"

    // MARK: - Anchor codec

    /// Encode a change-log cursor into the wire anchor. Decimal ASCII keeps it
    /// strictly ascending, lexicographically comparable, and far inside
    /// Apple's 500-byte anchor budget.
    static func encodeAnchor(_ seq: Int64) -> Data {
        Data("\(anchorVersionPrefix)\(seq)".utf8)
    }

    /// Decode an anchor into its change-log cursor. An empty anchor is
    /// genesis (0). Anything unparseable is nil — the caller answers with the
    /// `syncAnchorExpired` error so the system drops its caches and rescans
    /// (never silently re-enumerates from 0, which would double-deliver).
    static func decodeAnchor(_ data: Data) -> Int64? {
        let text = String(decoding: data, as: UTF8.self)
        if text.isEmpty {
            return 0
        }
        guard text.hasPrefix(anchorVersionPrefix) else {
            return nil
        }
        return Int64(text.dropFirst(anchorVersionPrefix.count))
    }

    /// Task 1697 review fix: the daemon speaks RAW decimal rowids ("1", "41")
    /// — but the wire anchor handed to the system must be the `v1:`-prefixed
    /// form, the only shape `decodeAnchor` accepts. Encode the daemon value
    /// at EVERY boundary where it reaches the system (finishEnumeratingChanges,
    /// currentSyncAnchor) or is persisted (recordAnchor): a raw decimal that
    /// comes back through `enumerateChanges(from:)` fails decodeAnchor →
    /// `syncAnchorExpired` → a full rescan after EVERY batch. `nil` when the
    /// daemon value is not a decimal rowid — callers must not advance the
    /// system's cursor with it.
    static func encodeDaemonAnchor(_ raw: String) -> String? {
        guard let seq = Int64(raw, radix: 10) else {
            return nil
        }
        return String(decoding: encodeAnchor(seq), as: UTF8.self)
    }

    /// Should a fresh daemon anchor replace the persisted one? Only a
    /// strictly LATER cursor advances the persisted state: an equal cursor is
    /// a no-op and an older one is daemon-side data loss (a stale anchor must
    /// never rewind the replica's view).
    static func anchorMonotonic(next: Int64, current: Int64?) -> Bool {
        guard let current else {
            return true
        }
        return next > current
    }

    // MARK: - Materialized-set filter

    /// Container identifiers that are ALWAYS treated as materialized.
    /// Task 1701: only the root container — Finder shows ONE root
    /// ("Beebeeb"); the synthetic namespace roots that used to sit beside it
    /// in this set are gone.
    static func alwaysMaterializedIdentifiers() -> Set<String> {
        [NSFileProviderItemIdentifier.rootContainer.rawValue]
    }

    /// Apple's Replicated contract (`NSFileProviderReplicatedExtension.h`):
    /// report a change when its NEW parent is materialized; for a reparented
    /// item the test is "old OR new parent is materialized". A change whose
    /// parents are both untracked is dropped — the system drops it anyway,
    /// and the container enumeration of a LATER-materialized folder covers
    /// the gap (dataless-dir traversal re-enumerates against us).
    ///
    /// Task 1701 exception: a row with NO parent on either side is a
    /// ROOT-CHILD change. In state.db `files.parent_id` is NULL only for
    /// items at the vault root (server root), so both-parents-nil means a
    /// top-level create/modify — or a top-level DELETION, which carries no
    /// item payload and whose old parent is gone with the row. The root is
    /// always materialized, so report it.
    static func changeTouchesMaterialized(oldParent: String?, newParent: String?, materialized: Set<String>?) -> Bool {
        guard let materialized, !materialized.isEmpty else {
            // Fail open: an empty/unknown set means the working set is the
            // entire dataset (Apple's documented fallback) — report everything.
            return true
        }
        if oldParent == nil && newParent == nil {
            return true
        }
        let always = alwaysMaterializedIdentifiers()
        if let newParent, always.contains(newParent) || materialized.contains(newParent) {
            return true
        }
        if let oldParent, always.contains(oldParent) || materialized.contains(oldParent) {
            return true
        }
        return false
    }

    // MARK: - Persistence

    private static func groupContainerURL() -> URL? {
        // Mirrors XPCBridge.resolveGroupContainer; the literal must stay in
        // sync with Rust's MACOS_APP_GROUP_ID (ipc_socket.rs).
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: "R8352WDJJR.io.beebeeb.app.fileprovider")
    }

    static func stateURL() -> URL? {
        groupContainerURL()?.appendingPathComponent(fileName, isDirectory: false)
    }

    // Injectable-path forms below are what the harness tests exercise; the
    // no-argument forms resolve the real App Group container.

    static func load(from url: URL) -> State {
        guard let data = try? Data(contentsOf: url),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return State(lastAnchor: nil, materializedContainers: [])
        }
        let containers = (object["materialized_containers"] as? [String] ?? []).asSet()
        let anchor = object["last_anchor"] as? String
        return State(lastAnchor: anchor, materializedContainers: containers)
    }

    /// Atomic write (`.atomic` = write-temp + rename inside Foundation) so a
    /// crash mid-write can never leave a torn state behind. Best-effort:
    /// returns false on failure, callers degrade to in-memory state.
    @discardableResult
    static func save(_ state: State, to url: URL) -> Bool {
        var object: [String: Any] = ["version": 1]
        if let anchor = state.lastAnchor {
            object["last_anchor"] = anchor
        }
        object["materialized_containers"] = state.materializedContainers.sorted()
        guard let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) else {
            return false
        }
        do {
            try data.write(to: url, options: .atomic)
        } catch {
            return false
        }
        // Owner-only, best-effort: the file sits in the shared group
        // container next to the IPC socket and the hydrate cache.
        try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
        return true
    }

    static func loadState() -> State {
        guard let url = stateURL() else {
            return State(lastAnchor: nil, materializedContainers: [])
        }
        return load(from: url)
    }

    @discardableResult
    static func persist(_ state: State) -> Bool {
        guard let url = stateURL() else {
            return false
        }
        return save(state, to: url)
    }

    /// Record an anchor delivered to the system (or read back from the
    /// daemon), guarded by the monotonic rule. Injectable-path form first so
    /// tests never touch the real App Group container; the no-path wrapper
    /// resolves the real container.
    @discardableResult
    static func recordAnchor(_ anchor: String, to url: URL) -> Bool {
        var state = load(from: url)
        guard let seq = decodeAnchor(Data(anchor.utf8)) else {
            return false
        }
        let current = state.lastAnchor.flatMap { decodeAnchor(Data($0.utf8)) }
        guard anchorMonotonic(next: seq, current: current) else {
            return false
        }
        state.lastAnchor = anchor
        return save(state, to: url)
    }

    @discardableResult
    static func recordAnchor(_ anchor: String) -> Bool {
        guard let url = stateURL() else {
            return false
        }
        return recordAnchor(anchor, to: url)
    }
}

private extension Array where Element == String {
    func asSet() -> Set<String> {
        Set(self)
    }
}
