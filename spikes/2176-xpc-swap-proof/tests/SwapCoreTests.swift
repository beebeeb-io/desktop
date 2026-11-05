// spike(2176): unit harness for SwapCore (the swap service's policy), compiled with SwapCore.swift by
// tests/run-core-tests.sh. It needs no signing identity, signs nothing, installs nothing and launches
// nothing; it writes only inside its work dir. Two fixtures:
//   - TextEdit (/System/Applications/TextEdit.app, no symlinks) for the refusals decided BEFORE the
//     signature check (paths, symlinks, special files, budget, writability, destination). A copy of a
//     system app can never pass strict validation (its resource seal is empty because the sealed
//     system volume vouches for it), so it is not used for anything at or after the signature check.
//   - The installed release /Applications/Beebeeb.app (Developer ID, team R8352WDJJR, no symlinks),
//     COPIED read-only, for the signature, version and ok-path tests under the PRODUCTION requirement.
//     Without it those tests are skipped, not passed.
// Every refusal is checked for its code AND for "the destination is unchanged and no sibling
// directory is left behind" (rule 2).
// Truth line: "swapcore-tests: N passed, 0 failed, K skipped".

import Darwin
import Foundation

@main
struct SwapCoreTests {
    static var passed = 0
    static var failed = 0
    static var skipped = 0

    static func check(_ name: String, _ ok: Bool, _ detail: @autoclosure () -> String = "") {
        if ok {
            passed += 1
            print("PASS \(name)")
        } else {
            failed += 1
            print("FAIL \(name) \(detail())")
        }
    }

    static func skip(_ name: String, _ why: String) {
        skipped += 1
        print("SKIP \(name): \(why)")
    }

    static let fm = FileManager.default

    struct Fixture {
        let source: String       // the bundle copied into the container as the "staged" update
        let name: String         // destination bundle name, e.g. "TextEdit.app"
        let identifier: String
        let requirement: String
    }

    static let textEdit = Fixture(source: "/System/Applications/TextEdit.app", name: "TextEdit.app",
                                  identifier: "com.apple.TextEdit",
                                  requirement: "identifier \"com.apple.TextEdit\" and anchor apple")
    static let release = Fixture(source: "/Applications/Beebeeb.app", name: "Beebeeb.app",
                                 identifier: BeebeebIdentity.appIdentifier,
                                 requirement: BeebeebIdentity.developerIDRequirement(identifier: BeebeebIdentity.appIdentifier))

    // Layout (one realpath'd work dir, so no symlink such as /var -> /private/var on the way):
    //   home/Library/Containers/io.beebeeb.app/Data/stage/...   the "container"
    //   Applications/<name>                                     the destination
    //   outside/                                                outside the container
    static var work = ""
    static var fixture = textEdit
    static var container: String { work + "/home/Library/Containers/io.beebeeb.app/Data" }
    static var stage: String { container + "/stage" }
    static var applications: String { work + "/Applications" }
    static var dest: String { applications + "/" + fixture.name }
    static var outside: String { work + "/outside" }

    static func policy(requirement: String? = nil) -> SwapPolicy {
        SwapPolicy(containerRoot: PathCheck.components(container)!, destination: PathCheck.components(dest)!,
                   requirement: requirement ?? fixture.requirement, bundleIdentifier: fixture.identifier)
    }

    static func reset(_ f: Fixture) {
        fixture = f
        _ = try? fm.removeItem(atPath: work)
        for dir in [stage, applications, outside] { try! fm.createDirectory(atPath: dir, withIntermediateDirectories: true) }
    }

