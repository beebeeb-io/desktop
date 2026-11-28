//! `content` items split out of `engine_bridge.rs`.

use super::*;

/// Whether a conflict side is text the window may show or bytes it may only size.
/// The one place a filename's "is text" fact is turned into a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ContentKind {
    Text,
    Binary,
}

impl ContentKind {
    pub(super) fn from_flag(is_text: bool) -> Self {
        if is_text {
            ContentKind::Text
        } else {
            ContentKind::Binary
        }
    }

    pub(super) fn is_text(self) -> bool {
        self == ContentKind::Text
    }
}

/// Pure classifier: turns real file bytes into what the conflict window can
/// safely show. Never fabricates content — a non-text file, an oversized
/// file, or invalid UTF-8 all report `text: None` plus an honest
/// `unavailable_reason`, never a placeholder string.
pub(super) fn content_side_from_bytes(bytes: Vec<u8>, kind: ContentKind) -> ConflictContentSide {
    let size_bytes = Some(bytes.len() as u64);
    if kind == ContentKind::Binary {
        return ConflictContentSide {
            size_bytes,
            text: None,
            unavailable_reason: None,
        };
    }
    if bytes.len() > CONFLICT_PREVIEW_TEXT_MAX_BYTES {
        return ConflictContentSide {
            size_bytes,
            text: None,
            unavailable_reason: Some("File is too large to preview inline".to_string()),
        };
    }
    match String::from_utf8(bytes) {
        Ok(text) => ConflictContentSide {
            size_bytes,
            text: Some(text),
            unavailable_reason: None,
        },
        Err(_) => ConflictContentSide {
            size_bytes,
            text: None,
            unavailable_reason: Some("File isn't valid UTF-8 text".to_string()),
        },
    }
}

/// Lower-case hex SHA-256 of `bytes`: a sync-folder file's agreed content (task 1640).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// Task 2232: what a cached content hash is reused against. Size and modification time alone let a
/// write that keeps the size and sets the modification time back return the old hash, so an edit
/// read as the agreed content was never sent. The change time moves on every write and on every
/// time change, and an ordinary tool cannot set it back (Unix `ctime`; Windows `ChangeTime`). A
/// file renamed over the path is another file (Unix device and inode; Windows volume and file index).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ContentFingerprint {
    pub(super) len: u64,
    pub(super) modified: std::time::SystemTime,
    pub(super) changed: std::time::SystemTime,
    pub(super) identity: (u64, u64),
}

impl ContentFingerprint {
    pub fn of(path: &Path) -> std::io::Result<Self> {
        let meta = std::fs::metadata(path)?;
        let modified = meta.modified()?;
        #[cfg(unix)]
        let (changed, identity) = {
            use std::os::unix::fs::MetadataExt;
            let changed = u64::try_from(meta.ctime())
                .ok()
                .and_then(|secs| {
                    std::time::UNIX_EPOCH.checked_add(std::time::Duration::new(secs, meta.ctime_nsec() as u32))
                })
                .unwrap_or(std::time::UNIX_EPOCH);
            (changed, (meta.dev(), meta.ino()))
        };
        #[cfg(target_os = "windows")]
        let (changed, identity) = {
            let stamp = crate::windows_cf::placeholders::change_stamp(path)?;
            (stamp.change_time, (u64::from(stamp.volume_serial), stamp.file_index))
        };
        #[cfg(not(any(unix, target_os = "windows")))]
        let (changed, identity) = (modified, (0, 0));
        Ok(Self {
            len: meta.len(),
            modified,
            changed,
            identity,
        })
    }

    /// The later of the modification and the change time: a hash taken within [`HASH_CACHE_SETTLED`] of it
    /// may have missed a write in the same timer tick.
    pub(super) fn last_touched(&self) -> std::time::SystemTime {
        self.modified.max(self.changed)
    }

    /// Is `now` at least [`HASH_CACHE_SETTLED`] past the file's last change? A hash of a file that is not settled
    /// may have missed a write in the same timer tick (task 2526: the reclaim keeps such a copy).
    pub(crate) fn settled_at(&self, now: std::time::SystemTime) -> bool {
        now >= self.last_touched() + HASH_CACHE_SETTLED
    }
}

