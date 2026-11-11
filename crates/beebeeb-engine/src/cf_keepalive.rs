//! LD-0105 S0: when a Windows Cloud Files fetch tells Windows how far it got.
//!
//! Windows fails every callback request that sees no provider activity for 60 s (a fixed timer),
//! then fetches the file again from its first byte. Our fetch callback answers only once the whole
//! file is downloaded and decrypted, so any download longer than a minute failed and started over
//! (LD-0105 H7: a 349 MB file fetched twice, 60.49 s apart). `CfReportProviderProgress` resets that
//! timer (Microsoft Learn; the P1 spike, (c)). This module decides WHEN to report and WHAT: real body
//! bytes only, counted per read ([`crate::transfer_progress::ReceiveMeter`]), at least every
//! [`REPORT_EVERY`] while bytes arrive, and nothing once no byte has arrived for [`SILENCE_LIMIT`],
//! so a dead network still fails the open instead of hanging the program that opened the file:
//! each report restarts Windows' 60 s, so the open fails at most `SILENCE_LIMIT` + 60 s = 90 s after
//! the last byte (lead decision 2026-11-06).
//!
//! Liveness and progress are apart (lead decision 2026-11-06): every byte read, a failed or retried
//! attempt's included, keeps the fetch alive; what is reported is the progress that is kept, which a
//! failed attempt takes back, and never more than the file.
//!
//! Pure: no clock of its own and no Windows call, so it is tested on every platform. The caller
//! (`windows_cf::callbacks::keep_fetch_alive`) owns the timer and the `CfReportProviderProgress` call.

use std::time::{Duration, Instant};

/// How often the fetch callback reads its meter.
pub const TICK: Duration = Duration::from_secs(1);
/// While bytes arrive, a report at least this often: well inside Windows' 60 s, and inside the
/// spec's "at least every 10 s".
pub const REPORT_EVERY: Duration = Duration::from_secs(5);
/// No new body byte for this long: stop reporting, and let Windows fail the request 60 s after the
/// last report. Bytes are counted per read, so 30 s without one is a dead link, not a slow one.
pub const SILENCE_LIMIT: Duration = Duration::from_secs(30);

/// The reporting state of one fetch.
#[derive(Debug)]
pub struct KeepAlive {
    /// The most bytes read the meter has shown (liveness).
    seen: u64,
    /// When `seen` last grew.
    last_byte_at: Instant,
    last_report_at: Option<Instant>,
}

impl KeepAlive {
    pub fn new(now: Instant) -> Self {
        KeepAlive {
            seen: 0,
            last_byte_at: now,
            last_report_at: None,
        }
    }

    /// One look at the meter (`received`: every body byte read so far, which only grows; `kept`: the
    /// progress, which a failed attempt takes back). `Some(bytes)`: report `kept` now. Nothing before
    /// the first byte (there is no real progress to report), nothing once [`SILENCE_LIMIT`] has passed
    /// without a new byte read, and otherwise at most one report per [`REPORT_EVERY`].
    pub fn tick(&mut self, now: Instant, received: u64, kept: u64) -> Option<u64> {
        if received > self.seen {
            self.seen = received;
            self.last_byte_at = now;
        }
        if self.seen == 0 || now.saturating_duration_since(self.last_byte_at) >= SILENCE_LIMIT {
            return None;
        }
        if self
            .last_report_at
            .is_some_and(|at| now.saturating_duration_since(at) < REPORT_EVERY)
        {
            return None;
        }
        self.last_report_at = Some(now);
        Some(kept)
    }
}

