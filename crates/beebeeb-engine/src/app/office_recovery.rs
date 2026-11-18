//! Encrypted recovery blobs for open Office documents, on this PC: one blob per open document under the account's
//! own folder in the app-data dir (`<state dir>/office-recovery/<account>/`), so the work in a document that was
//! never saved survives a crash or a power cut.
//!
//! - The engine calls core: every blob is sealed by `beebeeb_core::local_seal` under the account's key, bound to the
//!   file id (a blob of another account or another file id does not open). No crypto is written here.
//! - A blob's file name is a hash of the account's local id and the file id: it says nothing about the file. The
//!   file ids themselves are kept in one more sealed record (`index`), because the blob does not store its own id.
//! - A write goes to `<name>.new`, is synced, then renamed over `<name>`: the previous blob stays readable until
//!   the new one is complete. The index is written before the blob, so a crash leaves at worst an index line without
//!   a blob (skipped when listing), never a blob nobody can find from a first write that did land.
//! - [`Engine::put_office_snapshot_for`], [`Engine::office_snapshots_for`], [`Engine::take_office_snapshot_for`]
//!   (reads; the blob stays until [`Engine::discard_office_snapshot_for`], so a crash after a restore loses
//!   nothing), [`Engine::discard_office_snapshot_for`].
//! - Sign-out deletes every blob of the account ([`Engine::wipe_office_recovery_at_session_end`]). It is not a
//!   "waiting save" that the keep step keeps.
//!
//! BLOCKS: the disk (a blob is up to `MAX_SNAPSHOT_BYTES`). Must not: log a name, a path or a byte.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use beebeeb_core::kdf::MasterKey;
use beebeeb_core::local_seal::{
    MAX_SNAPSHOT_BYTES, SnapshotMeta, open_recovery_snapshot, peek_snapshot_meta, seal_recovery_snapshot,
};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;

const FOLDER: &str = "office-recovery";
const INDEX_NAME: &str = "index";
/// The id the index record is bound to. No file id starts with a NUL.
const INDEX_ID: &str = "\0office-recovery-index";
const NEW_SUFFIX: &str = ".new";
/// The most file ids an index holds (open documents are few; this bounds a damaged record).
const MAX_ENTRIES: usize = 4096;
/// One writer at a time over the folder (index and blob move together).
static WRITE: Mutex<()> = Mutex::new(());

/// Why a call did nothing. Never carries a name or a byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryRefused {
    /// No session, or the session moved on since the call was made.
    NotSignedIn,
    /// Larger than a snapshot may be, or not a package this knows the kind of.
    Refused,
    /// No blob for this file, or one that does not open (another account's, another file's, damaged).
    NotFound,
    /// The disk refused.
    Failed,
}

/// One document with a blob. `saved_at` and `base` are read from the blob's header for display; the blob is
/// authenticated when it is taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryEntry {
    pub file_id: String,
    pub saved_at: i64,
    pub base: i64,
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn hashed(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for part in parts {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part.as_bytes());
    }
    hex(&h.finalize())
}

/// The kind a snapshot records (1 docx, 2 xlsx, 3 pptx), by the part names in the package's directory (the end of
/// the file).
fn kind_of(package: &[u8]) -> Option<u8> {
    let tail = &package[package.len().saturating_sub(8 * 1024 * 1024)..];
    let has = |needle: &[u8]| tail.windows(needle.len()).any(|w| w == needle);
    if has(b"word/document.xml") {
        Some(1)
    } else if has(b"xl/workbook.xml") {
        Some(2)
    } else if has(b"ppt/presentation.xml") {
        Some(3)
    } else {
        None
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Where `data` goes at `path`: a new name, synced, then renamed over the old one. `before_rename` is the seam a
/// test uses to stop between the write and the rename.
fn replace_file(path: &Path, data: &[u8], before_rename: impl FnOnce() -> bool) -> std::io::Result<()> {
    let mut new = path.as_os_str().to_owned();
    new.push(NEW_SUFFIX);
    let new = PathBuf::from(new);
    {
        let mut file = OpenOptions::new().write(true).create(true).truncate(true).open(&new)?;
        file.write_all(data)?;
        file.sync_all()?;
    }
    if !before_rename() {
        return Ok(());
    }
    fs::rename(&new, path)?;
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        File::open(dir)?.sync_all()?;
    }
    Ok(())
}

/// The folder of one account and its key. Everything the engine calls does its work here.
pub(crate) struct Store<'a> {
    dir: PathBuf,
    account: String,
    master: &'a MasterKey,
}

