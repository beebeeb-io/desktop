//! Windows: the b in the notification area (`Shell_NotifyIconW`), the registry Run value, and
//! the taskbar's mode. Compiled only for Windows; the lead's native build is its first compile
//! against a real linker, so every function here is listed in the PR body.
//!
//! Why not Rune's `rune::Tray` (`crates/rune/src/tray/win32.rs` at c92e462): it takes one icon
//! at creation and has no way to change it (six states need six icons), it does not report
//! where the icon is (the flyout opens on the display that holds the b), and its window is
//! message-only (`HWND_MESSAGE`), which Windows does not send broadcasts to, so its
//! `TaskbarCreated` handler cannot run after Explorer restarts.
//!
//! The tray lives on its own thread with a hidden top-level window (never shown), so it gets
//! `TaskbarCreated`, `WM_SETTINGCHANGE` (the taskbar's light or dark mode) and
//! `WM_DISPLAYCHANGE` (a scale change), and answers while the app's window is closed.

use std::ptr;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Instant;

use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT,
    WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, MonitorFromRect,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_BINARY, REG_SZ, RRF_NOEXPAND, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RRF_RT_REG_EXPAND_SZ,
    RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4,
    NOTIFYICONDATAW, NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DispatchMessageW, FindWindowW, GWL_EXSTYLE, GWLP_USERDATA, GetCursorPos, GetMessageW, GetSystemMetrics,
    GetWindowLongPtrW, HICON, KillTimer, MB_ICONINFORMATION, MB_OK, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG,
    MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SM_CXSMICON,
    SPI_GETCLIENTAREAANIMATION, SetForegroundWindow, SetTimer, SetWindowLongPtrW, SystemParametersInfoW,
    TPM_BOTTOMALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTALIGN, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY, WM_DISPLAYCHANGE, WM_NCCREATE, WM_NCDESTROY, WM_NULL,
    WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};

use super::{Platform, PlatformError, TrayEvent, TrayHandler, TrayIcon};
use crate::shell::autostart;
use crate::shell::flyout::{Anchor, Rect};
use crate::shell::tray::{MenuAction, MenuEntry, TrayIconState, TrayModel};
use crate::shell::tray_icon::{self, SYNC_STEPS, TaskbarTheme};

const CALLBACK: u32 = WM_APP + 1;
const UPDATE: u32 = WM_APP + 2;
const ICON_ID: u32 = 1;
/// Turns the syncing badge: one turn a second.
const TIMER_ANIMATE: usize = 1;
/// Tries again to add the icon while the taskbar is not there yet (a start at login).
const TIMER_RETRY: usize = 2;
/// `NIN_SELECT` and `NIN_KEYSELECT` (shellapi.h): a click, and Enter or Space, on the icon.
const NIN_SELECT: u32 = 0x0400;
const NIN_KEYSELECT: u32 = 0x0401;
const PERSONALIZE_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";