/// S6 (R8): what a hydrate's start look saw at a file's place, taken before anything is downloaded. The place is
/// compared with it right before it is replaced, so bytes written there after the look are never replaced. `None`:
/// nothing there (an error reads as nothing there, as the last look's does: a failed look then an answered one
/// refuses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceSeen {
    pub(super) place: PathBuf,
    pub(super) fingerprint: Option<ContentFingerprint>,
}

/// One look at a place (R10): seen; or written within [`HASH_CACHE_SETTLED`], so wait this long and look once more
/// (file times are coarse: a write in the same timer tick would look unchanged); or dated ahead of the clock, which
/// never settles in time, so refuse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Settle {
    Seen(PlaceSeen),
    Wait(std::time::Duration),
    Refuse,
}

/// Past the settle boundary: the sleep is measured on the monotonic clock, the second look on the wall clock.
const SETTLE_MARGIN: std::time::Duration = std::time::Duration::from_millis(25);

impl PlaceSeen {
    /// The look at `place`, at `now` (a test moves it past the settle boundary, as a change time cannot be set back).
    pub(crate) fn take_at(place: &Path, now: std::time::SystemTime) -> Settle {
        let seen = |fingerprint| {
            Settle::Seen(PlaceSeen {
                place: place.to_path_buf(),
                fingerprint,
            })
        };
        let Ok(fingerprint) = ContentFingerprint::of(place) else {
            return seen(None);
        };
        if fingerprint.settled_at(now) {
            return seen(Some(fingerprint));
        }
        match (fingerprint.last_touched() + HASH_CACHE_SETTLED).duration_since(now) {
            Ok(wait) if wait <= HASH_CACHE_SETTLED => Settle::Wait(wait),
            _ => Settle::Refuse,
        }
    }

    /// The look, blocking: at most ONE wait, then the second look must see the place settled; anything else refuses.
    pub fn take(place: &Path) -> Result<PlaceSeen, UnsentLocalChange> {
        match Self::take_at(place, std::time::SystemTime::now()) {
            Settle::Wait(wait) => {
                std::thread::sleep(wait + SETTLE_MARGIN);
                Self::look_again(place)
            }
            first => Self::seen_or_refuse(first),
        }
    }

    /// [`Self::take`] on an async worker.
    pub async fn take_async(place: &Path) -> Result<PlaceSeen, UnsentLocalChange> {
        match Self::take_at(place, std::time::SystemTime::now()) {
            Settle::Wait(wait) => {
                tokio::time::sleep(wait + SETTLE_MARGIN).await;
                Self::look_again(place)
            }
            first => Self::seen_or_refuse(first),
        }
    }

    fn look_again(place: &Path) -> Result<PlaceSeen, UnsentLocalChange> {
        Self::seen_or_refuse(Self::take_at(place, std::time::SystemTime::now()))
    }

    fn seen_or_refuse(look: Settle) -> Result<PlaceSeen, UnsentLocalChange> {
        match look {
            Settle::Seen(seen) => Ok(seen),
            Settle::Wait(_) | Settle::Refuse => {
                tracing::warn!("hydrate refused: the local file is being written, or dated ahead of the clock");
                Err(UnsentLocalChange)
            }
        }
    }
}

/// Task 1640 (review P2-5): [`sha256_file_hex`] of `file_id`'s file at `path`, read again only
/// when its [`ContentFingerprint`] changed since the last read: a scan, a close and a stamp
/// pass otherwise re-hashed every out-of-sync file (a plain file an atomic save left stays one)
/// every few seconds. A cached hash is trusted only when it was taken at least
/// [`HASH_CACHE_SETTLED`] after the file's modification and change times: file times are coarse (a
/// timer tick), so a later write inside the same tick would otherwise look unchanged. A hash whose
/// file changed while it was read is not kept. The cache is one per process, keyed by the file id and
/// the path together.
pub fn cached_sha256_file_hex(file_id: &str, path: &Path) -> std::io::Result<String> {
    cached_sha256_file_hex_at(file_id, path, std::time::SystemTime::now())
}

