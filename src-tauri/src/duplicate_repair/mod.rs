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

// The host runs on Windows only; elsewhere the module is built for its tests.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use crate::lockfile::LockFile;

pub(crate) mod apply;
pub(crate) mod compare;
pub(crate) mod echo;
pub(crate) mod host;
pub(crate) mod list;
pub(crate) mod model;
pub(crate) mod server;

#[cfg(test)]
mod apply_tests;
#[cfg(test)]
mod compare_tests;
#[cfg(test)]
mod echo_tests;
#[cfg(test)]
mod host_tests;
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

const USAGE: &str = "usage: duplicate_repair list | server-read | compare --download-ok <count> | retention <days> | \
                     apply-one <pair> | check-echo <pair>";

/// One run of the tool: a phase of the spec (section 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    List,
    ServerRead,
    /// `download_ok` must equal the number of pairs waiting for a compare (spec A4).
    Compare { download_ok: usize },
    /// OP3: the trash retention the person read in the admin panel, in days.
    Retention { days: u32 },
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
        ["retention", days] => Command::Retention {
            days: days
                .parse()
                .ok()
                .filter(|days| *days > 0)
                .ok_or_else(|| anyhow::anyhow!("retention takes the days the admin panel shows. {USAGE}"))?,
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
        windows_run(command)
    }
}

/// The app's identifier: Tauri's app-local-data dir, which holds the state DB, is named after it
/// (`state_paths::init_from_app`).
#[cfg(target_os = "windows")]
const APP_IDENTIFIER: &str = "io.beebeeb.app";

/// The out dir on the person's PC (spec section 5): the run file, the list, the backups, the scratch copy.
#[cfg(target_os = "windows")]
const OUT_DIR: &str = r"C:\bb-win\diag\2274";

/// The Windows host: the app's own settings, state dir, sync folder and stored session (spec section 5, OP2). The
/// session is read in-process, only when a phase first calls the server and after the app's lock is held; it is never
/// printed, written or passed on.
#[cfg(target_os = "windows")]
fn windows_run(command: Command) -> anyhow::Result<i32> {
    let config =
        crate::config::DesktopConfig::load().map_err(|_| anyhow::anyhow!("The app's settings could not be read."))?;
    let root = config.sync_root.clone().ok_or_else(|| anyhow::anyhow!("The app has no sync folder set on this PC."))?;
    let state_dir = dirs::data_local_dir()
        .ok_or_else(|| anyhow::anyhow!("The app's state folder could not be found."))?
        .join(APP_IDENTIFIER);
    crate::state_paths::init_for_tool(state_dir.clone());
    let db_path = crate::state_paths::state_db_path_from_state_dir(&state_dir);
    let out_dir = std::path::PathBuf::from(OUT_DIR);
    std::fs::create_dir_all(&out_dir).map_err(|_| anyhow::anyhow!("The out dir could not be created."))?;
    let account = config.account_id.clone();
    let email = config.last_signed_in_email.clone();
    let api = move || -> anyhow::Result<server::RepairApi> {
        let account = account.as_deref().ok_or_else(|| anyhow::anyhow!("The app has no account on this PC."))?;
        let mut session = crate::load_session_from_keychain(account, email.clone())
            .map_err(|_| anyhow::anyhow!("The app's stored session could not be read."))?
            .ok_or_else(|| anyhow::anyhow!("The app has no stored session on this PC; sign in to the app first."))?;
        let token = std::mem::take(&mut session.token);
        let master_key = session.master_key;
        zeroize::Zeroize::zeroize(&mut session.master_key);
        Ok(server::RepairApi::new(crate::api_client::ApiClient::new(crate::runner::api_base_url(), token, master_key)))
    };
    let lock = || claim_app_lock().map(|lock| Box::new(lock) as Box<dyn std::any::Any>);
    let env = host::Env {
        db_path: &db_path,
        root: &root,
        out_dir: &out_dir,
        observe: &list::observe,
        lock: &lock,
        api: &api,
        echo_wait: std::time::Duration::from_secs(180),
        echo_poll: std::time::Duration::from_secs(5),
    };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let (code, screen) = runtime.block_on(host::execute(command, &env))?;
    println!("{screen}");
    Ok(code)
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
