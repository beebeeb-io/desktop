import FileProvider
import Foundation

// The pure field rule of `modifyItem`, moved verbatim out of FileProviderExtension.swift (LD-0113:
// that file is at its size ratchet, file-size-baseline.txt). Compiled into the extension
// (scripts/build-fileprovider-extension.sh takes BeebeebFileProvider/*.swift) and the framing tests
// (scripts/test-ipc-framing.sh lists it).
extension FileProviderExtension {
    /// The fields a modify reports as still pending (spec 2026-10-09 §7.2): a filename or
    /// parent the person changed that the item the app returned does not describe. Apple:
    /// "If a field in the returned item does not match the itemTemplate, and is not in the
    /// list of stillPendingFields, the value from the item will be propagated to the disk"
    /// (NSFileProviderReplicatedExtension.h, modifyItem). The app applies a save's name and
    /// parent in the save's accept, so with the app this extension ships in, this is empty.
    /// An app from another build (one still running across an update) may not, and its old
    /// name would undo the rename on disk; pending, the system sends the field again instead.
    /// A modify without contents reports nothing pending, as before: its reply describes the
    /// item as it was until the server's change arrives, and a pending field would be sent
    /// again until then.
    static func modifyStillPendingFields(
        changedFields: NSFileProviderItemFields,
        hasContents: Bool,
        template: NSFileProviderItem,
        returned: NSFileProviderItem?
    ) -> NSFileProviderItemFields {
        guard hasContents, let returned else { return [] }
        var pending: NSFileProviderItemFields = []
        if changedFields.contains(.filename), returned.filename != template.filename {
            pending.insert(.filename)
        }
        if changedFields.contains(.parentItemIdentifier),
           returned.parentItemIdentifier != template.parentItemIdentifier {
            pending.insert(.parentItemIdentifier)
        }
        return pending
    }
}
