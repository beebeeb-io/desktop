import FileProvider
import Foundation

// LD-0113 D2 (spec §6.3): the daemon's protection reaches the Finder item.
//
// Owns: decoding the protection words, the capabilities a protected item loses, and the extension's
// second-line refusal with the daemon's words. Must not: decide what is protected (the daemon does,
// from core's rule). Tests: called from FeatureChecks.swift's runner.

private let deleteWords = "Backups is a system folder and can't be deleted."
private let renameWords = "Backups is a system folder and can't be renamed."
private let moveWords = "Backups is a system folder and can't be moved."
private let writeWords = "Backups are read-only: each device writes its own. Change the file on that device, or save a copy outside Backups."

private func decoded(_ extra: [String: Any]) -> BeebeebProviderItem? {
    let bridge = XPCBridge()
    let base: [String: Any] = [
        "identifier": "ld0113-root",
        "parent_identifier": "NSFileProviderRootContainerItemIdentifier",
        "filename": "Backups",
        "kind": "folder",
        "status": "local",
        // Every bit the daemon can send, so the test sees what protection takes away.
        "capabilities": 0x7F,
    ]
    return bridge.test_decodeItemsPayload(["FileProviderItems": ["items": [base.merging(extra) { _, new in new }]]]).first
}

func runProtectedItemChecks() {
    check("LD-0113 D2: a protected item decodes its words and loses delete, trash, rename, move and adding items") {
        let item = decoded([
            "protected": true,
            "protected_reason": deleteWords,
            "protected_rename_reason": renameWords,
            "protected_move_reason": moveWords,
        ])
        try expect(item?.isProtected == true, "the daemon's `protected` flag marks the item protected")
        try expect(decoded(["protected_reason": deleteWords])?.isProtected == false, "words alone are not the flag (one field for the fact)")
        try expect(item?.protectedRenameReason == renameWords, "the rename words decode")
        let capabilities = FileProviderItem(model: item!).capabilities
        for taken: NSFileProviderItemCapabilities in [.allowsDeleting, .allowsTrashing, .allowsRenaming, .allowsReparenting] {
            try expect(!capabilities.contains(taken), "a protected item never offers \(taken), got \(capabilities)")
        }
        try expect(capabilities.contains(.allowsReading), "it stays readable")
        // Amendment 6: no New Folder in the tree, and Finder refuses a drop into it.
        try expect(!capabilities.contains(.allowsAddingSubItems), "nothing is added to a protected folder, got \(capabilities)")
    }

    check("LD-0113 D2: an item without the words (an older daemon, an ordinary item) keeps every capability") {
        let item = decoded([:])
        try expect(item?.isProtected == false, "absent words: not protected")
        let capabilities = FileProviderItem(model: item!).capabilities
        try expect(capabilities.contains(.allowsDeleting) && capabilities.contains(.allowsRenaming), "unchanged, got \(capabilities)")
        try expect(capabilities.contains(.allowsAddingSubItems), "an ordinary folder takes a drop, got \(capabilities)")
    }

    check("LD-0113 D2: the extension refuses a protected item's delete, trash, rename and move with the daemon's words") {
        let item = decoded(["protected": true, "protected_reason": deleteWords, "protected_rename_reason": renameWords, "protected_move_reason": moveWords])!
        let delete = BeebeebProviderItem.protectedRefusal(model: { item })
        try expect(delete?.domain == NSFileProviderErrorDomain && delete?.code == NSFileProviderError.deletionRejected.rawValue, "a delete is rejected")
        try expect(delete?.localizedDescription == deleteWords, "with the delete words, got \(String(describing: delete))")
        let rename = BeebeebProviderItem.protectedRefusal(changedFields: .filename, newParent: nil, model: { item })
        try expect(rename?.localizedDescription == renameWords, "a rename has the rename words")
        let trash = BeebeebProviderItem.protectedRefusal(changedFields: .parentItemIdentifier, newParent: .trashContainer, model: { item })
        try expect(trash?.localizedDescription == deleteWords, "a move into the Trash is a delete")
        let move = BeebeebProviderItem.protectedRefusal(changedFields: .parentItemIdentifier, newParent: .rootContainer, model: { item })
        try expect(move?.localizedDescription == moveWords, "a move has the move words")
        // Amendment 7: a content edit is refused too (Backups are read-only); other metadata alone is not.
        let edit = BeebeebProviderItem.protectedRefusal(changedFields: .contents, newParent: nil, model: { item })
        try expect(edit?.code == NSFileWriteNoPermissionError, "a content edit is refused, got \(String(describing: edit))")
        var asked = false
        let tagged = BeebeebProviderItem.protectedRefusal(changedFields: .tagData, newParent: nil, model: { asked = true; return item })
        try expect(tagged == nil && !asked, "a tag change is never intercepted, and asks the daemon nothing")
        let ordinary = decoded([:])!
        try expect(BeebeebProviderItem.protectedRefusal(model: { ordinary }) == nil, "an ordinary delete goes ahead")
        let wordsOnly = decoded(["protected_reason": deleteWords])!
        try expect(BeebeebProviderItem.protectedRefusal(model: { wordsOnly }) == nil, "words without the flag refuse nothing (one fact)")
    }

    check("LD-0113 Amendments 7 and 8a: a protected item is read-only with the daemon's words; the root carries the Read-only badge") {
        let root = decoded(["protected": true, "protected_reason": deleteWords, "protected_write_reason": writeWords, "read_only_badge": true])!
        try expect(root.protectedWriteReason == writeWords, "the write words decode")
        try expect(!FileProviderItem(model: root).capabilities.contains(.allowsWriting), "a protected item is never written")
        let edit = BeebeebProviderItem.protectedRefusal(changedFields: .contents, newParent: nil, model: { root })
        try expect(edit?.localizedDescription == writeWords, "a content edit has the write words, got \(String(describing: edit))")
        let badge = FileProviderItem(model: root).decorations?.map(\.rawValue)
        try expect(badge == [BeebeebProviderItem.decorationReadOnly], "the root's badge, got \(String(describing: badge))")
        let failing = decoded(["status": "error", "read_only_badge": true])!
        let errorFirst = FileProviderItem(model: failing).decorations?.map(\.rawValue)
        try expect(errorFirst == [BeebeebProviderItem.decorationError], "a sync problem takes the one slot first")
        let ordinary = decoded([:])!
        try expect(FileProviderItem(model: ordinary).decorations == nil, "an ordinary item has no badge")
        try expect(FileProviderItem(model: ordinary).capabilities.contains(.allowsWriting), "and is writable")
    }
}