    static func writePlist(_ dict: [String: Any], to path: String) {
        try! fm.createDirectory(atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        (dict as NSDictionary).write(toFile: path, atomically: true)
    }

    /// A fake installed bundle: the destination the swap replaces.
    static func makeDestination(version: String, identifier: String? = nil, at path: String? = nil) {
        let root = path ?? dest
        writePlist(["CFBundleIdentifier": identifier ?? fixture.identifier, "CFBundleShortVersionString": version],
                   to: root + "/Contents/Info.plist")
        try! "old".write(toFile: root + "/Contents/marker.txt", atomically: true, encoding: .utf8)
    }

    /// Copies the fixture bundle into `<dir>/<name>` and returns that path.
    @discardableResult
    static func stageCopy(in dir: String) -> String {
        try! fm.createDirectory(atPath: dir, withIntermediateDirectories: true)
        let path = dir + "/" + fixture.name
        try! fm.copyItem(atPath: fixture.source, toPath: path)
        return path
    }

    static func siblingNames() -> [String] {
        ((try? fm.contentsOfDirectory(atPath: applications)) ?? []).filter { $0.hasPrefix(SwapCore.siblingPrefix) }
    }

    static func destState() -> String {
        let info = NSDictionary(contentsOfFile: dest + "/Contents/Info.plist")
        let marker = (try? String(contentsOfFile: dest + "/Contents/marker.txt", encoding: .utf8)) ?? "-"
        return "\(info?["CFBundleShortVersionString"] ?? "-")/\(marker)"
    }

    /// A refusal: right code (and stage), destination unchanged, no sibling directory left.
    static func expectRefusal(_ name: String, staged: String, destination: String? = nil, policy p: SwapPolicy? = nil,
                              code: SwapCode, stage: SwapStage) {
        let before = destState()
        let o = SwapCore.performSwap(stagedPath: staged, destinationPath: destination ?? dest, policy: p ?? policy())
        check(name, o.code == code && o.stage == stage && destState() == before && siblingNames().isEmpty,
              "got \(o.code.rawValue)/\(o.stage.rawValue)/\(o.err), want \(code.rawValue)/\(stage.rawValue); dest \(before) -> \(destState()); siblings \(siblingNames().count)")
    }

    static func main() {
        guard CommandLine.arguments.count == 2, let base = realpath(CommandLine.arguments[1], nil) else {
            print("usage: swapcore-tests <work-dir>")
            exit(2)
        }
        work = String(cString: base) + "/swapcore-\(getpid())"
        free(base)
        print("work dir: \(work)")

        // MARK: requirement text (TN3127 form), checked read-only against the installed release
        let prod = BeebeebIdentity.developerIDRequirement(identifier: BeebeebIdentity.appIdentifier)
        for id in [BeebeebIdentity.appIdentifier, BeebeebIdentity.serviceIdentifier] {
            var req: SecRequirement?
            let status = SecRequirementCreateWithString(BeebeebIdentity.developerIDRequirement(identifier: id) as CFString, [], &req)
            check("requirement parses (\(id))", status == errSecSuccess, "OSStatus \(status)")
        }
        let haveRelease = fm.fileExists(atPath: release.source)
            && SwapCore.checkSignature(bundlePath: release.source, requirement: prod) == nil
        if fm.fileExists(atPath: release.source) {
            check("production requirement accepts the installed release Beebeeb", haveRelease)
            let wrongTeam = prod.replacingOccurrences(of: BeebeebIdentity.teamID, with: "AAAAAAAAAA")
            check("red control: wrong team is refused",
                  SwapCore.checkSignature(bundlePath: release.source, requirement: wrongTeam)?.code == .invalidSignature)
            check("red control: wrong identifier is refused",
                  SwapCore.checkSignature(bundlePath: release.source,
                                          requirement: BeebeebIdentity.developerIDRequirement(identifier: "io.beebeeb.other"))?.code == .invalidSignature)
        } else {
            skip("production requirement vs the installed release", "no /Applications/Beebeeb.app on this Mac")
        }
        check("red control: an Apple-signed app (valid in place) fails the Developer ID requirement",
              SwapCore.checkSignature(bundlePath: textEdit.source, requirement: textEdit.requirement) == nil
                  && SwapCore.checkSignature(bundlePath: textEdit.source, requirement: prod)?.code == .invalidSignature)

        // MARK: versions (rule 1)
        let v = SwapCore.parseVersion
        let newer = SwapCore.isStrictlyNewer
        check("0.0.2177 newer than 0.0.2176", newer(v("0.0.2177")!, v("0.0.2176")!))
        check("0.8.10 newer than 0.8.9 (numeric, not text)", newer(v("0.8.10")!, v("0.8.9")!))
        check("equal is not newer", !newer(v("0.0.2176")!, v("0.0.2176")!))
        check("older is not newer", !newer(v("0.0.2176")!, v("0.0.2177")!))
        check("1.0 equals 1.0.0 (not newer)", !newer(v("1.0")!, v("1.0.0")!))
        check("1.0.1 newer than 1.0", newer(v("1.0.1")!, v("1.0")!))
        for bad in ["", "1..2", "1.2.3-beta", "v1.2", "1.2.3.4.5", " 1.2", "1.2 ", "1234567890", "+1", "١.٢"] {
            check("version '\(bad)' is refused", v(bad) == nil)
        }

        // MARK: path parsing
        for bad in ["relative/path", "/a/../b", "/a/./b", "/a//b", "/a/b/", "/", ""] {
            check("path '\(bad)' is refused", PathCheck.components(bad) == nil)
        }

        // MARK: refusals decided before the signature check (TextEdit fixture)
        reset(textEdit); makeDestination(version: "1.0")
        let outsideApp = stageCopy(in: outside)
        expectRefusal("staged outside the container", staged: outsideApp, code: .pathOutsideContainer, stage: .paths)
        expectRefusal("staged path with .. back out of the container",
                      staged: container + "/stage/../../../../../../outside/TextEdit.app", code: .pathOutsideContainer, stage: .paths)
        expectRefusal("staged path relative", staged: "stage/TextEdit.app", code: .pathOutsideContainer, stage: .paths)
        expectRefusal("staged path == the container root itself", staged: container, code: .pathOutsideContainer, stage: .paths)
        expectRefusal("staged path in a sibling container", staged: work + "/home/Library/Containers/io.beebeeb.app/Other/TextEdit.app",
                      code: .pathOutsideContainer, stage: .paths)
        expectRefusal("staged path sharing the container's string prefix (Data-evil)", staged: container + "-evil/TextEdit.app",
                      code: .pathOutsideContainer, stage: .paths)

        try! fm.createSymbolicLink(atPath: stage + "/link.app", withDestinationPath: outsideApp)
        expectRefusal("staged bundle is a symlink", staged: stage + "/link.app", code: .symlinkRefused, stage: .stagedOpen)
        try! fm.createSymbolicLink(atPath: stage + "/linkdir", withDestinationPath: outside)
        expectRefusal("a directory on the staged path is a symlink", staged: stage + "/linkdir/TextEdit.app",
                      code: .symlinkRefused, stage: .stagedOpen)
        expectRefusal("staged bundle missing", staged: stage + "/missing/TextEdit.app", code: .ioError, stage: .stagedOpen)

        let good = stageCopy(in: stage + "/good")
        expectRefusal("destination is not this service's app", staged: good, destination: applications + "/Other.app",
                      code: .destinationRefused, stage: .destination)
        expectRefusal("destination path with ..", staged: good, destination: applications + "/../Applications/TextEdit.app",
                      code: .destinationRefused, stage: .paths)

        reset(textEdit)
        makeDestination(version: "1.0", at: outside + "/Real.app")
        try! fm.createSymbolicLink(atPath: dest, withDestinationPath: outside + "/Real.app")
        let viaLink = SwapCore.performSwap(stagedPath: stageCopy(in: stage + "/good"), destinationPath: dest, policy: policy())
        check("destination is a symlink", viaLink.code == .symlinkRefused && viaLink.stage == .destination && siblingNames().isEmpty,
              "got \(viaLink.code.rawValue)/\(viaLink.stage.rawValue)")

        reset(textEdit); makeDestination(version: "1.0", identifier: "com.example.other")
        expectRefusal("destination is another app (bundle id)", staged: stageCopy(in: stage + "/good"),
                      code: .destinationRefused, stage: .destination)

        reset(textEdit); makeDestination(version: "1.0")
        let good3 = stageCopy(in: stage + "/good")
        chmod(applications, 0o555)
        expectRefusal("destination parent not writable (no elevation)", staged: good3, code: .notWritable, stage: .writable)
        chmod(applications, 0o755)
        chmod(dest, 0o555)
        expectRefusal("destination bundle not writable (no elevation)", staged: good3, code: .notWritable, stage: .writable)
        chmod(dest, 0o755)

        // Refusals during the copy: the sibling directory existed and must be gone afterwards.
        let inner = stageCopy(in: stage + "/inner")
        try! fm.createSymbolicLink(atPath: inner + "/Contents/Resources/spike-link", withDestinationPath: "/etc/hosts")
        expectRefusal("a symlink inside the staged bundle", staged: inner, code: .symlinkRefused, stage: .copy)
        let fifo = stageCopy(in: stage + "/fifo")
        check("fixture: mkfifo", mkfifo(fifo + "/Contents/Resources/spike-fifo", 0o600) == 0)
        expectRefusal("a fifo inside the staged bundle", staged: fifo, code: .ioError, stage: .copy)
        var tiny = policy()
        tiny.maxEntries = 3
        expectRefusal("copy budget exceeded", staged: stageCopy(in: stage + "/budget"), policy: tiny, code: .ioError, stage: .copy)
        let unsigned = stage + "/unsigned/TextEdit.app"
        writePlist(["CFBundleIdentifier": "com.apple.TextEdit", "CFBundleShortVersionString": "99.0",
                    "CFBundleExecutable": "TextEdit"], to: unsigned + "/Contents/Info.plist")
        try! fm.createDirectory(atPath: unsigned + "/Contents/MacOS", withIntermediateDirectories: true)
        try! "#!/bin/sh\nexit 0\n".write(toFile: unsigned + "/Contents/MacOS/TextEdit", atomically: true, encoding: .utf8)
        chmod(unsigned + "/Contents/MacOS/TextEdit", 0o755)
        expectRefusal("an unsigned bundle", staged: unsigned, code: .invalidSignature, stage: .signature)

        // MARK: signature, version and the ok path (release Beebeeb fixture, production requirement)
        guard haveRelease, let releaseInfo = SwapCore.bundleInfo(release.source) else {
            for name in ["tampered", "wrong team", "equal version", "older version", "ok path (5 checks)", "cleanup (2 checks)"] {
                skip(name, "needs a Developer ID R8352WDJJR Beebeeb.app in /Applications")
            }
            finish()
        }
        let releaseVersion = releaseInfo.version.map(String.init).joined(separator: ".")
        print("release fixture: Beebeeb \(releaseVersion)")

        reset(release); makeDestination(version: "0.0.1")
        let tampered = stageCopy(in: stage + "/tampered")
        try! "tampered".write(toFile: tampered + "/Contents/Resources/spike-extra.txt", atomically: true, encoding: .utf8)
        expectRefusal("a tampered bundle (file added after signing)", staged: tampered, code: .invalidSignature, stage: .signature)
        let wrongTeam = policy(requirement: prod.replacingOccurrences(of: BeebeebIdentity.teamID, with: "AAAAAAAAAA"))
        expectRefusal("validly signed by another team", staged: stageCopy(in: stage + "/team"), policy: wrongTeam,
                      code: .invalidSignature, stage: .signature)

        reset(release); makeDestination(version: releaseVersion)
        expectRefusal("equal version (rule 1)", staged: stageCopy(in: stage + "/equal"), code: .notNewer, stage: .version)
        reset(release); makeDestination(version: "\(releaseInfo.version[0] + 1).0")
        expectRefusal("older version (rule 1)", staged: stageCopy(in: stage + "/older"), code: .notNewer, stage: .version)

        reset(release); makeDestination(version: "0.0.1")
        let staged = stageCopy(in: stage + "/ok")
        let o = SwapCore.performSwap(stagedPath: staged, destinationPath: dest, policy: policy())
        check("swap ok", o.code == .ok && o.stage == .done, "got \(o.code.rawValue)/\(o.stage.rawValue)/\(o.err)")
        check("destination now holds the new bundle", destState() == "\(releaseVersion)/-", "dest \(destState())")
        check("the swapped-in copy validates in place (production requirement)",
              SwapCore.checkSignature(bundlePath: dest, requirement: prod) == nil)
        check("the staged original is untouched", fm.fileExists(atPath: staged + "/Contents/MacOS/beebeeb-desktop"))
        let siblings = siblingNames()
        check("exactly one sibling directory, holding the previous bundle", siblings.count == 1)
        if let sibling = siblings.first {
            var st = stat()
            check("the sibling directory is 0700", lstat(applications + "/" + sibling, &st) == 0 && st.st_mode & 0o777 == 0o700,
                  String(format: "mode %o", st.st_mode & 0o777))
            let old = try? String(contentsOfFile: applications + "/" + sibling + "/Beebeeb.app/Contents/marker.txt", encoding: .utf8)
            check("the previous bundle is in the sibling directory", old == "old")
        }
        let foreign = SwapCore.removeStaleUpdates(destinationPath: applications + "/Other.app", policy: policy())
        check("cleanup refuses a foreign destination", foreign.code == .destinationRefused && siblingNames().count == 1)
        let cleanup = SwapCore.removeStaleUpdates(destinationPath: dest, policy: policy())
        check("cleanup removes the previous bundle", cleanup.code == .ok && cleanup.err == 1 && siblingNames().isEmpty,
              "got \(cleanup.code.rawValue) removed=\(cleanup.err) left=\(siblingNames().count)")
        finish()
    }

    static func finish() -> Never {
        _ = try? fm.removeItem(atPath: work)
        print("swapcore-tests: \(passed) passed, \(failed) failed, \(skipped) skipped")
        exit(failed == 0 && passed > 0 ? 0 : 1)
    }
}
