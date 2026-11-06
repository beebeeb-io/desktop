//! LD-0105 W2: what a Windows range session needs from the engine before it starts, and what the
//! engine records once it ended whole.
//!
//! Before: the pinned version (spec S3: a legacy file with no `download_version` keeps the
//! whole-file path and never streams), the placeholder's identity (task 2180: the minted version,
//! plus the format and v2 version id a session recorded), the 0783 size check against the size
//! Windows holds, the file key, the transport and the download limit, and one 1640 [`FetchTicket`]
//! for the whole session. After: the bytes become agreed content only on the session's one "whole"
//! verdict (I6), through [`EngineBridge::record_whole_fetch`].

use super::*;

/// A range session, ready to start.
pub(crate) struct RangePrep {
    pub(crate) request: beebeeb_transfer::RangeRequest,
    pub(crate) file_key: beebeeb_core::kdf::FileKey,
    /// The pinned version's number (`download_version.version_number`), for the 1640 candidate.
    pub(crate) version_number: Option<i64>,
    /// `desktop.toml` `download_kbps_limit` (0: none).
    pub(crate) kbps_limit: u64,
    /// Task 1640: one ticket per session, held until the session ended.
    pub(crate) ticket: FetchTicket,
}

/// Why no range session starts.
#[derive(Debug)]
pub(crate) enum PrepEnd {
    /// Spec S3: no version to pin (a legacy file). Today's whole-file path serves it.
    WholeFilePath,
    /// Task 0783: the version holds this many plaintext bytes, not the placeholder's size.
    ResolveSize(u64),
    Failed(anyhow::Error),
}

impl EngineBridge {
    /// Desktop #225 review P1: can `file_id` stream, so W3 may decline its open-time fetch? Only with
    /// a recorded version to pin (state.db's minted version; a legacy file has none, and is answered
    /// without a network call) and the direct range route (the transport not forced to the API, and
    /// the server's discovery offering direct downloads: cached for its trust period, else one call).
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) async fn can_stream(&self, file_id: &str) -> bool {
        let recorded = matches!(self.db.minted_version(file_id), Ok(Some(_)));
        if !recorded {
            return false;
        }
        let config = crate::config::DesktopConfig::load().ok();
        let forced_api = self.download_transport(config.as_ref().and_then(|c| c.transport.as_deref())) == crate::direct_download::Transport::Api;
        let offered = !forced_api && self.api.direct_download_offer().await.is_some();
        crate::cf_range::file_streams(recorded, forced_api, offered)
    }

    /// Prepare a range session for `file_id`, whose placeholder Windows holds at `file_size` bytes
    /// with `on_disk_bytes` of them on disk. Nothing is fetched from storage here: one metadata call.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) async fn range_prep(&self, file_id: &str, file_size: u64, on_disk_bytes: u64) -> Result<RangePrep, PrepEnd> {
        let failed = |e: anyhow::Error| PrepEnd::Failed(e);
        let meta = self.api.get_file(file_id).await.map_err(|e| failed(e.into()))?;
        let Some(pin) = crate::download_pin::PinnedVersion::from_metadata(&meta).map_err(|e| failed(e.into()))? else {
            return Err(PrepEnd::WholeFilePath);
        };
        let minted = self.db.minted_version(file_id).map_err(|e| failed(e.into()))?;
        // The session pins the placeholder's minted version (I4). With nothing on disk there is nothing
        // to trust, so a placeholder minted from an older version is served the current one (its size
        // is checked below; an equal-size newer version is served whole, T11).
        let continues_minted = on_disk_bytes > 0 && minted.as_deref().is_some_and(|m| !m.eq_ignore_ascii_case(&pin.object_version_id));
        let (object_version_id, geometry) = if continues_minted {
            // Its bytes are the minted version's: the session continues that version or fails (S6).
            // The current version's geometry is not that version's, so the session learns it.
            (minted.clone().unwrap_or_default(), None)
        } else {
            crate::cf_range::size_check(pin.plaintext_size_bytes, file_size).map_err(PrepEnd::ResolveSize)?;
            (
                pin.object_version_id.clone(),
                Some(beebeeb_core::direct_transfer::window::Geometry {
                    chunk_count: pin.chunk_count,
                    chunk_size: pin.chunk_size_bytes,
                    encrypted_size: pin.ciphertext_size_bytes,
                    original_size: pin.plaintext_size_bytes,
                }),
            )
        };
        let recorded = crate::cf_range::recorded_identity(self.db.range_identity(file_id).map_err(|e| failed(e.into()))?.as_ref());
        let (file_key, name_key) = self.file_keys_for_download(file_id).await.map_err(|e| failed(e.into()))?;
        let expected = recorded_content_format(&name_key, &meta).map_err(|e| failed(e.into()))?;
        let config = crate::config::DesktopConfig::load().ok();
        let kbps_limit = config.as_ref().map(|c| c.download_kbps_limit).unwrap_or(0);
        let transport = self.download_transport(config.as_ref().and_then(|c| c.transport.as_deref()));
        let ticket = self.begin_fetch(file_id);
        self.db.set_status(file_id, FileStatus::Downloading).map_err(|e| failed(e.into()))?;
        Ok(RangePrep {
            request: beebeeb_transfer::RangeRequest {
                api_base: self.api.api_base().to_string(),
                file_id: file_id.to_string(),
                object_version_id,
                file_size,
                pin: expected,
                geometry,
                recorded,
                on_disk_bytes,
                forced_api: transport == crate::direct_download::Transport::Api,
                jitter_seed: now_secs() as u64,
            },
            file_key,
            // Review P1: bytes of an older minted version are never recorded as the current version's
            // candidate or baseline (a lost update); with no number the content stays not known.
            version_number: if continues_minted { None } else { meta["download_version"]["version_number"].as_i64() },
            kbps_limit,
            ticket,
        })
    }

    /// The range session ended whole (I6): the file is this version's, `hash` its sha256 when it
    /// could be read under an unchanged content key (else `None`: the candidate stays, never agreed).
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn finish_whole_range(&self, file_id: &str, size: u64, version: Option<i64>, hash: Option<String>, ticket: &FetchTicket) {
        if let (Some(version), Some(hash)) = (version, hash) {
            let fetched = FetchedVersion { version, hash };
            if let Err(e) = self.record_whole_fetch(file_id, &fetched, ticket) {
                tracing::warn!(file_id = %file_id, error = %e, "could not record the streamed file as agreed");
            }
        }
        let _ = self.db.set_status(file_id, FileStatus::Local);
        let _ = self.db.mark_cached(file_id, "", size as i64, now_secs());
        let _ = self.enforce_configured_cache_limit();
    }
}
