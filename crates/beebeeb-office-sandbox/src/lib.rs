//! Starts the Office helper process confined.
//!
//! The helper parses files nobody trusts. This crate starts it so that it
//!
//! 1. cannot open a network connection,
//! 2. cannot write a file anywhere and cannot read the user's files,
//! 3. cannot start another process,
//! 4. is gone when its parent is gone, and can be killed by the parent at any time.
//!
//! The only things handed to the helper are two unnamed pipes (its standard input and output). It gets no
//! arguments, no inherited environment and no other handle. Document bytes travel over the pipes (or, later, over
//! memory the parent shares); they are never written to a file for it.
//!
//! **Fail closed.** There is no unconfined way to start the helper through this crate. On an operating system
//! without a sandbox module here (everything but Windows today) [`Sandbox::spawn`] returns
//! [`SandboxError::Unsupported`] and starts nothing. On Windows the helper is created suspended, the confinement is
//! read back from the new process, and only then does it run; any failure on the way ends the process.
//!
//! Errors carry a step name and an operating-system code, never a path, a name or file content.

#![deny(unsafe_code)]

use std::fmt;
use std::fs::File;
use std::path::Path;
use std::time::Duration;

mod block;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;

#[cfg(not(windows))]
mod unsupported;
#[cfg(not(windows))]
use unsupported as platform;

/// The viewer contract's cap on the helper's committed memory: 1 GiB.
pub const DEFAULT_MEMORY_LIMIT_BYTES: u64 = 1024 * 1024 * 1024;

/// The exit code a helper has after [`Child::kill`] (or after its `Child` was dropped).
pub const KILLED_EXIT_CODE: u32 = 0xBEEB_DEAD;

/// Resource limits of one helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Committed memory the helper may hold, in bytes. An allocation beyond it fails inside the helper.
    pub memory_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: DEFAULT_MEMORY_LIMIT_BYTES,
        }
    }
}

/// Why a helper was not started, or why an operation on it failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxError {
    /// This operating system has no sandbox module in this crate. Nothing was started.
    Unsupported,
    /// The helper path was refused before anything was started (the reason is a fixed text, never the path).
    HelperPath(&'static str),
    /// A limit was refused before anything was started.
    Limits(&'static str),
    /// An operating-system call failed. `step` is a fixed name of ours, `code` the system's error code.
    Os {
        /// Which step failed.
        step: &'static str,
        /// The operating system's error code.
        code: u32,
    },
    /// The new process did not have the confinement that was asked for. It was ended before it ran.
    NotConfined(&'static str),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => write!(f, "no sandbox on this operating system; the helper was not started"),
            Self::HelperPath(why) => write!(f, "helper path refused: {why}"),
            Self::Limits(why) => write!(f, "limits refused: {why}"),
            Self::Os { step, code } => write!(f, "sandbox step '{step}' failed with system code {code}"),
            Self::NotConfined(what) => write!(f, "the helper was not confined ({what}); it was ended before it ran"),
        }
    }
}

impl std::error::Error for SandboxError {}

/// How a helper ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    code: u32,
}

impl ExitStatus {
    /// The process exit code.
    pub fn code(&self) -> u32 {
        self.code
    }

    /// True when the helper was ended by [`Child::kill`] or by dropping its [`Child`].
    pub fn was_killed(&self) -> bool {
        self.code == KILLED_EXIT_CODE
    }
}

/// The two unnamed pipes between the parent and the helper.
#[derive(Debug)]
pub struct Pipes {
    /// Bytes written here arrive on the helper's standard input.
    pub to_helper: File,
    /// The helper's standard output. A read returns end-of-file once the helper is gone.
    pub from_helper: File,
}

/// Starts confined helpers.
#[derive(Debug, Clone, Copy)]
pub struct Sandbox {
    limits: Limits,
}

