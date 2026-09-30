//! Spec 50 revision 5, slice 1. This module is compiled ONLY by the test harness.
//! No function here grants Windows exclusion, root activation, or network authority.
#![allow(dead_code)]
use anyhow::{Context, Result, bail, ensure};
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

mod fault_vfs;
mod ledger;
mod records;
mod storage;
mod storage_tests;
mod tests;
mod wire;
use ledger::*;
use storage::*;

const MIB: u64 = 1024 * 1024;
const CHUNK: usize = 4 * 1024 * 1024;
const ROWS: usize = 256;
const DIRTY_LIMIT: u64 = 8 * MIB;
const WAL_LIMIT: u64 = 320 * MIB;
type Id = [u8; 32];
fn id() -> Id {
    let mut id = [0; 32];
    rand::rngs::OsRng.fill_bytes(&mut id);
    id
}
fn digest(bytes: &[u8]) -> Id {
    Sha256::digest(bytes).into()
}
fn hex(id: &Id) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}
fn batch_limit(requested: usize) -> usize {
    requested.min(ROWS)
}
fn payload_admitted(wal: u64) -> bool {
    wal < 128 * MIB
}
fn storage_proof(full: bool, manifest: bool, adopted: bool, bootstrap: bool) -> bool {
    full && manifest && adopted && bootstrap
}
fn protect_canary(bytes: &[u8]) -> Result<Vec<u8>> {
    ledger::encrypt_padded(&id(), &id(), &id(), "Envelope", bytes).map(|(_, b)| b)
}

