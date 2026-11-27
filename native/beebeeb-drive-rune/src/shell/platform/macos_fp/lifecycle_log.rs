//! The lifecycle log (spec 2026-10-06 §8): `<home>/Library/Logs/Beebeeb/lifecycle.log`, under the app sandbox
//! `<home>` is the app container. A closed vocabulary of typed events (the engine's `LifecycleEvent`); the only free
//! text is an NSError message, which passes `redact_for_export` with the state database's known names. It is NOT a
//! `tracing` sink and holds no user content, so it survives sign-out. Before `init` every event is dropped and the
//! tail is empty. Reference: `desktop-v0.8.25:src-tauri/src/lifecycle_log.rs`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

use beebeeb_engine::diagnostic_redaction::KnownNames;
use beebeeb_engine::finder_setup::driver::{token, LifecycleEvent};
use beebeeb_engine::surfaces::phase::FinderFailureReason;

use crate::log_file::redact;

pub const FILE_NAME: &str = "lifecycle.log";
pub const MAX_BYTES: u64 = 1024 * 1024;
/// `lifecycle.log`, `.1`, `.2`.
pub const KEEP_FILES: usize = 3;

/// One line, no newline: an RFC 3339 UTC timestamp to the second, then the event.
pub fn format_line(event: &LifecycleEvent, names: &KnownNames, at: SystemTime) -> String {
    let stamp = chrono::DateTime::<chrono::Utc>::from(at).format("%Y-%m-%dT%H:%M:%SZ");
    let body = match event {
        LifecycleEvent::Launch {
            app_version,
            macos_version,
            launch_location,
        } => format!(
            "launch app={} macos={} location={}",
            token(app_version),
            token(macos_version),
            launch_location.as_str()
        ),
        LifecycleEvent::Trigger(trigger) => format!("trigger {}", trigger.as_str()),
        LifecycleEvent::Transition {
            from,
            to,
            reason,
            error,
            attempt,
            max_attempts,
        } => {
            let mut line = format!(
                "transition from={} to={} reason={} attempt={attempt}/{max_attempts}",
                from.as_str(),
                to.as_str(),
                reason.map_or("none", FinderFailureReason::as_str)
            );
            if let Some(error) = error {
                line.push_str(&format!(" domain={} code={}", token(&error.domain), error.code));
                if let Some(underlying) = &error.underlying {
                    line.push_str(&format!(
                        " underlying={}:{}",
                        token(&underlying.domain),
                        underlying.code
                    ));
                }
                let redacted = in_words(&error.message, names);
                line.push_str(&format!(" message=\"{}\"", redacted.replace(['"', '\n', '\r'], " ")));
            }
            line
        }
        LifecycleEvent::SignedIn => "signed_in".to_string(),
        LifecycleEvent::SignedOut => "signed_out".to_string(),
        LifecycleEvent::UpdateDownloaded { from, to } => {
            format!("update_downloaded from={} to={}", token(from), token(to))
        }
        LifecycleEvent::UpdateInstalled { from, to } => {
            format!("update_installed from={} to={}", token(from), token(to))
        }
    };
    format!("{stamp} {body}")
}

/// An unknown word the main log's filter always masks: it stands in for a known name.
const NAME_STANDIN: &str = "zzname";

/// An OS or bridge message in words: a word stays when it is in the main log's fixed vocabulary (or a number), every
/// other word, path, URL or secret becomes `<…>`; the state DB's known names are removed first, so a file name that is
/// also an ordinary word is masked too.
fn in_words(message: &str, names: &KnownNames) -> String {
    let mut text = message.to_string();
    for name in names.names() {
        text = replace_name(&text, name);
    }
    redact::message(&text).replace(['"', '\n', '\r'], " ")
}

