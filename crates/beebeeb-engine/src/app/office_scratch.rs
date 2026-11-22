//! Scratch for large Office working copies: a document above the host's spill threshold is kept on disk only as a
//! sealed file under the session's scratch key; the key lives in this process's memory and nowhere else.
//!
//! - The engine calls core: every file is written by `beebeeb_core::local_seal::ScratchWriter` and read by
//!   `ScratchReader` under a [`ScratchKey`] (purpose `SCRATCH_PURPOSE_SPILL`). No crypto is written here.
//! - [`Engine::scratch_for`] gives the store of one session (the [`SessionGeneration`] the caller holds). A different
//!   generation (an unlock, a sign-in) is a new session: the old key is dropped and the folder is emptied.
//! - The first store of a process wipes `<state dir>/office-scratch/` first: files of an earlier run are unreadable
//!   by design (their key is gone) and are deleted.
//! - Lock and sign-out call [`Engine::end_office_scratch_at_session_end`]: the key is dropped (every [`ScratchFile`]
//!   of the session stops opening) and the folder is removed. The host takes its recovery snapshots BEFORE that call.
//! - A [`ScratchFile`] deletes its file when dropped. Names are counters: they say nothing about the document.
//!
//! BLOCKS: the disk. Must not: log a name, a path or a byte.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use beebeeb_core::local_seal::{
    DEFAULT_SCRATCH_CHUNK, SCRATCH_PURPOSE_SPILL, ScratchKey, ScratchReader, ScratchWriter,
};
use zeroize::Zeroizing;

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;

const FOLDER: &str = "office-scratch";
const SUFFIX: &str = ".bbsc";

/// Why a scratch call did nothing. Never carries a name or a byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScratchRefused {
    /// The session ended (key dropped): nothing opens any more.
    Ended,
    /// The disk refused, or the file did not authenticate.
    Failed,
}

struct Session {
    generation: SessionGeneration,
    /// `None` once the session ended.
    key: Mutex<Option<ScratchKey>>,
}

impl Session {
    fn with_key<T>(&self, f: impl FnOnce(&ScratchKey) -> Result<T, ScratchRefused>) -> Result<T, ScratchRefused> {
        let guard = self.key.lock().unwrap_or_else(|p| p.into_inner());
        match guard.as_ref() {
            Some(key) => f(key),
            None => Err(ScratchRefused::Ended),
        }
    }