/// No serialized grant is accepted. A fresh action gets a distinct token and run binding.
#[derive(Default)]
struct Admission {
    grants: std::collections::HashSet<(Id, Id, Id)>,
}
impl Admission {
    fn recovered_can_submit(&self, _denied: bool) -> bool {
        false
    }
    fn new_action(&mut self, account: Id, store: &mut Store, snapshot: &Artifact) -> Result<Id> {
        store.verify(snapshot)?;
        let (kind,body,format):(String,Vec<u8>,String)=store.db.query_row("SELECT o.kind,o.body,a.format FROM v2_owners o JOIN v2_artifacts a ON a.allocation_owner=o.owner_id WHERE a.artifact_id=?1",[snapshot.id.as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        ensure!(
            kind == "Snapshot" && format == "Full",
            "submission requires complete Snapshot"
        );
        let fields = records::decode(&kind, &body)?;
        ensure!(fields[0] == account, "authenticated account mismatch");
        let token = id();
        self.grants.insert((account, snapshot.id, token));
        Ok(token)
    }
    fn permits(&self, account: Id, snapshot: Id, token: Id, denied: bool) -> bool {
        !denied && self.grants.contains(&(account, snapshot, token))
    }
}
#[derive(Default)]
struct Budget {
    reserved: u64,
}
impl Budget {
    fn reserve(&mut self, bytes: u64, quota: u64) -> bool {
        let Some(total) = self.reserved.checked_add(bytes) else {
            return false;
        };
        if total > quota {
            return false;
        }
        self.reserved = total;
        true
    }
    fn release(&mut self, bytes: u64) -> Result<()> {
        self.reserved = self.reserved.checked_sub(bytes).context("reservation underflow")?;
        Ok(())
    }
}

fn schema_open(path: &Path) -> Result<Connection> {
    open_db(path, include_str!("account.sql"))
}
fn wal_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push("-wal");
    p.into()
}
fn file_len(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}
fn open_db(path: &Path, schema: &str) -> Result<Connection> {
    open_db_vfs(path, schema, None)
}
fn open_db_vfs(path: &Path, schema: &str, vfs: Option<&str>) -> Result<Connection> {
    let exists = path.exists();
    if exists {
        ensure!(
            !fs::symlink_metadata(path)?.file_type().is_symlink(),
            "reparse/symlink store"
        );
        ensure!(wal_path(path).exists(), "missing persistent WAL: recovery-only");
        let read = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let version: i64 = read.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(version == 1, "unknown format: recovery-only");
        let owners: bool = read.query_row("SELECT count(*)>0 FROM sqlite_schema WHERE name='v2_owners'", [], |r| {
            r.get(0)
        })?;
        if owners {
            let mut last: Vec<u8> = Vec::new();
            loop {
                let row:Option<(Vec<u8>,String,i64,Vec<u8>)>=read.query_row("SELECT owner_id,kind,body_version,body FROM v2_owners WHERE owner_id>?1 ORDER BY owner_id LIMIT 1",[&last],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
                let Some((owner, kind, version, body)) = row else { break };
                ensure!(owner.len() == 32 && version == 1, "unknown owner: recovery-only");
                records::decode(&kind, &body).context("unknown owner body: recovery-only")?;
                last = owner;
            }
        }
    }
    let db = if let Some(vfs) = vfs {
        Connection::open_with_flags_and_vfs(path, rusqlite::OpenFlags::default(), vfs)?
    } else {
        Connection::open(path)?
    };
    db.busy_timeout(std::time::Duration::from_millis(100))?;
    if !exists {
        db.execute_batch("PRAGMA page_size=4096; PRAGMA auto_vacuum=INCREMENTAL;")?;
    }
    db.execute_batch(
        "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA wal_autocheckpoint=0;",
    )?;
    // sqlite owns the WAL namespace; never unlink it to force reclamation.
    unsafe {
        let mut flag: std::os::raw::c_int = 1;
        let rc = rusqlite::ffi::sqlite3_file_control(
            db.handle(),
            c"main".as_ptr(),
            rusqlite::ffi::SQLITE_FCNTL_PERSIST_WAL,
            (&mut flag as *mut std::os::raw::c_int).cast(),
        );
        ensure!(rc == rusqlite::ffi::SQLITE_OK, "persistent WAL unsupported");
        flag = -1;
        let rc = rusqlite::ffi::sqlite3_file_control(
            db.handle(),
            c"main".as_ptr(),
            rusqlite::ffi::SQLITE_FCNTL_PERSIST_WAL,
            (&mut flag as *mut std::os::raw::c_int).cast(),
        );
        ensure!(
            rc == rusqlite::ffi::SQLITE_OK && flag == 1,
            "persistent WAL not enabled"
        );
    }
    if !exists {
        let tx = db.unchecked_transaction()?;
        tx.execute_batch(schema)?;
        tx.execute_batch("PRAGMA user_version=1")?;
        tx.commit()?;
    }
    verify_pragmas(&db)?;
    let bad: Option<String> = db.query_row("PRAGMA quick_check", [], |r| r.get(0)).optional()?;
    ensure!(bad.as_deref() == Some("ok"), "corrupt database: recovery-only");
    let violations: i64 = db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0))?;
    ensure!(violations == 0, "foreign key corruption: recovery-only");
    Ok(db)
}
fn verify_pragmas(db: &Connection) -> Result<()> {
    for (name, value) in [
        ("synchronous", 2),
        ("foreign_keys", 1),
        ("page_size", 4096),
        ("auto_vacuum", 2),
        ("wal_autocheckpoint", 0),
    ] {
        let actual: i64 = db.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))?;
        ensure!(actual == value, "invalid {name}: {actual}");
    }
    let mode: String = db.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    ensure!(mode == "wal", "not WAL");
    let disabled: i64 = db.query_row(
        "SELECT count(*) FROM pragma_compile_options WHERE compile_options LIKE '%OMIT_SYNC%'",
        [],
        |r| r.get(0),
    )?;
    ensure!(disabled == 0, "sync disabled");
    Ok(())
}
fn checkpoint(db: &Connection, path: &Path, settle: bool) -> Result<(i64, i64, i64)> {
    // The owning Store/Ledger is !Sync; its sole writer serializes checkpoint calls.
    let mode = if settle { "TRUNCATE" } else { "PASSIVE" };
    let counts: (i64, i64, i64) = db.query_row(&format!("PRAGMA wal_checkpoint({mode})"), [], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    if settle {
        ensure!(
            counts.0 == 0
                && counts.1 == counts.2
                && file_len(&wal_path(path)) <= 64 * MIB
                && allocated_len(&wal_path(path))? <= 64 * MIB,
            "pinned reader: physical WAL not reclaimed"
        );
    }
    Ok(counts)
}

/// Explicit fixture capability. Only the test harness can instantiate this type.
struct Harness {
    dir: tempfile::TempDir,
    volume: Arc<Mutex<Budget>>,
}
impl Harness {
    fn new() -> Result<Self> {
        Ok(Self {
            dir: tempfile::tempdir()?,
            volume: Arc::new(Mutex::new(Budget::default())),
        })
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// Cutpoints surround actual SQL commit boundaries; failure never removes source bytes.
#[derive(Default)]
struct Fault {
    cut: Option<usize>,
    seen: usize,
}
impl Fault {
    fn point(&mut self, name: &str) -> Result<()> {
        self.seen += 1;
        if self.cut == Some(self.seen) {
            bail!("injected cut {} at {name}", self.seen);
        }
        Ok(())
    }
}

fn allocated_len(path: &Path) -> Result<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(fs::metadata(path)?.blocks() * 512)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{FILE_STANDARD_INFO, FileStandardInfo, GetFileInformationByHandleEx},
        };
        let f = fs::File::open(path)?;
        let mut info = FILE_STANDARD_INFO::default();
        unsafe {
            GetFileInformationByHandleEx(
                HANDLE(f.as_raw_handle()),
                FileStandardInfo,
                (&mut info as *mut FILE_STANDARD_INFO).cast(),
                std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
            )?;
        }
        Ok(u64::try_from(info.AllocationSize)?)
    }
}

fn metadata_admitted(db: &Connection, path: &Path, terminal: bool) -> Result<()> {
    verify_pragmas(db)?;
    let wal = file_len(&wal_path(path));
    ensure!(terminal || wal < 256 * MIB, "non-terminal metadata backpressure");
    ensure!(
        wal.checked_add(DIRTY_LIMIT).is_some_and(|n| n <= WAL_LIMIT),
        "terminal WAL budget exhausted"
    );
    Ok(())
}

/// Actual pages written to WAL, including writes into reused capacity. Reset per
/// transaction. SQLite excludes rollback/recovery; callers only sample success.
fn take_page_write_bytes(db: &Connection) -> Result<u64> {
    let mut pages = 0;
    let mut highwater = 0;
    let result = unsafe {
        rusqlite::ffi::sqlite3_db_status(
            db.handle(),
            rusqlite::ffi::SQLITE_DBSTATUS_CACHE_WRITE,
            &mut pages,
            &mut highwater,
            1,
        )
    };
    ensure!(
        result == rusqlite::ffi::SQLITE_OK && pages >= 0,
        "page-write instrumentation unsupported"
    );
    Ok(pages as u64 * 4096)
}