/// Replace `name` (already lowercase) at word boundaries, ASCII case-insensitively.
fn replace_name(text: &str, name: &str) -> String {
    let hay = text.to_ascii_lowercase();
    if name.is_empty() || !hay.contains(name) {
        return text.to_string();
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let (mut out, mut last, mut from) = (String::new(), 0, 0);
    while let Some(rel) = hay[from..].find(name) {
        let (start, end) = (from + rel, from + rel + name.len());
        let ok = text[..start].chars().next_back().is_none_or(|c| !word(c))
            && text[end..].chars().next().is_none_or(|c| !word(c));
        if ok {
            out.push_str(&text[last..start]);
            out.push_str(NAME_STANDIN);
            last = end;
            from = end;
        } else {
            from = start + hay[start..].chars().next().map_or(1, char::len_utf8);
        }
    }
    out.push_str(&text[last..]);
    out
}

pub struct LifecycleLog {
    dir: PathBuf,
    max_bytes: u64,
}

impl LifecycleLog {
    pub fn new(dir: PathBuf, max_bytes: u64) -> Self {
        Self { dir, max_bytes }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    fn rotated(&self, n: usize) -> PathBuf {
        self.dir.join(format!("{FILE_NAME}.{n}"))
    }

    pub fn append_line(&self, line: &str) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let current = fs::metadata(self.path()).map(|m| m.len()).unwrap_or(0);
        if current > 0 && current + line.len() as u64 + 1 > self.max_bytes {
            self.rotate()?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(self.path())?;
        writeln!(file, "{line}")
    }

    fn rotate(&self) -> std::io::Result<()> {
        let oldest = self.rotated(KEEP_FILES - 1);
        if oldest.exists() {
            fs::remove_file(&oldest)?;
        }
        for n in (1..KEEP_FILES - 1).rev() {
            let from = self.rotated(n);
            if from.exists() {
                fs::rename(&from, self.rotated(n + 1))?;
            }
        }
        fs::rename(self.path(), self.rotated(1))
    }

    /// The last `n` lines across the rotated files, oldest first.
    pub fn tail(&self, n: usize) -> Vec<String> {
        let mut lines = Vec::new();
        let oldest_first = (1..KEEP_FILES)
            .rev()
            .map(|i| self.rotated(i))
            .chain(std::iter::once(self.path()));
        for path in oldest_first {
            if let Ok(text) = fs::read_to_string(&path) {
                lines.extend(text.lines().map(str::to_string));
            }
        }
        let skip = lines.len().saturating_sub(n);
        lines.split_off(skip)
    }
}

/// What `init` installs: the log, and how to read the state DB's known names.
type Installed = (LifecycleLog, fn() -> KnownNames);
/// The global is one of these; the tests build their own, so none of them touches the real one.
type Slot = Mutex<Option<Installed>>;

static GLOBAL: Slot = Mutex::new(None);

fn lock(slot: &Slot) -> MutexGuard<'_, Option<Installed>> {
    slot.lock().unwrap_or_else(|e| e.into_inner())
}

/// Called once at start-up on macOS. Before `init` (tests) `event` is a no-op.
///
/// `names` supplies the state DB's known names for redacting an NSError message. `event` calls
/// it with no lock of this module held, once per transition that carries an error, and never
/// caches its result. It must not panic and must not call `event` or `tail`. It should open its
/// own `StateDb` and add the sync root as an extra path, as the support bundle does
/// , instead of borrowing a handle from a caller
/// that may be holding its lock.
pub fn init(dir: PathBuf, names: fn() -> KnownNames) {
    *GLOBAL.lock().unwrap_or_else(|e| e.into_inner()) = Some((LifecycleLog::new(dir, MAX_BYTES), names));
}

/// `<home>/Library/Logs/Beebeeb`.
pub fn default_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library").join("Logs").join("Beebeeb"))
}

pub fn event(event: LifecycleEvent) {
    event_in(&GLOBAL, event, SystemTime::now());
}

fn event_in(slot: &Slot, event: LifecycleEvent, at: SystemTime) {
    // Known names cost a state.db read; only a transition with an NSError message needs them.
    // They are read with NO lock held: copy the `fn` pointer out under a short lock, release it,
    // call it, and take the lock again only to rotate and write. A caller-supplied `names` that
    // takes a lock the event caller already holds must not deadlock, and every other
    // `event`/`tail` must not wait behind a state.db scan. They are never cached: they go stale
    // as files sync and would span accounts.
    let known = if matches!(&event, LifecycleEvent::Transition { error: Some(_), .. }) {
        let names = {
            let guard = lock(slot);
            match guard.as_ref() {
                Some((_, names)) => *names,
                None => return,
            }
        };
        names()
    } else {
        KnownNames::new()
    };
    let guard = lock(slot);
    let Some((log, _)) = guard.as_ref() else { return };
    if let Err(error) = write_event(log, &event, &known, at) {
        tracing::warn!(%error, "could not write the lifecycle log");
    }
}

