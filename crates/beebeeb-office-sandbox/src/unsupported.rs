//! Every operating system without a sandbox module: nothing is ever started.

use std::path::Path;
use std::time::Duration;

use crate::{ExitStatus, Limits, Pipes, SandboxError};

/// No value of this type exists: a `Child` cannot be made on these systems.
#[derive(Debug)]
pub(crate) enum ChildInner {}

impl ChildInner {
    pub(crate) fn id(&self) -> u32 {
        match *self {}
    }

    pub(crate) fn wait(&self, _bound: Duration) -> Result<Option<ExitStatus>, SandboxError> {
        match *self {}
    }

    pub(crate) fn kill(&self) -> Result<(), SandboxError> {
        match *self {}
    }
}

pub(crate) fn spawn(_helper_exe: &Path, _limits: Limits) -> Result<(ChildInner, Pipes), SandboxError> {
    Err(SandboxError::Unsupported)
}
