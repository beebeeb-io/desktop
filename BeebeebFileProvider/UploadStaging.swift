import Foundation

/// Hands a write's contents to the app through the shared App Group container.
///
/// The system gives `createItem` / `modifyItem` a contents URL that only this
/// extension's sandbox can read. The app, which encrypts and uploads, runs in
/// a different sandbox and cannot open it, so a file created in the Finder
/// location never uploaded. The extension therefore copies the contents into
/// `upload-staging/` in the App Group container first and sends the app that
/// copy's path.
///
/// The app accepts contents from this directory only, takes its own copy, and
/// deletes this one when it answers (`open_staged_contents` and
/// `StagedContents` in `src-tauri/src/ipc_socket.rs`). The extension deletes it
/// after the reply as well, and the app purges anything a crash left behind
/// after an hour (`UPLOAD_STAGING_MAX_AGE`).
///
/// Pure Foundation, so `scripts/test-ipc-framing.sh` compiles and tests it.
enum UploadStaging {
    /// Mirrors `MACOS_UPLOAD_STAGING_DIRNAME` in `src-tauri/src/ipc_socket.rs`;
    /// keep both in sync.
    static let directoryName = "upload-staging"

    /// `<container>/upload-staging/`, created if needed, forced owner-only
    /// (0700) and excluded from backups: it only ever holds plaintext copies of
    /// files being saved.
    static func directory(in groupContainer: URL) throws -> URL {
        let dir = groupContainer.appendingPathComponent(directoryName, isDirectory: true)
        try FileManager.default.createDirectory(
            at: dir,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        // `createDirectory` leaves an existing directory's mode alone.
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: dir.path)
        // Best-effort: the app sets the same exclusion when it prepares the
        // directory at startup.
        var excluded = dir
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? excluded.setResourceValues(values)
        return dir
    }

    /// A fresh file URL in `directory`: a random UUID, never derived from the
    /// user's file name, so no two requests share a copy and the name reveals
    /// nothing about the file.
    static func newFileURL(in directory: URL) -> URL {
        directory.appendingPathComponent(UUID().uuidString.lowercased(), isDirectory: false)
    }

    /// Copy `source` into the staging directory and return the copy. The
    /// system's file is never moved or changed.
    ///
    /// `copyItem` clones on APFS, so a large file costs neither time nor space
    /// until one side changes. The copy is then made owner-only and its
    /// modification time set to now: the app's purge measures age from that
    /// time, and a copy that kept the source's (possibly years-old) mtime would
    /// look orphaned at once.
    ///
    /// Only a regular file is copied. A package (a folder that macOS shows as
    /// one file, such as an `.rtfd` document) reaches the extension as a
    /// directory, and a symlink is not contents either. Neither is copied:
    /// the app accepts regular files only, and a copied tree with mode 0600
    /// could not be deleted by either side. They are refused with a
    /// DEFINITIVE `daemonRejected` (`cannotSynchronize`), the same answer the
    /// app gives for such an item, because retrying cannot make it a file.
    ///
    /// Every other failure is `uploadStagingFailed`, a TRANSIENT error, so the
    /// system retries the write instead of giving up on the file, and nothing
    /// is left behind.
    static func stage(contentsOf source: URL, in groupContainer: URL) throws -> URL {
        // `attributesOfItem` describes a symlink itself, never its target.
        let attributes: [FileAttributeKey: Any]
        do {
            attributes = try FileManager.default.attributesOfItem(atPath: source.path)
        } catch {
            throw BeebeebIPCError.uploadStagingFailed(reason(error))
        }
        guard (attributes[.type] as? FileAttributeType) == .typeRegular else {
            throw BeebeebIPCError.daemonRejected(notARegularFileMessage)
        }
        let destination: URL
        do {
            destination = newFileURL(in: try directory(in: groupContainer))
        } catch {
            throw BeebeebIPCError.uploadStagingFailed(reason(error))
        }
        do {
            try FileManager.default.copyItem(at: source, to: destination)
            try FileManager.default.setAttributes(
                [.posixPermissions: 0o600, .modificationDate: Date()],
                ofItemAtPath: destination.path
            )
        } catch {
            // `copyItem` can create the destination and then fail partway.
            try? FileManager.default.removeItem(at: destination)
            throw BeebeebIPCError.uploadStagingFailed(reason(error))
        }
        return destination
    }

    /// What the person sees for a package or another non-regular item. It
    /// names no file.
    static let notARegularFileMessage =
        "Beebeeb can only upload regular files, and this item is a package or another special item. It was not uploaded."

    /// Delete a copy once the app has answered, or could not be reached. The
    /// app deletes it too; whichever runs second finds nothing.
    static func discard(_ copy: URL) {
        try? FileManager.default.removeItem(at: copy)
    }

    /// The error's domain and code only: the localized description of a copy
    /// failure names the user's file, and this text reaches the unified log.
    static func reason(_ error: Error) -> String {
        let ns = error as NSError
        return "\(ns.domain) \(ns.code)"
    }
}
