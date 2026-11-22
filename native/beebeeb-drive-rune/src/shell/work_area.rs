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
pub fn usable(displays: &[Monitor], work: &[(Rect, Rect)]) -> Vec<Monitor> {
    displays
        .iter()
        .map(|d| {
            let whole = Rect {
                x: d.x,
                y: d.y,
                w: d.w as i32,
                h: d.h as i32,
            };
            let area = work.iter().find(|(m, _)| *m == whole).map_or(whole, |(_, w)| *w);
            let scale = if d.scale.is_finite() && d.scale > 0.0 {
                d.scale
            } else {
                1.0
            };
            let (fw, fh) = ((FRAME_PX.0 * scale).ceil() as i32, (FRAME_PX.1 * scale).ceil() as i32);
            Monitor {
                x: area.x,
                y: area.y,
                w: (area.w - fw).max(1) as u32,
                h: (area.h - fh).max(1) as u32,
                ..*d
            }
        })
        .collect()
}

/// The first place of a window nothing is saved for: `size_points` at the primary display's scale, never larger
/// than its work area, never under `min_points` where the area allows it, centred in it. No usable display: the
/// OS and the window's defaults decide.
pub fn first_place(size_points: (f32, f32), min_points: (f32, f32), monitors: &[Monitor]) -> Restored {
    let usable_m = |m: &&Monitor| m.w > 0 && m.h > 0 && m.scale.is_finite() && m.scale > 0.0;
    let Some(m) = monitors
        .iter()
        .filter(usable_m)
        .find(|m| m.primary)
        .or_else(|| monitors.iter().find(usable_m))
    else {
        return Restored::default();
    };
    let fit = |want: f32, min: f32, room: u32| {
        let px = (f64::from(want.max(min)) * m.scale).round() as u32;
        px.min(room).max(1)
    };
    let (w, h) = (
        fit(size_points.0, min_points.0, m.w),
        fit(size_points.1, min_points.1, m.h),
    );
    Restored {
        position: Some((
            (i64::from(m.x) + (i64::from(m.w) - i64::from(w)) / 2) as i32,
            (i64::from(m.y) + (i64::from(m.h) - i64::from(h)) / 2) as i32,
        )),
        physical_size: Some((w, h)),
        maximized: false,
    }
}

/// The size and place of the one window when it changes kind (first run -> main) while it is open: `want_points`
/// at `scale`, cut to the work area of the display `at` (the window's position) is on, and moved so the outer
/// window (inner + frame) lies inside it. `None` when no work area is known (off Windows): the plain resize stands.
/// Returns the inner size in points and the position in physical pixels.
pub fn fit_on_change(
    want_points: (f32, f32),
    scale: f64,
    at: (i32, i32),
    work: &[(Rect, Rect)],
) -> Option<((f32, f32), (i32, i32))> {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let inside = |r: &Rect, p: (i32, i32)| p.0 >= r.x && p.0 < r.x + r.w && p.1 >= r.y && p.1 < r.y + r.h;
    let (_, area) = work
        .iter()
        .find(|(whole, _)| inside(whole, at))
        .or_else(|| work.first())?;
    let (fw, fh) = ((FRAME_PX.0 * scale).ceil() as i32, (FRAME_PX.1 * scale).ceil() as i32);
    let w = ((f64::from(want_points.0) * scale).round() as i32)
        .min(area.w - fw)
        .max(1);
    let h = ((f64::from(want_points.1) * scale).round() as i32)
        .min(area.h - fh)
        .max(1);
    let x = at.0.clamp(area.x, (area.x + area.w - w - fw).max(area.x));
    let y = at.1.clamp(area.y, (area.y + area.h - h - fh).max(area.y));
    Some((((f64::from(w) / scale) as f32, (f64::from(h) / scale) as f32), (x, y)))
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
