//! Source pins over the macOS File Provider sources that stay in this repo after the old shell goes:
//! the extension (`BeebeebFileProvider/*.swift`), its developer helper
//! (`BeebeebFileProviderTools/DomainControlTool.swift`), the Objective-C bridge
//! (`packaging/macos/FileProviderBridge.m`) and the extension's build script. They read text, run on
//! every OS, and need nothing from the engine. Rules and the rest of each rule's acceptance list:
//! `native/beebeeb-drive-rune/docs/macos/file-provider-bridge.md` (the log pins) and
//! `native/beebeeb-drive-rune/docs/macos/file-provider-removal-and-kept-folders.md` (the removal pins).
//!
//! Mutants (the checkpoint runs them; each must turn a pin red):
//! - drop `mode:NSFileProviderDomainRemovalModePreserveDirtyUserData` from the bridge's one removal in a scratch
//!   copy → `domain_removal::test_1882_every_domain_removal_in_the_repo_keeps_unsynced_files` red;
//! - add a line `NSLog("x")` to any `BeebeebFileProvider/*.swift` in a scratch copy →
//!   `extension_log::the_extension_logs_only_through_its_logger_and_makes_only_closed_values_public` red.

/// The repo root: this crate is `crates/beebeeb-engine`.
fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The File Provider extension writes every log line through its one `os.Logger`
/// (`BeebeebFileProvider/ExtensionLog.swift`, spec 2026-10-09 §11). On macOS 26 the unified log
/// shows every `NSLog` line of the extension as `<private>`, so a line written with `NSLog`
/// cannot be read at all, and a value a line marks public is shown to anyone who reads the log.
///
/// These pins read the extension's Swift source as the build compiles it (every
/// `BeebeebFileProvider/*.swift`), on every platform, skipping lines that begin with `//`. They fail
/// when:
/// - any file calls `NSLog`, `NSLogv`, `os_log`, `print` or `debugPrint`, bare or qualified by a
///   module (`Foundation.`, `Swift.`, `os.`, `OSLog.`);
/// - a file other than `ExtensionLog.swift` builds a `Logger` or an `OSLog` (bare, through `.init`,
///   or qualified by a module) or uses a name `logger.`;
/// - `ExtensionLog.swift` builds a `Logger` other than exactly once, or its subsystem is not the
///   build script's `BUNDLE_ID`;
/// - an interpolation in `ExtensionLog.swift` does not say `.public` or `.private`, or a `.public`
///   one is not a name from [`PUBLIC_VALUES`].
///
/// They match text, so they cannot see:
/// - what a public name is bound to. `ExtensionLog.swift` binds each one to a closed value (an
///   enum's raw value, a bitmask, an id that parsed as a UUID, an `NSError` domain and code); the
///   Swift harness tests those bindings;
/// - a call with a space or a line break before its parenthesis, or a log function passed by
///   reference;
/// - output that is not a log call, such as `FileHandle` or `fputs`.
mod extension_log {
    use super::repo_root;

    /// The one file that logs.
    const LOG_FILE: &str = "ExtensionLog.swift";

    /// The values a line may make public, by the name it interpolates.
    const PUBLIC_VALUES: &[&str] = &[
        "line.call",
        "line.options",
        "id",
        "call.rawValue",
        "fields.domain",
        "fields.code",
        "cause",
        "XPCBridge.appGroupID",
    ];

    /// Log calls no file may make: the extension logs through `ExtensionLog` only.
    const FORBIDDEN_CALLS: &[&str] = &["NSLog(", "NSLogv(", "os_log(", "print(", "debugPrint("];

    /// Logger constructions only the log file may make.
    const LOGGER_CONSTRUCTIONS: &[&str] = &["Logger(", "Logger.init(", "OSLog(", "OSLog.init("];

    /// The constructions that count as the log file's one `Logger`.
    const LOGGER_ONLY: &[&str] = &["Logger(", "Logger.init("];

    /// Modules that qualify a call without making it a member call: `Foundation.NSLog(` is `NSLog(`.
    const MODULES: &[&str] = &["Foundation", "Swift", "os", "OSLog"];

