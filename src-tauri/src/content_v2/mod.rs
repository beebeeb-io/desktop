//! Pre-P1 compiling compatibility adapter for the recorded behavioral red run.
//! This file is replaced by the guarded storage implementation after that run.
#![allow(dead_code)]
use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;
const MIB: u64 = 1024 * 1024;
const CHUNK: usize = 4 * 1024 * 1024;
fn schema_open(path: &Path) -> Result<Connection> {
    Ok(Connection::open(path)?)
}
fn storage_proof(full: bool, manifest: bool, adopted: bool, bootstrap: bool) -> bool {
    let _ = (full, manifest, adopted); bootstrap
}
#[derive(Default)]
struct Admission;
impl Admission {
    fn recovered_can_submit(&self, _denied: bool) -> bool { true }
}
#[derive(Default)]
struct Budget { reserved: u64 }
impl Budget {
    fn reserve(&mut self, bytes: u64, quota: u64) -> bool {
        let _ = quota; self.reserved += bytes; true
    }
}
fn payload_admitted(wal: u64) -> bool { let _ = wal; true }
fn protect_canary(bytes: &[u8]) -> Result<Vec<u8>> { Ok(bytes.to_vec()) }
fn batch_limit(requested: usize) -> usize { requested }
mod tests;
