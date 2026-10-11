import Foundation
import os

/// The extension's one logger (spec 2026-10-09 §11). Every line the extension writes goes to the
/// unified log through `logger`, under the extension's bundle id, so
/// `log show --predicate 'subsystem == "io.beebeeb.app.FileProvider"'` lists them. `NSLog` is not
/// used: on macOS 26 the unified log shows every `NSLog` line of the extension as `<private>`.
///
/// Each line is one function below, and only closed values are public: a fixed word, a bitmask,
/// an id that parses as a UUID (rule M5), a hex prefix, the App Group id, and an error's `NSError`
/// domain and code. A value that can carry a path, a file name or an error's text is never
/// logged; an error is logged by its domain and code, never by its message or description.
/// Informational lines are `.notice` and failures `.error`, so a plain `log show` lists both.
/// `extension_log_pin` in `src-tauri/src` checks the source: no other log call in the extension,
/// and no public value outside the closed set.
enum ExtensionLog {
    /// The extension's bundle id: `BUNDLE_ID` in `scripts/build-fileprovider-extension.sh`, which
    /// the build writes into the Info.plist's `CFBundleIdentifier`.
    static let subsystem = "io.beebeeb.app.FileProvider"
    static let category = "extension"
    static let logger = Logger(subsystem: subsystem, category: category)

    /// The two write calls whose options and request ids are logged.
    enum WriteCall: String {
        case create
        case modify
    }

    /// The fields of the write-options line: the call, the raw bitmask, and the item's id only
    /// when it parses as a UUID; any other id is logged as `id_kind=non_uuid` (rule M5).
    struct WriteOptionsLine: Equatable {
        let call: String
        let options: UInt
        let item: String?
    }

    /// An error as closed values: its `NSError` domain and code.
    struct ErrorFields: Equatable {
        let domain: String
        let code: Int
    }

    /// Rule M5: an id taken from the system is logged only when it parses as a UUID.
    static func loggableID(_ raw: String) -> String? {
        UUID(uuidString: raw) == nil ? nil : raw
    }

    /// The first 12 characters of a write's `request_id` (a SHA-256 in lowercase hex,
    /// `IPCWriteKey`), or `nil` when they are not 12 hex digits.
    static func requestIDPrefix(_ id: String) -> String? {
        let prefix = String(id.prefix(12))
        return prefix.count == 12 && prefix.allSatisfy({ $0.isHexDigit }) ? prefix : nil
    }

    static func errorFields(_ error: Error) -> ErrorFields {
        let ns = error as NSError
        return ErrorFields(domain: ns.domain, code: ns.code)
    }

    /// Why a write's contents could not be staged, when the error carries it as closed values.
    /// `UploadStaging.reason` writes a copy failure as "<domain> <code>"; only that shape (an
    /// identifier, one space, an integer) is returned, so nothing else the error carries is logged.
    static func stagingCause(_ error: Error) -> String? {
        guard case BeebeebIPCError.uploadStagingFailed(let reason) = error else {
            return nil
        }
        let parts = reason.split(separator: " ", omittingEmptySubsequences: false)
        guard parts.count == 2, Int(parts[1]) != nil, !parts[0].isEmpty,
              parts[0].allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "." || $0 == "_") })
        else {
            return nil
        }
        return reason
    }

    static func writeOptionsLine(call: WriteCall, options: UInt, item: String) -> WriteOptionsLine {
        WriteOptionsLine(call: call.rawValue, options: options, item: loggableID(item))
    }

    // MARK: - The lines

    /// Spec §8.8: a create or modify whose options are not empty, for the record of which bits
    /// the system sends. The write itself is handled as usual.
    static func writeOptions(call: WriteCall, options: UInt, item: String) {
        let line = writeOptionsLine(call: call, options: options, item: item)
        if let id = line.item {
            logger.notice(
                "BeebeebFileProvider: write options call=\(line.call, privacy: .public) options=\(line.options, privacy: .public) item=\(id, privacy: .public)"
            )
        } else {
            logger.notice(
                "BeebeebFileProvider: write options call=\(line.call, privacy: .public) options=\(line.options, privacy: .public) id_kind=non_uuid"
            )
        }
    }

    /// Task 1684: the first 12 characters of a write's `request_id`, so the log shows whether the
    /// system's retry of a timed-out write computed the same key.
    static func requestID(call: WriteCall, id: String) {
        if let prefix = requestIDPrefix(id) {
            logger.notice(
                "BeebeebFileProvider: \(call.rawValue, privacy: .public) request_id=\(prefix, privacy: .public)"
            )
        } else {
            logger.notice("BeebeebFileProvider: \(call.rawValue, privacy: .public) request_id_kind=non_hex")
        }
    }

    static func pendingItemsDidChange() {
        logger.notice("BeebeebFileProvider: pendingItemsDidChange (the system's pending set refreshed)")
    }

    static func signalErrorResolvedFailed(_ error: Error) {
        let fields = errorFields(error)
        logger.error(
            "BeebeebFileProvider: signalErrorResolved failed domain=\(fields.domain, privacy: .public) code=\(fields.code, privacy: .public)"
        )
    }

    static func appGroupContainerUnresolved() {
        logger.error(
            "BeebeebFileProvider: could not resolve app-group container for \(XPCBridge.appGroupID, privacy: .public); IPC will not reach the daemon"
        )
    }

    static func hydrateCacheDirFailed(_ error: Error) {
        let fields = errorFields(error)
        logger.error(
            "BeebeebFileProvider: could not create hydrate-cache dir domain=\(fields.domain, privacy: .public) code=\(fields.code, privacy: .public)"
        )
    }

    static func uploadStagingFailed(_ error: Error) {
        let fields = errorFields(error)
        if let cause = stagingCause(error) {
            logger.error(
                "BeebeebFileProvider: upload staging failed domain=\(fields.domain, privacy: .public) code=\(fields.code, privacy: .public) cause=\(cause, privacy: .public)"
            )
        } else {
            logger.error(
                "BeebeebFileProvider: upload staging failed domain=\(fields.domain, privacy: .public) code=\(fields.code, privacy: .public)"
            )
        }
    }
}
