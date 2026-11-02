//! The cached storage summary: when a snapshot may reuse the API's numbers, when it
//! must wait for a fetch, and when a failure is remembered instead of retried (spec
//! section 9, task 2179).
//!
//! The cache deduplicates concurrent fetches by ticket, drops an answer that a
//! sign-out or a newer session replaced, and never holds a snapshot longer than
//! `STORAGE_WAIT`. The fetch itself lives here too; nothing else talks to the storage
//! endpoint.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StorageUsage {
    pub used_bytes: i64,
    pub quota_bytes: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CacheEntry {
    pub(super) usage: StorageUsage,
    pub(super) fetched_at: i64,
    /// The in-memory session the summary was fetched under (`VAULT_EPOCH` in `lib.rs`, which moves
    /// whenever a session is installed or cleared): a sign-in, a lock or a sign-out changes it, so
    /// a summary from before is refetched. Not `UI_SESSION_REVISION`: a lock does not move that.
    pub(super) vault_epoch: u64,
}

#[derive(Debug, Default)]
pub struct StorageCache {
    pub(super) entry: Option<CacheEntry>,
    /// The fetch in flight: a snapshot meanwhile waits for it (at most [`STORAGE_WAIT`]) instead of
    /// starting another, and only its answer may land (#139 review P2-3).
    pub(super) inflight: Option<Inflight>,
    /// When a fetch for a session last failed (when its failure came, #139 review P3-9).
    pub(super) failed_at: Option<(u64, i64)>,
    /// Numbers the fetches. Kept across [`StorageCache::clear`], so a fetch from before a clear
    /// never holds the ticket of one after it.
    pub(super) next_ticket: u64,
}

#[derive(Debug)]
pub(super) struct Inflight {
    pub(super) ticket: u64,
    pub(super) vault_epoch: u64,
    pub(super) done: tokio::sync::watch::Receiver<bool>,
}

impl StorageCache {
    /// Forget the summary, the failure and the fetch in flight, whose answer then lands nowhere: what
    /// sign-out leaves of the account before (task 1887's rule; #139 review P2-3).
    pub fn clear(&mut self) {
        *self = Self {
            next_ticket: self.next_ticket,
            ..Self::default()
        };
    }

    /// Nothing cached, failed or in flight (what a sign-out leaves).
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entry.is_none() && self.failed_at.is_none() && self.inflight.is_none()
    }

    /// `ticket` is the fetch in flight: a newer session's fetch or a clear has not taken its place.
    pub(super) fn holds(&self, ticket: u64) -> bool {
        self.inflight.as_ref().is_some_and(|inflight| inflight.ticket == ticket)
    }

    /// A fetch for this session failed less than [`STORAGE_FAILURE_BACKOFF_SECS`] ago.
    pub(super) fn recently_failed(&self, now: i64, vault_epoch: u64) -> bool {
        self.failed_at
            .is_some_and(|(epoch, at)| epoch == vault_epoch && now.saturating_sub(at) < STORAGE_FAILURE_BACKOFF_SECS)
    }

    /// The best summary to show without fetching: the fresh one, else the last known one marked stale.
    pub(super) fn answer(&self, now: i64, vault_epoch: u64) -> Option<StorageDto> {
        if let Some((usage, fetched_at)) = self.fresh(now, vault_epoch) {
            return Some(StorageDto::of(usage, fetched_at, false));
        }
        self.last_known(vault_epoch)
            .map(|(usage, fetched_at)| StorageDto::of(usage, fetched_at, true))
    }

    /// The cached summary if it is from this session and younger than the TTL.
    pub(super) fn fresh(&self, now: i64, vault_epoch: u64) -> Option<(StorageUsage, i64)> {
        self.entry
            .filter(|e| e.vault_epoch == vault_epoch && now.saturating_sub(e.fetched_at) < STORAGE_TTL_SECS)
            .map(|e| (e.usage, e.fetched_at))
    }

    /// The cached summary from this session whatever its age (used when the
    /// refetch fails: an old number, marked stale, beats an empty header).
    pub(super) fn last_known(&self, vault_epoch: u64) -> Option<(StorageUsage, i64)> {
        self.entry
            .filter(|e| e.vault_epoch == vault_epoch)
            .map(|e| (e.usage, e.fetched_at))
    }

    pub(super) fn put(&mut self, usage: StorageUsage, fetched_at: i64, vault_epoch: u64) {
        self.entry = Some(CacheEntry {
            usage,
            fetched_at,
            vault_epoch,
        });
    }
}

/// [`StorageCache::clear`] through the shared handle, poisoned or not: a sign-out never fails on it.
pub fn clear_storage(cache: &Mutex<StorageCache>) {
    cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
}

/// One fetch's turn, ended however its task ends (an answer, a panic, a cancelled runtime): the slot
/// is freed if it is still this fetch's, and every snapshot waiting on it is woken (#139 review P3-8).
pub(super) struct FetchTurn {
    pub(super) cache: Arc<Mutex<StorageCache>>,
    pub(super) ticket: u64,
    pub(super) done: tokio::sync::watch::Sender<bool>,
}

