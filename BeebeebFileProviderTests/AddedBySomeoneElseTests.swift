// Task 2275 L15: the daemon's `added_by_someone_else` reaches the Finder item.
//
// Owns: decoding the field and the single badge slot it competes for.
// Must not: change the sync-state badge priority.
// Tests: called from main.swift's runner.

func runAddedBySomeoneElseChecks() {
    check("2275-L15: decodeItem reads `added_by_someone_else`, and an F item badges it") {
        let bridge = XPCBridge()
        let decoder = { (dict: [String: Any]) -> BeebeebProviderItem? in
            try? bridge.test_decodeItemsPayload(["FileProviderItems": ["items": [dict]]]).first
        }
        let base: [String: Any] = [
            "identifier": "f-added",
            "parent_identifier": "NSFileProviderRootContainerItemIdentifier",
            "filename": "someone-elses.txt",
            "kind": "file",
            "status": "cloud_only",
        ]
        let absent = decoder(base)
        try expect(absent?.addedBySomeoneElse == false, "absent added_by_someone_else defaults to false")
        let marked = decoder(base.merging(["added_by_someone_else": true]) { _, new in new })
        try expect(marked?.addedBySomeoneElse == true, "added_by_someone_else=true decodes")
        let item = FileProviderItem(model: marked!)
        try expect(
            item.decorations?.map(\.rawValue).contains(BeebeebProviderItem.decorationAddedBySomeoneElse) == true,
            "an F item the user did not add carries the Added-by-someone-else badge, got \(String(describing: item.decorations))"
        )
        // The badge slot is one: a sync-state badge wins over the L15 marker.
        let erroring = decoder(base.merging(["status": "error", "added_by_someone_else": true]) { _, new in new })
        let errorItem = FileProviderItem(model: erroring!)
        try expect(
            errorItem.decorations?.map(\.rawValue) == [BeebeebProviderItem.decorationError],
            "a sync-state badge keeps the single slot, got \(String(describing: errorItem.decorations))"
        )
    }
}
