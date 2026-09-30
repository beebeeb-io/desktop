//! Idempotency table for Finder write-queue requests (task 1684).
//!
//! **The problem.** `QueueFinderCreate` / `QueueFinderModify` with contents make
//! the daemon copy the whole file into staging before it replies. When that copy
//! outlasts the extension's socket timeout (`IPCFraming.stagedCopyTimeoutSeconds`,
//! 600 s) the extension reports failure while the daemon still queues the upload,
//! and the system then retries `createItem` / `modifyItem`. The retry is a NEW
//! invocation on a NEW connection, so without a stable key the daemon queues the
//! same upload a second time under a fresh uuid.
//!
//! **The mechanism.** The extension derives a key from the stable inputs of the
//! logical operation (`IPCWriteKey` in `BeebeebFileProvider/IPCFraming.swift`) and
//! sends it as `request_id`. This table maps key -> state for a bounded time:
//!
//! * absent      -> the caller becomes the leader and runs the work;
//! * `InFlight`  -> the caller waits for the leader and returns the leader's result
//!                  (the work is never run twice);
//! * `Done`      -> within the TTL the stored result is returned, after the caller's
//!                  `validate` closure confirms it still describes reality;
//! * a key that belongs to a different request shape (see `fingerprint`), or a
//!   table that is full of in-flight work, runs the work without deduplication
//!   (it fails open to the pre-1684 behaviour rather than refusing a user write).
//!
//! The leader's work runs on the blocking pool, detached from the connection: if
//! the leader's client hangs up (that is the whole scenario) the copy still
//! finishes and its result is recorded for the retry.
//!
//! The table is generic so it is unit-testable with a counting closure and an
//! injected clock; `ipc_socket.rs` instantiates it with `IpcResponse`.

#![cfg(unix)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::watch;

/// How long a finished result is remembered. Retries follow a timeout within
/// seconds to minutes; 30 minutes is generous for that and short enough that the
/// false-dedup window (see `docs/IPC_PROTOCOL.md`) stays small.
pub const DEFAULT_TTL: Duration = Duration::from_secs(30 * 60);

/// Hard cap on remembered keys. Each entry is a short string plus one small reply.
pub const DEFAULT_CAPACITY: usize = 4096;

/// Longest accepted key. The Swift side sends 64 hex characters (SHA-256).
pub const MAX_KEY_BYTES: usize = 128;

enum Entry<T> {
    InFlight {
        fingerprint: String,
        rx: watch::Receiver<Option<T>>,
        /// Identifies this leader so a stale guard can never clear a newer entry.
        generation: u64,
    },
    Done {
        fingerprint: String,
        value: T,
        at: Instant,
    },
}

struct State<T> {
    map: HashMap<String, Entry<T>>,
    next_generation: u64,
}

pub struct WriteDedup<T> {
    state: Mutex<State<T>>,
    ttl: Duration,
    capacity: usize,
}

/// What [`WriteDedup::claim`] decided for one request.
enum Claim<T> {
    Lead(Leader<T>),
    Wait(watch::Receiver<Option<T>>),
    Cached(T),
    Bypass,
}

/// Held by the request that runs the work. Dropping it without `complete` (the
/// work panicked) clears the in-flight entry so waiters retry instead of hanging.
struct Leader<T> {
    dedup: Arc<WriteDedup<T>>,
    key: String,
    generation: u64,
    fingerprint: String,
    tx: Option<watch::Sender<Option<T>>>,
}

impl<T: Clone> Leader<T> {
    fn complete(mut self, value: T, now: Instant, remember: bool) {
        {
            let mut state = self.dedup.lock();
            let still_ours = matches!(
                state.map.get(&self.key),
                Some(Entry::InFlight { generation, .. }) if *generation == self.generation
            );
            if still_ours {
                if remember {
                    state.map.insert(
                        self.key.clone(),
                        Entry::Done {
                            fingerprint: self.fingerprint.clone(),
                            value: value.clone(),
                            at: now,
                        },
                    );
                } else {
                    state.map.remove(&self.key);
                }
            }
        }
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Some(value));
        }
    }
}

impl<T> Drop for Leader<T> {
    fn drop(&mut self) {
        // Reached only when `complete` did not run (it consumes `tx`).
        if self.tx.is_some() {
            let mut state = self.dedup.lock();
            let still_ours = matches!(
                state.map.get(&self.key),
                Some(Entry::InFlight { generation, .. }) if *generation == self.generation
            );
            if still_ours {
                state.map.remove(&self.key);
            }
        }
        // Dropping `tx` wakes every waiter with "sender gone".
    }
}

impl<T> WriteDedup<T> {
    fn lock(&self) -> std::sync::MutexGuard<'_, State<T>> {
        // A poisoned lock only means another request panicked mid-update; the
        // map itself is still structurally valid.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl<T: Clone + Send + Sync + 'static> WriteDedup<T> {
    pub fn new() -> Arc<Self> {
        Self::with_limits(DEFAULT_TTL, DEFAULT_CAPACITY)
    }

