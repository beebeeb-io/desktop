//! Windows: the platform seam's implementation, the registry (HKCU only: the Run value and the
//! taskbar's mode) and the small native message box. The b is `windows_tray.rs`; the control
//! channel's named pipe is `windows_control.rs`. Compiled only for Windows.

use std::ptr;
use std::time::Duration;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, GetLastError, HWND};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_BINARY, REG_SZ, RRF_NOEXPAND, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RRF_RT_REG_EXPAND_SZ,
    RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, MB_ICONINFORMATION, MB_OK, MessageBoxW, SetWindowDisplayAffinity,
    SetWindowLongPtrW, WS_EX_TOOLWINDOW,
};

/// `SetWindowDisplayAffinity` values: normal and excluded from capture.
const WDA_NONE: u32 = 0x00;
const WDA_EXCLUDEFROMCAPTURE: u32 = 0x11;

use super::{
    ControlHandler, ControlServer, Platform, PlatformError, ServeError, TrayHandler, TrayIcon, windows_control,
    windows_tray,
};
use crate::shell::autostart::{self, RunChange, Wish};
use crate::shell::control::Request;
use crate::shell::identity::AppIdentity;
use crate::shell::instance::Delivery;
use crate::shell::tray::TrayModel;
use crate::shell::tray_icon::TaskbarTheme;

const PERSONALIZE_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";

pub struct WindowsPlatform;

pub(super) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub(super) fn os_error(what: &str) -> PlatformError {
    // SAFETY: reads this thread's last error right after the failed call.
    PlatformError::Os(format!("{what} failed: Win32 error {}", unsafe { GetLastError() }))
}

// ---------------------------------------------------------------------------
// Registry (HKEY_CURRENT_USER only)
// ---------------------------------------------------------------------------

/// A value's bytes, or None when the key or the value does not exist.
fn reg_read(key: &str, name: &str, flags: u32) -> Result<Option<Vec<u8>>, PlatformError> {
    let (key_w, name_w) = (wide(key), wide(name));
    let mut size: u32 = 0;
    // SAFETY: both names are NUL-terminated; a null buffer with a size pointer asks for the size.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key_w.as_ptr(),
            name_w.as_ptr(),
            flags,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut size,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(PlatformError::Os(format!(
            "reading the registry failed: Win32 error {status}"
        )));
    }
    // Two spare bytes: RegGetValueW may add a terminator the stored string lacks.
    let mut buffer = vec![0u8; size as usize + 2];
    let mut size = buffer.len() as u32;
    // SAFETY: `buffer` is `size` bytes long and outlives the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key_w.as_ptr(),
            name_w.as_ptr(),
            flags,
            ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(PlatformError::Os(format!(
            "reading the registry failed: Win32 error {status}"
        )));
    }
    buffer.truncate(size as usize);
    Ok(Some(buffer))
}

fn reg_read_string(key: &str, name: &str) -> Result<Option<String>, PlatformError> {
    let Some(bytes) = reg_read(key, name, RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND)? else {
        return Ok(None);
    };
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    Ok(Some(String::from_utf16_lossy(&units)))
}

fn reg_write(key: &str, name: &str, kind: u32, data: &[u8]) -> Result<(), PlatformError> {
    let (key_w, name_w) = (wide(key), wide(name));
    // SAFETY: the names are NUL-terminated and `data` is `data.len()` bytes for the call.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key_w.as_ptr(),
            name_w.as_ptr(),
            kind,
            data.as_ptr().cast(),
            data.len() as u32,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(PlatformError::Os(format!(
            "writing the registry failed: Win32 error {status}"
        )))
    }
}

fn reg_delete(key: &str, name: &str) -> Result<(), PlatformError> {
    let (key_w, name_w) = (wide(key), wide(name));
    // SAFETY: both names are NUL-terminated.
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key_w.as_ptr(), name_w.as_ptr()) };
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(PlatformError::Os(format!(
            "writing the registry failed: Win32 error {status}"
        )))
    }
}

fn current_exe() -> Result<String, PlatformError> {
    let exe = std::env::current_exe().map_err(|e| PlatformError::Os(format!("finding this program failed: {e}")))?;
    exe.to_str()
        .map(str::to_owned)
        .ok_or_else(|| PlatformError::Os("this program's path is not valid Unicode".into()))
}

fn run_value_name() -> &'static str {
    autostart::value_name(AppIdentity::CURRENT)
}

/// Makes the one change `autostart::plan` allows to THIS identity's Run value, for THIS
/// program's path. True when the registry changed. A value that starts another program is
/// never written over or deleted, whatever the wish.
fn apply_autostart(wish: Wish) -> Result<bool, PlatformError> {
    let existing = reg_read_string(autostart::RUN_KEY, run_value_name())?;
    match autostart::plan(wish, existing.as_deref(), &current_exe()?) {
        Ok(RunChange::Write(command)) => {
            let bytes: Vec<u8> = wide(&command).iter().flat_map(|unit| unit.to_le_bytes()).collect();
            reg_write(autostart::RUN_KEY, run_value_name(), REG_SZ, &bytes)?;
            Ok(true)
        }
        Ok(RunChange::Delete) => {
            reg_delete(autostart::RUN_KEY, run_value_name())?;
            Ok(true)
        }
        Ok(RunChange::Leave) => Ok(false),
        Err(autostart::OwnedByAnotherInstall) => Err(PlatformError::NotOurs(
            "start at login is set for another copy of this app",
        )),
    }
}

