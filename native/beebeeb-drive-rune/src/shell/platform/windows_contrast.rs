//! Windows contrast themes: `SPI_GETHIGHCONTRAST` says whether one is on, `GetSysColor` gives its colours.
//! Read once and kept; `refresh` re-reads (called on `WM_SETTINGCHANGE`). The mapping onto a `Theme` is
//! `theme::contrast::theme_from_system`, which has no Windows in it and is tested everywhere.

use std::sync::Mutex;

use windows_sys::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_HOTLIGHT, COLOR_WINDOW,
    COLOR_WINDOWTEXT, GetSysColor,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SystemParametersInfoW;

use crate::theme::contrast::SystemColors;

const SPI_GETHIGHCONTRAST: u32 = 0x0042;
const HCF_HIGHCONTRASTON: u32 = 0x0000_0001;

/// `HIGHCONTRASTW` (the windows-sys feature that has it is not enabled; the layout is fixed).
#[repr(C)]
struct HighContrastW {
    cb_size: u32,
    flags: u32,
    default_scheme: *mut u16,
}

static STATE: Mutex<Option<Option<SystemColors>>> = Mutex::new(None);

fn read() -> Option<SystemColors> {
    let mut hc = HighContrastW { cb_size: std::mem::size_of::<HighContrastW>() as u32, flags: 0, default_scheme: std::ptr::null_mut() };
    // SAFETY: `hc` is a live, correctly sized HIGHCONTRASTW and `cb_size` says so.
    let ok = unsafe { SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cb_size, &mut hc as *mut _ as *mut _, 0) };
    if ok == 0 || hc.flags & HCF_HIGHCONTRASTON == 0 {
        return None;
    }
    // SAFETY: plain reads of system colours.
    let c = |i| unsafe { GetSysColor(i) };
    Some(SystemColors {
        window: c(COLOR_WINDOW),
        window_text: c(COLOR_WINDOWTEXT),
        highlight: c(COLOR_HIGHLIGHT),
        highlight_text: c(COLOR_HIGHLIGHTTEXT),
        gray_text: c(COLOR_GRAYTEXT),
        hot_light: c(COLOR_HOTLIGHT),
        btn_face: c(COLOR_BTNFACE),
        btn_text: c(COLOR_BTNTEXT),
    })
}

/// Re-read the system's contrast theme (on `WM_SETTINGCHANGE`). True if the answer changed.
#[allow(dead_code)] // only `on_setting_change` calls it, which is not wired yet
pub(super) fn refresh() -> bool {
    let now = read();
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let changed = *s != Some(now);
    *s = Some(now);
    changed
}

/// The colours of the contrast theme that is on, if one is.
pub(crate) fn current() -> Option<SystemColors> {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    *s.get_or_insert_with(read)
}
