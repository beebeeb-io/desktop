//! `.beebeeb-sync.lock` — mutual exclusion between sync agents.
//!
//! Two Beebeeb agents can be active on a single user's machine at the
//! same time:
//!
//! - The desktop daemon (this binary), which runs continuously and
//!   syncs a single root folder
//! - The CLI's `bb watch`, which can be invoked from a terminal against
//!   any folder
//!
//! If both pointed at the same folder, the loopback registries are
//! process-local and they would echo each other's writes indefinitely.
//! The fix is a small advisory lock in Beebeeb's app-local state directory:
//! the first agent to start writes `.beebeeb-sync.lock` containing its
//! `{pid, agent, hostname, started_at}`. The second agent reads the file,
//! checks if the recorded pid is still alive, and either:
//!
//! - aborts with a clear "already running as <agent>" message, or
//! - overwrites the lock if the pid is dead (crash recovery)
//!
//! On graceful shutdown the lock file is deleted via the `Drop` impl.
//! After a hard kill, the next start sees a dead pid and recovers.
//!
//! The CLI repo carries a parallel implementation that follows the
//! same file format — this is by design (spec 030 §3 explicitly
//! permits duplication over a shared crate for ~80 LOC).
//!
//! ## Liveness check
//!
//! - **Unix:** `libc::kill(pid, 0)` — returns success if the process
//!   exists (even if owned by another user, in which case errno is
//!   `EPERM`, not `ESRCH`).
//! - **Windows:** conservatively returns `true` (lock is not broken).
//!   A user with a stale lock from a crashed process must delete the
//!   file by hand. TODO: switch to `OpenProcess` + `GetExitCodeProcess`
//!   once we ship the Windows installer.

use std::fs;
use std::path::PathBuf;
use std::process;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// On-disk JSON shape. Stable across releases — both the desktop and
/// the CLI must serialize/deserialize the same field names.
#[derive(Debug, Serialize, Deserialize)]
struct LockContents {
    pid: u32,
    agent: String,
    hostname: String,
    started_at: DateTime<Utc>,
}

/// Owned guard: deletes the lock file when dropped. Construct via
/// [`LockFile::acquire`].
pub struct LockFile {
    path: PathBuf,
}

impl LockFile {
    /// Try to claim `.beebeeb-sync.lock` inside the app-local state dir for
    /// `agent`. Returns:
    ///
    /// - `Ok(LockFile)` if no lock existed, or the prior holder is
    ///   dead and we successfully overwrote.
    /// - `Err(...)` if the prior holder is still alive — message
    ///   includes the recorded agent and pid so the operator can
    ///   identify the conflict.
    pub fn acquire(agent: &str) -> Result<Self, String> {
        let state_dir = crate::state_paths::beebeeb_state_dir()?;
        fs::create_dir_all(&state_dir).map_err(|e| format!("create lock dir {}: {e}", state_dir.display()))?;
        let path = state_dir.join(crate::state_paths::LOCK_FILENAME);

        // If a prior lock exists, decide whether to break it.
        if path.exists() {
            let raw = fs::read_to_string(&path).map_err(|e| format!("read existing lock {}: {e}", path.display()))?;
            match serde_json::from_str::<LockContents>(&raw) {
                Ok(prior) => {
                    if pid_alive(prior.pid) {
                        return Err(format!(
                            "another Beebeeb agent is already syncing this folder: \
                             {agent_name} (pid {pid} on {host}, since {since})",
                            agent_name = prior.agent,
                            pid = prior.pid,
                            host = prior.hostname,
                            since = prior.started_at.to_rfc3339(),
                        ));
                    }
                    // Stale lock from a dead pid — recover.
                    tracing::warn!(
                        prior_pid = prior.pid,
                        prior_agent = prior.agent,
                        "stale .beebeeb-sync.lock found; previous agent crashed — overwriting"
                    );
                }
                Err(e) => {
                    // Corrupt JSON — we can't reason about who holds
                    // it, but a corrupt file probably came from a
                    // crash too. Log and overwrite rather than refuse.
                    tracing::warn!(
                        error = %e,
                        path = %path.display(),
                        "lock file is corrupt; overwriting"
                    );
                }
            }
        }

        let contents = LockContents {
            pid: process::id(),
            agent: agent.to_string(),
            hostname: hostname_or_unknown(),
            started_at: Utc::now(),
        };
        let json = serde_json::to_string_pretty(&contents).map_err(|e| format!("serialize lock: {e}"))?;
        fs::write(&path, json).map_err(|e| format!("write lock {}: {e}", path.display()))?;

        Ok(Self { path })
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        // Best-effort: a failed delete just leaves a stale lock that
        // the next start recovers from.
        if let Err(e) = fs::remove_file(&self.path) {
            tracing::warn!(
                error = %e,
                path = %self.path.display(),
                "failed to delete sync lock file on shutdown"
            );
        }
    }
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // SAFETY: `kill(pid, 0)` is safe — it sends no signal, only
    // checks deliverability. Returns 0 if the process exists.
    // ESRCH (3) means no such process; EPERM (1) means it exists
    // but we don't own it (still alive for our purposes).
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if r == 0 {
        return true;
    }
    let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    errno == libc::EPERM
}

#[cfg(not(unix))]
fn pid_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    // SAFETY: `OpenProcess` returns either a valid handle or an error; we
    // only touch the handle after a successful open, and always close it
    // before returning. `GetExitCodeProcess` writes a single u32 into a
    // stack local we own. No pointers escape this function.
    unsafe {
        // PROCESS_QUERY_LIMITED_INFORMATION is the least-privileged right
        // that still lets us read the exit code, and (unlike
        // PROCESS_QUERY_INFORMATION) succeeds across integrity levels —
        // important since a CLI `bb watch` and the desktop daemon may run
        // at different elevations.
        let handle = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(h) => h,
            // Open failed: the process is gone (ERROR_INVALID_PARAMETER) or
            // we can't touch it. Either way we can't prove it's alive — and
            // the dominant cause after a crash is "pid no longer exists", so
            // treat the lock as breakable. Worst case (access-denied on a
            // live foreign-user pid) we re-acquire and the on-disk format's
            // single-writer assumption is already per-user.
            Err(_) => return false,
        };

        let mut exit_code: u32 = 0;
        let alive = match GetExitCodeProcess(handle, &mut exit_code) {
            // A still-running process reports STILL_ACTIVE (259). Any other
            // exit code means it has terminated — the lock is stale.
            Ok(()) => exit_code == STILL_ACTIVE.0 as u32,
            // If we can't read the exit code, fall back to "exists" — the
            // handle opened, so the pid is at least valid right now.
            Err(_) => true,
        };

        let _ = CloseHandle(handle);
        alive
    }
}

fn hostname_or_unknown() -> String {
    hostname::get()
        .ok()
        .and_then(|s| s.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string())
}
