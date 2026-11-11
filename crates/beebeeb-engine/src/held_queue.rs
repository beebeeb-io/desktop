//! Tasks 1888 + 2226 (spec `docs/superpowers/specs/2026-10-28-1888-2226-reconcile-then-resume-design.md`, workspace):
//! the reconcile pass over the queue, planned in memory from plain rows. Pure: no database, no disk, no network.
//!
//! A **ghost** is a folder row made on this PC (not server-known) whose create never reached the server. The same
//! folder reached the server another way, under another id: its **twin**. The plan folds each ghost into its twin
//! (re-points its children, deletes its create and its row), holds what cannot be folded safely, leaves a ghost with no
//! twin to replay, and re-arms the changes that gave up on the 404 the ghost caused. `StateDb::reconcile_held_queue`
//! applies the plan in batches of whole units.
#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::collections::{HashMap, HashSet};

#[cfg(test)]
mod tests;

mod types;
use types::*;
pub use types::{Counts, NEEDS_REVIEW, Op, Options, Row, Unit};

mod paths;
use paths::*;
pub use paths::{is_parent_404, same_leaf};

mod scaffolding;
use scaffolding::*;

mod twins;
use twins::*;

mod planning;
use planning::*;
pub use planning::{PresendBlock, plan, presend_block};