impl Sandbox {
    /// A sandbox with these limits. Refuses a memory limit of zero or one the platform cannot express.
    pub fn new(limits: Limits) -> Result<Self, SandboxError> {
        if limits.memory_bytes == 0 {
            return Err(SandboxError::Limits("the memory limit is zero"));
        }
        if usize::try_from(limits.memory_bytes).is_err() {
            return Err(SandboxError::Limits("the memory limit does not fit this platform"));
        }
        Ok(Self { limits })
    }

    /// The limits helpers of this sandbox get.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Starts `helper_exe` confined and returns it with its pipes.
    ///
    /// `helper_exe` must be an absolute path to a file; nothing is searched and no shell is involved. The helper
    /// gets no arguments. Its environment is exactly [`helper_environment_names`]. Its standard error is not
    /// connected.
    ///
    /// Never starts an unconfined process: see the crate documentation.
    pub fn spawn(&self, helper_exe: &Path) -> Result<Child, SandboxError> {
        let (inner, pipes) = platform::spawn(helper_exe, self.limits)?;
        Ok(Child {
            inner,
            pipes: Some(pipes),
        })
    }
}

/// The names of the environment variables a helper gets (the values come from the system, not from the parent's
/// environment). Nothing else is passed on.
pub fn helper_environment_names() -> &'static [&'static str] {
    block::ENVIRONMENT_NAMES
}

/// A running (or ended) confined helper. Dropping it kills the helper.
#[derive(Debug)]
pub struct Child {
    inner: platform::ChildInner,
    pipes: Option<Pipes>,
}

impl Child {
    /// The helper's process id.
    pub fn id(&self) -> u32 {
        self.inner.id()
    }

    /// The pipes, once. Later calls return `None`.
    pub fn take_pipes(&mut self) -> Option<Pipes> {
        self.pipes.take()
    }

    /// `Some` when the helper has ended, without waiting.
    pub fn try_wait(&self) -> Result<Option<ExitStatus>, SandboxError> {
        self.inner.wait(Duration::ZERO)
    }

    /// Waits at most `bound` for the helper to end. `None` means it is still running: the caller decides whether
    /// that is a hang, and calls [`Child::kill`].
    pub fn wait_timeout(&self, bound: Duration) -> Result<Option<ExitStatus>, SandboxError> {
        self.inner.wait(bound)
    }

    /// Ends the helper now. It is not asked. Fine to call on a helper that already ended.
    pub fn kill(&self) -> Result<(), SandboxError> {
        self.inner.kill()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_memory_limit_is_refused() {
        assert_eq!(
            Sandbox::new(Limits { memory_bytes: 0 }).unwrap_err(),
            SandboxError::Limits("the memory limit is zero")
        );
    }

    #[test]
    fn the_default_limit_is_the_contracts_one_gibibyte() {
        assert_eq!(Limits::default().memory_bytes, 1 << 30);
        assert_eq!(Sandbox::new(Limits::default()).unwrap().limits().memory_bytes, 1 << 30);
    }

    #[test]
    fn an_error_text_carries_a_step_and_a_code_and_nothing_else() {
        let text = SandboxError::Os {
            step: "create process",
            code: 5,
        }
        .to_string();
        assert_eq!(text, "sandbox step 'create process' failed with system code 5");
    }

    #[test]
    fn a_killed_helper_is_told_apart_from_one_that_exited() {
        assert!(ExitStatus { code: KILLED_EXIT_CODE }.was_killed());
        assert!(!ExitStatus { code: 0 }.was_killed());
        assert!(!ExitStatus { code: 1 }.was_killed());
    }

    #[cfg(not(windows))]
    #[test]
    fn without_a_sandbox_module_nothing_is_started() {
        let sandbox = Sandbox::new(Limits::default()).unwrap();
        // An existing, runnable program: were there an unconfined fallback, this would start it.
        let exe = std::env::current_exe().unwrap();
        assert_eq!(sandbox.spawn(&exe).unwrap_err(), SandboxError::Unsupported);
    }
}
