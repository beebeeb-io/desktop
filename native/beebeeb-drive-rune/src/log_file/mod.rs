//! The native app's local log file: what the engine and the shell say through `tracing`, kept on this computer.
//!
//! Until this file the app installed no `tracing` subscriber, so every warning of the sync runner went nowhere (the
//! the Windows test machine rung report, 2026-11-15). Now one subscriber is installed once, at the start of the one instance that is
//! the app (`shell::open_engine`), and writes one line per event of level INFO and above:
//!
//! ```text
//! 2026-11-15T07:41:09Z WARN beebeeb_engine::runner sync tick failed error=<redacted-by-shape text>
//! ```
//!
//! - **Where:** `<local data folder>/logs/beebeeb-drive.log`, the folder the app keeps its state in (the TEST
//!   identity's folder for the TEST build). The older half is `beebeeb-drive.log.1`.
//! - **Size:** two files of at most [`MAX_FILE_BYTES`] (plus one line): when the current file would pass the cap it
//!   becomes the `.1` file, the previous `.1` is deleted, and a new file starts. The oldest lines are dropped.
//! - **Privacy (hard rule):** no file or folder name, path, email address, token, key, password, phrase, code or
//!   server body reaches the file: [`redact`] drops fields by name, marks what it recognises, and then writes a word
//!   only when it is a word of the engine's own log calls, an OS error word or a number (an allow-list; every other
//!   word becomes `<…>`).
//! - **No network sink**, ever: the only writer is the file. Spans are not recorded (their fields are not read).
//! - **Never a reason not to start:** a folder that cannot be made or a write that fails loses the line and nothing
//!   else; the window and stderr behave as before (no subscriber wrote to them).
//!
//! It is a `tracing::Subscriber` of its own (no `tracing-subscriber`: no new crate is built, and every field passes
//! through one visitor that this file owns).

mod redact;
#[cfg(test)]
mod tests;

use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

/// The cap of one log file; two files exist at most.
pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
/// The folder under the local data folder, and the two file names.
pub const DIR_NAME: &str = "logs";
pub const FILE_NAME: &str = "beebeeb-drive.log";
const OLD_FILE_NAME: &str = "beebeeb-drive.log.1";

/// Installs the file log as the process's one `tracing` subscriber. Best effort: `false` when the folder or the file
/// could not be made, or when another subscriber was installed first; the app starts either way.
pub fn install(local_data: &Path) -> bool {
    let Some(log) = AppLog::open(&local_data.join(DIR_NAME), MAX_FILE_BYTES) else {
        return false;
    };
    let ok = tracing::subscriber::set_global_default(log).is_ok();
    if ok {
        let _ = LOG_DIR.set(local_data.join(DIR_NAME));
    }
    ok
}

static LOG_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// The folder the log is written to; `None` when no file log is installed in this process.
pub fn dir() -> Option<&'static Path> {
    LOG_DIR.get().map(PathBuf::as_path)
}

/// The subscriber: level filter, redaction, the rotating file.
pub struct AppLog {
    sink: Mutex<Sink>,
    next_span: AtomicU64,
}

impl AppLog {
    /// Opens (appending to) the current log file in `dir`, creating `dir`. `None` when that is not possible.
    pub fn open(dir: &Path, max_bytes: u64) -> Option<Self> {
        Self::open_with(dir, max_bytes, FileOps::OS)
    }

    /// [`open`](Self::open) with the two file operations a rotation uses (a test seam: see [`FileOps`]).
    fn open_with(dir: &Path, max_bytes: u64, ops: FileOps) -> Option<Self> {
        fs::create_dir_all(dir).ok()?;
        let sink = Sink::open(dir.to_path_buf(), max_bytes, ops)?;
        Some(Self {
            sink: Mutex::new(sink),
            next_span: AtomicU64::new(1),
        })
    }
}

impl Subscriber for AppLog {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        // ERROR < WARN < INFO in `Level`'s order: this is "INFO or less verbose".
        *metadata.level() <= Level::INFO
    }
    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::INFO)
    }
    fn new_span(&self, _attributes: &Attributes<'_>) -> Id {
        Id::from_u64(self.next_span.fetch_add(1, Ordering::Relaxed))
    }
    fn record(&self, _span: &Id, _values: &Record<'_>) {}
    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
    fn enter(&self, _span: &Id) {}
    fn exit(&self, _span: &Id) {}
    fn event(&self, event: &Event<'_>) {
        if !self.enabled(event.metadata()) {
            return;
        }
        let mut visitor = Fields::default();
        event.record(&mut visitor);
        let mut line = format!(
            "{} {} {}",
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            event.metadata().level(),
            event.metadata().target(),
        );
        if let Some(message) = visitor.message {
            line.push(' ');
            line.push_str(&redact::message(&message));
        }
        for (name, shown) in visitor.fields {
            line.push(' ');
            line.push_str(&name);
            line.push('=');
            line.push_str(&shown);
        }
        line.push('\n');
        // A poisoned lock still holds a usable file; a failed write loses this line only.
        let mut sink = self.sink.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        sink.write_line(&line);
    }
}

