//! LD-0105 S0: when a Windows Cloud Files fetch tells Windows how far it got.
//!
//! Windows fails every callback request that sees no provider activity for 60 s (a fixed timer),
//! then fetches the file again from its first byte. Our fetch callback answers only once the whole
//! file is downloaded and decrypted, so any download longer than a minute failed and started over
//! (LD-0105 H7: a 349 MB file fetched twice, 60.49 s apart). `CfReportProviderProgress` resets that
//! timer (Microsoft Learn; the P1 spike, (c)). This module decides WHEN to report and WHAT: real body
//! bytes only, counted per read ([`crate::transfer_progress::ReceiveMeter`]), at least every
//! [`REPORT_EVERY`] while bytes arrive, and nothing once no byte has arrived for [`SILENCE_LIMIT`],
//! so a dead network still fails the open instead of hanging the program that opened the file.
//!
//! Pure: no clock of its own and no Windows call, so it is tested on every platform. The caller
//! (`windows_cf::callbacks::keep_fetch_alive`) owns the timer and the `CfReportProviderProgress` call.

use std::time::{Duration, Instant};

/// How often the fetch callback reads its meter.
pub(crate) const TICK: Duration = Duration::from_secs(1);
/// While bytes arrive, a report at least this often: well inside Windows' 60 s, and inside the
/// spec's "at least every 10 s".
pub(crate) const REPORT_EVERY: Duration = Duration::from_secs(5);
/// No new body byte for this long: stop reporting, and let Windows fail the request.
pub(crate) const SILENCE_LIMIT: Duration = Duration::from_secs(60);

/// The reporting state of one fetch.
#[derive(Debug)]
pub(crate) struct KeepAlive {
    /// The most bytes the meter has shown.
    seen: u64,
    /// When `seen` last grew.
    last_byte_at: Instant,
    last_report_at: Option<Instant>,
}

impl KeepAlive {
    pub(crate) fn new(now: Instant) -> Self {
        KeepAlive {
            seen: 0,
            last_byte_at: now,
            last_report_at: None,
        }
    }

    /// One look at the meter (`received`: every body byte so far). `Some(bytes)`: report `bytes` now.
    /// Nothing before the first byte (there is no real progress to report), nothing once
    /// [`SILENCE_LIMIT`] has passed without a new byte, and otherwise at most one report per
    /// [`REPORT_EVERY`].
    pub(crate) fn tick(&mut self, now: Instant, received: u64) -> Option<u64> {
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
        Some(self.seen)
    }
}

/// The `ProviderProgressCompleted` value for `received` wire bytes of a file of `total` plaintext
/// bytes: never past the total (the wire carries 28 bytes of tag per chunk, and a retried chunk is
/// counted again), never negative.
pub(crate) fn completed(received: u64, total: i64) -> i64 {
    i64::try_from(received).unwrap_or(i64::MAX).min(total.max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive `keep` once a second for `secs` seconds; `bytes_at(s)` is the meter at second `s`.
    /// Returns the seconds at which a report was due, with its value.
    fn run(keep: &mut KeepAlive, start: Instant, secs: u64, bytes_at: impl Fn(u64) -> u64) -> Vec<(u64, u64)> {
        (1..=secs)
            .filter_map(|s| keep.tick(start + Duration::from_secs(s), bytes_at(s)).map(|b| (s, b)))
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
        assert_eq!(keep.tick(start + Duration::from_secs(31), 5), Some(5));
    }

    #[test]
    fn a_stall_keeps_its_count_alive_until_a_minute_without_a_byte_then_stops() {
        let start = Instant::now();
        let mut keep = KeepAlive::new(start);
        // 4 MiB arrive in the first 10 s, then the network dies.
        let reports = run(&mut keep, start, 200, |s| s.min(10) * 400 * 1024);
        let last_byte_second = 10;
        let after_silence: Vec<_> = reports.iter().filter(|(s, _)| *s >= last_byte_second + 60).collect();
        assert!(
            after_silence.is_empty(),
            "reports after a minute of silence: {after_silence:?}"
        );
        assert!(
            reports
                .iter()
                .any(|(s, _)| (last_byte_second + 50..last_byte_second + 60).contains(s)),
            "a stall shorter than the limit is still reported: {reports:?}"
        );
        assert!(reports.iter().all(|(_, b)| *b <= 10 * 400 * 1024));
        // Bytes come back: so do the reports.
        assert_eq!(keep.tick(start + Duration::from_secs(201), 5 << 20), Some(5 << 20));
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
