// spike(2176): the policy of the Beebeeb swap service, kept free of XPC so it can be unit-tested
// (tests/SwapCoreTests.swift runs it against throwaway bundles, no signing needed).
//
// THROWAWAY device-proof code on branch spike/2176-xpc-swap-proof; never merged. It is written to
// read as the first draft of the real service: docs/specs/2026-10-25-macos-sandboxed-updater.md §4.2,
// plus Guus's two rules of 2026-11-01:
//   rule 1: never install an older (or equal) version;
//   rule 2: take only a staged path inside the app's own container, open it with O_NOFOLLOW, refuse
//           any symlink anywhere in the staged bundle, stage into a 0700 sibling directory next to the
//           destination, and delete that directory on any refusal.
//
// The service trusts nothing the (sandboxed, WebView-hosting) app says except the two paths, and
// checks both. It trusts only the Developer ID signature of ITS OWN COPY of the staged bundle.
// It never elevates: a user who cannot write the destination gets `not_writable`.

import Darwin
import Foundation
import Security

/// Result codes. A closed set: the app maps every refusal to "Download update".
enum SwapCode: String {
    case ok
    case notWritable = "not_writable"
    case invalidSignature = "invalid_signature"
    case notNewer = "not_newer"
    case symlinkRefused = "symlink_refused"
    case pathOutsideContainer = "path_outside_container"
    case ioError = "io_error"
    /// Lane addition (spike, 2026-11-01): the destination is not the app bundle that contains this
    /// service. Without it the app could ask the service to swap any user-writable directory.
    case destinationRefused = "destination_refused"
}

/// Where an outcome was decided. Closed vocabulary for diagnostics; never a path or a file name.
enum SwapStage: String {
    case paths
    case stagedOpen = "staged_open"
    case destination
    case writable
    case sibling
    case copy
    case signature
    case version
    case swap
    case cleanup
    case done
}

/// `err` is an errno, except at stage `signature`, where it is the Security framework OSStatus.
struct SwapOutcome: Error, Equatable {
    let code: SwapCode
    let stage: SwapStage
    let err: Int32

    init(_ code: SwapCode, _ stage: SwapStage, _ err: Int32 = 0) {
        self.code = code
        self.stage = stage
        self.err = err
    }
}

enum BeebeebIdentity {
    static let teamID = "R8352WDJJR"
    static let appIdentifier = "io.beebeeb.app"
    static let serviceIdentifier = "io.beebeeb.app.SwapProbe"

    /// The Developer ID code signing requirement, written from Apple TN3127 "Inside Code Signing:
    /// Requirements" (the Developer ID designated-requirement form):
    ///   anchor apple generic                              chains to an Apple root
    ///   certificate 1[field.1.2.840.113635.100.6.2.6]     the intermediate is the Developer ID CA
    ///   certificate leaf[field.1.2.840.113635.100.6.1.13] the leaf is a Developer ID Application cert
    ///   certificate leaf[subject.OU] = <Team ID>          issued to Initlabs/Devidee's team
    /// plus `identifier`, the code signing identifier (Tauri signs the app as its bundle id).
    /// Read on this Mac (2026-11-01): the installed release Beebeeb 0.8.12 reports exactly this
    /// form as its designated requirement (`codesign -dr - /Applications/Beebeeb.app`).
    static func developerIDRequirement(identifier: String) -> String {
        "anchor apple generic"
            + " and identifier \"\(identifier)\""
            + " and certificate 1[field.1.2.840.113635.100.6.2.6] exists"
            + " and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
            + " and certificate leaf[subject.OU] = \"\(teamID)\""
    }
}

struct SwapPolicy {
    /// `<home>/Library/Containers/io.beebeeb.app/Data`, as components (no leading "/").
    let containerRoot: [String]
    /// The only bundle the service may replace: the app that contains it.
    let destination: [String]
    /// Requirement the staged COPY must satisfy.
    let requirement: String
    /// CFBundleIdentifier both the copy and the installed bundle must carry.
    let bundleIdentifier: String
    /// Copy limits, so a staged tree cannot fill the disk or recurse forever.
    var maxEntries = 50_000
    var maxBytes: Int64 = 4 << 30
    var maxDepth = 48