/// Whether the taskbar is in light mode (`SystemUsesLightTheme`); dark when unreadable.
pub(super) fn read_taskbar_theme() -> TaskbarTheme {
    match reg_read(PERSONALIZE_KEY, "SystemUsesLightTheme", RRF_RT_REG_DWORD) {
        Ok(Some(bytes)) if bytes.first().is_some_and(|b| *b != 0) => TaskbarTheme::Light,
        _ => TaskbarTheme::Dark,
    }
}

impl Platform for WindowsPlatform {
    fn name(&self) -> &'static str {
        "windows"
    }

    fn create_tray(&self, model: &TrayModel, on_event: TrayHandler) -> Result<Box<dyn TrayIcon>, PlatformError> {
        windows_tray::create(model, on_event)
    }

    fn taskbar_theme(&self) -> TaskbarTheme {
        read_taskbar_theme()
    }

    fn autostart_enabled(&self) -> Result<bool, PlatformError> {
        let run = reg_read_string(autostart::RUN_KEY, run_value_name())?;
        let approved = reg_read(autostart::STARTUP_APPROVED_KEY, run_value_name(), RRF_RT_REG_BINARY).unwrap_or(None);
        // An entry that starts another install is not this program's switch being on.
        let exe = current_exe().unwrap_or_default();
        let ours = run.as_deref().filter(|value| autostart::points_at(value, &exe));
        Ok(autostart::is_enabled(ours, approved.as_deref()))
    }

    fn set_autostart(&self, enabled: bool) -> Result<(), PlatformError> {
        if !enabled {
            return apply_autostart(Wish::Disable).map(|_| ());
        }
        // Refused (and nothing written) when the value starts another copy of this app.
        apply_autostart(Wish::Enable)?;
        // The value is this program's own from here. Turning it on also undoes "Disabled" in
        // Task Manager, as the 0.8.x app did.
        let _ = reg_write(
            autostart::STARTUP_APPROVED_KEY,
            run_value_name(),
            REG_BINARY,
            &autostart::STARTUP_APPROVED_ENABLED,
        );
        Ok(())
    }

    fn adopt_autostart(&self) -> Result<bool, PlatformError> {
        apply_autostart(Wish::Adopt)
    }

    fn adopt_flyout_window(&self, window: &rune::WindowHandle) {
        let hwnd = window.with_raw_handle(|handle| match handle.as_raw() {
            raw_window_handle::RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
            _ => None,
        });
        let Ok(Some(hwnd)) = hwnd else { return };
        // SAFETY: `hwnd` is this process's own live window, and this runs on its UI thread
        // while it is still hidden, which is when Windows lets the tool-window style change.
        // Neither call sends a message that this crate handles.
        unsafe {
            let style = GetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE, style | WS_EX_TOOLWINDOW as isize);
        }
        round_corners(hwnd as HWND);
    }

    fn show_start_refusal(&self, title: &str, message: &str) {
        let (text, title) = (wide(message), wide(title));
        // SAFETY: both strings are NUL-terminated and outlive the call; no owner window. The
        // box runs its own message loop on this thread, which owns no window of ours yet
        // (this is called before the app's windows exist).
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            )
        };
    }

    fn serve_control(
        &self,
        identity: AppIdentity,
        handler: ControlHandler,
    ) -> Result<Box<dyn ControlServer>, ServeError> {
        let name = windows_control::pipe_name(identity).map_err(ServeError::Os)?;
        windows_control::serve(&name, identity, handler)
    }

    fn send_control(&self, identity: AppIdentity, request: &Request, timeout: Duration) -> Delivery {
        match windows_control::pipe_name(identity) {
            Ok(name) => windows_control::send(&name, request, timeout),
            Err(_) => Delivery::Unreachable,
        }
    }

    fn set_capture_exclusion(&self, window: &rune::WindowHandle, exclude: bool) -> Result<(), PlatformError> {
        let hwnd = window.with_raw_handle(|handle| match handle.as_raw() {
            raw_window_handle::RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
            _ => None,
        });
        let Ok(Some(hwnd)) = hwnd else {
            return Err(PlatformError::Os("no Win32 window handle".into()));
        };
        let affinity = if exclude { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
        // SAFETY: `hwnd` is this process's own live window, obtained from `WindowHandle::with_raw_handle`.
        // `SetWindowDisplayAffinity` reads the u32 and returns; no callback, no message dispatch.
        let ok = unsafe { SetWindowDisplayAffinity(hwnd as HWND, affinity) };
        if ok != 0 {
            Ok(())
        } else {
            Err(os_error("SetWindowDisplayAffinity"))
        }
    }
}

/// The flyout's corners and shadow (DW-1201, `.wn-fly .mk-pop`: "border-radius: 8px" and a drop
/// shadow): on Windows 11 the compositor rounds a window that asks for it (8 px at 100 %) and
/// draws the shadow of a rounded window itself. A borderless window gets neither unless asked.
/// Windows 10 has no such attribute: the call fails there and the flyout stays square.
/// Returns the HRESULT (0: rounded).
fn round_corners(hwnd: HWND) -> i32 {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
    let preference = DWMWCP_ROUND;
    // SAFETY: `hwnd` is this process's own live window; the attribute is read from a local
    // of exactly the size passed, during the call only.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            std::ptr::from_ref(&preference).cast(),
            std::mem::size_of_val(&preference) as u32,
        )
    }
}