    fn end(&self) {
        *self.key.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}

/// The live session per state dir (one engine per dir; tests use their own temp dirs).
static SESSIONS: Mutex<Option<HashMap<PathBuf, Arc<Session>>>> = Mutex::new(None);
static NEXT_NAME: AtomicU64 = AtomicU64::new(1);

fn wipe(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

/// Where one session writes. Cheap to clone.
#[derive(Clone)]
pub struct ScratchStore {
    dir: PathBuf,
    session: Arc<Session>,
}

/// One sealed file. Deleted when dropped.
pub struct ScratchFile {
    path: PathBuf,
    session: Arc<Session>,
    len: usize,
}

impl ScratchStore {
    /// Seals `bytes` into a new file.
    pub fn put(&self, bytes: &[u8]) -> Result<ScratchFile, ScratchRefused> {
        fs::create_dir_all(&self.dir).map_err(|_| ScratchRefused::Failed)?;
        let path = self
            .dir
            .join(format!("{}{SUFFIX}", NEXT_NAME.fetch_add(1, Ordering::Relaxed)));
        let written = self.session.with_key(|key| {
            let file = File::create(&path).map_err(|_| ScratchRefused::Failed)?;
            let mut writer =
                ScratchWriter::new(key, BufWriter::new(file), SCRATCH_PURPOSE_SPILL, DEFAULT_SCRATCH_CHUNK)
                    .map_err(|_| ScratchRefused::Failed)?;
            writer.write_all(bytes).map_err(|_| ScratchRefused::Failed)?;
            let (mut out, _) = writer.finish().map_err(|_| ScratchRefused::Failed)?;
            out.flush().map_err(|_| ScratchRefused::Failed)?;
            Ok(())
        });
        match written {
            Ok(()) => Ok(ScratchFile {
                path,
                session: self.session.clone(),
                len: bytes.len(),
            }),
            Err(e) => {
                let _ = fs::remove_file(&path);
                Err(e)
            }
        }
    }
}

impl ScratchFile {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The plaintext, only after every frame and the end authenticated.
    pub fn read(&self) -> Result<Zeroizing<Vec<u8>>, ScratchRefused> {
        self.session.with_key(|key| {
            let mut input = BufReader::new(File::open(&self.path).map_err(|_| ScratchRefused::Failed)?);
            let mut header = [0u8; 26];
            input.read_exact(&mut header).map_err(|_| ScratchRefused::Failed)?;
            let id: [u8; 16] = header[10..26].try_into().expect("16 bytes");
            let mut whole = io::Cursor::new(header).chain(input);
            let mut reader =
                ScratchReader::new(key, &mut whole, SCRATCH_PURPOSE_SPILL, &id).map_err(|_| ScratchRefused::Failed)?;
            let mut plain = Zeroizing::new(Vec::with_capacity(self.len));
            io::copy(&mut reader, &mut *plain).map_err(|_| ScratchRefused::Failed)?;
            Ok(plain)
        })
    }
}

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn store_in(state_dir: &Path, session: SessionGeneration) -> Option<ScratchStore> {
    let dir = state_dir.join(FOLDER);
    let mut all = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
    let map = all.get_or_insert_with(HashMap::new);
    if let Some(existing) = map.get(state_dir) {
        if existing.generation == session {
            return Some(ScratchStore {
                dir,
                session: existing.clone(),
            });
        }
        existing.end();
        wipe(&dir);
    } else {
        // First store of this process: whatever is there is from an earlier run.
        wipe(&dir);
    }
    let key = ScratchKey::generate().ok()?;
    let fresh = Arc::new(Session {
        generation: session,
        key: Mutex::new(Some(key)),
    });
    map.insert(state_dir.to_path_buf(), fresh.clone());
    Some(ScratchStore { dir, session: fresh })
}

fn end_in(state_dir: &Path) {
    let mut all = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(session) = all.as_mut().and_then(|m| m.remove(state_dir)) {
        session.end();
    }
    wipe(&state_dir.join(FOLDER));
}

/// Test seam for the host's tests (no engine handle needed).
#[cfg(any(test, feature = "test-support"))]
pub fn store_for_test(state_dir: &Path, session: SessionGeneration) -> ScratchStore {
    store_in(state_dir, session).expect("scratch key")
}

/// Test seam: what lock and sign-out do.
#[cfg(any(test, feature = "test-support"))]
pub fn end_for_test(state_dir: &Path) {
    end_in(state_dir);
}

impl<C: CredentialStores> Engine<C> {
    /// The scratch store of `session`. `None` when the key could not be made (the caller keeps the bytes in memory).
    pub fn scratch_for(&self, session: SessionGeneration) -> Option<ScratchStore> {
        store_in(&self.config.state_dir, session)
    }

    /// WRITES (the disk), at lock and at sign-out: the session's scratch key is dropped and the folder removed.
    pub(super) fn end_office_scratch_at_session_end(&self) {
        end_in(&self.config.state_dir);
    }
}

#[cfg(test)]
mod office_scratch_tests {
    use super::*;

    const MARKER: &str = "TOP-SECRET-MARKER-5c1e";

    fn doc() -> Vec<u8> {
        let mut v = Vec::new();
        for i in 0..200_000u32 {
            v.extend_from_slice(MARKER.as_bytes());
            v.extend_from_slice(&i.to_be_bytes());
        }
        v
    }

    fn folder_has_marker(dir: &Path) -> bool {
        let Ok(rd) = fs::read_dir(dir.join(FOLDER)) else {
            return false;
        };
        rd.flatten().any(|e| {
            let bytes = fs::read(e.path()).unwrap();
            bytes.windows(MARKER.len()).any(|w| w == MARKER.as_bytes())
        })
    }

    fn files(dir: &Path) -> usize {
        fs::read_dir(dir.join(FOLDER)).map(|r| r.count()).unwrap_or(0)
    }

    #[test]
    fn roundtrip_without_plaintext_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_in(tmp.path(), SessionGeneration::for_test(1)).unwrap();
        let data = doc();
        let file = store.put(&data).unwrap();
        assert_eq!(files(tmp.path()), 1);
        assert!(!folder_has_marker(tmp.path()));
        assert_eq!(&**file.read().unwrap(), &data[..]);
        drop(file);
        assert_eq!(files(tmp.path()), 0);
    }

    #[test]
    fn session_end_leaves_nothing_readable() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_in(tmp.path(), SessionGeneration::for_test(1)).unwrap();
        let file = store.put(&doc()).unwrap();
        end_in(tmp.path());
        assert_eq!(file.read().unwrap_err(), ScratchRefused::Ended);
        assert_eq!(store.put(b"x").err(), Some(ScratchRefused::Ended));
        assert_eq!(files(tmp.path()), 0);
    }

    #[test]
    fn new_generation_ends_the_old_one() {
        let tmp = tempfile::tempdir().unwrap();
        let old = store_in(tmp.path(), SessionGeneration::for_test(1)).unwrap();
        let file = old.put(&doc()).unwrap();
        let _new = store_in(tmp.path(), SessionGeneration::for_test(2)).unwrap();
        assert_eq!(file.read().unwrap_err(), ScratchRefused::Ended);
        assert_eq!(files(tmp.path()), 0);
    }

    #[test]
    fn files_of_an_earlier_run_are_deleted_at_start() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(FOLDER)).unwrap();
        fs::write(tmp.path().join(FOLDER).join("99.bbsc"), b"left over").unwrap();
        let _ = store_in(tmp.path(), SessionGeneration::for_test(1)).unwrap();
        assert_eq!(files(tmp.path()), 0);
    }

    #[test]
    fn a_damaged_file_does_not_open() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_in(tmp.path(), SessionGeneration::for_test(1)).unwrap();
        let file = store.put(&doc()).unwrap();
        let mut bytes = fs::read(&file.path).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 1;
        fs::write(&file.path, bytes).unwrap();
        assert_eq!(file.read().unwrap_err(), ScratchRefused::Failed);
    }
}