    /// The policy of the shipped service: the current user's own container, and the app bundle
    /// three levels above this service (`X.app/Contents/XPCServices/BeebeebSwapProbe.xpc`).
    static func production(serviceBundle: URL = Bundle.main.bundleURL) -> SwapPolicy? {
        guard let pw = getpwuid(getuid()), let homeDir = pw.pointee.pw_dir,
              let home = PathCheck.components(String(cString: homeDir))
        else { return nil }
        let xpcServices = serviceBundle.deletingLastPathComponent()
        let contents = xpcServices.deletingLastPathComponent()
        let app = contents.deletingLastPathComponent()
        guard xpcServices.lastPathComponent == "XPCServices", contents.lastPathComponent == "Contents",
              app.pathExtension == "app", let destination = PathCheck.components(app.path)
        else { return nil }
        return SwapPolicy(
            containerRoot: home + ["Library", "Containers", BeebeebIdentity.appIdentifier, "Data"],
            destination: destination,
            requirement: BeebeebIdentity.developerIDRequirement(identifier: BeebeebIdentity.appIdentifier),
            bundleIdentifier: BeebeebIdentity.appIdentifier
        )
    }
}

enum PathCheck {
    /// The components of an absolute path, or nil. Refused rather than resolved: a relative path,
    /// an empty component ("//" or a trailing "/"), "." and "..", a NUL, and anything over PATH_MAX.
    static func components(_ path: String) -> [String]? {
        guard path.hasPrefix("/"), !path.utf8.contains(0), path.utf8.count < Int(PATH_MAX) else { return nil }
        var out: [String] = []
        for part in path.split(separator: "/", omittingEmptySubsequences: false).dropFirst() {
            guard !part.isEmpty, part != ".", part != ".." else { return nil }
            out.append(String(part))
        }
        return out.isEmpty ? nil : out
    }

    static func join(_ components: [String]) -> String {
        "/" + components.joined(separator: "/")
    }
}

enum SwapCore {
    static let siblingPrefix = ".beebeeb-update-"

    // MARK: - The swap

