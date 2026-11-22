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
    let version: String = version
        .chars()
        .filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-'))
        .collect();
    format!(
        "{when} CRASH version={version} thread={thread} location={}:{line}\n",
        safe_file(file)
    )
}

/// The last path component when it looks like `name.rs` (letters, digits, `_`), else `unknown`.
fn safe_file(file: &str) -> String {
    let base = file.rsplit(['/', '\\']).next().unwrap_or("");
    let ok = base
        .strip_suffix(".rs")
        .is_some_and(|stem| !stem.is_empty() && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    if ok { base.to_string() } else { "unknown".to_string() }
}

/// Writes the record into `dir`. Best effort; a failed write loses the record only.
pub fn write(dir: &Path, text: &str) {
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(dir.join(CRASH_FILE_NAME), text);
}

/// The record a previous run left in `dir`, if any (it stays until [`clear`]). The file is read as untrusted: only
/// lines of the exact shape [`render`] writes are kept ([`reduce`]), so whatever else the file holds never reaches a
/// notice or a report. A file with no such line is no record.
pub fn take_pending(dir: &Path) -> Option<CrashRecord> {
    let path = dir.join(CRASH_FILE_NAME);
    let raw = fs::read(&path).ok()?;
    let text = reduce(&String::from_utf8_lossy(&raw[..raw.len().min(4096)]));
    if text.is_empty() {
        return None;
    }
    Some(CrashRecord { text, path })
}

impl CrashRecord {
    /// Removes this record's file (after it was reported or dismissed).
    pub fn clear(&self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// The lines of `text` that have the exact shape [`render`] writes, at most four; every other line is dropped.
pub fn reduce(text: &str) -> String {
    text.lines()
        .filter(|l| shaped(l))
        .take(4)
        .map(|l| format!("{l}\n"))
        .collect()
}

/// `<when> CRASH version=<digits . -> thread=main|other location=<name>.rs|unknown:<digits>`.
fn shaped(line: &str) -> bool {
    let parts: Vec<&str> = line.split(' ').collect();
    let [when, "CRASH", version, thread, location] = parts[..] else {
        return false;
    };
    let when_ok = !when.is_empty()
        && when.len() <= 40
        && when
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, 'T' | 'Z' | ':' | '-' | '+' | '.'));
    let version_ok = version
        .strip_prefix("version=")
        .is_some_and(|v| v.len() <= 32 && v.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-')));
    let thread_ok = matches!(thread, "thread=main" | "thread=other");
    let location_ok = location
        .strip_prefix("location=")
        .and_then(|l| l.rsplit_once(':'))
        .is_some_and(|(file, n)| {
            !n.is_empty()
                && n.len() <= 10
                && n.chars().all(|c| c.is_ascii_digit())
                && file.len() <= 80
                && (file == "unknown" || safe_file(file) == file)
        });
    when_ok && version_ok && thread_ok && location_ok
}

/// Puts the record into a saved support bundle (the engine's JSON object) as `crash_record`, re-checked by shape.
/// `false` when the bundle could not be read, is not a JSON object, or could not be written back.
pub fn attach_to_bundle(bundle: &Path, record: &CrashRecord) -> bool {
    let Ok(raw) = fs::read(bundle) else {
        return false;
    };
    let Ok(serde_json::Value::Object(mut map)) = serde_json::from_slice::<serde_json::Value>(&raw) else {
        return false;
    };
    map.insert(
        "crash_record".to_string(),
        serde_json::Value::String(reduce(&record.text)),
    );
    match serde_json::to_vec_pretty(&serde_json::Value::Object(map)) {
        Ok(bytes) => fs::write(bundle, bytes).is_ok(),
        Err(_) => false,
    }
}

/// The record found at start, before this run could write one ([`remember_pending`]); the window takes it once.
static AT_START: std::sync::Mutex<Option<CrashRecord>> = std::sync::Mutex::new(None);

/// Reads what a previous run left in `dir`, once, at start (right after the file log is installed).
pub fn remember_pending(dir: &Path) {
    let found = take_pending(dir);
    *AT_START.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = found;
}

/// The record found at start, handed out once.
pub fn take_at_start() -> Option<CrashRecord> {
    AT_START
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

/// Removes the record (after it was reported or dismissed).
#[cfg_attr(not(test), allow(dead_code))] // no caller outside the tests yet
pub fn clear(dir: &Path) {
    let _ = fs::remove_file(dir.join(CRASH_FILE_NAME));
}

/// Installs the panic hook. The log folder is looked up when the panic happens (it is set once the file log is
/// installed); before that a panic writes no record. The previous hook still runs.
pub fn install_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = super::dir() {
            let when = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            write(dir, &record_for(info, &when));
        }
        previous(info);
    }));
}

/// The record of one panic: its location and thread only; the message (`info.payload()`) is never read.
pub fn record_for(info: &std::panic::PanicHookInfo<'_>, when: &str) -> String {
    let (file, line) = info.location().map_or(("", 0), |l| (l.file(), l.line()));
    render(
        env!("CARGO_PKG_VERSION"),
        std::thread::current().name(),
        file,
        line,
        when,
    )
}
