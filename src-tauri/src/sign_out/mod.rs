//! Spec 2026-10-10 (task 1887): a sign-out by choice never loses a Finder save that is still waiting.
//! The pure parts compile on every platform; the keep step and the folder writer are macOS only (W5).
// The sign-out state this module holds is read on macOS and in tests only; elsewhere it is built and never read.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

pub(crate) mod copy;
#[cfg(target_os = "macos")]
pub(crate) mod keep;
pub(crate) mod kept_names;
pub(crate) mod kept_record;
pub(crate) mod kept_set;
#[cfg(target_os = "macos")]
pub(crate) mod kept_writer;
pub(crate) mod snapshot;
pub(crate) mod waiting;

/// Test builds only (spec 2026-10-10 U42): a real process death at `point` when `BB_TEST_CRASH_AT` names it. It
/// writes `reached:<point>` first, so the parent can tell a death here from a death anywhere else.
#[cfg(test)]
pub(crate) fn crash_point(point: &str) {
    if std::env::var("BB_TEST_CRASH_AT").as_deref() == Ok(point) {
        if let Ok(path) = std::env::var("BB_TEST_CRASH_REACHED") {
            let _ = std::fs::write(path, format!("reached:{point}"));
        }
        std::process::abort();
    }
}

/// The sign-out flow's state in `AppState` (spec 2026-10-10 §5.1). The send phase, the late results and the flow
/// request join it later.
#[derive(Default)]
pub(crate) struct SignOutState {
    pub snapshots: std::sync::Arc<snapshot::SnapshotStore>,
}
