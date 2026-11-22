//! The crash record: when the app panics, one small redacted record is written next to the log, and the next start
//! can say "stopped last time" and offer Report a problem with it attached. Nothing is sent anywhere.
//!
//! The record holds only: the app version, the thread (`main` or `other`, never a thread's own name), and the
//! source location as `<file>.rs:<line>` (the app's own source file name, no folder, checked by shape). The panic
//! message is NEVER written (it can carry a name, a path or a value).
//!
//! File: `<log folder>/beebeeb-drive-crash.txt`, one record, the newest panic wins. [`take_pending`] reads it,
//! [`clear`] removes it once the person has dealt with it.

use std::fs;
use std::path::{Path, PathBuf};

pub const CRASH_FILE_NAME: &str = "beebeeb-drive-crash.txt";

/// One crash record as it reads back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashRecord {
    /// The record's text, ready to attach to a report.
    pub text: String,
    /// Where the record is.
    pub path: PathBuf,
}

/// The record text for a panic. `file` and `thread` are untrusted: both are reduced by shape.
pub fn render(version: &str, thread: Option<&str>, file: &str, line: u32, when: &str) -> String {
    let thread = if thread == Some("main") { "main" } else { "other" };
    let version: String = version.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-')).collect();
    format!("{when} CRASH version={version} thread={thread} location={}:{line}\n", safe_file(file))
}

/// The last path component when it looks like `name.rs` (letters, digits, `_`), else `unknown`.
fn safe_file(file: &str) -> String {
    let base = file.rsplit(['/', '\\']).next().unwrap_or("");
    let ok = base.strip_suffix(".rs").is_some_and(|stem| {
        !stem.is_empty() && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    });
    if ok { base.to_string() } else { "unknown".to_string() }
}

/// Writes the record into `dir`. Best effort; a failed write loses the record only.
pub fn write(dir: &Path, text: &str) {
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(dir.join(CRASH_FILE_NAME), text);
}

/// The record a previous run left in `dir`, if any (it stays until [`clear`]).
pub fn take_pending(dir: &Path) -> Option<CrashRecord> {
    let path = dir.join(CRASH_FILE_NAME);
    let text = fs::read_to_string(&path).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    Some(CrashRecord { text, path })
}

/// Removes the record (after it was reported or dismissed).
pub fn clear(dir: &Path) {
    let _ = fs::remove_file(dir.join(CRASH_FILE_NAME));
}

/// Installs the panic hook. The log folder is looked up when the panic happens (it is set once the file log is
/// installed); before that a panic writes no record. The previous hook still runs.
pub fn install_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = super::dir() {
            let (file, line) = info.location().map_or(("", 0), |l| (l.file(), l.line()));
            let thread = std::thread::current();
            let when = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            write(dir, &render(env!("CARGO_PKG_VERSION"), thread.name(), file, line, &when));
        }
        previous(info);
    }));
}