pub struct WindowsPlatform;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn os_error(what: &str) -> PlatformError {
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

fn write_run_value() -> Result<(), PlatformError> {
    let command = wide(&autostart::command_line(&current_exe()?));
    let bytes: Vec<u8> = command.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    reg_write(autostart::RUN_KEY, autostart::VALUE_NAME, REG_SZ, &bytes)
}

/// Whether the taskbar is in light mode (`SystemUsesLightTheme`); dark when unreadable.
fn read_taskbar_theme() -> TaskbarTheme {
    match reg_read(PERSONALIZE_KEY, "SystemUsesLightTheme", RRF_RT_REG_DWORD) {
        Ok(Some(bytes)) if bytes.first().is_some_and(|b| *b != 0) => TaskbarTheme::Light,
        _ => TaskbarTheme::Dark,
    }
}

// ---------------------------------------------------------------------------
// The tray icon
// ---------------------------------------------------------------------------

struct Context {
    model: Arc<Mutex<TrayModel>>,
    on_event: TrayHandler,
    icon: HICON,
    /// What `icon` shows: state, mode, size, animation step.
    shown: Option<(TrayIconState, TaskbarTheme, u32, u8)>,
    step: u8,
    added: bool,
    animating: bool,
    retrying: bool,
    taskbar_created: u32,
    last_key_select: Option<Instant>,
}

struct WindowsTray {
    window: isize,
    model: Arc<Mutex<TrayModel>>,
    worker: Option<JoinHandle<()>>,
}

fn lock(model: &Mutex<TrayModel>) -> std::sync::MutexGuard<'_, TrayModel> {
    model.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The small-icon size on the display that holds the taskbar, in physical pixels.
fn icon_size() -> u32 {
    let class = wide("Shell_TrayWnd");
    // SAFETY: the class name is NUL-terminated; a null window name matches any.
    let taskbar = unsafe { FindWindowW(class.as_ptr(), ptr::null()) };
    // SAFETY: plain queries; a null or stale handle yields 0, handled below.
    let size = unsafe {
        let dpi = if taskbar.is_null() { 0 } else { GetDpiForWindow(taskbar) };
        if dpi == 0 {
            GetSystemMetrics(SM_CXSMICON)
        } else {
            GetSystemMetricsForDpi(SM_CXSMICON, dpi)
        }
    };
    if size > 0 { size as u32 } else { 16 }
}

/// Whether Windows' animation effects are on (DW-1230: with them off the badge does not turn).
fn animations_enabled() -> bool {
    let mut enabled: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through the pointer.
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut enabled as *mut i32).cast(), 0) };
    ok == 0 || enabled != 0
}

/// A 32-bit icon from straight RGBA; the AND mask is ignored beside the alpha channel.
fn make_icon(state: TrayIconState, theme: TaskbarTheme, size: u32, step: u8) -> HICON {
    let image = tray_icon::render(state, theme, size, step);
    let mut bgra = image.pixels;
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    // One mask bit per pixel, rows padded to 16 bits; all zero (the alpha channel decides).
    let mask = vec![0u8; (image.width.div_ceil(16) * 2 * image.height) as usize];
    // SAFETY: both buffers cover width x height at the stated depths and outlive the call,
    // which copies them.
    unsafe {
        CreateIcon(
            ptr::null_mut(),
            image.width as i32,
            image.height as i32,
            1,
            32,
            mask.as_ptr(),
            bgra.as_ptr(),
        )
    }
}

fn icon_data(hwnd: HWND, flags: u32) -> NOTIFYICONDATAW {
    // SAFETY: NOTIFYICONDATAW is plain data and all-zero is its empty value.
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = ICON_ID;
    data.uFlags = flags;
    data
}

/// Brings the notification-area icon in line with the model: adds it when it is not there,
/// renders a new icon only when the state, the taskbar's mode, the scale or the step changed.
fn refresh(hwnd: HWND, context: &mut Context) {
    let model = lock(&context.model).clone();
    let (theme, size) = (read_taskbar_theme(), icon_size());
    let animate = model.icon == TrayIconState::Syncing && animations_enabled();
    if animate != context.animating {
        // SAFETY: timers on this thread's own window.
        unsafe {
            if animate {
                SetTimer(hwnd, TIMER_ANIMATE, 1000 / u32::from(SYNC_STEPS), None);
            } else {
                KillTimer(hwnd, TIMER_ANIMATE);
            }
        }
        context.animating = animate;
        context.step = 0;
    }
    let key = (model.icon, theme, size, if animate { context.step } else { 0 });
    let mut old_icon: HICON = ptr::null_mut();
    if context.shown != Some(key) || context.icon.is_null() {
        let icon = make_icon(key.0, key.1, key.2, key.3);
        if !icon.is_null() {
            old_icon = std::mem::replace(&mut context.icon, icon);
            context.shown = Some(key);
        }
    }
    let mut data = icon_data(hwnd, NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP);
    data.uCallbackMessage = CALLBACK;
    data.hIcon = context.icon;
    let tip: Vec<u16> = model.tooltip.encode_utf16().take(data.szTip.len() - 1).collect();
    data.szTip[..tip.len()].copy_from_slice(&tip);
    // SAFETY: `data` is fully initialised for this window's icon.
    unsafe {
        if context.added {
            Shell_NotifyIconW(NIM_MODIFY, &data);
        } else if Shell_NotifyIconW(NIM_ADD, &data) != 0 {
            context.added = true;
            // Version 4: clicks arrive as NIN_SELECT / WM_CONTEXTMENU with coordinates.
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            Shell_NotifyIconW(NIM_SETVERSION, &data);
        }
        if !old_icon.is_null() {
            DestroyIcon(old_icon);
        }
        // The taskbar is not there yet (a start at login): keep trying.
        if !context.added && !context.retrying {
            SetTimer(hwnd, TIMER_RETRY, 2000, None);
            context.retrying = true;
        } else if context.added && context.retrying {
            KillTimer(hwnd, TIMER_RETRY);
            context.retrying = false;
        }
    }
}

