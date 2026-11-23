//! The local thumbnail cache (079): thumbnails this PC made itself, for photos the server has none for, kept on
//! disk so each photo is decoded once, not on every scroll.
//!
//! - Sealed by core, never plaintext on disk: every entry is one `beebeeb_core::local_seal` snapshot frame (the
//!   office-recovery recipe) under the account's key, bound to the file id. An entry of another account or moved
//!   to another file's name does not open. No crypto is written here.
//! - Keyed by file id + version: one entry per file, its version authenticated inside the seal. A newer version
//!   replaces the entry; an older one is refused; a lookup at another version answers nothing (stale).
//! - The file name is a hash of the account's local id and the file id: it says nothing about the photo.
//! - Capped in bytes: after a write the least recently used entries go until the folder fits. A read touches the
//!   entry's modified time.
//! - Dropped at sign-out and account switch ([`wipe_account`]); the folder is per account, so another account never
//!   even looks at it.
//!
//! BLOCKS: the disk. Must not: log a name, a path or a byte.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use beebeeb_core::kdf::MasterKey;
use beebeeb_core::local_seal::{SnapshotMeta, open_recovery_snapshot, seal_recovery_snapshot};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const FOLDER: &str = "thumb-cache";
const NEW_SUFFIX: &str = ".new";
/// The default cap of one account's cache.
pub const DEFAULT_CAP_MB: u64 = 200;
/// The widest or highest thumbnail kept (the largest cut `photos_api` hands over).
pub const MAX_SIDE: u32 = 2048;
/// The plaintext inside the seal: `MAGIC || width(4) || height(4) || rgba`.
const MAGIC: &[u8; 4] = b"BBT1";
/// The same plus one flags byte after the magic: `MAGIC2 || flags(1) || width(4) || height(4) || rgba`. An entry
/// written before the flags ([`MAGIC`]) still opens, with no flag set.
const MAGIC2: &[u8; 4] = b"BBT2";
const HEAD: usize = 12;
const HEAD2: usize = 13;
/// The server answered "no thumbnail" for this file when this PC made its own (115).
const FLAG_SERVER_404: u8 = 1;
/// This PC sent its thumbnail to the server (once).
const FLAG_UPLOADED: u8 = 2;
/// The snapshot kind the seal records. Core knows document kinds only (1..=3); the cache is told apart by its own
/// magic inside the sealed bytes and by its own folder.
const SEAL_KIND: u8 = 1;
/// One writer at a time over every cache folder (a write and its eviction move together).
static WRITE: Mutex<()> = Mutex::new(());

/// Why a keep did nothing. Never carries a name or a byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbCacheError {
    /// No session, or the session moved on since the call was made.
    NotSignedIn,
    /// Not pixels this keeps: empty, too large, a length that does not match the size, or a negative version.
    Refused,
    /// The cache holds a newer version of this file already.
    Stale,
    /// The disk or the seal refused.
    Failed,
}

/// A thumbnail as the cache holds it, opened: the version it was made from and its RGBA pixels.
pub struct CachedThumb {
    pub version: i64,
    pub width: u32,
    pub height: u32,
    pub rgba: Zeroizing<Vec<u8>>,
    /// The server had no thumbnail for the file when this one was made.
    pub server_404: bool,
    /// This PC has sent it to the server.
    pub uploaded: bool,
}

impl std::fmt::Debug for CachedThumb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedThumb")
            .field("version", &self.version)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
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
    h.update(b"beebeeb-thumb-cache-name-v1");
    for part in parts {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part.as_bytes());
    }
    hex(&h.finalize())
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn account_dir(state_dir: &Path, account_id: &str) -> PathBuf {
    state_dir.join(FOLDER).join(hashed(&["account", account_id]))
}

/// The bytes `RGBA` of `width` x `height` must have, if that is a size this keeps.
fn expected_len(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return None;
    }
    (width as usize).checked_mul(height as usize)?.checked_mul(4)
}

/// `data` at `path`: a new name, synced, then renamed over the old one.
fn replace_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut new = path.as_os_str().to_owned();
    new.push(NEW_SUFFIX);
    let new = PathBuf::from(new);
    let written = (|| {
        let mut file = OpenOptions::new().write(true).create(true).truncate(true).open(&new)?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&new, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&new);
    }
    written
}

/// One account's cache: its folder, its key and its cap.
pub struct ThumbCache<'a> {
    dir: PathBuf,
    account: String,
    master: &'a MasterKey,
    cap_bytes: u64,
}