/// Dm-87: how one wait for the domain went, step by step, in its own closed shape (`wait ...`: fixed words and
/// numbers only, see [`WaitSteps::line`]). No OS text, so no names are read.
pub fn wait(steps: &WaitSteps) {
    let guard = lock(&GLOBAL);
    let Some((log, _)) = guard.as_ref() else { return };
    let stamp = chrono::DateTime::<chrono::Utc>::from(SystemTime::now()).format("%Y-%m-%dT%H:%M:%SZ");
    if let Err(error) = log.append_line(&format!("{stamp} {}", steps.line())) {
        tracing::warn!(%error, "could not write the lifecycle log");
    }
}

/// What made a wait for the domain end (dm-87).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitEnd {
    /// The domain is enabled and the extension connected to the IPC socket.
    Extension,
    /// The domain is enabled and macOS gives its root a place on disk: it serves reads.
    RootInPlace,
    /// `waitForStabilization` answered.
    Stabilized,
    /// None of these within the wait.
    TimedOut,
    /// The system answered an error that is not a timeout.
    Failed,
}

impl WaitEnd {
    pub fn as_str(self) -> &'static str {
        match self {
            WaitEnd::Extension => "ready_extension",
            WaitEnd::RootInPlace => "ready_root_in_place",
            WaitEnd::Stabilized => "ready_stabilized",
            WaitEnd::TimedOut => "timeout",
            WaitEnd::Failed => "failed",
        }
    }
}

/// How long each step of one wait for the domain took, in ms (dm-87: RC16 timed out three times and said nothing
/// of where the 60 s went).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WaitSteps {
    pub end: Option<WaitEnd>,
    pub total_ms: u64,
    /// How many times readiness was looked at.
    pub looks: u32,
    /// The slowest `getDomains` read (with its wait for the gate), and whether the last one said Enabled.
    pub domain_ms: u64,
    pub enabled: bool,
    pub extension: bool,
    /// The slowest root place read (with its wait for the gate), and whether it had one.
    pub root_ms: u64,
    pub root_in_place: bool,
    /// The `waitForStabilization` slices run, and their time together.
    pub slices: u32,
    pub stabilize_ms: u64,
    /// The working set was signalled once, to wake the extension.
    pub nudged: bool,
}

impl WaitSteps {
    pub fn line(&self) -> String {
        let yes = |b: bool| if b { "yes" } else { "no" };
        format!(
            "wait end={} total_ms={} looks={} domain_ms={} enabled={} extension={} root_ms={} root_in_place={} \
             slices={} stabilize_ms={} nudged={}",
            self.end.map_or("none", WaitEnd::as_str),
            self.total_ms,
            self.looks,
            self.domain_ms,
            yes(self.enabled),
            yes(self.extension),
            self.root_ms,
            yes(self.root_in_place),
            self.slices,
            self.stabilize_ms,
            yes(self.nudged),
        )
    }
}

pub fn tail(n: usize) -> Vec<String> {
    lock(&GLOBAL).as_ref().map(|(log, _)| log.tail(n)).unwrap_or_default()
}