    /// Validate, copy, check and swap. Every refusal before the copy leaves the disk untouched;
    /// every refusal after it removes the sibling directory (rule 2). On `ok` the previous bundle
    /// is left in the sibling directory for a rollback; `removeStaleUpdates` deletes it later.
    static func performSwap(stagedPath: String, destinationPath: String, policy: SwapPolicy) -> SwapOutcome {
        // 1. Both paths absolute and normalised; no traversal is resolved, it is refused.
        guard let staged = PathCheck.components(stagedPath) else {
            return SwapOutcome(.pathOutsideContainer, .paths, EINVAL)
        }
        guard let dest = PathCheck.components(destinationPath), dest.count >= 2 else {
            return SwapOutcome(.destinationRefused, .paths, EINVAL)
        }
        // 2. Rule 2: the staged bundle lies strictly inside the app's own container.
        let rootCount = policy.containerRoot.count
        guard staged.count > rootCount, Array(staged.prefix(rootCount)) == policy.containerRoot else {
            return SwapOutcome(.pathOutsideContainer, .paths)
        }
        // 3. The destination is this service's own app, and nothing else.
        guard dest == policy.destination else {
            return SwapOutcome(.destinationRefused, .destination)
        }

        // 4. Rule 2: open the staged bundle with no symlink anywhere below the container root.
        let stagedFD: Int32
        switch openDirectoryChain(root: policy.containerRoot, rest: Array(staged.dropFirst(rootCount))) {
        case .failure(let refusal): return refusal
        case .success(let fd): stagedFD = fd
        }
        defer { close(stagedFD) }

        // 5. The destination: a real directory (never a symlink) holding a Beebeeb bundle.
        var destStat = stat()
        guard lstat(destinationPath, &destStat) == 0 else { return SwapOutcome(.ioError, .destination, errno) }
        if destStat.st_mode & S_IFMT == S_IFLNK { return SwapOutcome(.symlinkRefused, .destination) }
        guard destStat.st_mode & S_IFMT == S_IFDIR else { return SwapOutcome(.ioError, .destination, ENOTDIR) }
        guard let installed = bundleInfo(destinationPath), installed.identifier == policy.bundleIdentifier else {
            return SwapOutcome(.destinationRefused, .destination)
        }

        // 6. No elevation, ever: the user must be able to write the destination and its parent
        //    (a rename of a directory to another parent rewrites its "..", so both need write).
        let name = dest[dest.count - 1]
        let parentPath = PathCheck.join(Array(dest.dropLast()))
        guard access(parentPath, W_OK) == 0 else { return SwapOutcome(.notWritable, .writable, errno) }
        guard access(destinationPath, W_OK) == 0 else { return SwapOutcome(.notWritable, .writable, errno) }

        // 7. Rule 2: a private sibling directory on the destination's volume. mkdtemp makes it 0700.
        var template = Array((parentPath + "/" + siblingPrefix + "XXXXXX").utf8CString)
        guard mkdtemp(&template) != nil else {
            let e = errno
            return SwapOutcome(isPermission(e) ? .notWritable : .ioError, .sibling, e)
        }
        let siblingPath = String(cString: template)
        // From here on, every refusal removes the sibling directory (rule 2).
        func refuse(_ outcome: SwapOutcome) -> SwapOutcome {
            _ = removeTree(siblingPath)
            return outcome
        }
        let siblingFD = open(siblingPath, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
        guard siblingFD >= 0 else { return refuse(SwapOutcome(.ioError, .sibling, errno)) }
        defer { close(siblingFD) }
        var siblingStat = stat()
        guard fstat(siblingFD, &siblingStat) == 0, siblingStat.st_mode & 0o777 == 0o700,
              siblingStat.st_uid == getuid()
        else { return refuse(SwapOutcome(.ioError, .sibling, EPERM)) }

        // 8. Copy the staged bundle into the sibling directory, refusing any symlink.
        guard mkdirat(siblingFD, name, 0o755) == 0 else { return refuse(SwapOutcome(.ioError, .copy, errno)) }
        let copyFD = openat(siblingFD, name, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
        guard copyFD >= 0 else { return refuse(SwapOutcome(.ioError, .copy, errno)) }
        var budget = CopyBudget()
        let copyRefusal = copyTree(from: stagedFD, to: copyFD, policy: policy, budget: &budget, depth: 0)
        close(copyFD)
        if let copyRefusal { return refuse(copyRefusal) }
        let copyPath = siblingPath + "/" + name

        // 9. Validate the COPY (never the app-writable original): Developer ID, team, identifier.
        if let signatureRefusal = checkSignature(bundlePath: copyPath, requirement: policy.requirement) {
            return refuse(signatureRefusal)
        }
        guard let incoming = bundleInfo(copyPath), incoming.identifier == policy.bundleIdentifier else {
            return refuse(SwapOutcome(.invalidSignature, .version))
        }

        // 10. Rule 1: strictly newer, read from the signed Info.plist of the copy.
        guard isStrictlyNewer(incoming.version, than: installed.version) else {
            return refuse(SwapOutcome(.notNewer, .version))
        }

        // 11. The atomic swap on one volume. The previous bundle ends up in the sibling directory.
        let parentFD = open(parentPath, O_RDONLY | O_DIRECTORY | O_CLOEXEC)
        guard parentFD >= 0 else { return refuse(SwapOutcome(.ioError, .swap, errno)) }
        defer { close(parentFD) }
        guard renameatx_np(siblingFD, name, parentFD, name, UInt32(RENAME_SWAP)) == 0 else {
            let e = errno
            return refuse(SwapOutcome(isPermission(e) ? .notWritable : .ioError, .swap, e))
        }
        // As the Tauri updater does after its move: bump the bundle's mtime so LaunchServices
        // notices the new bundle. Best effort; the swap already happened.
        _ = utimensat(parentFD, name, nil, AT_SYMLINK_NOFOLLOW)
        return SwapOutcome(.ok, .done)
    }

    /// Remove sibling directories a previous swap left (the old bundle kept for a rollback). Only
    /// directories this user owns, mode 0700, named exactly like mkdtemp names them, next to the
    /// destination. Returns how many were removed in `err`.
    static func removeStaleUpdates(destinationPath: String, policy: SwapPolicy) -> SwapOutcome {
        guard let dest = PathCheck.components(destinationPath), dest == policy.destination else {
            return SwapOutcome(.destinationRefused, .destination)
        }
        let parentPath = PathCheck.join(Array(dest.dropLast()))
        guard let names = try? FileManager.default.contentsOfDirectory(atPath: parentPath) else {
            return SwapOutcome(.ioError, .cleanup, errno)
        }
        var removed: Int32 = 0
        for entry in names where entry.hasPrefix(siblingPrefix) && entry.count == siblingPrefix.count + 6 {
            let path = parentPath + "/" + entry
            var st = stat()
            guard lstat(path, &st) == 0, st.st_mode & S_IFMT == S_IFDIR, st.st_uid == getuid(),
                  st.st_mode & 0o777 == 0o700
            else { continue }
            if removeTree(path) { removed += 1 }
        }
        return SwapOutcome(.ok, .cleanup, removed)
    }

    // MARK: - Pieces (internal so the tests can reach them)

    static func isPermission(_ e: Int32) -> Bool {
        e == EACCES || e == EPERM || e == EROFS
    }

    /// Opens `root` with O_NOFOLLOW_ANY (no symlink anywhere in its absolute path), then each
    /// component of `rest` with O_NOFOLLOW. A symlink at any step is `symlink_refused`.
    static func openDirectoryChain(root: [String], rest: [String]) -> Result<Int32, SwapOutcome> {
        var fd = open(PathCheck.join(root), O_RDONLY | O_DIRECTORY | O_NOFOLLOW_ANY | O_CLOEXEC)
        guard fd >= 0 else {
            let e = errno
            return .failure(SwapOutcome(e == ELOOP ? .symlinkRefused : .ioError, .stagedOpen, e))
        }
        for component in rest {
            let next = openat(fd, component, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
            if next < 0 {
                let refusal = openRefusal(errno, parent: fd, name: component, stage: .stagedOpen)
                close(fd)
                return .failure(refusal)
            }
            close(fd)
            fd = next
        }
        return .success(fd)
    }

    /// Why an `openat(..., O_NOFOLLOW)` failed: a symlink in the way, or something else.
    static func openRefusal(_ e: Int32, parent: Int32, name: String, stage: SwapStage) -> SwapOutcome {
        var st = stat()
        if e == ELOOP || (fstatat(parent, name, &st, AT_SYMLINK_NOFOLLOW) == 0 && st.st_mode & S_IFMT == S_IFLNK) {
            return SwapOutcome(.symlinkRefused, stage, e)
        }
        return SwapOutcome(.ioError, stage, e)
    }

    struct CopyBudget {
        var entries = 0
        var bytes: Int64 = 0
    }

    /// Copies the directory `src` into the (empty) directory `dst`. Regular files and directories
    /// only: a symlink anywhere is `symlink_refused`; a fifo, socket or device is `io_error`.
    /// File data only (no extended attributes, no ownership); permission bits masked to 0755.
    static func copyTree(from src: Int32, to dst: Int32, policy: SwapPolicy, budget: inout CopyBudget, depth: Int) -> SwapOutcome? {
        guard depth <= policy.maxDepth else { return SwapOutcome(.ioError, .copy, E2BIG) }
        let names: [[CChar]]
        switch listDirectory(src) {
        case .failure(let refusal): return refusal
        case .success(let list): names = list
        }
        for name in names {
            budget.entries += 1
            guard budget.entries <= policy.maxEntries else { return SwapOutcome(.ioError, .copy, EFBIG) }
            var st = stat()
            guard fstatat(src, name, &st, AT_SYMLINK_NOFOLLOW) == 0 else { return SwapOutcome(.ioError, .copy, errno) }
            switch st.st_mode & S_IFMT {
            case S_IFLNK:
                return SwapOutcome(.symlinkRefused, .copy)
            case S_IFDIR:
                let sub = openat(src, name, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
                guard sub >= 0 else { return openRefusal(errno, parent: src, name: String(cString: name), stage: .copy) }
                defer { close(sub) }
                guard mkdirat(dst, name, 0o755) == 0 else { return SwapOutcome(.ioError, .copy, errno) }
                let dsub = openat(dst, name, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
                guard dsub >= 0 else { return SwapOutcome(.ioError, .copy, errno) }
                defer { close(dsub) }
                if let refusal = copyTree(from: sub, to: dsub, policy: policy, budget: &budget, depth: depth + 1) {
                    return refusal
                }
            case S_IFREG:
                let input = openat(src, name, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
                guard input >= 0 else { return openRefusal(errno, parent: src, name: String(cString: name), stage: .copy) }
                defer { close(input) }
                // The entry may have been swapped between the lstat and the open: compare inodes.
                var opened = stat()
                guard fstat(input, &opened) == 0, opened.st_mode & S_IFMT == S_IFREG,
                      opened.st_ino == st.st_ino, opened.st_dev == st.st_dev
                else { return SwapOutcome(.ioError, .copy, EAGAIN) }
                budget.bytes += Int64(opened.st_size)
                guard budget.bytes <= policy.maxBytes else { return SwapOutcome(.ioError, .copy, EFBIG) }
                let mode = mode_t(opened.st_mode & 0o755)
                let output = openat(dst, name, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, mode)
                guard output >= 0 else { return SwapOutcome(.ioError, .copy, errno) }
                defer { close(output) }
                guard fcopyfile(input, output, nil, copyfile_flags_t(COPYFILE_DATA)) == 0 else {
                    return SwapOutcome(.ioError, .copy, errno)
                }
                guard fchmod(output, mode) == 0 else { return SwapOutcome(.ioError, .copy, errno) }
            default:
                return SwapOutcome(.ioError, .copy, EFTYPE)
            }
        }
        return nil
    }

    /// The entry names of an open directory ("." and ".." left out), as NUL-terminated C strings.
    static func listDirectory(_ fd: Int32) -> Result<[[CChar]], SwapOutcome> {
        let copy = dup(fd)
        guard copy >= 0 else { return .failure(SwapOutcome(.ioError, .copy, errno)) }
        guard let dir = fdopendir(copy) else {
            let e = errno
            close(copy)
            return .failure(SwapOutcome(.ioError, .copy, e))
        }
        defer { closedir(dir) }
        var names: [[CChar]] = []
        while true {
            errno = 0
            guard let entry = readdir(dir) else {
                if errno != 0 { return .failure(SwapOutcome(.ioError, .copy, errno)) }
                break
            }
            let length = Int(entry.pointee.d_namlen)
            var name: [CChar] = withUnsafeBytes(of: entry.pointee.d_name) { raw in
                raw.prefix(length).map { CChar(bitPattern: $0) }
            }
            if name == [46] || name == [46, 46] { continue }
            name.append(0)
            names.append(name)
        }
        return .success(names)
    }

    /// Security.framework validation of a bundle on disk against `requirement`. Nil when it passes.
    /// Flags: every architecture, strict validation, nested code (the appex, the helper, this
    /// service), no symlink escaping the bundle, no signature stored in extended attributes, app-like
    /// bundles only. Revocation is not enforced here (it needs the network).
    static func checkSignature(bundlePath: String, requirement text: String) -> SwapOutcome? {
        var requirement: SecRequirement?
        var status = SecRequirementCreateWithString(text as CFString, [], &requirement)
        guard status == errSecSuccess, let requirement else { return SwapOutcome(.invalidSignature, .signature, status) }
        var code: SecStaticCode?
        status = SecStaticCodeCreateWithPath(URL(fileURLWithPath: bundlePath, isDirectory: true) as CFURL, [], &code)
        guard status == errSecSuccess, let code else { return SwapOutcome(.invalidSignature, .signature, status) }
        let flags = SecCSFlags(rawValue: UInt32(
            kSecCSCheckAllArchitectures | kSecCSStrictValidate | kSecCSCheckNestedCode
                | kSecCSRestrictSymlinks | kSecCSRestrictSidebandData | kSecCSRestrictToAppLike))
        status = SecStaticCodeCheckValidityWithErrors(code, flags, requirement, nil)
        return status == errSecSuccess ? nil : SwapOutcome(.invalidSignature, .signature, status)
    }

    struct BundleInfo {
        let identifier: String
        let version: [Int]
    }

    static func bundleInfo(_ bundlePath: String) -> BundleInfo? {
        guard let plist = NSDictionary(contentsOfFile: bundlePath + "/Contents/Info.plist"),
              let identifier = plist["CFBundleIdentifier"] as? String,
              let raw = plist["CFBundleShortVersionString"] as? String,
              let version = parseVersion(raw)
        else { return nil }
        return BundleInfo(identifier: identifier, version: version)
    }

    /// "1", "1.2", "1.2.3" or "1.2.3.4", digits only, each part at most 9 digits. Anything else is
    /// nil, and a nil version never counts as newer.
    static func parseVersion(_ text: String) -> [Int]? {
        let parts = text.split(separator: ".", omittingEmptySubsequences: false)
        guard (1...4).contains(parts.count) else { return nil }
        var out: [Int] = []
        for part in parts {
            guard !part.isEmpty, part.count <= 9, part.allSatisfy({ $0 >= "0" && $0 <= "9" }), let n = Int(part) else {
                return nil
            }
            out.append(n)
        }
        return out
    }

    /// Numeric, part by part; missing parts count as 0, so "1.0" equals "1.0.0" (not newer).
    static func isStrictlyNewer(_ candidate: [Int], than installed: [Int]) -> Bool {
        for i in 0..<max(candidate.count, installed.count) {
            let a = i < candidate.count ? candidate[i] : 0
            let b = i < installed.count ? installed[i] : 0
            if a != b { return a > b }
        }
        return false
    }

    /// Removes a tree without following symlinks (FileManager removes a link, not its target).
    @discardableResult
    static func removeTree(_ path: String) -> Bool {
        (try? FileManager.default.removeItem(atPath: path)) != nil
    }
}
