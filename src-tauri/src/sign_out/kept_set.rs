//! Spec 2026-10-10 W6 (task 1887): what the keep step kept, by op id, for the purge guard.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeptSet {
    pub copied: BTreeSet<String>,
    pub covered: BTreeSet<String>,
    pub missing_acknowledged: BTreeSet<String>,
    pub others_acknowledged: BTreeSet<String>,
    /// The event folder, when a file was copied.
    pub folder: Option<PathBuf>,
    pub files: usize,
}

impl KeptSet {
    /// Copied, covered, or a missing write the person was shown: the purge may delete it.
    #[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 6
    pub fn keeps(&self, op_id: &str) -> bool {
        self.copied.contains(op_id) || self.covered.contains(op_id) || self.missing_acknowledged.contains(op_id)
    }
}

/// What the keep step may let go without a copy (W6, W7, W12).
#[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 7
#[derive(Debug, Clone, Copy)]
pub enum Acknowledge<'a> {
    /// The ask snapshot the gate accepted: its other changes and its missing writes.
    Asked {
        others: &'a BTreeSet<String>,
        missing: &'a BTreeSet<String>,
    },
    /// No snapshot (the gate counted nothing): nothing but copies.
    Nothing,
    /// The R10 reset asks nobody (W12).
    #[cfg_attr(not(test), allow(dead_code))] // first production caller: Task 14
    Everything,
}

impl Acknowledge<'_> {
    pub fn other(&self, op_id: &str) -> bool {
        match self {
            Acknowledge::Asked { others, .. } => others.contains(op_id),
            Acknowledge::Nothing => false,
            Acknowledge::Everything => true,
        }
    }

    pub fn missing(&self, op_id: &str) -> bool {
        match self {
            Acknowledge::Asked { missing, .. } => missing.contains(op_id),
            Acknowledge::Nothing => false,
            Acknowledge::Everything => true,
        }
    }
}
