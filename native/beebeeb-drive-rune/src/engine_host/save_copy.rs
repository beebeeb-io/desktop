//! A copy of one file at a place the person chose: the viewer's "Save a copy…" and "Download to…", and Photos' Save to
//! PC (`shell::viewer_save`).
//!
//! Where the bytes come from:
//! - sync is on and the item's file is in the Beebeeb folder: copied from there, in chunks, whatever the size (on
//!   Windows reading a placeholder hydrates it as it is read). The path is the engine's own (`file_ops::item_path_for`),
//!   refused when another row's file sits at that place.
//! - otherwise: downloaded and decrypted in memory (`Engine::open_file_in_memory`), up to [`IN_MEMORY_MAX`]; a larger file
//!   is refused with words ([`SaveRefused::TooLarge`]), not cut short.
//!
//! The copy is written to a part file next to the destination and renamed over it only when it is whole, the session of
//! the press is still the session and nobody cancelled. Any other end removes the part file: no partial file is ever
//! left at the destination, and a session that ended writes nothing at all.
//!
//! Must not: log a path, a name or a byte. The words of a refusal name no file.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::viewer_api::OpenFileError;
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::file_ops;

pub use super::viewer::IN_MEMORY_MAX;

/// What the copy reports as it goes: (bytes written, bytes in all). Cheap.
pub type Progress = dyn Fn(u64, u64) + Send + Sync + 'static;

/// Why a copy was not made. Nothing is left at the destination in any case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveRefused {
    /// The session of the press is not the session any more (a lock, a sign-out, another account).
    NotSignedIn,
    /// The person (or the window leaving) stopped it.
    Cancelled,
    /// Sync is off and the file is larger than what is decrypted in memory: `limit` bytes.
    TooLarge { limit: u64 },
    /// Beebeeb could not be reached.
    Offline,
    /// The file is gone, or no longer shared with this account.
    NotThere,
    /// The place could not be written (full, no right to write, in use) or the copy could not be read.
    Failed,
}

impl SaveRefused {
    /// The words for the person, naming no file and no folder.
    pub fn words(self) -> String {
        match self {
            Self::NotSignedIn => "Not saved: you are not signed in any more".into(),
            Self::Cancelled => "Not saved: stopped".into(),
            Self::TooLarge { limit } => format!(
                "Not saved: with sync off, files over {} MB cannot be saved from here. Turn sync on, or download it from the web app",
                limit / (1024 * 1024)
            ),
            Self::Offline => "Not saved: Beebeeb could not be reached".into(),
            Self::NotThere => "Not saved: the file is gone or no longer shared with you".into(),
            Self::Failed => "Not saved: the copy could not be written there".into(),
        }
    }
}

/// What the shell asks for a copy: the engine behind it is generic over the credential stores, the shell is not.
pub trait CopySaver: Send + Sync {
    /// The session now (`Engine::session_turn`): it moves at every sign-in, unlock, lock and sign-out.
    fn turn(&self) -> SessionGeneration;
    /// [`save_copy`] on this engine.
    fn save(
        &self,
        session: SessionGeneration,
        file_id: &str,
        dest: &Path,
        progress: &Progress,
        cancel: &AtomicBool,
    ) -> Result<(), SaveRefused>;
}

/// The running app's saver, on its one engine handle.
pub struct EngineSaver<C: CredentialStores>(pub Arc<Engine<C>>);

impl<C: CredentialStores + 'static> CopySaver for EngineSaver<C> {
    fn turn(&self) -> SessionGeneration {
        self.0.session_turn()
    }
    fn save(
        &self,
        session: SessionGeneration,
        file_id: &str,
        dest: &Path,
        progress: &Progress,
        cancel: &AtomicBool,
    ) -> Result<(), SaveRefused> {
        save_copy(&self.0, session, file_id, dest, progress, cancel)
    }
}