/// Reads an event's fields: the message apart, every other field named and redacted.
#[derive(Default)]
struct Fields {
    message: Option<String>,
    fields: Vec<(String, String)>,
}

impl Fields {
    fn text(&mut self, field: &Field, text: String) {
        if field.name() == "message" {
            self.message = Some(text);
        } else if !field.name().starts_with("log.") {
            let shown = if redact::denied_name(field.name()) {
                redact::REDACTED.to_string()
            } else {
                redact::value(field.name(), &text)
            };
            self.fields.push((field.name().to_string(), shown));
        }
    }
    fn number(&mut self, field: &Field, shown: String) {
        if field.name() == "message" || field.name().starts_with("log.") {
            return;
        }
        let shown = if redact::denied_numeric_name(field.name()) {
            redact::REDACTED.to_string()
        } else {
            redact::number(field.name(), shown)
        };
        self.fields.push((field.name().to_string(), shown));
    }
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.text(field, format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.text(field, value.to_string());
    }
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.text(field, value.to_string());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.number(field, value.to_string());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.number(field, value.to_string());
    }
    fn record_u128(&mut self, field: &Field, value: u128) {
        self.number(field, value.to_string());
    }
    fn record_i128(&mut self, field: &Field, value: i128) {
        self.number(field, value.to_string());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        // A yes or no holds no name, path or secret: kept whatever the field is called.
        if field.name() != "message" && !field.name().starts_with("log.") {
            self.fields.push((field.name().to_string(), value.to_string()));
        }
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.number(field, value.to_string());
    }
}

/// The two operations a rotation needs. Real ones in the app; a test swaps in ones that fail, to stand for a log file
/// another program holds open (on Windows, a handle without delete sharing refuses both).
#[derive(Clone, Copy)]
struct FileOps {
    rename: fn(&Path, &Path) -> std::io::Result<()>,
    remove: fn(&Path) -> std::io::Result<()>,
}

impl FileOps {
    const OS: Self = Self {
        rename: |from, to| fs::rename(from, to),
        remove: |path| fs::remove_file(path),
    };
}

/// The current file and the one rotation.
struct Sink {
    dir: PathBuf,
    max_bytes: u64,
    ops: FileOps,
    file: Option<File>,
    size: u64,
}

impl Sink {
    fn open(dir: PathBuf, max_bytes: u64, ops: FileOps) -> Option<Self> {
        let mut sink = Self {
            dir,
            max_bytes,
            ops,
            file: None,
            size: 0,
        };
        sink.start_file()?;
        Some(sink)
    }

    /// Opens the current file for appending and reads its size.
    fn start_file(&mut self) -> Option<()> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(FILE_NAME))
            .ok()?;
        self.size = file.metadata().map_or(0, |m| m.len());
        self.file = Some(file);
        Some(())
    }

    fn write_line(&mut self, line: &str) {
        let len = line.len() as u64;
        if self.size > 0 && self.size + len > self.max_bytes {
            self.rotate();
        }
        if self.file.is_none() && self.start_file().is_none() {
            return;
        }
        // Checked with the size of the file as it is now it is open (a rotation that could neither move nor delete a
        // held file, or a reopen that failed once, leaves it over the cap): the line is dropped, and the next line
        // tries the rotation again, so the cap holds in every case.
        if self.size > 0 && self.size + len > self.max_bytes {
            return;
        }
        if let Some(file) = self.file.as_mut() {
            if file.write_all(line.as_bytes()).is_ok() {
                self.size += len;
            } else {
                // The handle is not trusted after a failed write; the next line opens it again.
                self.file = None;
            }
        }
    }

    /// The current file becomes the `.1` file (the older one is deleted), and a new current file starts.
    fn rotate(&mut self) {
        self.file = None;
        let current = self.dir.join(FILE_NAME);
        let old = self.dir.join(OLD_FILE_NAME);
        let _ = (self.ops.remove)(&old);
        if (self.ops.rename)(&current, &old).is_err() {
            // Could not move it aside (a held handle on Windows): try to delete it. If that fails too, the file keeps
            // its size, `start_file` reads it, and `write_line` drops lines until a rotation works.
            let _ = (self.ops.remove)(&current);
        }
        self.size = 0;
        let _ = self.start_file();
    }
}