fn anchor_of(monitor: HMONITOR) -> Option<Anchor> {
    if monitor.is_null() {
        return None;
    }
    // SAFETY: MONITORINFO is plain data; cbSize is set before the call that fills it.
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // SAFETY: `monitor` came from MonitorFrom*; the out-pointers are valid for the calls.
    unsafe {
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) != 0 {
            dpi_x = 96;
        }
    }
    let rect = |r: RECT| Rect {
        x: r.left,
        y: r.top,
        w: r.right - r.left,
        h: r.bottom - r.top,
    };
    Some(Anchor {
        monitor: rect(info.rcMonitor),
        work: rect(info.rcWork),
        scale: f64::from(dpi_x) / 96.0,
    })
}

/// The display that holds the b: from the icon's own rectangle, else from the pointer.
fn tray_anchor(hwnd: HWND) -> Option<Anchor> {
    // SAFETY: NOTIFYICONIDENTIFIER is plain data; the zero GUID means "by window and id".
    let mut id: NOTIFYICONIDENTIFIER = unsafe { std::mem::zeroed() };
    id.cbSize = size_of::<NOTIFYICONIDENTIFIER>() as u32;
    id.hWnd = hwnd;
    id.uID = ICON_ID;
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `id` and `rect` are valid for the call.
    if unsafe { Shell_NotifyIconGetRect(&id, &mut rect) } == 0 {
        // SAFETY: `rect` was filled by the call above.
        return anchor_of(unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST) });
    }
    let mut at = POINT { x: 0, y: 0 };
    // SAFETY: `at` is valid for the call; on failure it stays at the origin (the primary).
    unsafe {
        GetCursorPos(&mut at);
        anchor_of(MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST))
    }
}

