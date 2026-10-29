//! Task 2274: the duplicates repair tool. desktop 0.8.18 left paths with two live server files: A, the copy this PC's
//! file is bound to, and B, the file that was already there. The tool lists those pairs, compares bytes on the
//! person's OK, and for each pair the person marks it forgets B's row on this PC and then moves B to Beebeeb's trash,
//! one pair at a time (spec and lead rulings: workspace
//! `docs/superpowers/specs/2026-10-29-2274-duplicates-repair-design.md`).
//!
//! Zero-knowledge (spec A3, I5): the tool prints counts, classes and pair numbers only. Names, paths and file ids go
//! only to the run file in the out dir on this PC. The host installs no tracing subscriber, so no module's log line
//! reaches the screen either.
//!
//! Never in a shipped build: the module exists only with the `duplicate-repair` feature.

use crate::lockfile::LockFile;

pub(crate) mod compare;
pub(crate) mod list;
pub(crate) mod model;
pub(crate) mod server;

#[cfg(test)]
mod compare_tests;
#[cfg(test)]
mod list_tests;
#[cfg(test)]
mod server_tests;
#[cfg(test)]
mod tests;

/// The refusal while the app holds its sync lock (T15). Fixed text: the lock's own error names the state dir.
pub(crate) const APP_RUNNING: &str = "The Beebeeb app is running. Quit it from the tray first, then run this again.";

/// The agent name this tool writes into the app's sync lock while it holds it.
const AGENT: &str = "duplicate-repair";

const USAGE: &str = "usage: duplicate_repair list | server-read | compare --download-ok <count> | apply-one <pair> | \
                     check-echo <pair>";

/// One run of the tool: a phase of the spec (section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    List,
    ServerRead,
    /// `download_ok` must equal the number of pairs waiting for a compare (spec A4).
    Compare { download_ok: usize },
    ApplyOne { pair: u32 },
    CheckEcho { pair: u32 },
}

pub(crate) fn parse(args: &[String]) -> anyhow::Result<Command> {
    let pair = |raw: &str| match raw.parse::<u32>() {
        Ok(number) if number > 0 => Ok(number),
        _ => Err(anyhow::anyhow!("a pair number is a whole number from 1. {USAGE}")),
    };
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    Ok(match words.as_slice() {
        ["list"] => Command::List,
        ["server-read"] => Command::ServerRead,
        ["compare", "--download-ok", count] => Command::Compare {
            download_ok: count
                .parse()
                .map_err(|_| anyhow::anyhow!("--download-ok takes the number of downloads you agreed to. {USAGE}"))?,
        },
        ["apply-one", number] => Command::ApplyOne { pair: pair(number)? },
        ["check-echo", number] => Command::CheckEcho { pair: pair(number)? },
        _ => anyhow::bail!(USAGE),
    })
}

/// The host's entry point: the exit code, or the error to print.
pub fn run(args: Vec<String>) -> anyhow::Result<i32> {
    let command = parse(&args)?;
    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
        anyhow::bail!("Windows only: the pairs this tool repairs exist only in the Windows app's sync folder.")
    }
    #[cfg(target_os = "windows")]
    {
        anyhow::bail!("not built yet: {command:?}")
    }
}

/// The app's sync lock, held for the whole of a server phase so the app cannot start syncing meanwhile (T15). Refused
/// while the app holds it; the refusal names no path.
pub(crate) fn claim_app_lock() -> anyhow::Result<LockFile> {
    LockFile::acquire(AGENT).map_err(|error| {
        if error.contains("already syncing") {
            anyhow::anyhow!(APP_RUNNING)
        } else {
            anyhow::anyhow!("Could not claim the app's sync lock.")
        }
    })
}
