//! The lifecycle log (spec 2026-10-06 §8). `<home>/Library/Logs/Beebeeb/lifecycle.log`. Under
//! the app sandbox `<home>` is the app container, so a shipped build writes
//! `~/Library/Containers/io.beebeeb.app/Data/Library/Logs/Beebeeb/lifecycle.log` (plan "Spec
//! issues" 4).
//!
//! A closed vocabulary of typed events. The only free text is an NSError message, which is
//! passed through `diagnostic_redaction::redact_for_export`. It is NOT a `tracing` sink:
//! nothing from the general tracing stream (engine errors embed paths and file names) reaches
//! the file. It holds no user content, so it survives sign-out.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

use crate::diagnostic_redaction::{KnownNames, redact_for_export};
use crate::finder_setup::core::Trigger;
use crate::finder_setup::error::FpError;
use crate::finder_setup::launch_location::LaunchLocation;
use crate::surfaces::phase::{FinderFailureReason, FinderSetup};

pub const FILE_NAME: &str = "lifecycle.log";
pub const MAX_BYTES: u64 = 1024 * 1024;
/// `lifecycle.log`, `.1`, `.2`.
pub const KEEP_FILES: usize = 3;
/// What "Copy details" and the 1685 support bundle include.
pub const TAIL_LINES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleEvent {
    Launch {
        app_version: String,
        macos_version: String,
        launch_location: LaunchLocation,
    },
    Trigger(Trigger),
    Transition {
        from: FinderSetup,
        to: FinderSetup,
        reason: Option<FinderFailureReason>,
        error: Option<FpError>,
        attempt: u8,
        max_attempts: u8,
    },
    /// A sign-in completed: keys arrived from a sign-in (an unlock after a Lock is not logged). No email, no account
    /// id. The next four are written by `lib.rs`: `keys_arrived`, `clear_session_impl` and the two update commands.
    SignedIn,
    SignedOut,
    UpdateDownloaded {
        from: String,
        to: String,
    },
    UpdateInstalled {
        from: String,
        to: String,
    },
}

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
                let redacted = redact_for_export(&error.message, names).text;
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

/// Versions and NSError domains are ours or Apple's, but they still pass a whitelist, so no free
/// text can slip in: `[A-Za-z0-9._-]`, at most 64 characters.
pub(crate) fn token(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .take(64)
        .collect()
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

/// Called once from `setup()` on macOS. Before `init` (tests, Windows, Linux) `event` is a no-op.
///
/// `names` supplies the state DB's known names for redacting an NSError message. `event` calls
/// it with no lock of this module held, once per transition that carries an error, and never
/// caches its result. It must not panic and must not call `event` or `tail`. It should open its
/// own `StateDb` and add the sync root as an extra path, as the support bundle does
/// (`diagnostics` in `lib.rs`, around line 4530), instead of borrowing a handle from a caller
/// that may be holding its lock.
pub fn init(dir: PathBuf, names: fn() -> KnownNames) {
    *GLOBAL.lock().unwrap_or_else(|e| e.into_inner()) = Some((LifecycleLog::new(dir, MAX_BYTES), names));
}

/// `<home>/Library/Logs/Beebeeb`.
pub fn default_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join("Library").join("Logs").join("Beebeeb"))
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
    use crate::finder_setup::error::COCOA_DOMAIN;
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
        assert!(line.contains("[path]"), "{line}");
        assert!(line.contains("[name]"), "{line}");

        // The state DB's names must reach the redactor, and the assertions above cannot tell:
        // `redact_for_export` turns any unknown word into `[name]` by itself. `2025` is a bare
        // number its allow-list keeps, so only the known-name scan can remove it. The control
        // proves the number survives without the names, so the second assertion can go red.
        let numbered = transition("could not open Tax 2025");
        let control = format_line(&numbered, &KnownNames::new(), at());
        assert!(
            control.contains("2025"),
            "control: a bare number must survive without names: {control}"
        );
        let with_names = format_line(&numbered, &names(&["Tax 2025/aangifte.pdf"]), at());
        assert!(
            !with_names.contains("2025"),
            "the known names did not reach the redactor: {with_names}"
        );
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

    /// Uses a local `LifecycleLog`, never the global: other test modules call `event()` in
    /// parallel (Task 9's keys-arrived sites), and a global initialised here would catch them.
    #[test]
    fn the_general_tracing_stream_never_reaches_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let log = LifecycleLog::new(dir.path().to_path_buf(), MAX_BYTES);
        let subscriber = tracing_subscriber::fmt().with_writer(std::io::sink).finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(
                path = "/Users/sam/Documents/secret-plan.pdf",
                "engine error while syncing secret-plan.pdf"
            );
            write_event(&log, &LifecycleEvent::SignedIn, &KnownNames::new(), at()).unwrap();
            tracing::error!("another engine error mentioning secret-plan.pdf");
        });
        let text = fs::read_to_string(log.path()).unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.trim_end().ends_with(" signed_in"), "{text}");
        assert!(!text.contains("secret-plan"), "{text}");
        // Nothing wires the file into tracing: no Layer/MakeWriter here, and run()'s tracing
        // init does not mention this module.
        // As this file was checked out: a Windows checkout has CRLF line endings.
        let module = include_str!("lifecycle_log.rs").replace("\r\n", "\n");
        let module = &module[..module.find("#[cfg(test)]\nmod tests").expect("tests module")];
        assert!(
            !module.contains("Layer") && !module.contains("MakeWriter"),
            "the lifecycle log must not be a tracing sink"
        );
        let lib = include_str!("lib.rs");
        let init = &lib[lib.find("pub fn run() {").expect("run()")..];
        let init = &init[..init.find(".try_init()").expect("tracing init")];
        assert!(!init.contains("lifecycle"), "{init}");
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
