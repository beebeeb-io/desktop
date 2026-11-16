//! Where the main window may be, as the part of each display the taskbar leaves free: the first-run and the
//! restored size are kept inside it, and a place that is off every display comes back onto the primary one.
//!
//! Pure math over [`Monitor`] (`window_state`); the OS read is [`os_work_areas`], the same `rcWork` the flyout
//! pins itself to (`flyout::Anchor::work`).

use super::flyout::Rect;
use super::window_state::{Monitor, Restored};

/// What a window's frame adds around the inner size Rune is given, at scale 1: measured 1282 x 801 for a
/// 1280 x 800 inner size (rung R-UP). It is kept out of the room so the OUTER window fits the work area.
pub const FRAME_PX: (f64, f64) = (2.0, 1.0);

/// The displays as the window code should see them: each one's rectangle is its work area (when the OS gave
/// one for that display) less the frame allowance. `work`: `(whole display, its work area)` pairs, as
/// [`os_work_areas`] reads them; a display with no pair keeps its whole rectangle.
pub fn usable(displays: &[Monitor], _work: &[(Rect, Rect)]) -> Vec<Monitor> {
    displays.to_vec()
}

/// The first place of a window nothing is saved for: `size_points` at the primary display's scale, never larger
/// than its work area, never under `min_points` where the area allows it, centred in it. No usable display: the
/// OS and the window's defaults decide.
pub fn first_place(_s: (f32, f32), _m: (f32, f32), _mo: &[Monitor]) -> Restored {
    Restored::default()
}

/// Every display's `(whole rectangle, work area)`, physical pixels. Empty when the OS cannot say (and off
/// Windows): [`usable`] then keeps the whole displays.
pub fn os_work_areas() -> Vec<(Rect, Rect)> {
    #[cfg(windows)]
    {
        os::work_areas()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
mod os {
    use super::Rect;
    use windows_sys::Win32::Foundation::{LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};

    unsafe extern "system" fn collect(monitor: HMONITOR, _dc: HDC, _rect: *mut RECT, data: LPARAM) -> i32 {
        // SAFETY: `data` is the address of the Vec `work_areas` passed below, alive for the whole call, and the
        // callback runs on the calling thread.
        let out = unsafe { &mut *(data as *mut Vec<(Rect, Rect)>) };
        // SAFETY: MONITORINFO is plain data; all-zero is valid and cbSize is set before it is filled.
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MONITORINFO>() as u32;
        // SAFETY: `monitor` came from the enumeration just now; `info` is a live local.
        if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
            let rect = |r: RECT| Rect {
                x: r.left,
                y: r.top,
                w: r.right - r.left,
                h: r.bottom - r.top,
            };
            out.push((rect(info.rcMonitor), rect(info.rcWork)));
        }
        1
    }

    pub fn work_areas() -> Vec<(Rect, Rect)> {
        let mut out: Vec<(Rect, Rect)> = Vec::new();
        // SAFETY: a null device context and clip rectangle enumerate every display; the callback only reads
        // `out` through `data` while this call runs.
        unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(collect),
                &mut out as *mut _ as LPARAM,
            );
        }
        out
    }
}

#[cfg(test)]
#[path = "work_area_tests.rs"]
mod tests;
