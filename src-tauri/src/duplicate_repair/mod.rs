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
        // Review F8: the copy is wiped when it drops, like the session's own.
        let master_key = zeroize::Zeroizing::new(session.master_key);
        zeroize::Zeroize::zeroize(&mut session.master_key);
        Ok(server::RepairApi::new(crate::api_client::ApiClient::new(crate::runner::api_base_url(), token, *master_key)))
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

/// The app's sync lock as this tool holds it: the app's own lock file (`lockfile.rs` format), naming this process.
pub(crate) struct ToolLock {
    path: std::path::PathBuf,
}

impl Drop for ToolLock {
    /// Removes the lock only while it still names this tool's process: a lock the app wrote is the app's.
    fn drop(&mut self) {
        if holder(&self.path).is_some_and(|(pid, agent)| pid == std::process::id() && agent == AGENT) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// The pid and agent a lock file at `path` names, when it can be read as one.
fn holder(path: &std::path::Path) -> Option<(u32, String)> {
    let raw = std::fs::read_to_string(path).ok()?;
    let lock: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let pid = u32::try_from(lock.get("pid")?.as_u64()?).ok()?;
    Some((pid, lock.get("agent")?.as_str()?.to_string()))
}

#[cfg(test)]
thread_local! {
    /// Runs between the lock check and the write: a test writes the app's lock there (review P2).
    pub(super) static BEFORE_LOCK_WRITE: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

/// The app's sync lock, held for the whole of a server phase so the app cannot start syncing meanwhile (T15). Refused
/// while the app holds it, and (review P2) while any app process runs: the app holds the lock only while its runner
/// syncs, not while it is paused, signed out, locked or starting. The refusal names no path.
pub(crate) fn claim_app_lock() -> anyhow::Result<ToolLock> {
    claim_app_lock_with(&app_process_running)
}

/// [`claim_app_lock`], with the check for a running app given. The claim creates the lock file only if no one else has
/// (review P2: the check and the write are one step), so a lock the app writes after the check is never overwritten.
/// A stale lock, whose process is gone or which does not read as a lock, is removed first. The app is checked again
/// once the lock is held, in case it started meanwhile.
pub(crate) fn claim_app_lock_with(app_running: &dyn Fn() -> bool) -> anyhow::Result<ToolLock> {
    if app_running() {
        anyhow::bail!(APP_RUNNING);
    }
    let failed = |_| anyhow::anyhow!("Could not claim the app's sync lock.");
    let dir = crate::state_paths::beebeeb_state_dir().map_err(|_| anyhow::anyhow!("Could not claim the app's sync lock."))?;
    std::fs::create_dir_all(&dir).map_err(failed)?;
    let path = dir.join(crate::state_paths::LOCK_FILENAME);
    if path.exists() {
        if holder(&path).is_some_and(|(pid, _)| crate::lockfile::pid_alive(pid)) {
            anyhow::bail!(APP_RUNNING);
        }
        match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(failed(error)),
            _ => {}
        }
    }
    #[cfg(test)]
    BEFORE_LOCK_WRITE.with(|hook| {
        if let Some(hook) = hook.borrow().as_ref() {
            hook();
        }
    });
    let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => anyhow::bail!(APP_RUNNING),
        Err(error) => return Err(failed(error)),
    };
    let lock = ToolLock { path };
    std::io::Write::write_all(&mut file, lock_json().as_bytes()).map_err(failed)?;
    if app_running() {
        anyhow::bail!(APP_RUNNING);
    }
    Ok(lock)
}

/// The lock's contents, in the app's format (`lockfile.rs` `LockContents`), naming this process.
fn lock_json() -> String {
    serde_json::json!({
        "pid": std::process::id(),
        "agent": AGENT,
        "hostname": hostname::get().ok().and_then(|name| name.into_string().ok()).unwrap_or_else(|| "unknown".into()),
        "started_at": chrono::Utc::now().to_rfc3339(),
    })
    .to_string()
}

/// The app's executable, without `.exe`.
const APP_EXECUTABLE: &str = "beebeeb-desktop";

/// Whether a Beebeeb app process runs on this PC, the person's own or any other build (review P2). Reads the process
/// list only; it signals nothing.
fn app_process_running() -> bool {
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    system.processes().values().any(|process| is_app_process(process.name()))
}

/// Whether a process named `name` is the Beebeeb app (Windows names it in any case).
pub(crate) fn is_app_process(name: &std::ffi::OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name) == APP_EXECUTABLE
}