/// Shows the b's menu above the icon and returns what was picked.
fn pick(hwnd: HWND, context: &Context, at: POINT) -> Option<MenuAction> {
    let menu = lock(&context.model).menu.clone();
    // SAFETY: the menu is created, shown and destroyed on this window's thread.
    unsafe {
        let popup = CreatePopupMenu();
        if popup.is_null() {
            return None;
        }
        for (index, entry) in menu.iter().enumerate() {
            match entry {
                MenuEntry::Item { label, enabled, .. } => {
                    let flags = if *enabled { MF_STRING } else { MF_STRING | MF_GRAYED };
                    AppendMenuW(popup, flags, index + 1, wide(label).as_ptr());
                }
                MenuEntry::Separator => {
                    AppendMenuW(popup, MF_SEPARATOR, 0, ptr::null());
                }
            }
        }
        // Without the foreground the menu does not close on a click elsewhere.
        SetForegroundWindow(hwnd);
        let flags = TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_RIGHTALIGN | TPM_BOTTOMALIGN;
        let picked = TrackPopupMenu(popup, flags, at.x, at.y, 0, hwnd, ptr::null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(popup);
        match menu.get((picked as usize).checked_sub(1)?) {
            Some(MenuEntry::Item {
                action, enabled: true, ..
            }) => Some(*action),
            _ => None,
        }
    }
}

fn send(context: &Context, event: TrayEvent) {
    // A panic in the app's handler must not unwind through the window procedure.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (context.on_event)(event)));
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: Windows owns the message parameters. The context is a Box that outlives the
    // window, is only touched on this thread, and is detached at WM_NCDESTROY.
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(l as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let Some(context) = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Context).as_mut() else {
            return DefWindowProcW(hwnd, message, w, l);
        };
        match message {
            CALLBACK => {
                // Version 4: the event is the low word of lParam, the anchor point is in wParam.
                let at = POINT {
                    x: i32::from((w & 0xFFFF) as u16 as i16),
                    y: i32::from(((w >> 16) & 0xFFFF) as u16 as i16),
                };
                match (l as u32) & 0xFFFF {
                    NIN_SELECT => send(
                        context,
                        TrayEvent::Activate {
                            anchor: tray_anchor(hwnd),
                        },
                    ),
                    NIN_KEYSELECT => {
                        // Enter sends this twice; one keypress is one toggle.
                        let repeat = context.last_key_select.is_some_and(|t| t.elapsed().as_millis() < 250);
                        context.last_key_select = Some(Instant::now());
                        if !repeat {
                            send(
                                context,
                                TrayEvent::Activate {
                                    anchor: tray_anchor(hwnd),
                                },
                            );
                        }
                    }
                    WM_CONTEXTMENU => {
                        if let Some(action) = pick(hwnd, context, at) {
                            send(context, TrayEvent::Menu(action));
                        }
                    }
                    _ => {}
                }
                0
            }
            UPDATE => {
                refresh(hwnd, context);
                0
            }
            WM_TIMER => {
                if w == TIMER_ANIMATE {
                    context.step = (context.step + 1) % SYNC_STEPS;
                }
                refresh(hwnd, context);
                0
            }
            // The taskbar's mode, the scale or the displays changed: the icon may need redrawing.
            WM_SETTINGCHANGE | WM_DISPLAYCHANGE => {
                refresh(hwnd, context);
                DefWindowProcW(hwnd, message, w, l)
            }
            WM_DESTROY => {
                KillTimer(hwnd, TIMER_ANIMATE);
                KillTimer(hwnd, TIMER_RETRY);
                Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd, 0));
                PostQuitMessage(0);
                0
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, message, w, l)
            }
            // Explorer restarted: the icon is gone and has to be added again.
            m if m == context.taskbar_created => {
                context.added = false;
                refresh(hwnd, context);
                0
            }
            _ => DefWindowProcW(hwnd, message, w, l),
        }
    }
}

/// Creates the hidden window on this thread. It is top-level (not message-only) so it
/// receives broadcasts, and a tool window so it never shows in the taskbar or Alt+Tab.
fn create_window(context: &mut Context) -> Result<HWND, PlatformError> {
    let class = wide("BeebeebTray");
    // SAFETY: the class name is NUL-terminated; the context is boxed and stable until the
    // message loop ends.
    unsafe {
        let instance = GetModuleHandleW(ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(os_error("RegisterClassW"));
        }
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            (context as *mut Context).cast(),
        );
        if hwnd.is_null() {
            Err(os_error("CreateWindowExW"))
        } else {
            Ok(hwnd)
        }
    }
}

fn tray_thread(
    model: Arc<Mutex<TrayModel>>,
    on_event: TrayHandler,
    ready: mpsc::SyncSender<Result<isize, PlatformError>>,
) {
    let name = wide("TaskbarCreated");
    // SAFETY: the name is NUL-terminated.
    let taskbar_created = unsafe { RegisterWindowMessageW(name.as_ptr()) };
    let mut context = Box::new(Context {
        model,
        on_event,
        icon: ptr::null_mut(),
        shown: None,
        step: 0,
        added: false,
        animating: false,
        retrying: false,
        taskbar_created,
        last_key_select: None,
    });
    let hwnd = match create_window(&mut context) {
        Ok(hwnd) => hwnd,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    refresh(hwnd, &mut context);
    let _ = ready.send(Ok(hwnd as isize));
    // SAFETY: a plain message loop for this thread's window; it ends with WM_QUIT.
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        if !context.icon.is_null() {
            DestroyIcon(context.icon);
        }
    }
}

