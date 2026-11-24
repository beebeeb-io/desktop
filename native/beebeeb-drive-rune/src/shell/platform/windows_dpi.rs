//! Windows: a maximized window follows a scale change (a workaround for a gap in rune, remove it when rune
//! fixes it).
//!
//! The gap (RC3-2, `board/3-in-review/drive-complete-RC3-2-scale-change-live.md`, rune `c92e462e`, winit
//! 0.30.13): when the display's scale changes while one of this app's windows is MAXIMIZED, Windows re-fits the
//! window to the new work area but never gives it the new DPI. `GetDpiForWindow` stays at the old value, so winit
//! emits no `ScaleFactorChanged` and the app keeps drawing at the old scale until the window is restored or
//! resized. A window that is not maximized follows live (96 -> 120 -> 96 on the Windows test machine). Plain per-monitor-v2 test
//! windows and Notepad do not have the gap; which call in the rune/winit window makes Windows skip it is not known.
//!
//! What un-sticks it: any real size change. One `SetWindowPos` one pixel shorter makes Windows apply the DPI at
//! once and re-fit the maximized rect itself (a same-rect or frame-changed `SetWindowPos` does nothing).
//!
//! The nudge alone leaves a layout that does not fit (RC5: laid out 1600x925 in a 1280x740 window at 125 %, and
//! 1024x602 in a 1280x752 one on the way back). Why: on `ScaleFactorChanged` rune (`window/events.rs`, Windows
//! arm) sizes its drawable and viewport to the window's size times new/old scale, "Windows resizes the window
//! after this event", and waits for the `Resized` that corrects it. A maximized window keeps its size (winit's
//! `WM_DPICHANGED` does not resize a maximized window, rune's own frame says the same), so no `WM_SIZE` and no
//! `Resized` follow, and the layout stays at real size x new/old. What fits it again: a `WM_SIZE` carrying the
//! window's real client size. winit turns it into `Resized` with that size, rune lays out at it with the new
//! scale, and a `Resized` that changes nothing returns at once. So each check also posts every maximized window
//! its own size ([`resend_sizes`]); nothing on screen moves.
//!
//! When: the tray's hidden top-level window gets the broadcasts of a scale change (`WM_SETTINGCHANGE`,
//! `WM_DISPLAYCHANGE`, `WM_DPICHANGED`). It runs [`check`] then, and again on a short timer for a while
//! after, since the order in which Windows re-fits the window and sends the broadcasts is not fixed. A window
//! whose DPI matches its monitor's is never moved, so the repeats cost one `EnumWindows` and one posted message
//! per maximized window each.

use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows_sys::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed,
    PostMessageW, SIZE_MAXIMIZED, SIZE_RESTORED, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER,
    SWP_NOZORDER, SetWindowPos, WM_SIZE,
};

