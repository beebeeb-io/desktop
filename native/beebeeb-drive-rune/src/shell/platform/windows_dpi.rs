//! Windows: a maximized window follows a scale change (a workaround for a gap in rune, remove it when rune
//! fixes it).
//!
//! The gap (RC3-2, `board/3-in-review/drive-complete-RC3-2-scale-change-live.md`, rune `c92e462e`, winit
//! 0.30.13): when the display's scale changes while one of this app's windows is MAXIMIZED, Windows re-fits the
//! window to the new work area but never gives it the new DPI. `GetDpiForWindow` stays at the old value, so winit
//! emits no `ScaleFactorChanged` and the app keeps drawing at the old scale until the window is restored or
//! resized. A window that is not maximized follows live (96 -> 120 -> 96 on VM 401). Plain per-monitor-v2 test
//! windows and Notepad do not have the gap; which call in the rune/winit window makes Windows skip it is not known.
//!
//! What un-sticks it: any real size change. One `SetWindowPos` one pixel shorter makes Windows apply the DPI at
//! once and re-fit the maximized rect itself (a same-rect or frame-changed `SetWindowPos` does nothing).
//!
//! When: the tray's hidden top-level window gets the broadcasts of a scale change (`WM_SETTINGCHANGE`,
//! `WM_DISPLAYCHANGE`, `WM_DPICHANGED`). It runs [`nudge_stuck`] then, and again on a short timer for a while
//! after, since the order in which Windows re-fits the window and sends the broadcasts is not fixed. A window
//! whose DPI matches its monitor's is never touched, so the repeats cost one `EnumWindows` each.

use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows_sys::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible, IsZoomed, SWP_ASYNCWINDOWPOS,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOZORDER, SetWindowPos,
};

/// How often, and how many times, the check repeats after a broadcast: every 200 ms for 3 s.
pub(super) const RECHECK_MS: u32 = 200;
const RECHECKS: u32 = 15;

/// Ticks left in the current burst of re-checks (the tray thread's only).
static LEFT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// A scale-change broadcast arrived: check now, and (re)start the burst of re-checks.
pub(super) fn on_broadcast() {
    nudge_stuck();
    LEFT.store(RECHECKS, std::sync::atomic::Ordering::SeqCst);
}

/// One re-check tick. False when the burst is over (the caller kills the timer).
pub(super) fn on_tick() -> bool {
    nudge_stuck();
    let left = LEFT.load(std::sync::atomic::Ordering::SeqCst).saturating_sub(1);
    LEFT.store(left, std::sync::atomic::Ordering::SeqCst);
    left > 0
}

/// A window needs the nudge: maximized, and its DPI is not its monitor's. A DPI of 0 is a failed read: no nudge.
fn is_stuck(maximized: bool, window_dpi: u32, monitor_dpi: u32) -> bool {
    maximized && window_dpi != 0 && monitor_dpi != 0 && window_dpi != monitor_dpi
}

/// This process's visible top-level windows that are maximized with a stale DPI each get a one-pixel-shorter
/// `SetWindowPos`; Windows then applies the monitor's DPI and re-fits the maximized rect. Returns how many.
pub(super) fn nudge_stuck() -> usize {
    let mut nudged = 0usize;
    // SAFETY: `each` only reads `lparam` as the `usize` passed here, alive for the whole call.
    unsafe { EnumWindows(Some(each), &mut nudged as *mut usize as LPARAM) };
    nudged
}

unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY (whole body): `hwnd` is a live top-level window handed by `EnumWindows`; the reads below take it and
    // return. A window of another thread of this process (the app's UI thread) is moved with
    // `SWP_ASYNCWINDOWPOS`: the request is posted to its thread, so this one never waits on it.
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != GetCurrentProcessId() || IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let maximized = IsZoomed(hwnd) != 0;
        if !maximized {
            return 1;
        }
        let window_dpi = GetDpiForWindow(hwnd);
        let (mut x, mut y) = (0u32, 0u32);
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let monitor_dpi = if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) == 0 {
            x
        } else {
            0
        };
        if !is_stuck(maximized, window_dpi, monitor_dpi) {
            return 1;
        }
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return 1;
        }
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        if SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            width,
            (height - 1).max(1),
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        ) != 0
        {
            *(lparam as *mut usize) += 1;
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::is_stuck;

    #[test]
    fn scale_nudge_only_for_a_maximized_window_with_a_stale_dpi() {
        assert!(is_stuck(true, 96, 120));
        assert!(is_stuck(true, 120, 96));
        assert!(!is_stuck(true, 120, 120));
        assert!(!is_stuck(false, 96, 120));
        assert!(!is_stuck(true, 0, 120));
        assert!(!is_stuck(true, 96, 0));
    }
}
