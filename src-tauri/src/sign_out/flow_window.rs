//! Spec 2026-10-10 §5.1 "A late result" (task 1887): one record per window that called `clear_session`, from the call's
//! start until its result is acknowledged or shown. Taking the record out is the one atomic step, so a result is shown
//! at most once, and, if the webview never displays it, at least once.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// What the native dialog shows: a failure in "Sign-out paused", a completed sign-out's warning in "Signed out".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    Failure(String),
    Warning(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Record {
    Running { closed: bool },
    Pending(Shown),
}

#[derive(Default)]
pub struct LateResults {
    records: Mutex<HashMap<String, Record>>,
}

impl LateResults {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Record>> {
        self.records.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A `clear_session` from `label` started; it replaces that window's record.
    pub fn begin(&self, label: &str) {
        self.lock().insert(label.to_string(), Record::Running { closed: false });
    }

    /// The call returned. `Some`: show it now (its window closed while it ran).
    pub fn complete(&self, label: &str, result: Option<Shown>) -> Option<Shown> {
        let mut records = self.lock();
        match (records.remove(label), result) {
            (Some(Record::Running { closed: true }), result) => result,
            (Some(Record::Running { closed: false }), Some(shown)) => {
                records.insert(label.to_string(), Record::Pending(shown));
                None
            }
            _ => None,
        }
    }

    /// `label` closed, was destroyed or reloads. `Some`: show it now.
    pub fn window_closed(&self, label: &str) -> Option<Shown> {
        let mut records = self.lock();
        match records.get_mut(label) {
            Some(Record::Running { closed }) => {
                *closed = true;
                None
            }
            Some(Record::Pending(_)) => match records.remove(label) {
                Some(Record::Pending(shown)) => Some(shown),
                _ => None,
            },
            None => None,
        }
    }

    /// The webview displayed the result. Whether it removed one.
    pub fn acknowledge(&self, label: &str) -> bool {
        let mut records = self.lock();
        if matches!(records.get(label), Some(Record::Pending(_))) {
            records.remove(label);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure() -> Option<Shown> {
        Some(Shown::Failure("Sign-out paused: purge failed".into()))
    }

    /// U44 (a)-(j).
    #[test]
    fn a_result_that_returned_before_the_close_is_still_shown() {
        let s = "macos-settings";
        // (a) the exact race: returned, then closed, no acknowledgement → one dialog, record gone.
        let late = LateResults::default();
        late.begin(s);
        assert_eq!(
            late.complete(s, failure()),
            None,
            "pending: the window may still show it"
        );
        assert_eq!(late.window_closed(s), failure());
        assert_eq!(
            late.window_closed(s),
            None,
            "(i) a second close (Destroyed) shows nothing"
        );
        // (b) a warning.
        let warning = Some(Shown::Warning("Beebeeb couldn’t confirm Finder was removed.".into()));
        late.begin(s);
        late.complete(s, warning.clone());
        assert_eq!(late.window_closed(s), warning);
        // (c) an acknowledgement after the dialog: nothing, no panic, nothing removed.
        assert!(!late.acknowledge(s));
        // (d) acknowledged first, then the close: no dialog.
        late.begin(s);
        late.complete(s, failure());
        assert!(late.acknowledge(s));
        assert_eq!(late.window_closed(s), None);
        // (e) nothing to show: no record.
        late.begin(s);
        assert_eq!(late.complete(s, None), None);
        assert_eq!(late.window_closed(s), None);
        // (f) closed while running, then complete: shown at completion.
        late.begin(s);
        assert_eq!(late.window_closed(s), None);
        assert_eq!(late.complete(s, failure()), failure());
        // (g) another window's close leaves the record alone.
        late.begin(s);
        late.complete(s, failure());
        assert_eq!(late.window_closed("settings"), None);
        assert_eq!(late.window_closed("onboarding"), None);
        assert_eq!(late.window_closed(s), failure());
        // (j) a second begin from the same window replaces its record.
        late.begin(s);
        late.complete(s, failure());
        late.begin(s);
        assert_eq!(late.window_closed(s), None, "running again: marked, nothing shown yet");
        assert_eq!(late.complete(s, None), None);
    }

    /// U44 (k): the hook and the acknowledgement raced, 10000 rounds: one dialog or none, never two. The two threads
    /// meet on a counter they spin on, not on a `Barrier`: a `Barrier` wakes its waiting thread through the OS,
    /// microseconds after the other has already finished, so a take-out split over two locks was never met.
    #[test]
    fn the_close_and_the_acknowledgement_never_both_show() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        fn meet(arrived: &AtomicUsize) {
            arrived.fetch_add(1, Ordering::SeqCst);
            while arrived.load(Ordering::SeqCst) < 2 {
                std::hint::spin_loop();
            }
        }
        for round in 0..10_000 {
            let late = Arc::new(LateResults::default());
            late.begin("w");
            late.complete("w", failure());
            let arrived = Arc::new(AtomicUsize::new(0));
            let (hook_late, hook_arrived) = (late.clone(), arrived.clone());
            let hook = std::thread::spawn(move || {
                meet(&hook_arrived);
                hook_late.window_closed("w")
            });
            meet(&arrived);
            let removed = late.acknowledge("w");
            let shown = hook.join().unwrap();
            assert_eq!(
                shown.is_some(),
                !removed,
                "round {round}: shown {shown:?}, acknowledged {removed}"
            );
        }
    }
}