/// [`cached_sha256_file_hex`], the hash taken at `now`: a test moves it past [`HASH_CACHE_SETTLED`], as a
/// change time cannot be set back.
pub(super) fn cached_sha256_file_hex_at(
    file_id: &str,
    path: &Path,
    now: std::time::SystemTime,
) -> std::io::Result<String> {
    type CacheEntry = (ContentFingerprint, std::time::SystemTime, String);
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<(String, PathBuf), CacheEntry>>> =
        std::sync::OnceLock::new();
    let fingerprint = ContentFingerprint::of;
    let before = fingerprint(path)?;
    let cache = CACHE.get_or_init(Default::default);
    let key = (file_id.to_string(), path.to_path_buf());
    if let Some((seen, hashed_at, hash)) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(&key)
        && *seen == before
        && *hashed_at >= before.last_touched() + HASH_CACHE_SETTLED
    {
        return Ok(hash.clone());
    }
    let hashed_at = now;
    #[cfg(any(test, feature = "test-support"))]
    HASH_READS.with(|reads| reads.set(reads.get() + 1));
    let hash = sha256_file_hex(path)?;
    if fingerprint(path)? == before {
        cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key, (before, hashed_at, hash.clone()));
    }
    Ok(hash)
}

/// How long after a file's modification and change times a hash of it must have been taken to be reused.
pub(super) const HASH_CACHE_SETTLED: std::time::Duration = std::time::Duration::from_secs(2);

/// [`sha256_hex`] of the file at `path`, read in a stream.
pub fn sha256_file_hex(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Normalize the legacy JSON transport envelope, without decryption or format fallback.
pub fn content_frame(bytes: &[u8]) -> Result<Vec<u8>, EngineError> {
    if let Ok(blob) = serde_json::from_slice::<EncryptedBlob>(bytes) {
        if blob.nonce.len() != 12 {
            return Err(anyhow::anyhow!("invalid chunk envelope").into());
        };
        let mut frame = blob.nonce;
        frame.extend(blob.ciphertext);
        Ok(frame)
    } else {
        Ok(bytes.to_vec())
    }
}

#[cfg(any(target_os = "windows", any(test, feature = "test-support")))]
pub(super) fn chunk_layout_matches_metadata(size_bytes: u64, chunk_count: u64, chunk_size_bytes: u64) -> bool {
    if chunk_count == 0 || chunk_size_bytes == 0 {
        return false;
    }

    let size = size_bytes as u128;
    let count = chunk_count as u128;
    let chunk = chunk_size_bytes as u128;
    if count == 1 {
        return size <= chunk;
    }

    let full_prefix = (count - 1) * chunk;
    let full_span = count * chunk;
    size > full_prefix && size <= full_span
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    /// S6 (R10): a place written a moment ago is not seen yet (wait, then look again); once settled it is seen; nothing
    /// there needs no settle.
    #[test]
    fn s6_start_a_place_written_a_moment_ago_is_not_seen_yet() {
        let dir = tempfile::tempdir().unwrap();
        let place = dir.path().join("a.txt");
        std::fs::write(&place, b"just written").unwrap();
        let now = SystemTime::now();
        let first = PlaceSeen::take_at(&place, now);
        let later = PlaceSeen::take_at(&place, now + Duration::from_secs(3));
        println!("s6 start look: at once {first:?}, 3 s later {later:?}");
        assert!(
            matches!(first, Settle::Wait(wait) if !wait.is_zero() && wait <= HASH_CACHE_SETTLED),
            "not seen yet: {first:?}"
        );
        assert!(
            matches!(&later, Settle::Seen(seen) if seen.fingerprint.is_some() && seen.place == place),
            "seen once settled: {later:?}"
        );
        let none = dir.path().join("none.txt");
        assert_eq!(
            PlaceSeen::take_at(&none, now),
            Settle::Seen(PlaceSeen {
                place: none,
                fingerprint: None
            }),
            "nothing there is seen at once"
        );
    }

    /// S6 (R10): a place dated ahead of the clock (a tool set its modification time) never settles in time: refused at
    /// once, with no long sleep.
    #[test]
    fn s6_start_a_place_dated_ahead_refuses_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let place = dir.path().join("a.txt");
        std::fs::write(&place, b"dated ahead").unwrap();
        let now = SystemTime::now();
        let file = std::fs::File::options().write(true).open(&place).unwrap();
        file.set_modified(now + Duration::from_secs(3600)).unwrap();
        drop(file);
        let look = PlaceSeen::take_at(&place, now);
        println!("s6 start look, dated ahead: {look:?}");
        assert_eq!(look, Settle::Refuse);
        let started = std::time::Instant::now();
        assert!(PlaceSeen::take(&place).is_err(), "the blocking look refuses too");
        assert!(started.elapsed() < Duration::from_secs(1), "without a long sleep");
    }
}
