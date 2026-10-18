//! Spec 2026-10-10 §6 (task 1887): the send phase's bound, its end and its progress, as pure state. The driver in
//! `lib.rs` reads the world once a second and hands it here.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::runner::AttemptSignals;

/// §6.3: one constant, from the start of the phase.
pub const SIGN_OUT_SEND_LIMIT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseEnd {
    AllSent,
    Held,
    TimeLimit,
    Stopped,
    Closed,
    Offline,
    Session,
    EngineStopped,
    /// Spec §15 question 4: a refusal the account cures.
    Account,
}

impl PhaseEnd {
    pub fn as_str(self) -> &'static str {
        match self {
            PhaseEnd::AllSent => "all_sent",
            PhaseEnd::Held => "held",
            PhaseEnd::TimeLimit => "time_limit",
            PhaseEnd::Stopped => "stopped",
            PhaseEnd::Closed => "closed",
            PhaseEnd::Offline => "offline",
            PhaseEnd::Session => "session",
            PhaseEnd::EngineStopped => "engine_stopped",
            PhaseEnd::Account => "account",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseObservation {
    pub elapsed: Duration,
    pub changes_left: usize,
    pub any_live: bool,
    pub engine_running: bool,
    pub session_expired: bool,
    pub link_failed: bool,
    pub attempt: AttemptSignals,
    pub stop_requested: bool,
    pub window_closed: bool,
}

/// §6.4, checked at least once a second.
pub fn phase_end(o: &PhaseObservation) -> Option<PhaseEnd> {
    if o.window_closed {
        Some(PhaseEnd::Closed)
    } else if o.stop_requested {
        Some(PhaseEnd::Stopped)
    } else if o.changes_left == 0 {
        Some(PhaseEnd::AllSent)
    } else if !o.engine_running {
        Some(PhaseEnd::EngineStopped)
    } else if o.session_expired || o.attempt.session_refused {
        Some(PhaseEnd::Session)
    } else if o.link_failed || o.attempt.link_failed {
        Some(PhaseEnd::Offline)
    } else if o.attempt.account_refused {
        Some(PhaseEnd::Account)
    } else if !o.any_live {
        Some(PhaseEnd::Held)
    } else if o.elapsed >= SIGN_OUT_SEND_LIMIT {
        Some(PhaseEnd::TimeLimit)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseStep {
    Continue,
    Progress(usize),
    End(PhaseEnd),
}

#[derive(Debug, Default)]
pub struct SendPhase {
    last_reported: Option<usize>,
}

impl SendPhase {
    pub fn observe(&mut self, o: &PhaseObservation) -> PhaseStep {
        if let Some(end) = phase_end(o) {
            return PhaseStep::End(end);
        }
        if self.last_reported == Some(o.changes_left) {
            return PhaseStep::Continue;
        }
        self.last_reported = Some(o.changes_left);
        PhaseStep::Progress(o.changes_left)
    }
}

/// The one phase that runs (§5.1 "One flow at a time").
pub struct RunningPhase {
    #[allow(dead_code)] // first production caller: none in the plan; no test reads it either
    pub ask_id: String,
    /// The label of the window that started it.
    #[allow(dead_code)] // first production caller: Task 12; no test reads it before then
    pub owner: String,
    pub stop: Arc<AtomicBool>,
    #[allow(dead_code)] // first production caller: Task 12; no test reads it before then
    pub closed: Arc<AtomicBool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::AttemptSignals;

    fn observing() -> PhaseObservation {
        PhaseObservation {
            elapsed: Duration::from_secs(1),
            changes_left: 2,
            any_live: true,
            engine_running: true,
            session_expired: false,
            link_failed: false,
            attempt: AttemptSignals::default(),
            stop_requested: false,
            window_closed: false,
        }
    }

    /// U16.
    #[test]
    fn the_send_phase_ends_on_each_condition() {
        let base = observing();
        assert_eq!(phase_end(&base), None);
        let cases = [
            (
                PhaseObservation {
                    changes_left: 0,
                    ..base
                },
                PhaseEnd::AllSent,
            ),
            (
                PhaseObservation {
                    any_live: false,
                    ..base
                },
                PhaseEnd::Held,
            ),
            (
                PhaseObservation {
                    stop_requested: true,
                    ..base
                },
                PhaseEnd::Stopped,
            ),
            (
                PhaseObservation {
                    window_closed: true,
                    ..base
                },
                PhaseEnd::Closed,
            ),
            (
                PhaseObservation {
                    link_failed: true,
                    ..base
                },
                PhaseEnd::Offline,
            ),
            (
                PhaseObservation {
                    attempt: AttemptSignals {
                        link_failed: true,
                        ..Default::default()
                    },
                    ..base
                },
                PhaseEnd::Offline,
            ),
            (
                PhaseObservation {
                    session_expired: true,
                    ..base
                },
                PhaseEnd::Session,
            ),
            (
                PhaseObservation {
                    attempt: AttemptSignals {
                        session_refused: true,
                        ..Default::default()
                    },
                    ..base
                },
                PhaseEnd::Session,
            ),
            (
                PhaseObservation {
                    engine_running: false,
                    ..base
                },
                PhaseEnd::EngineStopped,
            ),
            // The account end (spec §15 question 4).
            (
                PhaseObservation {
                    attempt: AttemptSignals {
                        account_refused: true,
                        ..Default::default()
                    },
                    ..base
                },
                PhaseEnd::Account,
            ),
        ];
        for (observation, end) in cases {
            assert_eq!(phase_end(&observation), Some(end), "{observation:?}");
        }
        assert_eq!(
            phase_end(&PhaseObservation {
                elapsed: Duration::from_millis(59_900),
                ..base
            }),
            None,
            "not at 59.9 s"
        );
        assert_eq!(
            phase_end(&PhaseObservation {
                elapsed: Duration::from_secs(60),
                ..base
            }),
            Some(PhaseEnd::TimeLimit),
            "at exactly 60 s"
        );
        assert_eq!(SIGN_OUT_SEND_LIMIT, Duration::from_secs(60));
        assert_eq!(serde_json::to_value(PhaseEnd::TimeLimit).unwrap(), "time_limit");
    }

    /// §6.4: the progress line changes when the count changes, and never shows 0.
    #[test]
    fn progress_follows_the_count_and_never_reads_zero() {
        let mut phase = SendPhase::default();
        assert_eq!(phase.observe(&observing()), PhaseStep::Progress(2));
        assert_eq!(phase.observe(&observing()), PhaseStep::Continue);
        assert_eq!(
            phase.observe(&PhaseObservation {
                changes_left: 1,
                ..observing()
            }),
            PhaseStep::Progress(1)
        );
        assert_eq!(
            phase.observe(&PhaseObservation {
                changes_left: 0,
                ..observing()
            }),
            PhaseStep::End(PhaseEnd::AllSent)
        );
    }

    /// U34 (a): the deadline is Rust's: with no webview call at all, the phase ends at 60 s.
    #[test]
    fn the_deadline_needs_no_webview() {
        let mut phase = SendPhase::default();
        let mut ended = None;
        for second in 0..=61u64 {
            if let PhaseStep::End(end) = phase.observe(&PhaseObservation {
                elapsed: Duration::from_secs(second),
                ..observing()
            }) {
                ended = Some((second, end));
                break;
            }
        }
        assert_eq!(ended, Some((60, PhaseEnd::TimeLimit)));
    }
}