    pub fn with_limits(ttl: Duration, capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                map: HashMap::new(),
                next_generation: 1,
            }),
            ttl,
            capacity: capacity.max(1),
        })
    }

    /// Number of remembered keys (in-flight + done). Test/diagnostic only.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.lock().map.len()
    }

    fn claim(self: &Arc<Self>, key: &str, fingerprint: &str, now: Instant) -> Claim<T> {
        let mut state = self.lock();

        // Drop an expired Done entry for this key first.
        let expired = matches!(
            state.map.get(key),
            Some(Entry::Done { at, .. }) if now.saturating_duration_since(*at) >= self.ttl
        );
        if expired {
            state.map.remove(key);
        }

        match state.map.get(key) {
            Some(Entry::InFlight {
                fingerprint: fp, rx, ..
            }) => {
                return if fp == fingerprint {
                    Claim::Wait(rx.clone())
                } else {
                    Claim::Bypass
                };
            }
            Some(Entry::Done {
                fingerprint: fp, value, ..
            }) => {
                return if fp == fingerprint {
                    Claim::Cached(value.clone())
                } else {
                    Claim::Bypass
                };
            }
            None => {}
        }

        // New key: make room if needed.
        if state.map.len() >= self.capacity {
            let ttl = self.ttl;
            state.map.retain(|_, entry| match entry {
                Entry::Done { at, .. } => now.saturating_duration_since(*at) < ttl,
                Entry::InFlight { .. } => true,
            });
        }
        if state.map.len() >= self.capacity {
            let oldest = state
                .map
                .iter()
                .filter_map(|(k, entry)| match entry {
                    Entry::Done { at, .. } => Some((k.clone(), *at)),
                    Entry::InFlight { .. } => None,
                })
                .min_by_key(|(_, at)| *at)
                .map(|(k, _)| k);
            match oldest {
                Some(k) => {
                    state.map.remove(&k);
                }
                // Full of in-flight work: fail open (no dedup for this request).
                None => return Claim::Bypass,
            }
        }

        let generation = state.next_generation;
        state.next_generation += 1;
        let (tx, rx) = watch::channel(None);
        state.map.insert(
            key.to_string(),
            Entry::InFlight {
                fingerprint: fingerprint.to_string(),
                rx,
                generation,
            },
        );
        Claim::Lead(Leader {
            dedup: Arc::clone(self),
            key: key.to_string(),
            generation,
            fingerprint: fingerprint.to_string(),
            tx: Some(tx),
        })
    }

    /// Forget a finished entry (used when `validate` says it is stale).
    fn forget_done(&self, key: &str, fingerprint: &str) {
        let mut state = self.lock();
        if matches!(
            state.map.get(key),
            Some(Entry::Done { fingerprint: fp, .. }) if fp == fingerprint
        ) {
            state.map.remove(key);
        }
    }

    /// Run `work` at most once per `key` (and `fingerprint`) within the TTL.
    ///
    /// * `fingerprint` describes the request's semantic shape; a repeat of a key
    ///   with a different fingerprint is a different request and is not merged.
    /// * `validate` is asked whether a remembered result still describes reality
    ///   (for a create: the row it created still exists under that name). A stale
    ///   result is dropped and the work runs again.
    /// * `remember` decides whether a finished result is kept for later repeats;
    ///   waiters that were already attached get the result either way. A failure
    ///   is not remembered, so a retry after a failed attempt runs again.
    /// * `now` supplies the clock, so TTL behaviour is testable without sleeping.
    pub async fn run<W, V, R, N>(
        self: &Arc<Self>,
        key: &str,
        fingerprint: &str,
        now: N,
        validate: V,
        remember: R,
        work: W,
        on_panic: T,
    ) -> T
    where
        W: FnOnce() -> T + Send + 'static,
        V: Fn(&T) -> bool,
        R: Fn(&T) -> bool + Send + 'static,
        N: Fn() -> Instant + Send + Sync + 'static,
    {
        let mut work = Some(work);
        let now = Arc::new(now);
        loop {
            match self.claim(key, fingerprint, now()) {
                Claim::Cached(value) => {
                    if validate(&value) {
                        return value;
                    }
                    self.forget_done(key, fingerprint);
                }
                Claim::Wait(mut rx) => match rx.wait_for(|v| v.is_some()).await {
                    Ok(guard) => {
                        if let Some(value) = guard.as_ref() {
                            return value.clone();
                        }
                    }
                    // The leader panicked: take over (loop and claim again).
                    Err(_) => {}
                },
                Claim::Bypass => {
                    if let Some(work) = work.take() {
                        return work();
                    }
                }
                Claim::Lead(leader) => {
                    let Some(work) = work.take() else {
                        return on_panic;
                    };
                    let clock = Arc::clone(&now);
                    let handle = tokio::task::spawn_blocking(move || {
                        let value = work();
                        let keep = remember(&value);
                        leader.complete(value.clone(), clock(), keep);
                        value
                    });
                    return handle.await.unwrap_or(on_panic);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "ipc_write_dedup_tests.rs"]
mod tests;