impl TrayIcon for WindowsTray {
    fn apply(&self, model: &TrayModel) {
        {
            let mut current = lock(&self.model);
            if *current == *model {
                return;
            }
            *current = model.clone();
        }
        // SAFETY: posts to the tray thread's window; a window that is gone drops the message.
        unsafe { PostMessageW(self.window as HWND, UPDATE, 0, 0) };
    }
}

impl Drop for WindowsTray {
    fn drop(&mut self) {
        // SAFETY: WM_CLOSE destroys the window on its own thread, which then ends its loop.
        unsafe { PostMessageW(self.window as HWND, WM_CLOSE, 0, 0) };
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Platform for WindowsPlatform {
    fn name(&self) -> &'static str {
        "windows"
    }

    fn create_tray(&self, model: &TrayModel, on_event: TrayHandler) -> Result<Box<dyn TrayIcon>, PlatformError> {
        let model = Arc::new(Mutex::new(model.clone()));
        let shared = model.clone();
        let (ready, result) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("beebeeb-tray".into())
            .spawn(move || tray_thread(shared, on_event, ready))
            .map_err(|e| PlatformError::Os(format!("starting the tray thread failed: {e}")))?;
        match result.recv() {
            Ok(Ok(window)) => Ok(Box::new(WindowsTray {
                window,
                model,
                worker: Some(worker),
            })),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                let _ = worker.join();
                Err(PlatformError::Os("the tray thread ended before it started".into()))
            }
        }
    }

    fn taskbar_theme(&self) -> TaskbarTheme {
        read_taskbar_theme()
    }

    fn autostart_enabled(&self) -> Result<bool, PlatformError> {
        let run = reg_read_string(autostart::RUN_KEY, autostart::VALUE_NAME)?;
        let approved = reg_read(
            autostart::STARTUP_APPROVED_KEY,
            autostart::VALUE_NAME,
            RRF_RT_REG_BINARY,
        )
        .unwrap_or(None);
        Ok(autostart::is_enabled(run.as_deref(), approved.as_deref()))
    }

    fn set_autostart(&self, enabled: bool) -> Result<(), PlatformError> {
        if !enabled {
            return reg_delete(autostart::RUN_KEY, autostart::VALUE_NAME);
        }
        write_run_value()?;
        // Turning it on here also undoes "Disabled" in Task Manager, as the 0.8.x app did.
        let _ = reg_write(
            autostart::STARTUP_APPROVED_KEY,
            autostart::VALUE_NAME,
            REG_BINARY,
            &autostart::STARTUP_APPROVED_ENABLED,
        );
        Ok(())
    }

    fn adopt_autostart(&self) -> Result<bool, PlatformError> {
        let run = reg_read_string(autostart::RUN_KEY, autostart::VALUE_NAME)?;
        if !autostart::needs_rewrite(run.as_deref(), &current_exe()?) {
            return Ok(false);
        }
        write_run_value()?;
        Ok(true)
    }

    fn adopt_flyout_window(&self, window: &rune::WindowHandle) {
        let hwnd = window.with_raw_handle(|handle| match handle.as_raw() {
            raw_window_handle::RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
            _ => None,
        });
        let Ok(Some(hwnd)) = hwnd else { return };
        // SAFETY: `hwnd` is this process's own live window, and this runs on its UI thread
        // while it is still hidden, which is when Windows lets the tool-window style change.
        unsafe {
            let style = GetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE, style | WS_EX_TOOLWINDOW as isize);
        }
    }

    fn show_start_refusal(&self, message: &str) {
        let (text, title) = (wide(message), wide("Beebeeb"));
        // SAFETY: both strings are NUL-terminated; no owner window.
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            )
        };
    }
}