/// Write one event to `log`. `event` does exactly this once `init` has run.
pub fn write_event(
    log: &LifecycleLog,
    event: &LifecycleEvent,
    names: &KnownNames,
    at: SystemTime,
) -> std::io::Result<()> {
    log.append_line(&format_line(event, names, at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use beebeeb_engine::finder_setup::driver::TAIL_LINES;
    use beebeeb_engine::finder_setup::core::Trigger;
    use beebeeb_engine::finder_setup::error::{FpError, COCOA_DOMAIN};
    use beebeeb_engine::finder_setup::launch_location::LaunchLocation;
    use beebeeb_engine::surfaces::phase::FinderSetup;
    use std::time::{Duration, UNIX_EPOCH};

    /// 2026-10-06T13:05:09Z, the moment a developer Mac's 0.8.11 saved its last Finder failure.
    fn at() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_791_291_909)
    }

    fn names(paths: &[&str]) -> KnownNames {
        let mut known = KnownNames::new();
        for path in paths {
            known.add_path(path);
        }
        known.finish()
    }

    fn transition(message: &str) -> LifecycleEvent {
        LifecycleEvent::Transition {
            from: FinderSetup::Adding,
            to: FinderSetup::Failed,
            reason: Some(FinderFailureReason::FolderTaken),
            error: Some(FpError::new(COCOA_DOMAIN, 516, message).with_underlying("NSPOSIXErrorDomain", 17)),
            attempt: 1,
            max_attempts: 1,
        }
    }

    #[test]
    fn every_event_has_one_closed_line_shape() {
        let none = KnownNames::new();
        let line = |event: LifecycleEvent| format_line(&event, &none, at());
        assert_eq!(
            line(LifecycleEvent::Launch {
                app_version: "0.8.12".into(),
                macos_version: "26.0".into(),
                launch_location: LaunchLocation::Applications
            }),
            "2026-10-06T13:05:09Z launch app=0.8.12 macos=26.0 location=applications"
        );
        assert_eq!(
            line(LifecycleEvent::Trigger(Trigger::KeysArrived)),
            "2026-10-06T13:05:09Z trigger keys_arrived"
        );
        assert_eq!(line(LifecycleEvent::SignedIn), "2026-10-06T13:05:09Z signed_in");
        assert_eq!(line(LifecycleEvent::SignedOut), "2026-10-06T13:05:09Z signed_out");
        assert_eq!(
            line(LifecycleEvent::UpdateDownloaded {
                from: "0.8.11".into(),
                to: "0.8.12-alpha.1".into()
            }),
            "2026-10-06T13:05:09Z update_downloaded from=0.8.11 to=0.8.12-alpha.1"
        );
        assert_eq!(
            line(LifecycleEvent::UpdateInstalled {
                from: "0.8.11".into(),
                to: "0.8.12-alpha.1".into()
            }),
            "2026-10-06T13:05:09Z update_installed from=0.8.11 to=0.8.12-alpha.1"
        );
        assert_eq!(
            line(transition("failed")),
            "2026-10-06T13:05:09Z transition from=adding to=failed reason=folder_taken attempt=1/1 domain=NSCocoaErrorDomain code=516 underlying=NSPOSIXErrorDomain:17 message=\"failed\""
        );
    }

    #[test]
    fn the_nserror_message_is_redacted_known_names_and_paths_never_reach_the_file() {
        let message = "could not create /Users/sam/Library/CloudStorage/Beebeeb-Beebeeb/Tax 2025/aangifte.pdf: aangifte.pdf exists";
        let line = format_line(&transition(message), &names(&["Tax 2025/aangifte.pdf"]), at());
        for leaked in [
            "/Users",
            "sam",
            "CloudStorage",
            "Beebeeb-Beebeeb",
            "Tax",
            "aangifte",
            ".pdf",
        ] {
            assert!(!line.contains(leaked), "{leaked} leaked: {line}");
        }
        assert!(line.contains('<'), "{line}");
    }

    #[test]
    fn the_rc6_timeout_line_reads_in_words() {
        let line = format_line(
            &transition("Timed out waiting for the Beebeeb File Provider domain to become available"),
            &KnownNames::new(),
            at(),
        );
        assert!(
            line.ends_with("message=\"Timed out waiting for the Beebeeb File Provider domain to become available\""),
            "{line}"
        );
    }

    #[test]
    fn a_file_name_that_is_also_an_ordinary_word_stays_redacted() {
        let message = transition("Timed out waiting for the Provider domain");
        let control = format_line(&message, &KnownNames::new(), at());
        assert!(control.contains("Provider"), "control: an ordinary word stays: {control}");
        let line = format_line(&message, &names(&["Provider"]), at());
        assert!(!line.contains("Provider"), "{line}");
        let line = format_line(&transition("could not open Letter.docx"), &KnownNames::new(), at());
        assert!(!line.contains("Letter") && !line.contains("docx"), "{line}");
    }

    #[test]
    fn free_text_cannot_enter_through_versions_or_domains() {
        let line = format_line(
            &LifecycleEvent::UpdateInstalled {
                from: "0.8.11 \"x\" /Users/sam".into(),
                to: "1".into(),
            },
            &KnownNames::new(),
            at(),
        );
        assert_eq!(line, "2026-10-06T13:05:09Z update_installed from=0.8.11xUserssam to=1");
    }

    #[test]
    fn rotates_at_one_megabyte_and_keeps_three_files() {
        let dir = tempfile::tempdir().unwrap();
        let log = LifecycleLog::new(dir.path().to_path_buf(), MAX_BYTES);
        let line = "x".repeat(1023); // 1024 bytes with the newline: 1024 lines fill one file exactly
        for _ in 0..(4 * 1024 + 10) {
            log.append_line(&line).unwrap();
        }
        let mut files: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        files.sort();
        assert_eq!(files, vec!["lifecycle.log", "lifecycle.log.1", "lifecycle.log.2"]);
        for file in &files {
            let len = fs::metadata(dir.path().join(file)).unwrap().len();
            assert!(len <= MAX_BYTES, "{file}: {len}");
        }
        assert_eq!(
            fs::metadata(dir.path().join("lifecycle.log.1")).unwrap().len(),
            MAX_BYTES
        );
        assert_eq!(fs::metadata(dir.path().join("lifecycle.log")).unwrap().len(), 10 * 1024);
    }

    #[test]
    fn tail_reads_the_last_lines_oldest_first_across_rotated_files() {
        let dir = tempfile::tempdir().unwrap();
        let log = LifecycleLog::new(dir.path().to_path_buf(), 64); // 8 lines of 8 bytes per file
        for i in 0..30 {
            log.append_line(&format!("line {i:02}")).unwrap();
        }
        assert_eq!(
            log.tail(10),
            (20..30).map(|i| format!("line {i:02}")).collect::<Vec<_>>()
        );
        assert_eq!(
            log.tail(1000).first().map(String::as_str),
            Some("line 08"),
            "lines 0..8 rotated out"
        );
    }

    /// `names` runs with NO lock held (Opus review of Task 5). A `names` that takes a lock an
    /// event caller already holds would otherwise deadlock, and every other `event`/`tail` would
    /// wait behind a full state.db scan. It also runs once per failing transition, never cached:
    /// names go stale as files sync and would span accounts. Uses a local slot, never the real
    /// global (no `--lib` test calls `init`), and `try_lock` instead of a blocking probe, so a
    /// regression is a failed assertion and not a hang.
    #[test]
    fn names_are_read_with_no_log_lock_held_and_never_cached() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static SLOT: Slot = Mutex::new(None);
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        static LOCK_FREE: AtomicUsize = AtomicUsize::new(0);
        fn names_that_probe_the_lock() -> KnownNames {
            CALLS.fetch_add(1, Ordering::SeqCst);
            if SLOT.try_lock().is_ok() {
                LOCK_FREE.fetch_add(1, Ordering::SeqCst);
            }
            names(&["Tax 2025/aangifte.pdf"])
        }
        let dir = tempfile::tempdir().unwrap();
        *lock(&SLOT) = Some((
            LifecycleLog::new(dir.path().to_path_buf(), MAX_BYTES),
            names_that_probe_the_lock,
        ));

        event_in(&SLOT, LifecycleEvent::SignedIn, at());
        assert_eq!(
            CALLS.load(Ordering::SeqCst),
            0,
            "an event with no NSError must not read the names"
        );

        event_in(&SLOT, transition("could not open Tax 2025"), at());
        event_in(&SLOT, transition("could not open Tax 2025"), at());
        assert_eq!(
            CALLS.load(Ordering::SeqCst),
            2,
            "names are read once per failing transition, never cached"
        );
        assert_eq!(
            LOCK_FREE.load(Ordering::SeqCst),
            2,
            "names() ran while the log lock was held"
        );

        let lines = lock(&SLOT).as_ref().map(|(log, _)| log.tail(10)).unwrap();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].ends_with(" signed_in"), "{lines:?}");
        for line in &lines[1..] {
            assert!(
                !line.contains("2025"),
                "the names read outside the lock were not used: {line}"
            );
        }
    }

    /// No test initialises the global, so before `init` it is empty (Windows/Linux behave so).
    #[test]
    fn before_init_events_are_dropped_and_tail_is_empty() {
        event(LifecycleEvent::SignedOut);
        assert!(tail(TAIL_LINES).is_empty());
    }
}
