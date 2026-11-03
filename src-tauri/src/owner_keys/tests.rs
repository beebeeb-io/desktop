//! Owner-key tests, split by area. `harness` holds the HTTP fixture and the shared world builders;
//! each other module owns one behaviour and opens with `use super::*` so the hub's re-exports keep
//! every name resolving exactly as it did before the split.
//!
//! Owns: the module list, the inherited imports, and the re-export globs.
//! Must not: define a test or a helper; a new area is a new child module.
//! Tests: every child below.

use super::*;
use crate::state_db::{FileEntry, FileStatus, ItemKind};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

mod custody;
mod harness;
mod index_repair;
mod ingestion;
mod invites;
mod migration;
mod owner_copy;
mod prepare;
mod request;

use harness::*;