/// How often, and how many times, the check repeats after a broadcast: every 100 ms for 3 s. The layout fits one
/// tick after the nudge at the latest, so the tick is short.
pub(super) const RECHECK_MS: u32 = 100;
const RECHECKS: u32 = 30;
/// Ticks that still follow a nudge made near the end of a burst (once per burst), so its size is re-sent.
const TAIL: u32 = 5;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Ticks left in the current burst of re-checks (the tray thread's only).
static LEFT: AtomicU32 = AtomicU32::new(0);
/// The current burst has not been lengthened for a late nudge yet.
static MAY_EXTEND: AtomicBool = AtomicBool::new(false);
/// The windows this burst has seen maximized (handles as numbers; the tray thread's only). Each check re-sends
/// them their size, also once they are no longer maximized: a size posted just before the user restored the
/// window is corrected by the next one.
static SEEN: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

/// A scale-change broadcast arrived: check now, and (re)start the burst of re-checks.
pub(super) fn on_broadcast() {
    check();
    LEFT.store(RECHECKS, Ordering::SeqCst);
    MAY_EXTEND.store(true, Ordering::SeqCst);
}

/// One re-check tick. False when the burst is over (the caller kills the timer).
pub(super) fn on_tick() -> bool {
    let nudged = check();
    let mut left = LEFT.load(Ordering::SeqCst).saturating_sub(1);
    if nudged > 0 && left < TAIL && MAY_EXTEND.swap(false, Ordering::SeqCst) {
        left = TAIL;
    }
    LEFT.store(left, Ordering::SeqCst);
    if left == 0 {
        SEEN.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    left > 0
}

/// One check: nudge the stuck windows, then re-send every window this burst saw maximized its size. Returns how
/// many were nudged.
fn check() -> usize {
    let mut pass = Pass::default();
    // SAFETY: `each` only reads `lparam` as the `Pass` passed here, alive for the whole call.
    unsafe { EnumWindows(Some(each), &mut pass as *mut Pass as LPARAM) };
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    for window in pass.maximized {
        if !seen.contains(&window) {
            seen.push(window);
        }
    }
    resend_sizes(&seen);
    pass.nudged
}

/// What one `EnumWindows` pass found: this process's visible maximized windows, and how many it nudged.
#[derive(Default)]
struct Pass {
    maximized: Vec<usize>,
    nudged: usize,
}

/// `WM_SIZE`'s `lparam` for a client area: the width in the low word, the height in the high word. None for an
/// empty area or one that does not fit the two words (no message is sent then).
fn size_lparam(width: i32, height: i32) -> Option<LPARAM> {
    let fits = |v: i32| (1..=0xFFFF).contains(&v);
    (fits(width) && fits(height)).then_some(((height as LPARAM) << 16) | width as LPARAM)
}

/// Posts each window a `WM_SIZE` with its real client size and its real state (maximized or not). winit emits
/// `Resized` with that size; rune lays out at it with the scale it has now. A window that is gone, another
/// process's (a reused handle), hidden or minimized gets nothing.
fn resend_sizes(windows: &[usize]) {
    for &window in windows {
        let hwnd = window as HWND;
        // SAFETY: every call takes the handle and returns; a dead handle makes `GetWindowThreadProcessId`
        // return 0 and leaves `pid` 0, which is not this process. `PostMessageW` queues to the window's own thread.
        unsafe {
            let mut pid = 0u32;
            if GetWindowThreadProcessId(hwnd, &mut pid) == 0 || pid != GetCurrentProcessId() {
                continue;
            }
            if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
                continue;
            }
            let mut client = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetClientRect(hwnd, &mut client) == 0 {
                continue;
            }
            let Some(lparam) = size_lparam(client.right - client.left, client.bottom - client.top) else {
                continue;
            };
            let state = if IsZoomed(hwnd) != 0 {
                SIZE_MAXIMIZED
            } else {
                SIZE_RESTORED
            };
            PostMessageW(hwnd, WM_SIZE, state as usize, lparam);
        }
    }
}

/// A window needs the nudge: maximized, and its DPI is not its monitor's. A DPI of 0 is a failed read: no nudge.
fn is_stuck(maximized: bool, window_dpi: u32, monitor_dpi: u32) -> bool {
    maximized && window_dpi != 0 && monitor_dpi != 0 && window_dpi != monitor_dpi
}

/// One window of the pass: this process's visible maximized ones are noted, and one with a stale DPI gets a
/// one-pixel-shorter `SetWindowPos`; Windows then applies the monitor's DPI and re-fits the maximized rect.
unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY (whole body): `hwnd` is a live top-level window handed by `EnumWindows`; the reads below take it and
    // return. A window of another thread of this process (the app's UI thread) is moved with
    // `SWP_ASYNCWINDOWPOS`: the request is posted to its thread, so this one never waits on it. `lparam` is the
    // `Pass` of `check`, alive for the whole `EnumWindows` call and touched by this thread only.
    unsafe {
        let pass = &mut *(lparam as *mut Pass);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != GetCurrentProcessId() || IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let maximized = IsZoomed(hwnd) != 0;
        if !maximized {
            return 1;
        }
        pass.maximized.push(hwnd as usize);
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
            pass.nudged += 1;
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::{is_stuck, size_lparam};

    #[test]
    fn dpi_resend_packs_the_client_size_as_wm_size_does() {
        // 1280 x 740: winit reads the width from the low word and the height from the high word.
        let lparam = size_lparam(1280, 740).unwrap();
        assert_eq!((lparam as u32 & 0xFFFF, (lparam as u32 >> 16) & 0xFFFF), (1280, 740));
        assert_eq!(size_lparam(0xFFFF, 1), Some((1 << 16) | 0xFFFF));
        // An empty area (a minimized window) or one past the two words sends nothing.
        assert_eq!(size_lparam(0, 740), None);
        assert_eq!(size_lparam(1280, 0), None);
        assert_eq!(size_lparam(-4, 10), None);
        assert_eq!(size_lparam(0x1_0000, 10), None);
    }

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