impl<'a> Store<'a> {
    pub(crate) fn new(state_dir: &Path, account_id: &str, master: &'a MasterKey) -> Self {
        Self { dir: account_dir(state_dir, account_id), account: account_id.to_string(), master }
    }

    fn blob_path(&self, file_id: &str) -> PathBuf {
        self.dir.join(hashed(&[&self.account, file_id]))
    }

    fn read_index(&self) -> Vec<String> {
        let Ok(sealed) = fs::read(self.dir.join(INDEX_NAME)) else { return Vec::new() };
        let Ok((_, plain)) = open_recovery_snapshot(self.master, INDEX_ID, &sealed) else { return Vec::new() };
        let mut ids = Vec::new();
        let mut rest: &[u8] = &plain;
        while let Some((len, tail)) = rest.split_first_chunk::<4>() {
            let len = u32::from_be_bytes(*len) as usize;
            let Some(id) = tail.get(..len).and_then(|b| std::str::from_utf8(b).ok()) else { break };
            ids.push(id.to_string());
            rest = &tail[len..];
            if ids.len() >= MAX_ENTRIES {
                break;
            }
        }
        ids
    }

    fn write_index(&self, ids: &[String]) -> Result<(), RecoveryRefused> {
        let mut plain = Zeroizing::new(Vec::new());
        for id in ids {
            plain.extend_from_slice(&(id.len() as u32).to_be_bytes());
            plain.extend_from_slice(id.as_bytes());
        }
        let meta = SnapshotMeta { base_version: 0, saved_at: now(), kind: 1 };
        let sealed = seal_recovery_snapshot(self.master, INDEX_ID, &meta, &plain).map_err(|_| RecoveryRefused::Failed)?;
        replace_file(&self.dir.join(INDEX_NAME), &sealed, || true).map_err(|_| RecoveryRefused::Failed)
    }

    pub(crate) fn put(&self, file_id: &str, base: i64, bytes: &[u8]) -> Result<(), RecoveryRefused> {
        self.put_stopping_at(file_id, base, bytes, || true)
    }

    fn put_stopping_at(
        &self,
        file_id: &str,
        base: i64,
        bytes: &[u8],
        before_rename: impl FnOnce() -> bool,
    ) -> Result<(), RecoveryRefused> {
        if file_id.is_empty() || file_id.starts_with('\0') || bytes.len() > MAX_SNAPSHOT_BYTES || base < 0 {
            return Err(RecoveryRefused::Refused);
        }
        let kind = kind_of(bytes).ok_or(RecoveryRefused::Refused)?;
        let meta = SnapshotMeta { base_version: base, saved_at: now(), kind };
        let sealed = seal_recovery_snapshot(self.master, file_id, &meta, bytes).map_err(|_| RecoveryRefused::Failed)?;
        let _one = WRITE.lock().unwrap_or_else(|p| p.into_inner());
        fs::create_dir_all(&self.dir).map_err(|_| RecoveryRefused::Failed)?;
        let mut ids = self.read_index();
        if !ids.iter().any(|id| id == file_id) {
            if ids.len() >= MAX_ENTRIES {
                return Err(RecoveryRefused::Refused);
            }
            ids.push(file_id.to_string());
            self.write_index(&ids)?;
        }
        replace_file(&self.blob_path(file_id), &sealed, before_rename).map_err(|_| RecoveryRefused::Failed)
    }

    pub(crate) fn list(&self) -> Vec<RecoveryEntry> {
        let _one = WRITE.lock().unwrap_or_else(|p| p.into_inner());
        let mut out = Vec::new();
        for file_id in self.read_index() {
            // Display only: the blob is authenticated when it is taken.
            let Ok(head) = read_head(&self.blob_path(&file_id)) else { continue };
            let Ok(meta) = peek_snapshot_meta(&head) else { continue };
            out.push(RecoveryEntry { file_id, saved_at: meta.saved_at, base: meta.base_version });
        }
        out
    }

    pub(crate) fn take(&self, file_id: &str) -> Result<Zeroizing<Vec<u8>>, RecoveryRefused> {
        let sealed = fs::read(self.blob_path(file_id)).map_err(|_| RecoveryRefused::NotFound)?;
        open_recovery_snapshot(self.master, file_id, &sealed)
            .map(|(_, plain)| plain)
            .map_err(|_| RecoveryRefused::NotFound)
    }

