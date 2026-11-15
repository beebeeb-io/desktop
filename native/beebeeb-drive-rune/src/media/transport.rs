//! The confined helper and its two pipes: [`HelperLink`] starts `beebeeb-media-helper` through the sandbox crate
//! (`beebeeb-office-sandbox`: an AppContainer with no capabilities, in a job object of [`HELPER_LIMITS`], gone with
//! this app; fail closed) and speaks the protocol on its standard input and output, every message checked by the
//! protocol's decoder. There is no other way to start the helper: no unconfined path, no flag.
//!
//! Slice M2 (`media-host`) fills it: the start, a reader thread with a channel, `recv` with its timeout, `kill`. The
//! scaffold starts nothing: [`HelperLink::start`] answers [`Refusal::NoSandbox`].

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use beebeeb_media_protocol::{FromHelper, ToHelper, WireError};
use beebeeb_office_sandbox::{Child, Limits};

use super::player::{Link, LinkFactory, Refusal};

/// The helper's committed memory: 1 GiB (decision D-MED-3).
pub const HELPER_LIMITS: Limits = Limits { memory_bytes: 1 << 30 };

/// The helper's file name, next to the app's exe.
const HELPER_NAME: &str = if cfg!(windows) {
    "beebeeb-media-helper.exe"
} else {
    "beebeeb-media-helper"
};

/// Why nothing came from the helper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// Nothing arrived within the timeout.
    Timeout,
    /// The helper is gone (its pipe ended).
    Gone,
    /// What arrived broke the protocol: the player kills the helper.
    Protocol(WireError),
}

/// One running, confined helper.
pub struct HelperLink {
    child: Option<Child>,
}

impl HelperLink {
    /// Starts the helper at `exe`, confined. Where the OS has no sandbox module, or the sandbox refuses:
    /// [`Refusal::NoSandbox`], and nothing runs.
    pub fn start(exe: &Path) -> Result<HelperLink, Refusal> {
        let _ = exe;
        Err(Refusal::NoSandbox)
    }

    pub fn send(&mut self, msg: &ToHelper) -> io::Result<()> {
        let _ = msg;
        Err(io::ErrorKind::NotConnected.into())
    }

    pub fn recv(&mut self, timeout: Duration) -> Result<FromHelper, LinkError> {
        let _ = timeout;
        Err(LinkError::Gone)
    }

    /// Ends the helper now.
    pub fn kill(&mut self) {
        if let Some(child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

impl Link for HelperLink {
    fn send(&mut self, msg: &ToHelper) -> io::Result<()> {
        HelperLink::send(self, msg)
    }

    fn recv(&mut self, timeout: Duration) -> Result<FromHelper, LinkError> {
        HelperLink::recv(self, timeout)
    }

    fn kill(&mut self) {
        HelperLink::kill(self)
    }
}

/// The helper next to the app's exe. `None`: it is not there (the player says playback isn't available).
pub fn helper_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.with_file_name(HELPER_NAME);
    exe.is_file().then_some(exe)
}

/// The real helpers: [`helper_exe`], started by [`HelperLink::start`]. The player's tests hand their own factory.
pub struct SandboxedHelpers;

impl LinkFactory for SandboxedHelpers {
    fn start(&self) -> Result<Box<dyn Link>, Refusal> {
        let exe = helper_exe().ok_or(Refusal::NotInstalled)?;
        Ok(Box::new(HelperLink::start(&exe)?))
    }
}