    /// `source` without its comment lines, so a comment that names a call is not a call.
    fn code_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
        source
            .lines()
            .enumerate()
            .map(|(index, line)| (index + 1, line))
            .filter(|(_, line)| !line.trim_start().starts_with("//"))
    }

    /// `true` when the text before a name ends where a name may start: not inside a longer name and
    /// not after a member dot.
    fn starts_a_name(before: &str) -> bool {
        before
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '.'))
    }

    /// Every place `needle` starts in `line` as a call of its own: not the tail of a longer name
    /// (`IPCContentFingerprint(` is not `print(`) and not a member of a value (`x.print(` is not
    /// `print(`). A module qualifier keeps it a call: `Foundation.NSLog(`, `Swift.print(` and
    /// `os.Logger(` count.
    fn calls_in(line: &str, needle: &str) -> usize {
        line.match_indices(needle)
            .filter(|(at, _)| {
                let before = &line[..*at];
                starts_a_name(before)
                    || before.strip_suffix('.').is_some_and(|qualified| {
                        MODULES
                            .iter()
                            .any(|module| qualified.strip_suffix(module).is_some_and(starts_a_name))
                    })
            })
            .count()
    }

    /// How many of `needles` `line` calls.
    fn calls_of(line: &str, needles: &[&str]) -> usize {
        needles.iter().map(|needle| calls_in(line, needle)).sum()
    }

    /// Each `\( … )` interpolation of `line`, as its text between the parentheses.
    fn interpolations(line: &str) -> Vec<&str> {
        let mut found = Vec::new();
        let mut rest = line;
        while let Some(start) = rest.find("\\(") {
            let body = &rest[start + 2..];
            let mut depth = 1usize;
            let mut end = None;
            for (at, c) in body.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(at);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(end) = end else {
                found.push(body);
                break;
            };
            found.push(&body[..end]);
            rest = &body[end + 1..];
        }
        found
    }

    /// What breaks the rules in one extension source file, one entry per finding, each naming the
    /// file and line. `file_name` is the file's name in `BeebeebFileProvider/`.
    fn violations(file_name: &str, source: &str) -> Vec<String> {
        let source = source.replace("\r\n", "\n");
        let mut found = Vec::new();
        let is_log_file = file_name == LOG_FILE;
        for (number, line) in code_lines(&source) {
            let at = format!("{file_name}:{number}");
            for call in FORBIDDEN_CALLS {
                if calls_in(line, call) > 0 {
                    found.push(format!("{at}: `{call}` logs outside ExtensionLog"));
                }
            }
            if !is_log_file && (calls_of(line, LOGGER_CONSTRUCTIONS) > 0 || line.contains("logger.")) {
                found.push(format!("{at}: only {LOG_FILE} builds or calls the logger"));
            }
            if is_log_file {
                for inner in interpolations(line) {
                    match inner.rsplit_once(", privacy: ") {
                        None => found.push(format!("{at}: `\\({inner})` does not say its privacy")),
                        Some((value, ".public")) if !PUBLIC_VALUES.contains(&value.trim()) => {
                            found.push(format!("{at}: `{}` is public but not a closed value", value.trim()))
                        }
                        Some((_, ".public" | ".private")) => {}
                        Some((_, other)) => found.push(format!("{at}: privacy `{other}` is not .public or .private")),
                    }
                }
            }
        }
        if is_log_file
            && code_lines(&source)
                .map(|(_, line)| calls_of(line, LOGGER_ONLY))
                .sum::<usize>()
                != 1
        {
            found.push(format!("{file_name}: builds a Logger other than once"));
        }
        found
    }

    fn extension_dir() -> std::path::PathBuf {
        repo_root().join("BeebeebFileProvider")
    }

    /// Every `BeebeebFileProvider/*.swift`, the set `scripts/build-fileprovider-extension.sh` compiles.
    fn extension_sources() -> Vec<(String, String)> {
        let dir = extension_dir();
        let mut found: Vec<(String, String)> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .map(|entry| entry.expect("dir entry").path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("swift"))
            .map(|path| {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
                (name, source)
            })
            .collect();
        found.sort();
        found
    }

    #[test]
    fn the_extension_logs_only_through_its_logger_and_makes_only_closed_values_public() {
        let sources = extension_sources();
        let names: Vec<&str> = sources.iter().map(|(name, _)| name.as_str()).collect();
        // The scan saw the extension, and the files that log today.
        for expected in [
            LOG_FILE,
            "FileProviderExtension.swift",
            "XPCBridge.swift",
            "UploadStaging.swift",
        ] {
            assert!(names.contains(&expected), "{expected} not scanned: {names:?}");
        }
        let found: Vec<String> = sources
            .iter()
            .flat_map(|(name, source)| violations(name, source))
            .collect();
        assert!(found.is_empty(), "{}", found.join("\n"));
        // It checked something: the log file's public values.
        let log = &sources.iter().find(|(name, _)| name == LOG_FILE).unwrap().1;
        let public = code_lines(log)
            .flat_map(|(_, line)| interpolations(line))
            .filter(|inner| inner.ends_with(", privacy: .public"))
            .count();
        assert!(public >= 10, "only {public} public values seen");
    }

    #[test]
    fn the_logger_subsystem_is_the_extensions_bundle_id() {
        let build = std::fs::read_to_string(repo_root().join("scripts/build-fileprovider-extension.sh"))
            .unwrap()
            .replace("\r\n", "\n");
        let bundle_id = build
            .lines()
            .find_map(|line| line.strip_prefix("BUNDLE_ID=\"")?.strip_suffix('"'))
            .expect("the build script sets BUNDLE_ID");
        let log = std::fs::read_to_string(extension_dir().join(LOG_FILE))
            .unwrap()
            .replace("\r\n", "\n");
        let subsystem = log
            .lines()
            .find_map(|line| line.trim().strip_prefix("static let subsystem = \"")?.strip_suffix('"'))
            .expect("ExtensionLog sets its subsystem");
        assert_eq!(subsystem, bundle_id);
        assert!(
            log.contains("Logger(subsystem: subsystem, category: category)"),
            "{log}"
        );
    }

    /// The pin goes red on what it guards against: a planted log call in any file, a logger
    /// outside the log file, and a value made public that is not closed.
    #[test]
    fn the_pin_flags_a_planted_log_call_and_a_public_value_that_is_not_closed() {
        for planted in [
            "    NSLog(\"BeebeebFileProvider: planted\")",
            "        os_log(\"x\")",
            "print(error)",
            "  debugPrint(item)",
        ] {
            let source = format!("func f() {{\n{planted}\n}}\n");
            assert_eq!(
                violations("XPCBridge.swift", &source).len(),
                1,
                "not flagged: {planted}"
            );
            assert_eq!(
                violations(
                    LOG_FILE,
                    &format!("{source}let logger = Logger(subsystem: s, category: c)\n")
                )
                .len(),
                1
            );
        }
        for clean in [
            "return IPCContentFingerprint(sizeBytes: 1, modifiedNanos: 2)",
            "// NSLog(\"a comment names it\")",
            "ExtensionLog.writeOptions(call: .modify, options: 1, item: id)",
        ] {
            assert!(violations("XPCBridge.swift", clean).is_empty(), "flagged: {clean}");
        }
        assert_eq!(
            violations(
                "XPCBridge.swift",
                "let logger = Logger(subsystem: \"x\", category: \"y\")"
            )
            .len(),
            1
        );
        assert_eq!(
            violations("XPCBridge.swift", "ExtensionLog.logger.notice(\"x\")").len(),
            1
        );

        let line = |text: &str| format!("let logger = Logger(subsystem: subsystem, category: category)\n{text}\n");
        for leaks in [
            "logger.error(\"failed \\(error.localizedDescription, privacy: .public)\")",
            "logger.notice(\"at \\(url.path, privacy: .public)\")",
            "logger.notice(\"name \\(item.filename, privacy: .public)\")",
            "logger.notice(\"no privacy \\(error)\")",
            "logger.notice(\"auto \\(id, privacy: .auto)\")",
        ] {
            assert_eq!(violations(LOG_FILE, &line(leaks)).len(), 1, "not flagged: {leaks}");
        }
        for fine in [
            "logger.notice(\"item=\\(id, privacy: .public)\")",
            "logger.notice(\"kept \\(item.filename, privacy: .private)\")",
            "logger.error(\"domain=\\(fields.domain, privacy: .public) code=\\(fields.code, privacy: .public)\")",
        ] {
            assert!(violations(LOG_FILE, &line(fine)).is_empty(), "flagged: {fine}");
        }
        assert_eq!(
            violations(LOG_FILE, "enum ExtensionLog {}\n").len(),
            1,
            "no Logger at all"
        );
        // A Windows checkout: CRLF line ends.
        assert_eq!(
            violations("XPCBridge.swift", "func f() {\r\n    NSLog(\"x\")\r\n}\r\n").len(),
            1
        );
    }

    /// A call qualified by its module is the same call: `Foundation.NSLog(…)` logs like `NSLog(…)`,
    /// and `os.Logger(…)`, `OSLog(…)` or `Logger.init(…)` outside the log file builds a second
    /// logger, whose lines nothing here checks. A member of a value (`request.print(…)`) is not one.
    #[test]
    fn the_pin_flags_module_qualified_log_calls_and_loggers_built_elsewhere() {
        let planted = [
            "    Foundation.NSLog(\"BeebeebFileProvider: planted\")",
            "    Foundation.NSLogv(format, args)",
            "    Swift.print(error)",
            "    Swift.debugPrint(item)",
            "    os.os_log(\"x\")",
            "    let trace = os.Logger(subsystem: \"x\", category: \"y\")",
            "    let trace = Logger.init(subsystem: \"x\", category: \"y\")",
            "    let trace = OSLog(subsystem: \"x\", category: \"y\")",
            "    let trace = os.OSLog(subsystem: \"x\", category: \"y\")",
        ];
        let missed: Vec<&str> = planted
            .iter()
            .copied()
            .filter(|line| violations("XPCBridge.swift", &format!("func f() {{\n{line}\n}}\n")).len() != 1)
            .collect();
        assert!(missed.is_empty(), "not flagged once:\n{}", missed.join("\n"));
        for clean in [
            "    request.print(item)",
            "    self.os.Logger(item)",
            "    let store = OSLogStore.local()",
        ] {
            assert!(violations("XPCBridge.swift", clean).is_empty(), "flagged: {clean}");
        }
        // The log file's one logger may be spelled with its module.
        assert!(
            violations(
                LOG_FILE,
                "static let logger = os.Logger(subsystem: subsystem, category: category)\n"
            )
            .is_empty()
        );
        assert_eq!(
            violations(
                LOG_FILE,
                "static let logger = os.Logger(subsystem: s, category: c)\nstatic let other = Logger.init(subsystem: s, category: c)\n"
            )
            .len(),
            1,
            "a second Logger in the log file"
        );
    }
}