/// The `ProviderProgressCompleted` value for `received` kept wire bytes of a file of `total`
/// plaintext bytes: never past the total (the wire carries 28 bytes of tag per chunk), never negative.
pub fn completed(received: u64, total: i64) -> i64 {
    i64::try_from(received).unwrap_or(i64::MAX).min(total.max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive `keep` once a second for `secs` seconds; `bytes_at(s)` is the meter at second `s`.
    /// Returns the seconds at which a report was due, with its value.
    fn run(keep: &mut KeepAlive, start: Instant, secs: u64, bytes_at: impl Fn(u64) -> u64) -> Vec<(u64, u64)> {
        (1..=secs)
            .filter_map(|s| {
                let b = bytes_at(s);
                keep.tick(start + Duration::from_secs(s), b, b).map(|b| (s, b))
            })
            .collect()
    }

    #[test]
    fn while_bytes_arrive_a_report_comes_at_least_every_ten_seconds() {
        let start = Instant::now();
        let mut keep = KeepAlive::new(start);
        // 128 KiB every second for 5 minutes: a 32 MiB chunk on a slow shared link.
        let reports = run(&mut keep, start, 300, |s| s * 128 * 1024);
        assert!(!reports.is_empty());
        assert_eq!(reports[0], (1, 128 * 1024), "the first byte is reported at once");
        let mut last = 0;
        for (s, bytes) in &reports {
            assert!(s - last <= 10, "a gap of {} s before second {s}", s - last);
            assert_eq!(*bytes, s * 128 * 1024, "the real count, nothing invented");
            last = *s;
        }
        assert!(300 - last <= 10);
    }

    #[test]
    fn nothing_is_reported_before_the_first_byte() {
        let start = Instant::now();
        let mut keep = KeepAlive::new(start);
        assert_eq!(run(&mut keep, start, 30, |_| 0), Vec::new());
        assert_eq!(keep.tick(start + Duration::from_secs(31), 5, 5), Some(5));
    }

    #[test]
    fn a_dead_network_fails_the_open_within_90_s_of_the_last_byte() {
        // Review P2 (lead decision 2026-11-06): each report restarts Windows' own 60 s, so the open
        // fails 60 s after the LAST REPORT. That must be at most 90 s after the last byte.
        const WINDOWS_TIMER: u64 = 60;
        let start = Instant::now();
        let mut keep = KeepAlive::new(start);
        // 4 MiB arrive in the first 10 s, then the network dies.
        let reports = run(&mut keep, start, 300, |s| s.min(10) * 400 * 1024);
        let last_byte_second = 10;
        let last_report = reports.last().map(|(s, _)| *s).unwrap();
        assert!(
            last_report + WINDOWS_TIMER <= last_byte_second + 90,
            "the last report at second {last_report}: Windows fails the open at {}, more than 90 s after the last byte ({last_byte_second})",
            last_report + WINDOWS_TIMER
        );
        assert!(
            reports
                .iter()
                .any(|(s, _)| (last_byte_second + 20..last_byte_second + 30).contains(s)),
            "a stall shorter than the limit is still reported: {reports:?}"
        );
        assert!(reports.iter().all(|(_, b)| *b <= 10 * 400 * 1024));
        // Bytes come back: so do the reports.
        assert_eq!(
            keep.tick(start + Duration::from_secs(301), 5 << 20, 5 << 20),
            Some(5 << 20)
        );
    }

    #[test]
    fn a_failed_attempt_keeps_the_fetch_alive_but_its_bytes_are_not_progress() {
        // Lead decision 2026-11-06: liveness from every byte read, progress from what is kept.
        let start = Instant::now();
        let mut keep = KeepAlive::new(start);
        let at = |s| start + Duration::from_secs(s);
        assert_eq!(keep.tick(at(1), 600, 600), Some(600));
        // The 600 bytes' body broke off and was discarded; reads go on (the retry), so the fetch stays
        // alive, and what is reported is the kept progress, not the bytes read.
        assert_eq!(keep.tick(at(20), 650, 0), Some(0));
        assert_eq!(keep.tick(at(25), 700, 100), Some(100), "the progress went back");
        // 39 s after the kept count last passed 600, reads still arrive: alive.
        assert_eq!(keep.tick(at(40), 800, 200), Some(200), "liveness is the bytes read");
        // A retried chunk read again never reports past what is kept.
        let mut keep = KeepAlive::new(start);
        assert_eq!(keep.tick(at(1), 1_000, 400), Some(400));
    }

    #[test]
    fn completed_is_clamped_to_the_file() {
        assert_eq!(completed(0, 100), 0);
        assert_eq!(completed(99, 100), 99);
        assert_eq!(
            completed(128, 100),
            100,
            "the tags and a retried chunk never pass the total"
        );
        assert_eq!(completed(u64::MAX, i64::MAX), i64::MAX);
        assert_eq!(completed(5, -1), 0);
    }
}
