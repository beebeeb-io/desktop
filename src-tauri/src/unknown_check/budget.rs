//! Durable, job-wide request and ciphertext limits (2191 throughput amendment).
use serde::{Deserialize, Serialize};

pub const METERED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const UNMETERED_BYTES: u64 = 64 * 1024 * 1024 * 1024;
pub const MAX_KIB_PER_SEC: u64 = 8 * 1024;

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Ledger {
    pub day: i64,
    pub bytes: u64,
    pub in_flight: u64,
    pub requests: Vec<i64>,
    pub cooldown_until: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stop {
    Until(i64),
    Daily,
}

impl Ledger {
    pub fn reserve(&mut self, now_ms: i64, day: i64, bytes: u64, limit: u64, oversize: bool) -> Result<(), Stop> {
        if day > self.day {
            self.day = day;
            self.bytes = self.in_flight;
        }
        if now_ms < self.cooldown_until {
            return Err(Stop::Until(self.cooldown_until));
        }
        self.requests.retain(|t| *t > now_ms.saturating_sub(60_000));
        let paced = self.requests.last().map_or(now_ms, |t| t.saturating_add(100));
        let ready = if self.requests.len() >= 600 {
            paced.max(self.requests[0].saturating_add(60_000))
        } else {
            paced
        };
        if now_ms < ready {
            return Err(Stop::Until(ready));
        }
        if bytes > 0 && self.bytes.saturating_add(bytes) > limit && !(oversize && self.bytes == 0) {
            return Err(Stop::Daily);
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.in_flight = self.in_flight.saturating_add(bytes);
        self.requests.push(now_ms);
        Ok(())
    }
    pub fn settle(&mut self, reserved: u64, used: u64) {
        self.bytes = self.bytes.saturating_sub(reserved.saturating_sub(used));
        self.in_flight = self.in_flight.saturating_sub(reserved);
    }
    pub fn finished(&mut self) {
        self.in_flight = 0;
    }
    pub fn rate_limited(&mut self, now_ms: i64, retry_after_secs: u64, stops: u32) {
        let backoff = (300u64.saturating_mul(1u64 << stops.min(9))).min(86_400);
        let millis = retry_after_secs.max(backoff).saturating_mul(1000).min(i64::MAX as u64) as i64;
        self.cooldown_until = self.cooldown_until.max(now_ms.saturating_add(millis));
    }
}

pub fn allowance(unmetered: bool) -> u64 {
    if unmetered { UNMETERED_BYTES } else { METERED_BYTES }
}
pub fn speed(configured: u64) -> u64 {
    if configured == 0 {
        MAX_KIB_PER_SEC
    } else {
        configured.min(MAX_KIB_PER_SEC)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_2191_s2_policy_is_conservative_and_lower_user_limit_wins() {
        assert_eq!(allowance(false), METERED_BYTES);
        assert_eq!(allowance(true), UNMETERED_BYTES);
        assert_eq!(speed(0), 8192);
        assert_eq!(speed(12000), 8192);
        assert_eq!(speed(100), 100);
    }
    #[test]
    fn task_2191_s2_requests_are_paced_across_quanta_and_restart() {
        let mut l = Ledger::default();
        for i in 0..600 {
            assert_eq!(l.reserve(i * 100, 0, 0, METERED_BYTES, false), Ok(()));
        }
        assert_eq!(l.reserve(59999, 0, 0, METERED_BYTES, false), Err(Stop::Until(60000)));
        let mut l: Ledger = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(l.reserve(59999, 0, 0, METERED_BYTES, false), Err(Stop::Until(60000)));
        assert_eq!(l.reserve(60000, 0, 0, METERED_BYTES, false), Ok(()));
    }
    #[test]
    fn task_2191_s2_429_stops_other_files_and_survives_restart() {
        let mut l = Ledger::default();
        l.rate_limited(1000, 700, 1);
        let mut l: Ledger = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(l.reserve(700999, 0, 0, METERED_BYTES, false), Err(Stop::Until(701000)));
        assert_eq!(l.reserve(701000, 0, 0, METERED_BYTES, false), Ok(()));
    }
    #[test]
    fn task_2191_s2_reserved_bytes_survive_midnight_restart_and_cost_downgrade() {
        let mut l = Ledger::default();
        assert_eq!(l.reserve(0, 1, 3 * 1024 * 1024 * 1024, UNMETERED_BYTES, false), Ok(()));
        let mut l: Ledger = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(l.reserve(100, 1, 1, METERED_BYTES, false), Err(Stop::Daily));
        assert_eq!(l.reserve(200, 2, 1, METERED_BYTES, false), Err(Stop::Daily));
        l.finished();
        assert_eq!(l.reserve(300, 3, METERED_BYTES, METERED_BYTES, false), Ok(()));
        assert_eq!(l.reserve(400, 3, 1, METERED_BYTES, false), Err(Stop::Daily));
    }
    #[test]
    fn task_2191_s2_oversize_hash_candidate_requires_an_untouched_day() {
        let mut l = Ledger::default();
        assert_eq!(l.reserve(0, 1, 1, METERED_BYTES, false), Ok(()));
        l.finished();
        assert_eq!(
            l.reserve(100, 1, METERED_BYTES + 1, METERED_BYTES, true),
            Err(Stop::Daily)
        );
        assert_eq!(l.reserve(200, 2, METERED_BYTES + 1, METERED_BYTES, true), Ok(()));
        l.finished();
        assert_eq!(l.reserve(300, 2, 1, METERED_BYTES, false), Err(Stop::Daily));
    }
}

#[cfg(test)]
mod settlement_tests {
    use super::*;
    #[test]
    fn task_2191_s2_live_settlement_refunds_only_known_unused_reservation() {
        let mut l = Ledger::default();
        l.reserve(0, 1, 1000, 2000, false).unwrap();
        l.settle(1000, 100);
        assert_eq!((l.bytes, l.in_flight), (100, 0));
        l.reserve(100, 1, 500, 2000, false).unwrap();
        // A crash has no settlement: the whole unfinished reservation carries over.
        l.reserve(200, 2, 1, 2000, false).unwrap();
        assert_eq!((l.bytes, l.in_flight), (501, 501));
    }
}
