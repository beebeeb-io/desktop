//! Every operating system without a sandbox module: nothing is ever started.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use crate::{Access, ExitStatus, Granted, LaunchGuard, Limits, Pipes, Profile, SandboxError};

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

    pub(crate) fn grant(&self, _section: &SectionInner, _access: Access) -> Result<Granted, SandboxError> {
        match *self {}
    }

    pub(crate) fn handle_count(&self) -> Result<u32, SandboxError> {
        match *self {}
    }

    pub(crate) fn granted_bytes(&self) -> &Arc<AtomicU64> {
        match *self {}
    }

    pub(crate) fn killed_for_memory(&self) -> bool {
        match *self {}
    }
}

/// No value of this type exists: a section cannot be made on these systems.
pub(crate) enum SectionInner {}

impl SectionInner {
    pub(crate) fn new(_len: usize) -> Result<Self, SandboxError> {
        Err(SandboxError::Unsupported)
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        match *self {}
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        match *self {}
    }

    pub(crate) fn len(&self) -> usize {
        match *self {}
    }

    pub(crate) fn copy_into(&self, _out: &mut [u8]) {
        match *self {}
    }
}

/// No value of this type exists: nothing is mapped on these systems.
pub(crate) enum MappedInner {}

impl MappedInner {
    pub(crate) fn as_slice(&self) -> &[u8] {
        match *self {}
    }

    pub(crate) fn as_mut_slice(&mut self) -> Option<&mut [u8]> {
        match *self {}
    }
}

pub(crate) fn map(_wire: u64, _len: usize, _access: Access) -> Result<MappedInner, SandboxError> {
    Err(SandboxError::Unsupported)
}

pub(crate) fn remove_left_over_profiles() {}

/// Nothing is started; the launch lock is released on return.
pub(crate) fn spawn(
    _helper_exe: &Path,
    _limits: Limits,
    _profile: Profile,
    _mode: crate::Mode,
    _launch: LaunchGuard,
) -> Result<(ChildInner, Pipes), SandboxError> {
    Err(SandboxError::Unsupported)
}