impl<'a> ThumbCache<'a> {
    pub fn new(state_dir: &Path, account_id: &str, master: &'a MasterKey, cap_mb: u64) -> Self {
        Self {
            dir: account_dir(state_dir, account_id),
            account: account_id.to_string(),
            master,
            cap_bytes: cap_mb.saturating_mul(1024 * 1024),
        }
    }

    fn entry_path(&self, file_id: &str) -> PathBuf {
        self.dir.join(hashed(&["thumb", &self.account, file_id]))
    }

    /// The entry of `file_id`, opened and checked. `None`: no entry, or one that does not open (another account's,
    /// another file's, damaged); a damaged entry is deleted.
    fn open(&self, file_id: &str) -> Option<CachedThumb> {
        let path = self.entry_path(file_id);
        let sealed = fs::read(&path).ok()?;
        let Ok((meta, plain)) = open_recovery_snapshot(self.master, file_id, &sealed) else {
            let _ = fs::remove_file(&path);
            return None;
        };
        let (flags, head) = match plain.get(0..4) {
            Some(m) if m == MAGIC => (0u8, HEAD),
            Some(m) if m == MAGIC2 && plain.len() >= HEAD2 => (plain[4], HEAD2),
            _ => {
                let _ = fs::remove_file(&path);
                return None;
            }
        };
        let dims = head - 8;
        let width = u32::from_be_bytes(plain[dims..dims + 4].try_into().ok()?);
        let height = u32::from_be_bytes(plain[dims + 4..dims + 8].try_into().ok()?);
        if expected_len(width, height) != Some(plain.len() - head) {
            let _ = fs::remove_file(&path);
            return None;
        }
        // Least recently used: a read counts as a use.
        if let Ok(file) = OpenOptions::new().write(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        Some(CachedThumb {
            version: meta.base_version,
            width,
            height,
            rgba: Zeroizing::new(plain[head..].to_vec()),
            server_404: flags & FLAG_SERVER_404 != 0,
            uploaded: flags & FLAG_UPLOADED != 0,
        })
    }

    /// The newest thumbnail kept for `file_id`, whatever its version.
    pub fn get(&self, file_id: &str) -> Option<CachedThumb> {
        self.open(file_id)
    }

    /// The thumbnail of `file_id` made from `version`. An entry of another version is stale: nothing.
    pub fn get_at(&self, file_id: &str, version: i64) -> Option<CachedThumb> {
        self.open(file_id).filter(|thumb| thumb.version == version)
    }

    /// WRITES (the disk): keep `rgba` as the thumbnail of `file_id` at `version`, replacing an older version's,
    /// then evict the least recently used entries past the cap.
    pub fn put(
        &self,
        file_id: &str,
        version: i64,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), ThumbCacheError> {
        self.write(file_id, version, width, height, rgba)
    }

    /// Note that the server had no thumbnail for `file_id` at `version` (the entry's own version).
    pub fn mark_server_404(&self, file_id: &str, version: i64) -> Result<(), ThumbCacheError> {
        self.add_flag(file_id, version, FLAG_SERVER_404)
    }

    /// Note that this PC sent the thumbnail of `file_id` at `version` to the server.
    pub fn mark_uploaded(&self, file_id: &str, version: i64) -> Result<(), ThumbCacheError> {
        self.add_flag(file_id, version, FLAG_UPLOADED)
    }

    fn add_flag(&self, file_id: &str, version: i64, flag: u8) -> Result<(), ThumbCacheError> {
        let _one = WRITE.lock().unwrap_or_else(|p| p.into_inner());
        let held = self
            .open(file_id)
            .filter(|held| held.version == version)
            .ok_or(ThumbCacheError::Stale)?;
        let flags = Self::flags_of(&held) | flag;
        self.write_locked(file_id, version, held.width, held.height, &held.rgba, flags)
    }

    fn flags_of(held: &CachedThumb) -> u8 {
        (u8::from(held.server_404) * FLAG_SERVER_404) | (u8::from(held.uploaded) * FLAG_UPLOADED)
    }

    /// A rewrite of the same version keeps what the entry noted; a newer version starts clean.
    fn write(&self, file_id: &str, version: i64, width: u32, height: u32, rgba: &[u8]) -> Result<(), ThumbCacheError> {
        let _one = WRITE.lock().unwrap_or_else(|p| p.into_inner());
        let flags = match self.open(file_id) {
            Some(held) if held.version > version => return Err(ThumbCacheError::Stale),
            Some(held) if held.version == version => Self::flags_of(&held),
            _ => 0,
        };
        self.write_locked(file_id, version, width, height, rgba, flags)
    }

    /// Called under [`WRITE`].
    fn write_locked(
        &self,
        file_id: &str,
        version: i64,
        width: u32,
        height: u32,
        rgba: &[u8],
        flags: u8,
    ) -> Result<(), ThumbCacheError> {
        if version < 0 || expected_len(width, height) != Some(rgba.len()) {
            return Err(ThumbCacheError::Refused);
        }
        let mut plain = Zeroizing::new(Vec::with_capacity(HEAD2 + rgba.len()));
        plain.extend_from_slice(MAGIC2);
        plain.push(flags);
        plain.extend_from_slice(&width.to_be_bytes());
        plain.extend_from_slice(&height.to_be_bytes());
        plain.extend_from_slice(rgba);
        let meta = SnapshotMeta {
            base_version: version,
            saved_at: now(),
            kind: SEAL_KIND,
        };
        let sealed =
            seal_recovery_snapshot(self.master, file_id, &meta, &plain).map_err(|_| ThumbCacheError::Failed)?;
        if sealed.len() as u64 > self.cap_bytes {
            return Err(ThumbCacheError::Refused);
        }
        fs::create_dir_all(&self.dir).map_err(|_| ThumbCacheError::Failed)?;
        let path = self.entry_path(file_id);
        replace_file(&path, &sealed).map_err(|_| ThumbCacheError::Failed)?;
        self.evict_keeping(&path);
        Ok(())
    }

    /// WRITES (the disk): delete the entry of `file_id`.
    pub fn remove(&self, file_id: &str) {
        let _one = WRITE.lock().unwrap_or_else(|p| p.into_inner());
        let _ = fs::remove_file(self.entry_path(file_id));
    }

    /// Delete the least recently used entries until the folder fits the cap; `keep` (the entry just written) goes
    /// last. Leftover `.new` files of an interrupted write go first. Called under [`WRITE`].
    fn evict_keeping(&self, keep: &Path) {
        let Ok(read) = fs::read_dir(&self.dir) else { return };
        let mut entries: Vec<(bool, SystemTime, u64, PathBuf)> = Vec::new();
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == &NEW_SUFFIX[1..]) {
                let _ = fs::remove_file(&path);
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let used = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            entries.push((path == keep, used, meta.len(), path));
        }
        let mut total: u64 = entries.iter().map(|e| e.2).sum();
        // Oldest first; the entry just written sorts after every other.
        entries.sort_by_key(|a| (a.0, a.1));
        for (_, _, len, path) in entries {
            if total <= self.cap_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }

    /// The bytes the cache takes on disk now.
    pub fn bytes_on_disk(&self) -> u64 {
        fs::read_dir(&self.dir)
            .map(|read| {
                read.flatten()
                    .filter_map(|e| e.metadata().ok())
                    .filter(|m| m.is_file())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0)
    }
}

/// Delete every entry of the account (and the folder): at sign-out and at account switch. Nothing there is kept.
pub fn wipe_account(state_dir: &Path, account_id: &str) -> std::io::Result<()> {
    let _one = WRITE.lock().unwrap_or_else(|p| p.into_inner());
    match fs::remove_dir_all(account_dir(state_dir, account_id)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod thumb_cache_tests {
    use super::*;

    const FILE_A: &str = "0190d8a6-1b2c-7d3e-8f40-0123456789ab";
    const FILE_B: &str = "0190d8a6-1b2c-7d3e-8f40-0123456789ac";

    fn key(n: u8) -> MasterKey {
        MasterKey::from_bytes([n; 32])
    }

    fn pixels(marker: u8, side: u32) -> Vec<u8> {
        let mut v = vec![marker; (side * side * 4) as usize];
        v[..8].copy_from_slice(b"PLAINPIX");
        v
    }

    fn files(dir: &Path) -> Vec<PathBuf> {
        fn walk(d: &Path, out: &mut Vec<PathBuf>) {
            for e in fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() { walk(&p, out) } else { out.push(p) }
            }
        }
        let mut out = Vec::new();
        walk(dir, &mut out);
        out
    }

    #[test]
    fn thumb_cache_round_trip_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let k = key(1);
        let cache = ThumbCache::new(dir.path(), "acct", &k, DEFAULT_CAP_MB);
        cache.put(FILE_A, 3, 8, 8, &pixels(7, 8)).unwrap();
        let got = cache.get(FILE_A).unwrap();
        assert_eq!((got.version, got.width, got.height), (3, 8, 8));
        assert_eq!(&got.rgba[..], &pixels(7, 8)[..]);
        assert!(cache.get_at(FILE_A, 3).is_some());
        // A version change makes the old one stale.
        assert!(cache.get_at(FILE_A, 4).is_none());
        cache.put(FILE_A, 4, 8, 8, &pixels(9, 8)).unwrap();
        assert!(cache.get_at(FILE_A, 3).is_none());
        assert_eq!(cache.get_at(FILE_A, 4).unwrap().rgba[20], 9);
        // An older version never replaces a newer one.
        assert_eq!(cache.put(FILE_A, 2, 8, 8, &pixels(1, 8)), Err(ThumbCacheError::Stale));
        assert_eq!(cache.get(FILE_A).unwrap().version, 4);
        assert!(cache.get(FILE_B).is_none());
    }

    #[test]
    fn thumb_cache_other_account_or_file_does_not_open() {
        let dir = tempfile::tempdir().unwrap();
        let (k1, k2) = (key(1), key(2));
        let mine = ThumbCache::new(dir.path(), "acct", &k1, DEFAULT_CAP_MB);
        mine.put(FILE_A, 1, 4, 4, &pixels(5, 4)).unwrap();
        // Same folder, another key: nothing opens.
        assert!(
            ThumbCache::new(dir.path(), "acct", &k2, DEFAULT_CAP_MB)
                .get(FILE_A)
                .is_none()
        );
        // Another account's folder: nothing there.
        mine.put(FILE_A, 1, 4, 4, &pixels(5, 4)).unwrap();
        assert!(
            ThumbCache::new(dir.path(), "other", &k1, DEFAULT_CAP_MB)
                .get(FILE_A)
                .is_none()
        );
        // An entry moved to another file's name does not open.
        mine.put(FILE_A, 1, 4, 4, &pixels(5, 4)).unwrap();
        fs::copy(mine.entry_path(FILE_A), mine.entry_path(FILE_B)).unwrap();
        assert!(mine.get(FILE_B).is_none());
        assert!(mine.get(FILE_A).is_some());
    }

    #[test]
    fn thumb_cache_holds_no_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let k = key(1);
        let cache = ThumbCache::new(dir.path(), "acct", &k, DEFAULT_CAP_MB);
        cache.put(FILE_A, 1, 16, 16, &pixels(0xAB, 16)).unwrap();
        let all = files(dir.path());
        assert_eq!(all.len(), 1);
        for path in all {
            let name = path.to_string_lossy().to_string();
            assert!(!name.contains(FILE_A) && !name.contains("acct"));
            let bytes = fs::read(&path).unwrap();
            assert!(!bytes.windows(8).any(|w| w == b"PLAINPIX"));
            assert!(!bytes.windows(16).any(|w| w.iter().all(|b| *b == 0xAB)));
        }
    }

    #[test]
    fn thumb_cache_cap_drops_least_recently_used() {
        let dir = tempfile::tempdir().unwrap();
        let k = key(1);
        // 1 MB cap; each entry is 256x256x4 = 256 KiB (+ seal): three fit, the fourth pushes one out.
        let cache = ThumbCache::new(dir.path(), "acct", &k, 1);
        let ids: Vec<String> = (0..4)
            .map(|n| format!("0190d8a6-1b2c-7d3e-8f40-00000000000{n}"))
            .collect();
        for (n, id) in ids.iter().enumerate().take(3) {
            cache.put(id, 1, 256, 256, &pixels(n as u8, 256)).unwrap();
            let past = SystemTime::now() - std::time::Duration::from_secs(100 - n as u64 * 10);
            OpenOptions::new()
                .write(true)
                .open(cache.entry_path(id))
                .unwrap()
                .set_modified(past)
                .unwrap();
        }
        // Reading the oldest makes it recent.
        assert!(cache.get(&ids[0]).is_some());
        cache.put(&ids[3], 1, 256, 256, &pixels(3, 256)).unwrap();
        assert!(cache.bytes_on_disk() <= 1024 * 1024);
        assert!(cache.get(&ids[1]).is_none(), "the least recently used goes");
        assert!(cache.get(&ids[0]).is_some());
        assert!(cache.get(&ids[2]).is_some());
        assert!(cache.get(&ids[3]).is_some());
    }

    #[test]
    fn thumb_cache_refuses_bad_pixels_and_wipes() {
        let dir = tempfile::tempdir().unwrap();
        let k = key(1);
        let cache = ThumbCache::new(dir.path(), "acct", &k, DEFAULT_CAP_MB);
        assert_eq!(cache.put(FILE_A, 1, 4, 4, &[0; 10]), Err(ThumbCacheError::Refused));
        assert_eq!(cache.put(FILE_A, 1, 0, 4, &[]), Err(ThumbCacheError::Refused));
        assert_eq!(cache.put(FILE_A, -1, 1, 1, &[0; 4]), Err(ThumbCacheError::Refused));
        assert_eq!(
            cache.put(FILE_A, 1, MAX_SIDE + 1, 1, &vec![0; (MAX_SIDE as usize + 1) * 4]),
            Err(ThumbCacheError::Refused)
        );
        cache.put(FILE_A, 1, 4, 4, &pixels(1, 4)).unwrap();
        // A damaged entry opens to nothing and is deleted.
        let path = cache.entry_path(FILE_A);
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(cache.get(FILE_A).is_none());
        assert!(!path.exists());
        cache.put(FILE_A, 1, 4, 4, &pixels(1, 4)).unwrap();
        wipe_account(dir.path(), "acct").unwrap();
        assert!(files(dir.path()).is_empty());
        wipe_account(dir.path(), "acct").unwrap();
    }

    #[test]
    fn thumb_cache_flags_survive_a_rewrite_and_reset_on_a_new_version() {
        let dir = tempfile::tempdir().unwrap();
        let k = key(1);
        let cache = ThumbCache::new(dir.path(), "acct", &k, DEFAULT_CAP_MB);
        cache.put(FILE_A, 3, 8, 8, &pixels(7, 8)).unwrap();
        let fresh = cache.get(FILE_A).unwrap();
        assert!(!fresh.server_404 && !fresh.uploaded);
        cache.mark_server_404(FILE_A, 3).unwrap();
        cache.put(FILE_A, 3, 8, 8, &pixels(7, 8)).unwrap();
        let got = cache.get_at(FILE_A, 3).unwrap();
        assert!(got.server_404 && !got.uploaded);
        assert_eq!(&got.rgba[..], &pixels(7, 8)[..]);
        cache.mark_uploaded(FILE_A, 3).unwrap();
        let got = cache.get(FILE_A).unwrap();
        assert!(got.server_404 && got.uploaded);
        // A mark for another version changes nothing.
        assert_eq!(cache.mark_uploaded(FILE_A, 2), Err(ThumbCacheError::Stale));
        // A newer version starts clean.
        cache.put(FILE_A, 4, 8, 8, &pixels(9, 8)).unwrap();
        let got = cache.get(FILE_A).unwrap();
        assert!(!got.server_404 && !got.uploaded);
    }

    #[test]
    fn thumb_cache_reads_an_entry_written_before_the_flags_as_not_404() {
        let dir = tempfile::tempdir().unwrap();
        let k = key(1);
        let cache = ThumbCache::new(dir.path(), "acct", &k, DEFAULT_CAP_MB);
        let mut plain = Vec::new();
        plain.extend_from_slice(b"BBT1");
        plain.extend_from_slice(&8u32.to_be_bytes());
        plain.extend_from_slice(&8u32.to_be_bytes());
        plain.extend_from_slice(&pixels(5, 8));
        let meta = SnapshotMeta {
            base_version: 2,
            saved_at: 1,
            kind: SEAL_KIND,
        };
        let sealed = seal_recovery_snapshot(&k, FILE_A, &meta, &plain).unwrap();
        fs::create_dir_all(&cache.dir).unwrap();
        fs::write(cache.entry_path(FILE_A), sealed).unwrap();
        let got = cache.get_at(FILE_A, 2).expect("an old entry still opens");
        assert_eq!(
            (got.width, got.height, got.server_404, got.uploaded),
            (8, 8, false, false)
        );
        assert_eq!(&got.rgba[..], &pixels(5, 8)[..]);
        cache.mark_server_404(FILE_A, 2).unwrap();
        assert!(cache.get(FILE_A).unwrap().server_404);
    }
}
