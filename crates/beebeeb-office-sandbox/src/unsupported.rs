//! Every operating system without a sandbox module: nothing is ever started.

use std::path::Path;
use std::time::Duration;

use crate::{Access, ExitStatus, Granted, Limits, Pipes, SandboxError};

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

pub(crate) fn spawn(_helper_exe: &Path, _limits: Limits) -> Result<(ChildInner, Pipes), SandboxError> {
    Err(SandboxError::Unsupported)
}