impl Drop for FetchTurn {
    fn drop(&mut self) {
        let mut guard = self.cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.holds(self.ticket) {
            guard.inflight = None;
        }
        drop(guard);
        let _ = self.done.send(true);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StorageDto {
    pub used_bytes: i64,
    pub quota_bytes: i64,
    pub fetched_at: i64,
    /// `true` when the refetch failed and this is an older number.
    pub stale: bool,
}

impl StorageDto {
    pub(super) fn of(usage: StorageUsage, fetched_at: i64, stale: bool) -> Self {
        StorageDto {
            used_bytes: usage.used_bytes,
            quota_bytes: usage.quota_bytes,
            fetched_at,
            stale,
        }
    }
}

/// The storage summary for a snapshot: the cached one while it is fresh; otherwise what a
/// fetch returns within [`STORAGE_WAIT`]; otherwise an older cached one marked stale;
/// otherwise nothing (the header then shows the email only).
///
/// The fetch runs as a task of its own (one per session at a time) and fills the cache
/// when it finishes, so a fetch slower than the wait still serves the next snapshot. Only
/// the fetch still in flight writes: one that a newer session's fetch or a sign-out's
/// clear replaced answers into nothing. A failed fetch is not started again for
/// [`STORAGE_FAILURE_BACKOFF_SECS`] after its failure came. No lock is held across an await.
pub async fn cached_storage<F, Fut>(cache: &Arc<Mutex<StorageCache>>, now: i64, vault_epoch: u64, fetch: F) -> Option<StorageDto>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<StorageUsage, String>> + Send + 'static,
{
    cached_storage_with(cache, now, vault_epoch, STORAGE_WAIT, fetch).await
}

/// [`cached_storage`] with the wait as an argument (#139 review P3-11: tests pass their own).
pub async fn cached_storage_with<F, Fut>(
    cache: &Arc<Mutex<StorageCache>>,
    now: i64,
    vault_epoch: u64,
    wait: std::time::Duration,
    fetch: F,
) -> Option<StorageDto>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<StorageUsage, String>> + Send + 'static,
{
    let mut finished = {
        let mut guard = cache.lock().ok()?;
        if guard.fresh(now, vault_epoch).is_some() || guard.recently_failed(now, vault_epoch) {
            return guard.answer(now, vault_epoch);
        }
        match &guard.inflight {
            Some(inflight) if inflight.vault_epoch == vault_epoch => inflight.done.clone(),
            _ => {
                let (done, running) = tokio::sync::watch::channel(false);
                let ticket = guard.next_ticket;
                guard.next_ticket = ticket.wrapping_add(1);
                guard.inflight = Some(Inflight {
                    ticket,
                    vault_epoch,
                    done: running.clone(),
                });
                let pending = fetch();
                let turn = FetchTurn {
                    cache: Arc::clone(cache),
                    ticket,
                    done,
                };
                let started = tokio::time::Instant::now();
                tokio::spawn(async move {
                    let result = pending.await;
                    // When the answer came, on the clock `now` is on (a test's paused clock too).
                    let at = now.saturating_add(i64::try_from(started.elapsed().as_secs()).unwrap_or(i64::MAX));
                    if let Ok(mut guard) = turn.cache.lock()
                        && guard.holds(turn.ticket)
                    {
                        match result {
                            Ok(usage) => {
                                guard.put(usage, now, vault_epoch);
                                guard.failed_at = None;
                            }
                            Err(error) => {
                                tracing::debug!(error = %error, "storage summary refetch failed; using the last known one if any");
                                guard.failed_at = Some((vault_epoch, at));
                            }
                        }
                    }
                    drop(turn);
                });
                running
            }
        }
    };
    // Wait for the fetch, briefly: a slower one finishes in the background either way.
    let _ = tokio::time::timeout(wait, async {
        while !*finished.borrow_and_update() {
            if finished.changed().await.is_err() {
                break;
            }
        }
    })
    .await;
    cache.lock().ok()?.answer(now, vault_epoch)
}

#[derive(Debug, Deserialize)]
pub(super) struct BillingUsage {
    pub(super) used_bytes: i64,
    pub(super) quota_bytes: i64,
}

/// `GET /api/v1/billing/usage` with the session token. A short timeout: the
/// popover's first frame must not wait 30 s on a dead server. The error text never
/// contains the token.
pub async fn fetch_storage_usage(api_base: &str, token: &str) -> Result<StorageUsage, String> {
    let client = reqwest::Client::builder()
        .default_headers(crate::api_client::provenance_headers())
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| format!("reqwest build: {e}"))?;
    let usage: BillingUsage = client
        .get(format!("{api_base}/api/v1/billing/usage"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("load storage usage: {e}"))?
        .error_for_status()
        .map_err(|e| format!("load storage usage: {e}"))?
        .json()
        .await
        .map_err(|e| format!("parse storage usage: {e}"))?;
    Ok(StorageUsage {
        used_bytes: usage.used_bytes,
        quota_bytes: usage.quota_bytes,
    })
}

/// Used at or over a positive quota. A quota of 0 or less means "unknown", never
/// "full".
pub fn storage_is_full(usage: &StorageUsage) -> bool {
    usage.quota_bytes > 0 && usage.used_bytes >= usage.quota_bytes
}