/// Removing Beebeeb's Finder location keeps the files that never reached the server (spec
/// `docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md`). Every `NSFileProviderManager`
/// domain removal passes `NSFileProviderDomainRemovalModePreserveDirtyUserData` (macOS 12+; Beebeeb
/// requires 14); the system then reports the folder where it kept the files that had not synced.
///
/// These pins read the Objective-C bridge and the Swift helper as text, so they run on every OS, and
/// they read a Windows checkout (CRLF text, `\` paths) the same as any other.
mod domain_removal {
    use super::repo_root;

    /// The bridge, where this repo keeps it.
    const BRIDGE: &str = "packaging/macos/FileProviderBridge.m";
    /// The developer helper, the repo's only other removal.
    const HELPER: &str = "BeebeebFileProviderTools/DomainControlTool.swift";

    // The folder states the bridge reports after a removal, and the bounded re-check of a folder that
    // reads as missing. These are the contract values: the macOS app's decode must use the same
    // numbers (the removal note lists what each one means).
    const KEPT_NONE_REPORTED: i32 = 0;
    const KEPT_MISSING: i32 = 1;
    const KEPT_EMPTY: i32 = 2;
    const KEPT_HAS_ENTRIES: i32 = 3;
    const KEPT_UNCHECKED: i32 = 4;
    const KEPT_NO_PATH: i32 = 5;
    /// How often a folder that reads as missing is looked at again before "nothing kept".
    const KEPT_MISSING_RECHECKS: u32 = 4;
    /// The wait before each of those looks.
    const KEPT_MISSING_RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

