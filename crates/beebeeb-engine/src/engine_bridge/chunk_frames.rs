//! `chunk_frames`: one content frame from the API, and a range of them decrypted into one buffer. Moved out of
//! `hydrate.rs` (the file-size limit) unchanged but for the in-memory limit a viewer's open passes.

use super::*;
use futures_util::StreamExt as _;

impl EngineBridge {
    /// Download + decrypt chunk indices `[range.start, range.end)` for
    /// `file_id`, concatenating the plaintext into one [`Zeroizing`] buffer.
    /// Shared by [`Self::do_hydrate`] (the whole-file range `0..chunk_count`)
    /// and [`Self::hydrate_file_range`] (a covering sub-range) so the
    /// pacing/wire-counting/zeroize discipline lives in exactly one place.
    ///
    /// `approx_size_hint` only sizes the initial allocation (`Vec::with_capacity`)
    /// — it never bounds or truncates the actual read, so a wrong hint costs at
    /// most a reallocation, never a silent short read.
    ///
    /// Task 1643: with a `pin`, every chunk request carries the pinned version's id and every chunk's
    /// wire size must be the manifest's; a 409 comes back as [`crate::download_pin::VersionGone`].
    /// Without one (a legacy file) the requests are today's unpinned ones.
    pub(super) async fn download_content_frame(
        &self,
        file_id: &str,
        i: u32,
        pin: Option<&crate::download_pin::PinnedVersion>,
        meter: Option<&crate::transfer_progress::ReceiveMeter>,
        // The longest body an unpinned chunk may have, for a caller with a limit. `None`: unbounded, as before.
        cap: Option<crate::api_client::BodyCap>,
    ) -> Result<Vec<u8>, EngineError> {
        // LD-0105 S0: every piece of the body is counted as it arrives, not once the chunk is whole.
        let on_body = |b: crate::api_client::BodyBytes| {
            if let Some(meter) = meter {
                match b {
                    crate::api_client::BodyBytes::Read(n) => meter.add(n),
                    crate::api_client::BodyBytes::Discarded(n) => meter.discard(n),
                }
            }
        };
        Ok(match pin {
            // Task 1643: the pinned route (shared with the direct path's fallback): exactly the manifest's
            // length, a transient failure tried again, a 409 = the version is gone, no URL in an error.
            Some(pin) => {
                let expected = pin
                    .wire_size(i)
                    .ok_or_else(|| anyhow::anyhow!("chunk {i} is outside the pinned version"))?;
                self.api
                    .download_chunk_pinned(file_id, i, &pin.object_version_id, expected, &on_body)
                    .await
                    .map_err(|e| match e {
                        crate::api_client::ChunkApiError::Restart => {
                            anyhow::Error::new(crate::download_pin::VersionGone)
                        }
                        crate::api_client::ChunkApiError::Terminal(status) => {
                            anyhow::Error::new(crate::direct_download::DirectError::Refused(status))
                        }
                        crate::api_client::ChunkApiError::Failed(message) => anyhow::anyhow!(message),
                        // The same message, with a cause a caller can read "can't reach" from.
                        crate::api_client::ChunkApiError::Unreachable(message) => {
                            anyhow::Error::new(crate::api_client::ChunkUnreachable(message))
                        }
                    })?
            }
            None => match cap {
                Some(cap) => self.api.download_chunk_capped(file_id, i, cap, &on_body).await?,
                None => self.api.download_chunk(file_id, i, &on_body).await?,
            },
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn download_and_decrypt_chunk_range(
        &self,
        file_id: &str,
        file_key: &beebeeb_core::kdf::FileKey,
        range: std::ops::Range<u32>,
        download_kbps_limit: u64,
        approx_size_hint: usize,
        progress: Option<&HydrateProgressFn>,
        meter: Option<&crate::transfer_progress::ReceiveMeter>,
        pin: Option<&crate::download_pin::PinnedVersion>,
        record: crate::content_record::ContentRecord,
        chunk_count: u32,
        // The most plaintext the caller holds ([`Self::hydrate_whole_file_within`]). `None`: no limit.
        limit: Option<u64>,
    ) -> Result<Zeroizing<Vec<u8>>, EngineError> {
        // A limit never sizes the first allocation from what the metadata says.
        let approx_size_hint = match limit {
            Some(limit) => approx_size_hint.min(usize::try_from(limit).unwrap_or(usize::MAX)),
            None => approx_size_hint,
        };
        let mut acc: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(approx_size_hint));
        // Task 1670 issue 3: tell the caller (the macOS IPC handler, which
        // forwards it to Finder's progress bar) how far along we are. `total`
        // is the size hint (server's plaintext size); the initial 0/total
        // report lets the UI switch from indeterminate to determinate before
        // the first chunk lands.
        let total = approx_size_hint as u64;
        // Task 1683 slice 2: the board first, so a caller that reads it from inside its
        // own callback sees the same bytes it was just told about. A no-op for a file
        // that is not registered (a partial range read).
        self.transfers.report(file_id, 0, total);
        if let Some(report) = progress {
            report(0, total);
        }

        // Core's readers, built from the file's authenticated record; desktop owns no crypto. A whole
        // read is one sequential stream (order, last frame and `finish` are checked). A partial read
        // opens each chunk at its own index against the record's version commitment, so it never
        // trusts a served frame 0 to say which upload it is reading.
        let whole = range.start == 0 && range.end == chunk_count;
        let mut reader = record.decryptor(beebeeb_core::kdf::FileKey::from_bytes(*file_key.as_bytes()), file_id);
        // RC6 (dm-58): the whole-file API route fetched one chunk at a time, so a 367 MB Finder download ran at
        // one round trip per chunk. Frames are now fetched a few at a time, in order; decryption stays in order.
        let parallel = api_fetch_parallel(limit, download_kbps_limit, range.len());
        let mut prefetched = (parallel > 1).then(|| {
            Box::pin(ordered_fetch(range.clone(), parallel, move |i| async move {
                self.download_content_frame(file_id, i, pin, meter, None).await
            }))
        });
        let started = std::time::Instant::now();
        for i in range {
            let chunk_start = std::time::Instant::now();
            let chunk_bytes = if let Some(frames) = prefetched.as_mut() {
                frames
                    .next()
                    .await
                    .ok_or_else(|| anyhow::anyhow!("chunk {i} was not fetched"))??
            } else {
                // What a chunk of the plaintext still allowed may be on the wire: a longer body is given up as it
                // streams, and the limit is the answer.
                let cap = limit.map(|limit| body_cap(limit, acc.len() as u64));
                match self.download_content_frame(file_id, i, pin, meter, cap).await {
                    Err(e) if e.is_caused_by::<crate::api_client::ChunkOverLimit>() => {
                        return Err(anyhow::Error::new(OverInMemoryLimit {
                            held: acc.len() as u64,
                            body_given_up: true,
                        })
                        .into());
                    }
                    other => other?,
                }
            };
            let wire_len = chunk_bytes.len() as u64;
            // Decrypt into a temporary buffer, copy into the accumulator,
            // then zeroize the temporary. This limits live plaintext to
            // the accumulator + one chunk at any moment.
            let frame = content_frame(&chunk_bytes)?;
            let mut decrypted = if whole {
                reader.push_frame(&frame)?.data
            } else {
                record.open_chunk(file_key, file_id, i, i + 1 == chunk_count, &frame)?
            };
            // Asked BEFORE the chunk is added: what is held never passes the limit, not even for a moment.
            if limit.is_some_and(|limit| decrypted.len() as u64 > limit.saturating_sub(acc.len() as u64)) {
                decrypted.zeroize();
                return Err(anyhow::Error::new(OverInMemoryLimit {
                    held: acc.len() as u64,
                    body_given_up: false,
                })
                .into());
            }
            acc.extend_from_slice(&decrypted);
            decrypted.zeroize();
            self.transfers.report(file_id, acc.len() as u64, total);
            if let Some(report) = progress {
                report(acc.len() as u64, total);
            }

            // P1 — wire-byte counter: count raw wire bytes received.
            self.wire.download_bytes.fetch_add(wire_len, Ordering::Relaxed);

            // E — token-bucket pacing for downloads.
            if download_kbps_limit > 0 {
                let budget_secs = wire_len as f64 / (download_kbps_limit as f64 * 1024.0);
                let elapsed_secs = chunk_start.elapsed().as_secs_f64();
                if budget_secs > elapsed_secs {
                    let sleep_ms = ((budget_secs - elapsed_secs) * 1000.0) as u64;
                    if sleep_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                    }
                }
            }
        }

        if whole {
            reader.finish()?;
        }
        tracing::info!(
            source = "api",
            parallel,
            bytes = acc.len() as u64,
            millis = started.elapsed().as_millis() as u64,
            rate = %rate_label(acc.len() as u64, started.elapsed()),
            "hydrate chunks fetched"
        );
        Ok(acc)
    }
}

/// How many chunk frames the API route keeps in flight (the direct route's window is four too).
pub(super) const API_FETCH_PARALLEL: usize = 4;

/// One at a time when a limit caps each body by what is left of it, or the person's ceiling paces each chunk;
/// otherwise [`API_FETCH_PARALLEL`] (never more than there are chunks).
pub(super) fn api_fetch_parallel(limit: Option<u64>, download_kbps_limit: u64, chunks: usize) -> usize {
    if limit.is_some() || download_kbps_limit > 0 {
        1
    } else {
        API_FETCH_PARALLEL.min(chunks.max(1))
    }
}

/// `fetch(i)` for every index of `range`, up to `parallel` running at once, yielded in index order.
pub(super) fn ordered_fetch<T, F, Fut>(
    range: std::ops::Range<u32>,
    parallel: usize,
    fetch: F,
) -> impl futures_util::Stream<Item = T>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = T>,
{
    futures_util::stream::iter(range).map(fetch).buffered(parallel.max(1))
}

/// "12.3 MB/s" for the hydrate log line.
pub(super) fn rate_label(bytes: u64, took: Duration) -> String {
    let secs = took.as_secs_f64().max(0.001);
    format!("{:.2} MB/s", bytes as f64 / 1_000_000.0 / secs)
}

#[cfg(test)]
mod parallel_tests {
    use super::*;
    use futures_util::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A 300 MB file in 4 MB chunks is fetched with the planned number of requests in flight, in order.
    #[tokio::test]
    async fn a_300_mb_hydrate_fetches_chunks_in_parallel_and_in_order() {
        let chunks = 300_000_000usize.div_ceil(4_000_000);
        let parallel = api_fetch_parallel(None, 0, chunks);
        assert_eq!(parallel, API_FETCH_PARALLEL);
        let (now, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let out: Vec<u32> = ordered_fetch(0..chunks as u32, parallel, |i| {
            let (now, peak) = (&now, &peak);
            async move {
                let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(if i % 2 == 0 { 8 } else { 2 })).await;
                now.fetch_sub(1, Ordering::SeqCst);
                i
            }
        })
        .collect()
        .await;
        assert_eq!(out, (0..chunks as u32).collect::<Vec<_>>());
        assert_eq!(peak.load(Ordering::SeqCst), parallel);
    }

    #[test]
    fn a_limit_or_a_ceiling_keeps_one_fetch_at_a_time() {
        assert_eq!(api_fetch_parallel(Some(10), 0, 50), 1);
        assert_eq!(api_fetch_parallel(None, 500, 50), 1);
        assert_eq!(api_fetch_parallel(None, 0, 2), 2);
        assert_eq!(rate_label(300_000_000, Duration::from_secs(10)), "30.00 MB/s");
    }
}