/// Copies the file `file_id` to `dest` for `session` (the session of the press). Blocks: run it on a worker.
pub fn save_copy<C: CredentialStores>(
    engine: &Engine<C>,
    session: SessionGeneration,
    file_id: &str,
    dest: &Path,
    progress: &Progress,
    cancel: &AtomicBool,
) -> Result<(), SaveRefused> {
    let still = || engine.session_turn() == session;
    place(dest, &still, cancel, |out| {
        if let Some(source) = kept_path(engine, file_id) {
            return copy_stream(&source, out, &still, cancel, progress);
        }
        let opened = engine
            .open_file_in_memory(file_id, IN_MEMORY_MAX, progress, cancel)
            .map_err(open_refused)?;
        if opened.session != session {
            return Err(SaveRefused::NotSignedIn);
        }
        let total = opened.bytes.len() as u64;
        write_all(out, &opened.bytes, &still, cancel)?;
        progress(total, total);
        Ok(())
    })
}

/// The item's file in the Beebeeb folder, while sync is on and the file is there.
fn kept_path<C: CredentialStores>(engine: &Engine<C>, file_id: &str) -> Option<PathBuf> {
    if !engine.is_syncing() {
        return None;
    }
    let root = engine.sync_root().ok()?;
    let path = engine
        .local_state_for_changes()
        .ok()?
        .with(|db| {
            let entry = file_ops::resolve(db, file_id).ok()?;
            if entry.is_dir() {
                return None;
            }
            file_ops::item_path_for(db, &root, &entry).ok()
        })
        .ok()??;
    path.is_file().then_some(path)
}

fn open_refused(error: OpenFileError) -> SaveRefused {
    match error {
        OpenFileError::TooLarge { .. } | OpenFileError::OverLimit => SaveRefused::TooLarge { limit: IN_MEMORY_MAX },
        OpenFileError::Cancelled => SaveRefused::Cancelled,
        OpenFileError::NotSignedIn | OpenFileError::SessionEnded | OpenFileError::NotYours => SaveRefused::NotSignedIn,
        OpenFileError::NotFound | OpenFileError::NoAccess => SaveRefused::NotThere,
        OpenFileError::Offline => SaveRefused::Offline,
        _ => SaveRefused::Failed,
    }
}

const CHUNK: usize = 256 * 1024;

fn stopped(still: &dyn Fn() -> bool, cancel: &AtomicBool) -> Result<(), SaveRefused> {
    if cancel.load(Ordering::SeqCst) {
        return Err(SaveRefused::Cancelled);
    }
    if !still() {
        return Err(SaveRefused::NotSignedIn);
    }
    Ok(())
}

fn write_all(
    out: &mut File,
    bytes: &[u8],
    still: &dyn Fn() -> bool,
    cancel: &AtomicBool,
) -> Result<(), SaveRefused> {
    for chunk in bytes.chunks(CHUNK) {
        stopped(still, cancel)?;
        out.write_all(chunk).map_err(|_| SaveRefused::Failed)?;
    }
    Ok(())
}

/// Streams `source` into `out`, asking `still` and `cancel` between chunks.
fn copy_stream(
    source: &Path,
    out: &mut File,
    still: &dyn Fn() -> bool,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), SaveRefused> {
    let mut input = File::open(source).map_err(|_| SaveRefused::Failed)?;
    let total = input.metadata().map(|m| m.len()).map_err(|_| SaveRefused::Failed)?;
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    loop {
        stopped(still, cancel)?;
        let n = input.read(&mut buf).map_err(|_| SaveRefused::Failed)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|_| SaveRefused::Failed)?;
        done += n as u64;
        progress(done, total.max(done));
    }
    Ok(())
}

/// The part file next to `dest`.
fn part_of(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".beebeeb-part");
    dest.with_file_name(name)
}

