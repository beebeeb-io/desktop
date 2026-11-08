//! Windows 11 snap layouts for the app-drawn maximize button (desktop v2 spec 8.1, lead decision 2026-11-08).
//!
//! The v2 main window has no native title bar (`decorations: false`), so Windows only offers the snap-layout
//! flyout over a button it believes is the maximize button: a window answers `WM_NCHITTEST` with `HTMAXBUTTON`.
//! The web view sits on top of the window in its own child windows and swallows the hit test, so the web page
//! reports the maximize button's rectangle (`v2_set_maximize_rect`, physical pixels in the window's client area)
//! and this module subclasses the main window and every child window of it to answer for that rectangle.
//! A click on it arrives as a non-client message, so the subclass also maximizes or restores the window.
//!
//! Nothing here touches keys, tokens or files. On other platforms the command is a no-op.

/// The web page's maximize button, in physical pixels of the window's client area. Called on mount and on every
/// resize of the caption bar (a child window the web view creates later is covered by the next call).
#[tauri::command]
pub fn v2_set_maximize_rect(window: tauri::WebviewWindow, x: i32, y: i32, width: i32, height: i32) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let hwnd = window.hwnd().map_err(|e| format!("failed: {e}"))?;
        imp::install(hwnd.0 as isize, [x, y, x + width, y + height]);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (&window, x, y, width, height);
    }
    Ok(())
}

/// True when a client-area point lies inside `rect` (left, top, right, bottom; right and bottom exclusive).
/// Pure, so it is tested on every platform.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn rect_contains(rect: [i32; 4], x: i32, y: i32) -> bool {
    rect[2] > rect[0] && rect[3] > rect[1] && x >= rect[0] && x < rect[2] && y >= rect[1] && y < rect[3]
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;
    use std::sync::Mutex;

    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, IsZoomed, PostMessageW, HTMAXBUTTON, SC_MAXIMIZE, SC_RESTORE, WM_NCDESTROY, WM_NCHITTEST,
        WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_SYSCOMMAND,
    };

    const SUBCLASS_ID: usize = 0x6262_5632; // "bbV2"

    /// (top-level window, maximize button rect in client physical pixels)
    static STATE: Mutex<Option<(isize, [i32; 4])>> = Mutex::new(None);

    fn hwnd_of(raw: isize) -> HWND {
        HWND(raw as *mut c_void)
    }

    pub fn install(top: isize, rect: [i32; 4]) {
        *STATE.lock().unwrap_or_else(|p| p.into_inner()) = Some((top, rect));
        unsafe {
            let _ = SetWindowSubclass(hwnd_of(top), Some(subclass_proc), SUBCLASS_ID, top as usize);
            let _ = EnumChildWindows(hwnd_of(top), Some(each_child), LPARAM(top));
        }
    }

    unsafe extern "system" fn each_child(child: HWND, top: LPARAM) -> BOOL {
        let _ = SetWindowSubclass(child, Some(subclass_proc), SUBCLASS_ID, top.0 as usize);
        BOOL(1)
    }

    fn over_button(top: isize, lparam: LPARAM) -> bool {
        let Some((owner, rect)) = *STATE.lock().unwrap_or_else(|p| p.into_inner()) else { return false };
        if owner != top {
            return false;
        }
        // lparam carries signed 16-bit screen coordinates.
        let mut pt = POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 };
        unsafe {
            if !ScreenToClient(hwnd_of(top), &mut pt).as_bool() {
                return false;
            }
        }
        super::rect_contains(rect, pt.x, pt.y)
    }

    unsafe extern "system" fn subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        top: usize,
    ) -> LRESULT {
        let top = top as isize;
        match msg {
            WM_NCHITTEST if over_button(top, lparam) => return LRESULT(HTMAXBUTTON as isize),
            // The press and the double-click on the button are ours: no resize or drag from a child window.
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam.0 as u32 == HTMAXBUTTON => return LRESULT(0),
            WM_NCLBUTTONUP if wparam.0 as u32 == HTMAXBUTTON => {
                let top_hwnd = hwnd_of(top);
                let command = if IsZoomed(top_hwnd).as_bool() { SC_RESTORE } else { SC_MAXIMIZE };
                let _ = PostMessageW(top_hwnd, WM_SYSCOMMAND, WPARAM(command as usize), LPARAM(0));
                return LRESULT(0);
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::rect_contains;

    #[test]
    fn the_rect_contains_its_own_pixels_and_not_the_edge() {
        let r = [100, 0, 146, 32];
        assert!(rect_contains(r, 100, 0));
        assert!(rect_contains(r, 145, 31));
        assert!(!rect_contains(r, 146, 10));
        assert!(!rect_contains(r, 120, 32));
        assert!(!rect_contains(r, 99, 10));
    }

    #[test]
    fn an_empty_rect_contains_nothing() {
        assert!(!rect_contains([0, 0, 0, 0], 0, 0));
        assert!(!rect_contains([10, 10, 5, 5], 7, 7));
    }
}
