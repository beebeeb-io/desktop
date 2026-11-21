//! The engine's calls over the local thumbnail cache (079, [`crate::thumb_cache`]): a photo the server has no
//! thumbnail for is decoded once on this PC, kept sealed, and handed back from the cache after that.
//!
//! - Every call is FOR a session: the account key is copied only while that session is the one in memory, and a
//!   thumbnail is handed over only while it still is.
//! - [`Engine::local_thumbnail_for`] answers the newest kept version; [`Engine::local_thumbnail_at`] answers only the
//!   version asked for (a changed file's old thumbnail is stale).
//! - Sign-out and account switch call [`Engine::wipe_thumb_cache_at_session_end`].
//!
//! BLOCKS: the disk. Must not: log a name, a path or a byte.

use beebeeb_core::kdf::MasterKey;
use zeroize::Zeroizing;

use super::photos_api::ThumbPixels;
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::thumb_cache::{CachedThumb, DEFAULT_CAP_MB, ThumbCache, ThumbCacheError, wipe_account};

impl<C: CredentialStores> Engine<C> {
    /// The account key of `session`, copied while it is the session in memory.
    fn thumb_cache_key(&self, session: SessionGeneration) -> Option<Zeroizing<[u8; 32]>> {
        if self.acct.session.is_poisoned() {
            return None;
        }
        self.acct
            .while_session_is(&self.writes, session, |current| Zeroizing::new(current.master_key))
    }

    fn with_thumb_cache<T>(
        &self,
        session: SessionGeneration,
        with: impl FnOnce(&ThumbCache<'_>) -> T,
    ) -> Option<(T, Zeroizing<[u8; 32]>)> {
        let key = self.thumb_cache_key(session)?;
        let master = MasterKey::from_bytes(*key);
        let cache = ThumbCache::new(&self.config.state_dir, &self.config.account_id, &master, DEFAULT_CAP_MB);
        Some((with(&cache), key))
    }

    fn hand_over(
        &self,
        session: SessionGeneration,
        found: Option<(Option<CachedThumb>, Zeroizing<[u8; 32]>)>,
    ) -> Option<ThumbPixels> {
        let (thumb, key) = found?;
        let thumb = thumb?;
        // The hand-over: only while the session in memory is the one the key was copied from.
        if !self.session_is_still(session, &key) {
            return None;
        }
        Some(ThumbPixels {
            session,
            width: thumb.width,
            height: thumb.height,
            rgba: thumb.rgba,
        })
    }

    /// The thumbnail this PC made of `file_id`, from the cache, FOR `session`. `None`: none kept, it does not open,
    /// or `session` is not the session in memory.
    pub fn local_thumbnail_for(&self, session: SessionGeneration, file_id: &str) -> Option<ThumbPixels> {
        let found = self.with_thumb_cache(session, |cache| cache.get(file_id));
        self.hand_over(session, found)
    }

    /// [`Self::local_thumbnail_for`], only if it was made from `version` (another version's is stale).
    pub fn local_thumbnail_at(&self, session: SessionGeneration, file_id: &str, version: i64) -> Option<ThumbPixels> {
        let found = self.with_thumb_cache(session, |cache| cache.get_at(file_id, version));
        self.hand_over(session, found)
    }

    /// WRITES (the disk): keep `pixels` as the thumbnail of `file_id` at `version`, sealed, FOR `session`. Pixels
    /// decoded for another session are refused.
    pub fn keep_local_thumbnail(
        &self,
        session: SessionGeneration,
        file_id: &str,
        version: i64,
        pixels: &ThumbPixels,
    ) -> Result<(), ThumbCacheError> {
        if pixels.session != session {
            return Err(ThumbCacheError::NotSignedIn);
        }
        let (result, key) = self
            .with_thumb_cache(session, |cache| {
                cache.put(file_id, version, pixels.width, pixels.height, &pixels.rgba)
            })
            .ok_or(ThumbCacheError::NotSignedIn)?;
        result?;
        // Kept under a session that ended meanwhile: the wipe may have run before the write landed. Take it back.
        if !self.session_is_still(session, &key) {
            let _ = wipe_account(&self.config.state_dir, &self.config.account_id);
            return Err(ThumbCacheError::NotSignedIn);
        }
        Ok(())
    }

    /// WRITES (the disk), at sign-out and account switch: every cached thumbnail of this account goes. A failure is
    /// logged (no name, no path) and does not stop the sign-out.
    pub(crate) fn wipe_thumb_cache_at_session_end(&self) {
        if wipe_account(&self.config.state_dir, &self.config.account_id).is_err() {
            tracing::warn!("thumbnail cache not deleted at session end");
        }
    }
}
