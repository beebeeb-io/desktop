//! P1c: one in-place install at a time, across every Beebeeb window.
//!
//! The frontend's single-flight (`src/v2/update.ts`) lives in one webview's module state; the main window,
//! the settings/update window and the tray each have their own. Both install commands claimed here, in one
//! process-wide flag, so a second window is refused before any download. The slot is released in `Drop`, so
//! every early `?` return frees it; the success path calls `app.restart()` and the process ends.

/// The code a refused second install leads with, so a caller can tell it from a failed one.
pub(super) const UPDATE_INSTALL_IN_PROGRESS_CODE: &str = "update_install_in_progress";

/// False when no window is installing. Both install commands take it with compare-and-swap.
static INSTALL_IN_PROGRESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Claims the process-wide install slot. Refused before any download if another window already holds it.
pub(super) fn claim_install() -> Result<InstallClaim, String> {
    INSTALL_IN_PROGRESS
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .map(|_| InstallClaim)
        .map_err(|_| {
            format!("{UPDATE_INSTALL_IN_PROGRESS_CODE}: an update is already installing in another Beebeeb window.")
        })
}

/// Holds the install slot for as long as it lives; every error path returns early, so `Drop` releases it.
pub(super) struct InstallClaim;

impl Drop for InstallClaim {
    fn drop(&mut self) {
        INSTALL_IN_PROGRESS.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(test)]
#[path = "install_guard_tests.rs"]
mod install_guard_tests;