    /// One `NSFileProviderManager` domain removal found in a source file.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RemovalSite {
        line: usize,
        preserving: bool,
    }

    /// The scanner (spec §6). Finds every domain removal in one Objective-C or Swift source and says
    /// whether it passes the preserving mode. Comments are skipped. A remove-all call
    /// (`removeAllDomains…`) is always a non-preserving site.
    fn removal_sites(file_name: &str, source: &str) -> Vec<RemovalSite> {
        let _ = file_name; // every pattern is checked in every file, whatever its language
        let code = without_comments(source);
        let line_of = |index: usize| code[..index].matches('\n').count() + 1;
        let squash = |text: &str| text.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        let mut sites: Vec<(usize, bool)> = Vec::new();

        // Objective-C: `removeDomain:` up to its `completionHandler:` must carry the mode.
        for (index, _) in code.match_indices("removeDomain:") {
            let rest = &code[index..];
            let send = rest.find("completionHandler:").map_or(rest, |end| &rest[..end]);
            sites.push((
                index,
                squash(send).contains("mode:NSFileProviderDomainRemovalModePreserveDirtyUserData"),
            ));
        }
        // Swift: the argument list of `NSFileProviderManager.remove(` must carry the mode.
        for (index, matched) in code.match_indices("NSFileProviderManager.remove(") {
            let open = index + matched.len();
            let mut depth = 1usize;
            let mut close = code.len();
            for (offset, ch) in code[open..].char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            close = open + offset;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let args = squash(&code[open..close]);
            sites.push((
                index,
                args.contains("mode:.preserveDirtyUserData")
                    || args.contains("mode:NSFileProviderManager.DomainRemovalMode.preserveDirtyUserData"),
            ));
        }
        // Remove-all, in either language, never keeps anything.
        for (index, _) in code.match_indices("removeAllDomains") {
            sites.push((index, false));
        }

        sites.sort();
        sites
            .into_iter()
            .map(|(index, preserving)| RemovalSite {
                line: line_of(index),
                preserving,
            })
            .collect()
    }

    /// `source` with `//` and `/* */` comments blanked to spaces (newlines kept, so line numbers
    /// hold). String literals are skipped as written, so a `//` inside one is not a comment.
    fn without_comments(source: &str) -> String {
        let mut out = String::with_capacity(source.len());
        let mut chars = source.chars().peekable();
        let blank = |ch: char| if ch == '\n' { '\n' } else { ' ' };
        while let Some(ch) = chars.next() {
            match ch {
                '"' => {
                    out.push(ch);
                    while let Some(inner) = chars.next() {
                        out.push(inner);
                        match inner {
                            '\\' => {
                                if let Some(escaped) = chars.next() {
                                    out.push(escaped);
                                }
                            }
                            '"' | '\n' => break,
                            _ => {}
                        }
                    }
                }
                '/' if chars.peek() == Some(&'/') => {
                    out.push(' ');
                    for inner in chars.by_ref() {
                        out.push(blank(inner));
                        if inner == '\n' {
                            break;
                        }
                    }
                }
                '/' if chars.peek() == Some(&'*') => {
                    out.push(' ');
                    out.push(blank(chars.next().unwrap_or(' ')));
                    let mut previous = ' ';
                    for inner in chars.by_ref() {
                        out.push(blank(inner));
                        if previous == '*' && inner == '/' {
                            break;
                        }
                        previous = inner;
                    }
                }
                _ => out.push(ch),
            }
        }
        out
    }

    // ── portable reading (r5) ────────────────────────────────────────────────
    //
    // A Windows checkout turns line endings into CRLF and path separators into `\`, so a pin that
    // looks for `"\n}\n"` or compares a path with `/` finds nothing there. Every pin reads its source
    // through these functions and takes the raw text as an argument, so a test can hand it the
    // Windows form on any OS.

    /// A source's text with `\r\n` turned into `\n`: what every pin anchors on.
    fn lf(raw: &str) -> String {
        raw.replace("\r\n", "\n")
    }

    /// The Windows form of a source: every line ends in `\r\n`.
    fn crlf(text: &str) -> String {
        lf(text).replace('\n', "\r\n")
    }

    /// A path relative to the repo root with `/` separators, whatever the OS wrote.
    fn slash(path: &str) -> String {
        path.replace('\\', "/")
    }

    /// One repo source's text, read from disk.
    fn read(rel: &str) -> String {
        let path = repo_root().join(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    /// One scanned source as the pins see it, from the path and text the OS gave: the path with
    /// `/` separators and the text with `\n` line ends, so a Windows checkout reads like the rest.
    fn scanned_source(rel: &str, raw: &str) -> (String, String) {
        (slash(rel), lf(raw))
    }

    /// Every Objective-C and Swift source in this repo. `target`, `node_modules`, `.git`, `dist`,
    /// `graphify-out` and `docs` are skipped (build output, dependencies, and prose that quotes
    /// the old calls on purpose).
    fn repo_native_sources() -> Vec<(String, String)> {
        let root = repo_root();
        let mut found = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
            for entry in entries {
                let path = entry.expect("dir entry").path();
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                if path.is_dir() {
                    if !matches!(
                        name.as_str(),
                        "target" | "node_modules" | ".git" | "dist" | "graphify-out" | "docs"
                    ) {
                        stack.push(path);
                    }
                } else if matches!(path.extension().and_then(|e| e.to_str()), Some("m" | "mm" | "swift")) {
                    let rel = path.strip_prefix(&root).unwrap_or(&path).display().to_string();
                    let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {rel}: {e}"));
                    found.push(scanned_source(&rel, &source));
                }
            }
        }
        found.sort();
        found
    }

    fn every_domain_removal_keeps_unsynced_files(sources: &[(String, String)]) {
        let mut sites = Vec::new();
        for (file, source) in sources {
            for site in removal_sites(file, source) {
                sites.push((file.clone(), site));
            }
        }
        // The scan must prove it looked: the bridge's one removal and the helper's.
        let in_file = |suffix: &str| sites.iter().filter(|(file, _)| file.ends_with(suffix)).count();
        assert_eq!(in_file(BRIDGE), 1, "sites: {sites:?}");
        assert_eq!(in_file(HELPER), 1, "sites: {sites:?}");
        assert!(
            sites.len() >= 2,
            "a scan that found fewer than 2 removals scanned the wrong tree: {sites:?}"
        );

        // The bridge's one removal serves both exported entry points (sign-out / Repair /
        // rollback, and the sweep): one definition plus two calls, comments excluded.
        let (_, bridge) = sources
            .iter()
            .find(|(file, _)| file.ends_with(BRIDGE))
            .expect("the bridge was scanned");
        let bridge_code = without_comments(bridge);
        assert_eq!(bridge_code.matches("BeebeebRemoveDomainKeepingUnsynced(").count(), 3);
        for entry in ["int beebeeb_fp_remove(", "int beebeeb_fp_remove_domain_by_id("] {
            let start = bridge_code.find(entry).unwrap_or_else(|| panic!("{entry} exists"));
            let body = &bridge_code[start..start + bridge_code[start..].find("\n}\n").expect("function ends")];
            assert!(
                body.contains("BeebeebRemoveDomainKeepingUnsynced("),
                "{entry} must remove through the keeping helper"
            );
        }

        let offenders: Vec<String> = sites
            .iter()
            .filter(|(_, site)| !site.preserving)
            .map(|(file, site)| format!("{file}:{}", site.line))
            .collect();
        assert!(
            offenders.is_empty(),
            "every File Provider domain removal must pass the preserve-dirty-user-data mode \
             (spec 2026-10-09 §3); without it macOS deletes files that never reached the server: {offenders:?}"
        );
    }

    fn the_helper_prints_preserved_for_any_existing_folder(raw: &str) {
        let helper = without_comments(&lf(raw));
        let remove = &helper[helper.find("static func remove()").expect("remove()")..];
        let remove = &remove[..remove.find("static func signalRoot()").expect("next function")];
        let squashed: String = remove.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            squashed.contains(
                "switchsettledKeptFolder(preservedLocation){case.hasEntries,.empty,.unchecked:print(\"preserved:\\(path)\")"
            ),
            "`preserved:` is printed for any folder that exists, empty or not, after the re-checks (re-review D1):\n{remove}"
        );
        assert_eq!(
            remove.matches("print(\"preserved:").count(),
            1,
            "one `preserved:` line, in the kept arm"
        );
        assert!(
            remove.contains("which is missing"),
            "the helper says so when the folder is still missing after the re-checks"
        );
        assert!(
            !remove.contains("which is empty"),
            "an empty folder is kept now, so the helper never calls it 'nothing kept'"
        );
        // The same bounded re-check as the app: a few looks, about a second.
        let settle = &helper[helper
            .find("static func settledKeptFolder(")
            .expect("settledKeptFolder()")..];
        let settle = &settle[..settle.find("\n    }\n").expect("function ends")];
        let settle: String = settle.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            settle.contains("for_in0..<keptMissingRechecks{")
                && settle.contains("Thread.sleep(forTimeInterval:keptMissingRecheckInterval)")
                && settle.contains("keptFolder(location)"),
            "the helper looks again at a missing folder, on the same URL:\n{settle}"
        );
        let looks = format!("static let keptMissingRechecks = {KEPT_MISSING_RECHECKS}");
        let seconds = format!(
            "static let keptMissingRecheckInterval: TimeInterval = {}",
            KEPT_MISSING_RECHECK_INTERVAL.as_secs_f64()
        );
        assert!(
            helper.contains(&looks) && helper.contains(&seconds),
            "the helper's constants are the app's ({looks}; {seconds})"
        );
        let check = &helper[helper.find("static func keptFolder(").expect("keptFolder()")..];
        assert!(
            check.contains("startAccessingSecurityScopedResource()"),
            "it looks through the URL's own scope"
        );
    }

    fn the_bridge_checks_the_folder_before_the_error(raw: &str) {
        let bridge = without_comments(&lf(raw));
        let start = bridge
            .find("static int BeebeebRemoveDomainKeepingUnsynced(")
            .expect("the shared removal");
        let body = &bridge[start..start + bridge[start..].find("\n}\n").expect("function ends")];
        let checked = body
            .find("BeebeebKeptFolderState(found_location)")
            .expect("the folder is checked");
        let copied = body
            .find("BeebeebCopyString(path, location_buffer")
            .expect("its path is copied");
        let error = body.find("if (found_error != nil)").expect("the error is handled");
        assert!(
            checked < error && copied < error,
            "folder state and path come before the error return:\n{body}"
        );
    }

    fn the_folder_states_match_the_bridge(raw: &str) {
        let bridge = lf(raw);
        for (name, value) in [
            ("BeebeebKeptNoneReported", KEPT_NONE_REPORTED),
            ("BeebeebKeptMissing", KEPT_MISSING),
            ("BeebeebKeptEmpty", KEPT_EMPTY),
            ("BeebeebKeptHasEntries", KEPT_HAS_ENTRIES),
            ("BeebeebKeptUnchecked", KEPT_UNCHECKED),
            ("BeebeebKeptNoPath", KEPT_NO_PATH),
        ] {
            assert!(
                bridge.contains(&format!("{name} = {value},")),
                "FileProviderBridge.m must define {name} = {value}"
            );
        }
    }

    // ── the source pin (spec §6) ─────────────────────────────────────────────

    #[test]
    fn test_1882_scanner_flags_objc_removals_without_the_mode() {
        let src = "\
// [NSFileProviderManager removeDomain:old completionHandler:nil]; a comment, not a call
/* [NSFileProviderManager removeDomain:x completionHandler:nil]; */
[NSFileProviderManager removeDomain:domain completionHandler:^(NSError *error) {}];
[NSFileProviderManager removeDomain:domain
                               mode:NSFileProviderDomainRemovalModePreserveDirtyUserData
                  completionHandler:^(NSURL *url, NSError *error) {}];
[NSFileProviderManager removeDomain:domain mode:NSFileProviderDomainRemovalModeRemoveAll completionHandler:nil];
[NSFileProviderManager removeAllDomainsWithCompletionHandler:^(NSError *error) {}];
";
        assert_eq!(
            removal_sites("Bridge.m", src),
            vec![
                RemovalSite {
                    line: 3,
                    preserving: false
                },
                RemovalSite {
                    line: 4,
                    preserving: true
                },
                RemovalSite {
                    line: 7,
                    preserving: false
                },
                RemovalSite {
                    line: 8,
                    preserving: false
                },
            ]
        );
    }

    #[test]
    fn test_1882_scanner_flags_swift_removals_without_the_mode() {
        let src = "\
// NSFileProviderManager.remove(domain) in a comment is not a call
NSFileProviderManager.remove(domain) { error in }
NSFileProviderManager.remove(domain, mode: .preserveDirtyUserData) { url, error in }
try await NSFileProviderManager.remove(
    domain,
    mode: .preserveDirtyUserData
)
NSFileProviderManager.remove(domain, mode: .removeAll) { url, error in }
NSFileProviderManager.removeAllDomains { error in }
";
        assert_eq!(
            removal_sites("Tool.swift", src),
            vec![
                RemovalSite {
                    line: 2,
                    preserving: false
                },
                RemovalSite {
                    line: 3,
                    preserving: true
                },
                RemovalSite {
                    line: 4,
                    preserving: true
                },
                RemovalSite {
                    line: 8,
                    preserving: false
                },
                RemovalSite {
                    line: 9,
                    preserving: false
                },
            ]
        );
    }

    #[test]
    fn test_1882_every_domain_removal_in_the_repo_keeps_unsynced_files() {
        every_domain_removal_keeps_unsynced_files(&repo_native_sources());
    }

    // ── r5: the pins read a Windows checkout (CRLF text, `\` paths) like any other ──

    #[test]
    fn test_1882_r5_the_reader_gives_a_windows_source_the_same_path_and_text() {
        let bridge = read(BRIDGE);
        let windows = crlf(&bridge);
        assert!(windows.contains("\r\n"), "the copy really has Windows line ends");
        assert_eq!(
            scanned_source("packaging\\macos\\FileProviderBridge.m", &windows),
            (BRIDGE.to_string(), lf(&bridge)),
            "the `\\` path and the CRLF text read as the `/` path and the LF text"
        );
        assert_eq!(
            scanned_source(BRIDGE, &bridge),
            (BRIDGE.to_string(), lf(&bridge)),
            "and a Unix source is unchanged"
        );
    }

    #[test]
    fn test_1882_r5_the_bridge_pins_pass_on_a_windows_checkout() {
        let bridge = crlf(&read(BRIDGE));
        the_bridge_checks_the_folder_before_the_error(&bridge);
        the_folder_states_match_the_bridge(&bridge);
    }

    #[test]
    fn test_1882_r5_the_repo_scan_passes_on_a_windows_checkout() {
        let windows: Vec<(String, String)> = repo_native_sources()
            .iter()
            .map(|(file, source)| scanned_source(&file.replace('/', "\\"), &crlf(source)))
            .collect();
        assert_eq!(windows, repo_native_sources(), "the reader undoes the Windows form");
        every_domain_removal_keeps_unsynced_files(&windows);
    }

    /// The pins stay meaningful in the Windows form: a real violation still fails them there, and
    /// fails them for the reason it should (not because the Windows text defeated an anchor).
    #[test]
    fn test_1882_r5_the_pins_still_fail_a_real_violation_in_the_windows_form() {
        fn panic_message(check: impl FnOnce()) -> Option<String> {
            let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(check)).err()?;
            Some(
                payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|text| (*text).to_string()))
                    .unwrap_or_default(),
            )
        }

        // The bridge no longer checks the folder.
        let bridge = crlf(&read(BRIDGE)).replace(
            "BeebeebKeptFolderState(found_location)",
            "BeebeebNotTheCheck(found_location)",
        );
        let message = panic_message(|| the_bridge_checks_the_folder_before_the_error(&bridge));
        assert_eq!(message.as_deref(), Some("the folder is checked"));

        // The helper stops looking again at a missing folder.
        let helper = crlf(&read(HELPER)).replace("Thread.sleep(forTimeInterval: keptMissingRecheckInterval)", "");
        let message = panic_message(|| the_helper_prints_preserved_for_any_existing_folder(&helper));
        assert!(
            message
                .as_deref()
                .is_some_and(|m| m.starts_with("the helper looks again at a missing folder")),
            "{message:?}"
        );

        // A removal without the mode, in a CRLF Swift source with a `\` path, is an offender.
        let mut sources = repo_native_sources();
        sources.push(scanned_source(
            "BeebeebFileProviderTools\\Other.swift",
            "NSFileProviderManager.remove(domain) { error in }\r\n",
        ));
        let message = panic_message(|| every_domain_removal_keeps_unsynced_files(&sources));
        assert!(
            message
                .as_deref()
                .is_some_and(|m| m.contains("BeebeebFileProviderTools/Other.swift:1")),
            "{message:?}"
        );
    }

    // ── the same pins, from the removal's other test files, over the sources that stay ──

    /// The bridge checks the reported folder BEFORE it looks at the error, so a reply that carries
    /// both reaches the app with the folder's state.
    #[test]
    fn test_1882_r2_the_bridge_checks_the_folder_before_the_error() {
        the_bridge_checks_the_folder_before_the_error(&read(BRIDGE));
    }

    /// The bridge's folder-state numbers are the contract values above.
    #[test]
    fn test_1882_r2_the_folder_states_match_the_bridge() {
        the_folder_states_match_the_bridge(&read(BRIDGE));
    }

    /// The developer helper follows the same rule: `preserved:` for any folder that exists, after the
    /// same bounded re-check. The helper has no test harness of its own, so its source is pinned here.
    #[test]
    fn test_1882_r3_the_helper_prints_preserved_for_any_existing_folder() {
        the_helper_prints_preserved_for_any_existing_folder(&read(HELPER));
    }

    #[test]
    fn test_1882_r5_the_helper_pin_passes_on_a_windows_checkout() {
        the_helper_prints_preserved_for_any_existing_folder(&crlf(&read(HELPER)));
    }
}