    pub(crate) fn discard(&self, file_id: &str) -> Result<(), RecoveryRefused> {
        let _one = WRITE.lock().unwrap_or_else(|p| p.into_inner());
        let blob = self.blob_path(file_id);
        for path in [blob.clone(), PathBuf::from(format!("{}{NEW_SUFFIX}", blob.display()))] {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(RecoveryRefused::Failed),
            }
        }
        let mut ids = self.read_index();
        let before = ids.len();
        ids.retain(|id| id != file_id);
        if ids.len() != before {
            if ids.is_empty() {
                let _ = fs::remove_file(self.dir.join(INDEX_NAME));
            } else {
                self.write_index(&ids)?;
            }
        }
        Ok(())
    }
}

fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut head = vec![0u8; 22];
    File::open(path)?.read_exact(&mut head)?;
    Ok(head)
}

fn account_dir(state_dir: &Path, account_id: &str) -> PathBuf {
    state_dir.join(FOLDER).join(hashed(&["account", account_id]))
}

/// Delete every blob of the account (and the folder). Nothing there is kept.
pub(crate) fn wipe_account(state_dir: &Path, account_id: &str) -> std::io::Result<()> {
    let _one = WRITE.lock().unwrap_or_else(|p| p.into_inner());
    match fs::remove_dir_all(account_dir(state_dir, account_id)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

impl<C: CredentialStores> Engine<C> {
    fn recovery_store<T>(
        &self,
        session: SessionGeneration,
        with: impl FnOnce(&Store<'_>) -> Result<T, RecoveryRefused>,
    ) -> Result<T, RecoveryRefused> {
        let (_token, key) = self.edit_credentials(session).map_err(|_| RecoveryRefused::NotSignedIn)?;
        let master = MasterKey::from_bytes(*key);
        with(&Store::new(&self.config.state_dir, &self.config.account_id, &master))
    }

    /// WRITES (the disk): the working copy `bytes` of the document `file_id`, edited from version `base`, as this
    /// document's one blob (the previous blob stays until this one is complete).
    pub fn put_office_snapshot_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        base: i64,
        bytes: &Zeroizing<Vec<u8>>,
    ) -> Result<(), RecoveryRefused> {
        self.recovery_store(session, |s| s.put(file_id, base, bytes))
    }

    /// The documents of this account that have a blob.
    pub fn office_snapshots_for(&self, session: SessionGeneration) -> Result<Vec<RecoveryEntry>, RecoveryRefused> {
        self.recovery_store(session, |s| Ok(s.list()))
    }

    /// The document's blob, opened. It stays until [`Self::discard_office_snapshot_for`].
    pub fn take_office_snapshot_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
    ) -> Result<Zeroizing<Vec<u8>>, RecoveryRefused> {
        self.recovery_store(session, |s| s.take(file_id))
    }

    /// WRITES (the disk): delete the document's blob.
    pub fn discard_office_snapshot_for(&self, session: SessionGeneration, file_id: &str) -> Result<(), RecoveryRefused> {
        self.recovery_store(session, |s| s.discard(file_id))
    }

    /// WRITES (the disk), inside the sign-out past its stop gate: every blob of this account goes. A failure is
    /// logged (no name, no path) and does not stop the sign-out.
    pub(super) fn wipe_office_recovery_at_session_end(&self) {
        if wipe_account(&self.config.state_dir, &self.config.account_id).is_err() {
            tracing::warn!("office recovery blobs not deleted at sign-out");
        }
    }
}

#[cfg(test)]
mod office_recovery_tests {
    use super::*;

    const MARKER: &str = "TOP-SECRET-MARKER-9f3a";

    fn key(n: u8) -> MasterKey {
        MasterKey::from_bytes([n; 32])
    }

    /// A package the kind check accepts, with a marker inside.
    fn package(marker: &str) -> Vec<u8> {
        let mut v = marker.as_bytes().to_vec();
        v.extend_from_slice(b"word/document.xml");
        v
    }

    fn files(dir: &Path) -> Vec<PathBuf> {
        fn walk(d: &Path, out: &mut Vec<PathBuf>) {
            let Ok(rd) = fs::read_dir(d) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() { walk(&p, out) } else { out.push(p) }
            }
        }
        let mut out = Vec::new();
        walk(dir, &mut out);
        out
    }

    #[test]
    fn office_recovery_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let k = key(1);
        let s = Store::new(tmp.path(), "acct", &k);
        s.put("file-1", 7, &package(MARKER)).unwrap();
        let list = s.list();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].file_id.as_str(), list[0].base), ("file-1", 7));
        assert_eq!(&**s.take("file-1").unwrap(), package(MARKER).as_slice());
        s.discard("file-1").unwrap();
        assert!(s.list().is_empty());
        assert_eq!(s.take("file-1").unwrap_err(), RecoveryRefused::NotFound);
    }

    #[test]
    fn office_recovery_other_account_or_file_does_not_open() {
        let tmp = tempfile::tempdir().unwrap();
        let (k1, k2) = (key(1), key(2));
        let s1 = Store::new(tmp.path(), "acct", &k1);
        s1.put("file-1", 1, &package(MARKER)).unwrap();
        // Same folder, another key: does not open.
        let s2 = Store::new(tmp.path(), "acct", &k2);
        assert_eq!(s2.take("file-1").unwrap_err(), RecoveryRefused::NotFound);
        // The blob moved to another file's slot: does not open.
        let from = s1.blob_path("file-1");
        fs::copy(&from, s1.blob_path("file-2")).unwrap();
        assert_eq!(s1.take("file-2").unwrap_err(), RecoveryRefused::NotFound);
        // Another account's folder holds nothing of this one's.
        let other = Store::new(tmp.path(), "acct-b", &k1);
        assert!(other.list().is_empty());
        assert_eq!(other.take("file-1").unwrap_err(), RecoveryRefused::NotFound);
    }

    #[test]
    fn office_recovery_crash_between_write_and_rename_keeps_old_blob() {
        let tmp = tempfile::tempdir().unwrap();
        let k = key(1);
        let s = Store::new(tmp.path(), "acct", &k);
        s.put("file-1", 1, &package("old")).unwrap();
        s.put_stopping_at("file-1", 2, &package("new"), || false).unwrap();
        assert_eq!(&**s.take("file-1").unwrap(), package("old").as_slice());
        assert_eq!(s.list()[0].base, 1);
        // The next put goes over the leftover new name.
        s.put("file-1", 3, &package("newer")).unwrap();
        assert_eq!(&**s.take("file-1").unwrap(), package("newer").as_slice());
    }

    #[test]
    fn office_recovery_sign_out_leaves_no_blob() {
        let tmp = tempfile::tempdir().unwrap();
        let k = key(1);
        let s = Store::new(tmp.path(), "acct", &k);
        s.put("file-1", 1, &package(MARKER)).unwrap();
        s.put("file-2", 1, &package(MARKER)).unwrap();
        s.put_stopping_at("file-1", 2, &package(MARKER), || false).unwrap();
        wipe_account(tmp.path(), "acct").unwrap();
        assert!(files(tmp.path()).is_empty());
        assert!(s.list().is_empty());
        wipe_account(tmp.path(), "acct").unwrap();
    }

    #[test]
    fn office_recovery_folder_holds_no_plaintext() {
        let tmp = tempfile::tempdir().unwrap();
        let k = key(1);
        let s = Store::new(tmp.path(), "acct", &k);
        s.put("file-secret-name", 1, &package(MARKER)).unwrap();
        s.put_stopping_at("file-secret-name", 2, &package(MARKER), || false).unwrap();
        let all = files(tmp.path());
        assert!(all.len() >= 2);
        for p in all {
            let name = p.to_string_lossy().into_owned();
            assert!(!name.contains("file-secret-name") && !name.contains("acct"));
            let data = fs::read(&p).unwrap();
            assert!(!data.windows(MARKER.len()).any(|w| w == MARKER.as_bytes()));
            assert!(!data.windows(16).any(|w| w == b"file-secret-name"));
        }
    }

    #[test]
    fn office_recovery_refuses_non_packages() {
        let tmp = tempfile::tempdir().unwrap();
        let k = key(1);
        let s = Store::new(tmp.path(), "acct", &k);
        assert_eq!(s.put("f", 1, b"not a package").unwrap_err(), RecoveryRefused::Refused);
        assert!(files(tmp.path()).is_empty());
    }
}