/// Runs `fill` on a new part file next to `dest` and renames it over `dest` only when `fill` finished, nobody
/// cancelled and `still` holds. Any other end removes the part file. Nothing is created when `still` fails first.
fn place(
    dest: &Path,
    still: &dyn Fn() -> bool,
    cancel: &AtomicBool,
    fill: impl FnOnce(&mut File) -> Result<(), SaveRefused>,
) -> Result<(), SaveRefused> {
    stopped(still, cancel)?;
    let part = part_of(dest);
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)
        .map_err(|_| SaveRefused::Failed)?;
    let end = fill(&mut out)
        .and_then(|()| out.sync_all().map_err(|_| SaveRefused::Failed))
        .and_then(|()| stopped(still, cancel));
    drop(out);
    let end = end.and_then(|()| std::fs::rename(&part, dest).map_err(|_| SaveRefused::Failed));
    if end.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    end
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn save_copy_writes_the_whole_file_in_chunks_and_leaves_no_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.bin");
        let data: Vec<u8> = (0..(CHUNK * 2 + 17)).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &data).unwrap();
        let dest = dir.path().join("copy.bin");
        let seen = std::sync::Mutex::new(0u64);
        let progress = |done: u64, _| *seen.lock().unwrap() = done;
        place(&dest, &|| true, &AtomicBool::new(false), |out| {
            copy_stream(&src, out, &|| true, &AtomicBool::new(false), &progress)
        })
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(*seen.lock().unwrap(), data.len() as u64);
        assert_eq!(names(dir.path()), ["a.bin", "copy.bin"]);
    }

    #[test]
    fn save_copy_a_failed_copy_leaves_no_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("copy.bin");
        let end = place(&dest, &|| true, &AtomicBool::new(false), |out| {
            out.write_all(b"half").unwrap();
            Err(SaveRefused::Failed)
        });
        assert_eq!(end, Err(SaveRefused::Failed));
        assert!(names(dir.path()).is_empty());
        // A source that is not there fails the same way.
        let end = place(&dest, &|| true, &AtomicBool::new(false), |out| {
            copy_stream(&dir.path().join("gone"), out, &|| true, &AtomicBool::new(false), &|_, _| {})
        });
        assert_eq!(end, Err(SaveRefused::Failed));
        assert!(names(dir.path()).is_empty());
    }

    #[test]
    fn save_copy_a_failed_replace_keeps_the_file_that_was_there() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("copy.bin");
        std::fs::write(&dest, b"old").unwrap();
        let end = place(&dest, &|| true, &AtomicBool::new(false), |_| Err(SaveRefused::Offline));
        assert_eq!(end, Err(SaveRefused::Offline));
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
        assert_eq!(names(dir.path()), ["copy.bin"]);
    }

    #[test]
    fn save_copy_after_sign_out_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("copy.bin");
        let ran = Cell::new(false);
        let end = place(&dest, &|| false, &AtomicBool::new(false), |_| {
            ran.set(true);
            Ok(())
        });
        assert_eq!(end, Err(SaveRefused::NotSignedIn));
        assert!(!ran.get(), "nothing was started");
        assert!(names(dir.path()).is_empty());
    }

    #[test]
    fn save_copy_a_sign_out_while_copying_removes_what_was_written() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.bin");
        std::fs::write(&src, vec![7u8; CHUNK * 3]).unwrap();
        let dest = dir.path().join("copy.bin");
        let asked = Cell::new(0);
        // The session holds for the start and the first chunk, then ends.
        let still = || {
            asked.set(asked.get() + 1);
            asked.get() <= 3
        };
        let cancel = AtomicBool::new(false);
        let end = place(&dest, &still, &cancel, |out| copy_stream(&src, out, &still, &cancel, &|_, _| {}));
        assert_eq!(end, Err(SaveRefused::NotSignedIn));
        assert_eq!(names(dir.path()), ["a.bin"]);
    }

    #[test]
    fn save_copy_a_cancel_removes_the_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.bin");
        std::fs::write(&src, vec![7u8; CHUNK * 2]).unwrap();
        let dest = dir.path().join("copy.bin");
        let cancel = AtomicBool::new(false);
        let progress = |_, _| cancel.store(true, Ordering::SeqCst);
        let end = place(&dest, &|| true, &cancel, |out| copy_stream(&src, out, &|| true, &cancel, &progress));
        assert_eq!(end, Err(SaveRefused::Cancelled));
        assert_eq!(names(dir.path()), ["a.bin"]);
    }

    #[test]
    fn save_copy_words_name_no_file_and_the_cap_in_megabytes() {
        assert!(SaveRefused::TooLarge { limit: IN_MEMORY_MAX }.words().contains("64 MB"));
        assert!(SaveRefused::Failed.words().starts_with("Not saved"));
    }
}
