//! The File Provider extension writes every log line through its one `os.Logger`
//! (`BeebeebFileProvider/ExtensionLog.swift`, spec 2026-10-09 §11). On macOS 26 the unified log
//! shows every `NSLog` line of the extension as `<private>`, so a line written with `NSLog`
//! cannot be read at all, and a value a line marks public is shown to anyone who reads the log.
//!
//! These pins read the extension's Swift source as the build compiles it (every
//! `BeebeebFileProvider/*.swift`), on every platform:
//! - only `ExtensionLog.swift` logs: no other file calls `NSLog`, `os_log`, `print` or
//!   `debugPrint`, builds a `Logger` or calls `logger`;
//! - `ExtensionLog.swift` builds one `Logger`, under the extension's bundle id;
//! - every value a line interpolates says its privacy, and a public one is a name from
//!   [`PUBLIC_VALUES`], each bound in `ExtensionLog.swift` to a closed value, so a path, a file
//!   name or an error's text never becomes public.
//!
//! What they cannot see: what a public name is bound to. `ExtensionLog.swift` binds each one to a
//! closed value (an enum's raw value, a bitmask, an id that parsed as a UUID, a hex prefix, an
//! `NSError` domain and code); the Swift harness tests those bindings.

/// The one file that logs.
const LOG_FILE: &str = "ExtensionLog.swift";

/// The values a line may make public, by the name it interpolates.
const PUBLIC_VALUES: &[&str] = &[
    "line.call",
    "line.options",
    "id",
    "call.rawValue",
    "prefix",
    "fields.domain",
    "fields.code",
    "cause",
    "XPCBridge.appGroupID",
];

/// Log calls no file may make: the extension logs through `ExtensionLog` only.
const FORBIDDEN_CALLS: &[&str] = &["NSLog(", "NSLogv(", "os_log(", "print(", "debugPrint("];

/// `source` without its comment lines, so a comment that names a call is not a call.
fn code_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
    source
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line))
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
}

/// Every place `needle` starts in `line` as a name of its own: not the tail of a longer name
/// (`IPCContentFingerprint(` is not `print(`) and not a member (`x.print(` is not `print(`).
fn calls_in(line: &str, needle: &str) -> usize {
    line.match_indices(needle)
        .filter(|(at, _)| {
            line[..*at]
                .chars()
                .next_back()
                .is_none_or(|before| !(before.is_alphanumeric() || before == '_' || before == '.'))
        })
        .count()
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
        if !is_log_file && (calls_in(line, "Logger(") > 0 || line.contains("logger.")) {
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
            .map(|(_, line)| calls_in(line, "Logger("))
            .sum::<usize>()
            != 1
    {
        found.push(format!("{file_name}: builds a Logger other than once"));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extension_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../BeebeebFileProvider")
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
        let build = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/build-fileprovider-extension.sh"),
        )
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
}
