//! Windows: the b in the notification area (`Shell_NotifyIconW`). The pure decisions (callback
//! decoding, the tooltip cut, the menu pick) are in `shell/tray_win.rs`.
//!
//! Why not Rune's `rune::Tray` (`crates/rune/src/tray/win32.rs` at c92e462): it takes one icon
//! at creation and has no way to change it (six states need six icons), it does not report
//! where the icon is (the flyout opens on the display that holds the b), and its window is
//! message-only (`HWND_MESSAGE`), which Windows does not send broadcasts to, so its
//! `TaskbarCreated` handler cannot run after Explorer restarts.
//!
//! The tray lives on its own thread with a hidden top-level window (never shown), so it gets
//! `TaskbarCreated`, `WM_SETTINGCHANGE` (the taskbar's light or dark mode), `WM_DISPLAYCHANGE`
//! (a scale change) and `WM_ENDSESSION`, and answers while the app's window is closed.
//!
//! # The aliasing rule of this file
//!
//! The window procedure RE-ENTERS: `TrackPopupMenu` runs a modal loop that dispatches this
//! window's timer and update messages, `Shell_NotifyIconW` is a cross-process send during
//! which sent messages (`WM_SETTINGCHANGE`) arrive, and `DestroyWindow` sends `WM_DESTROY` and
//! `WM_NCDESTROY` from inside the handler that called it. So:
//!
//! - **No `&mut Context` exists, anywhere.** The window owns one strong `Rc<Context>`; every
//!   procedure call takes its own strong reference for its duration; all of them are shared.
//! - **Everything that changes is in a `Cell` or the one `RefCell<IconState>`, and a borrow of
//!   that `RefCell` never spans a call that can re-enter** (anything of [`ShellCalls`],
//!   `DestroyWindow`, the app's handler). Read a copy out, release, call, write back.
//! - A borrow that does span one is not undefined behaviour here but a `RefCell` panic in the
//!   nested message, which the procedure catches and counts ([`Context::faults`]). The harness
//!   at the bottom delivers a timer tick and an update inside the menu call and inside
//!   `Shell_NotifyIconW`, and fails when that count is not zero.
//! - Nothing unwinds out of the procedure: its whole body runs under `catch_unwind`.

use std::cell::{Cell, RefCell};
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Instant;

use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, MonitorFromRect,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4,
    NOTIFYICONDATAW, NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, FindWindowW, GWLP_USERDATA, GetCursorPos, GetMessageW, GetSystemMetrics,
    GetWindowLongPtrW, HICON, HMENU, KillTimer, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SPI_GETCLIENTAREAANIMATION, SetForegroundWindow, SetTimer,
    SetWindowLongPtrW, SystemParametersInfoW, TPM_BOTTOMALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTALIGN,
    TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE, WM_ENDSESSION,
    WM_NCCREATE, WM_NCDESTROY, WM_NULL, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};

use super::windows::{os_error, read_taskbar_theme, wide};
use super::{PlatformError, TrayEvent, TrayHandler, TrayIcon};
use crate::shell::flyout::{Anchor, Rect};
use crate::shell::identity::AppIdentity;
use crate::shell::tray::{MenuAction, MenuEntry, TrayIconState, TrayModel};
use crate::shell::tray_icon::{self, SYNC_STEPS, TaskbarTheme};
use crate::shell::tray_win::{self, NIN_KEYSELECT, NIN_SELECT, WM_CONTEXTMENU};

const CALLBACK: u32 = WM_APP + 1;
const UPDATE: u32 = WM_APP + 2;
/// The app's own request to remove the icon and end the tray thread (`WindowsTray::drop`).
const SHUTDOWN: u32 = WM_APP + 3;
/// The app's quit hook ran (`TrayIcon::remove_for_quit`): take the icon away, keep the window.
const REMOVE_FOR_QUIT: u32 = WM_APP + 4;
/// Posted by `WM_ENDSESSION(FALSE)` to this window itself: the shutdown was cancelled. Posted,
/// not handled in place, so it queues BEHIND a `REMOVE_FOR_QUIT` the quit hook posted during
/// the query phase, whatever order the two threads ran in.
const RESTORE_AFTER_CANCEL: u32 = WM_APP + 5;
const ICON_ID: u32 = 1;
/// Turns the syncing badge: one turn a second.
const TIMER_ANIMATE: usize = 1;
/// Tries again to add the icon while the taskbar is not there yet (a start at login).
const TIMER_RETRY: usize = 2;

/// The calls that leave this thread's control: a cross-process send, or a modal loop. Each can
/// re-enter [`procedure`] before it returns. Behind a trait so the harness can re-enter on
/// purpose without a notification area.
trait ShellCalls {
    /// `Shell_NotifyIconW`. True when the shell took it.
    fn notify(&self, message: u32, data: &NOTIFYICONDATAW) -> bool;
    /// `TrackPopupMenu` with `TPM_RETURNCMD`: the picked command, or 0.
    fn track_menu(&self, hwnd: HWND, popup: HMENU, at: POINT) -> u32;
}

struct RealShell;

impl ShellCalls for RealShell {
    fn notify(&self, message: u32, data: &NOTIFYICONDATAW) -> bool {
        // SAFETY: `data` is a fully initialised NOTIFYICONDATAW with its cbSize set
        // (`icon_data`), valid for the call. The call can dispatch sent messages to this
        // thread's window: the caller holds no `RefCell` borrow (the file's aliasing rule).
        unsafe { Shell_NotifyIconW(message, data) != 0 }
    }

    fn track_menu(&self, hwnd: HWND, popup: HMENU, at: POINT) -> u32 {
        let flags = TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_RIGHTALIGN | TPM_BOTTOMALIGN;
        // SAFETY: `popup` is a live menu created on this thread and `hwnd` is this thread's
        // window. The call runs a modal message loop that dispatches this window's messages:
        // the caller holds no `RefCell` borrow (the file's aliasing rule).
        unsafe { TrackPopupMenu(popup, flags, at.x, at.y, 0, hwnd, ptr::null()) as u32 }
    }
}

/// What `IconState::icon` shows: state, mode, size, animation step.
type Shown = (TrayIconState, TaskbarTheme, u32, u8);

/// Everything about the icon that changes. `Copy`, so a handler reads a snapshot out of the
/// `RefCell` and holds no borrow.
#[derive(Clone, Copy)]
struct IconState {
    icon: HICON,
    shown: Option<Shown>,
    step: u8,
    /// `NIM_ADD` worked and nothing has said the icon is gone since.
    added: bool,
    animating: bool,
    retrying: bool,
    /// The quit hook took the icon away; nothing adds it again until a cancelled shutdown.
    removed_for_quit: bool,
    last_key_select: Option<Instant>,
}

struct Context {
    model: Arc<Mutex<TrayModel>>,
    on_event: TrayHandler,
    /// `IconState::added`, readable from the app's thread (`TrayIcon::present`).
    present: Arc<AtomicBool>,
    shell: Box<dyn ShellCalls>,
    taskbar_created: u32,
    state: RefCell<IconState>,
    /// A refresh is running further up this thread's stack.
    refreshing: Cell<bool>,
    /// A refresh was asked for while one was running: the running one goes round again.
    refresh_again: Cell<bool>,
    /// Panics caught at the procedure's boundary. Zero in a correct build.
    faults: Arc<AtomicU32>,
}

impl Context {
    fn snapshot(&self) -> IconState {
        *self.state.borrow()
    }

    /// One short write. The borrow ends before this returns.
    fn update(&self, change: impl FnOnce(&mut IconState)) {
        change(&mut self.state.borrow_mut());
    }

    /// Tells the app. The handler is cloned out first; no borrow is held while it runs.
    fn send(&self, event: TrayEvent) {
        let handler = self.on_event.clone();
        handler(event);
    }
}

fn lock(model: &Mutex<TrayModel>) -> std::sync::MutexGuard<'_, TrayModel> {
    model.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The small-icon size on the display that holds the taskbar, in physical pixels.
fn icon_size() -> u32 {
    let class = wide("Shell_TrayWnd");
    // SAFETY: the class name is NUL-terminated and outlives the call; a null window name
    // matches any. FindWindowW sends no message.
    let taskbar = unsafe { FindWindowW(class.as_ptr(), ptr::null()) };
    // SAFETY: plain queries that take no pointer of ours; a null or stale handle yields 0,
    // handled below.
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
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through the pointer, which is a live
    // local for the call.
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut enabled as *mut i32).cast(), 0) };
    ok == 0 || enabled != 0
}

/// A 32-bit icon from straight RGBA; the AND mask is ignored beside the alpha channel.
fn make_icon(shown: Shown) -> HICON {
    let image = tray_icon::render(shown.0, shown.1, shown.2, shown.3);
    let mut bgra = image.pixels;
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    // All zero: the alpha channel decides.
    let mask = vec![0u8; tray_win::and_mask_len(image.width, image.height)];
    // SAFETY: `mask` holds one bit per pixel in 16-bit-padded rows and `bgra` four bytes per
    // pixel for width x height (`tray_icon::render`'s contract, `and_mask_len`); both outlive
    // the call, which copies them.
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
    // SAFETY: NOTIFYICONDATAW is plain data (integers, handles, arrays) and all-zero is its
    // documented empty value.
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = ICON_ID;
    data.uFlags = flags;
    data
}

const MAX_REFRESH_PASSES: u32 = 3;

/// Resets `Context::refreshing` also when a refresh panics.
struct Refreshing<'a>(&'a Context);

impl Drop for Refreshing<'_> {
    fn drop(&mut self) {
        self.0.refreshing.set(false);
    }
}

/// Brings the notification-area icon in line with the model. A refresh asked for while one is
/// running (a message that arrived inside `Shell_NotifyIconW`) does not nest: the running one
/// goes round once more, so every caller sees one consistent pass at a time.
fn refresh(hwnd: HWND, context: &Context) {
    if context.refreshing.replace(true) {
        context.refresh_again.set(true);
        return;
    }
    let _running = Refreshing(context);
    // Bounded: a storm of messages arriving inside every pass must not keep this handler
    // going round for ever. What a dropped pass missed, the next message or tick shows.
    for _ in 0..MAX_REFRESH_PASSES {
        context.refresh_again.set(false);
        refresh_once(hwnd, context);
        if !context.refresh_again.get() {
            break;
        }
    }
}

/// One pass: adds the icon when it is not there, renders a new one only when the state, the
/// taskbar's mode, the scale or the step changed. No `RefCell` borrow is held across a call
/// of [`ShellCalls`].
fn refresh_once(hwnd: HWND, context: &Context) {
    if context.snapshot().removed_for_quit {
        return;
    }
    let model = lock(&context.model).clone();
    let (theme, size) = (read_taskbar_theme(), icon_size());
    let animate = model.icon == TrayIconState::Syncing && animations_enabled();
    if animate != context.snapshot().animating {
        // SAFETY: a timer on this thread's own live window. Neither call dispatches messages.
        unsafe {
            if animate {
                SetTimer(hwnd, TIMER_ANIMATE, 1000 / u32::from(SYNC_STEPS), None);
            } else {
                KillTimer(hwnd, TIMER_ANIMATE);
            }
        }
        context.update(|state| {
            state.animating = animate;
            state.step = 0;
        });
    }
    let before = context.snapshot();
    let key: Shown = (model.icon, theme, size, if animate { before.step } else { 0 });
    let mut old_icon: HICON = ptr::null_mut();
    if before.shown != Some(key) || before.icon.is_null() {
        let icon = make_icon(key);
        if !icon.is_null() {
            old_icon = before.icon;
            context.update(|state| {
                state.icon = icon;
                state.shown = Some(key);
            });
        }
    }
    let mut data = icon_data(hwnd, NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP);
    data.uCallbackMessage = CALLBACK;
    data.hIcon = context.snapshot().icon;
    let tip = tray_win::tip_units(&model.tooltip);
    data.szTip[..tip.len()].copy_from_slice(&tip);

    // A failed modify means the shell no longer has the icon (Explorer restarted without the
    // broadcast reaching us): fall through to adding it again.
    let mut added = before.added && context.shell.notify(NIM_MODIFY, &data);
    if !added {
        // `TaskbarCreated` also arrives when the icon is still there (a DPI change of the
        // primary display): adding over it fails, so clear the slot first.
        context.shell.notify(NIM_DELETE, &icon_data(hwnd, 0));
        added = context.shell.notify(NIM_ADD, &data);
        if added {
            // Version 4: clicks arrive as NIN_SELECT / WM_CONTEXTMENU with coordinates.
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            context.shell.notify(NIM_SETVERSION, &data);
        }
    }
    context.update(|state| state.added = added);
    if context.present.swap(added, Ordering::SeqCst) != added {
        context.send(TrayEvent::Presence(added));
    }
    if !old_icon.is_null() {
        // SAFETY: `old_icon` came from CreateIcon, the shell was handed its replacement above,
        // and it was taken out of the state before any re-entrant call, so nothing else holds
        // or frees it.
        unsafe { DestroyIcon(old_icon) };
    }
    // The taskbar is not there (a start at login, Explorer stopped): keep trying.
    let retrying = context.snapshot().retrying;
    if added == retrying {
        // SAFETY: a timer on this thread's own live window. Neither call dispatches messages.
        unsafe {
            if added {
                KillTimer(hwnd, TIMER_RETRY);
            } else {
                SetTimer(hwnd, TIMER_RETRY, 2000, None);
            }
        }
        context.update(|state| state.retrying = !added);
    }
}

/// The quit hook ran: the icon goes now, the window and its thread stay.
fn remove_for_quit(hwnd: HWND, context: &Context) {
    context.update(|state| {
        state.removed_for_quit = true;
        state.added = false;
        state.animating = false;
        state.retrying = false;
    });
    // SAFETY: timers on this thread's own live window. Neither call dispatches messages.
    unsafe {
        KillTimer(hwnd, TIMER_ANIMATE);
        KillTimer(hwnd, TIMER_RETRY);
    }
    context.shell.notify(NIM_DELETE, &icon_data(hwnd, 0));
    // No `Presence(false)` event: the app is quitting, and that event opens the window.
    context.present.store(false, Ordering::SeqCst);
}

fn anchor_of(monitor: HMONITOR) -> Option<Anchor> {
    if monitor.is_null() {
        return None;
    }
    // SAFETY: MONITORINFO is plain data and all-zero is valid; cbSize is set before the call
    // that fills it.
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // SAFETY: `monitor` came from MonitorFrom* just now; the out-pointers are live locals for
    // the calls. Neither call dispatches messages.
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
    // SAFETY: `id` and `rect` are live locals for the call. Called from a handler that holds
    // no `RefCell` borrow, since it asks the shell's process.
    if unsafe { Shell_NotifyIconGetRect(&id, &mut rect) } == 0 {
        // SAFETY: `rect` was filled by the call above.
        return anchor_of(unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST) });
    }
    let mut at = POINT { x: 0, y: 0 };
    // SAFETY: `at` is a live local for the call; on failure it stays at the origin (the primary).
    unsafe {
        GetCursorPos(&mut at);
        anchor_of(MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST))
    }
}

/// Shows the b's menu above the icon and returns what was picked. The menu is a copy taken
/// before the modal loop; the model may change under it, and the pick is read against the
/// copy that was shown.
fn pick(hwnd: HWND, context: &Context, at: POINT) -> Option<MenuAction> {
    let menu = lock(&context.model).menu.clone();
    // SAFETY: creates a menu owned by this function; destroyed below on every path.
    let popup = unsafe { CreatePopupMenu() };
    if popup.is_null() {
        return None;
    }
    for (index, entry) in menu.iter().enumerate() {
        match entry {
            MenuEntry::Item { label, enabled, .. } => {
                let flags = if *enabled { MF_STRING } else { MF_STRING | MF_GRAYED };
                let label = wide(label);
                // SAFETY: `popup` is live; `label` is NUL-terminated and outlives the call,
                // which copies it.
                unsafe { AppendMenuW(popup, flags, tray_win::menu_command(index), label.as_ptr()) };
            }
            MenuEntry::Separator => {
                // SAFETY: `popup` is live; a separator takes no string.
                unsafe { AppendMenuW(popup, MF_SEPARATOR, 0, ptr::null()) };
            }
        }
    }
    // Without the foreground the menu does not close on a click elsewhere.
    // SAFETY: this thread's own live window. No `RefCell` borrow is held (it can activate).
    unsafe { SetForegroundWindow(hwnd) };
    let picked = context.shell.track_menu(hwnd, popup, at);
    // SAFETY: this thread's own window and the menu created above; the post is the documented
    // companion of TrackPopupMenu for a notification icon.
    unsafe {
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(popup);
    }
    tray_win::menu_pick(&menu, picked)
}

/// One message. `None`: Windows' default handling.
fn handle(hwnd: HWND, context: &Context, message: u32, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
    match message {
        CALLBACK => {
            let callback = tray_win::decode_callback(w, l);
            match callback.event {
                NIN_SELECT => context.send(TrayEvent::Activate {
                    anchor: tray_anchor(hwnd),
                }),
                NIN_KEYSELECT => {
                    // Enter sends this twice; one keypress is one toggle.
                    let since = context.snapshot().last_key_select.map(|at| at.elapsed().as_millis());
                    context.update(|state| state.last_key_select = Some(Instant::now()));
                    if !tray_win::is_key_repeat(since) {
                        context.send(TrayEvent::Activate {
                            anchor: tray_anchor(hwnd),
                        });
                    }
                }
                WM_CONTEXTMENU => {
                    let at = POINT {
                        x: callback.x,
                        y: callback.y,
                    };
                    if let Some(action) = pick(hwnd, context, at) {
                        context.send(TrayEvent::Menu(action));
                    }
                }
                _ => {}
            }
            Some(0)
        }
        UPDATE => {
            refresh(hwnd, context);
            Some(0)
        }
        WM_TIMER => {
            if w == TIMER_ANIMATE {
                context.update(|state| state.step = (state.step + 1) % SYNC_STEPS);
            }
            refresh(hwnd, context);
            Some(0)
        }
        // The taskbar's mode, the scale or the displays changed: the icon may need redrawing.
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE => {
            refresh(hwnd, context);
            None
        }
        REMOVE_FOR_QUIT => {
            remove_for_quit(hwnd, context);
            Some(0)
        }
        // The session is not ending after all (another app refused the shutdown). Posted to
        // the back of the queue: see `RESTORE_AFTER_CANCEL`.
        WM_ENDSESSION if w == 0 => {
            // SAFETY: a post to this thread's own live window.
            unsafe { PostMessageW(hwnd, RESTORE_AFTER_CANCEL, 0, 0) };
            Some(0)
        }
        RESTORE_AFTER_CANCEL => {
            if context.snapshot().removed_for_quit {
                context.update(|state| state.removed_for_quit = false);
                refresh(hwnd, context);
                context.send(TrayEvent::ShutdownCancelled);
            }
            Some(0)
        }
        SHUTDOWN => {
            // SAFETY: this thread's own live window. DestroyWindow sends WM_DESTROY and
            // WM_NCDESTROY to `procedure` before it returns: no `RefCell` borrow is held, and
            // this call's own `Rc` keeps the context alive past the window's reference.
            unsafe { DestroyWindow(hwnd) };
            Some(0)
        }
        // Something outside asks the app to close (`taskkill` without /F, Restart Manager, an
        // installer). The window stays (only the app ends the tray); the app is asked to quit.
        WM_CLOSE => {
            context.send(TrayEvent::CloseRequested);
            Some(0)
        }
        WM_DESTROY => {
            // SAFETY: timers on this thread's own window, which is being destroyed.
            unsafe {
                KillTimer(hwnd, TIMER_ANIMATE);
                KillTimer(hwnd, TIMER_RETRY);
            }
            context.shell.notify(NIM_DELETE, &icon_data(hwnd, 0));
            context.update(|state| state.added = false);
            context.present.store(false, Ordering::SeqCst);
            // SAFETY: ends this thread's message loop; takes no pointer.
            unsafe { PostQuitMessage(0) };
            Some(0)
        }
        // Explorer restarted: the icon is gone and has to be added again.
        m if m == context.taskbar_created => {
            context.update(|state| state.added = false);
            refresh(hwnd, context);
            Some(0)
        }
        _ => None,
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if message == WM_NCCREATE {
        // SAFETY: for WM_NCCREATE, `l` points at the CREATESTRUCTW of the CreateWindowExW call
        // in `create_window`, whose lpCreateParams is the `Rc<Context>` raw pointer made there.
        unsafe {
            let create = &*(l as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }
    // SAFETY: reads this window's own user data; no pointer of ours is passed.
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Context;
    if raw.is_null() {
        // Before WM_NCCREATE, or after WM_NCDESTROY.
        // SAFETY: the default procedure with the message's own parameters.
        return unsafe { DefWindowProcW(hwnd, message, w, l) };
    }
    // SAFETY: `raw` is the pointer `Rc::into_raw` produced in `create_window`: the window's own
    // strong reference, which stays alive until the WM_NCDESTROY branch below releases it, and
    // user data is cleared in that same branch, so `raw` is never read after the release. The
    // window and every `Rc` live on this one thread. Taking a second strong reference here
    // (increment, then from_raw) gives this call its own, so the context outlives a nested
    // WM_NCDESTROY (`DestroyWindow` inside `handle`). Only shared references to `Context`
    // are ever made; see the module's aliasing rule.
    let context = unsafe {
        Rc::increment_strong_count(raw);
        Rc::from_raw(raw)
    };
    if message == WM_NCDESTROY {
        // SAFETY: clears the user data, then releases the window's strong reference (the one
        // `Rc::into_raw` leaked), exactly once: no later message finds `raw` again.
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Rc::from_raw(raw));
            return DefWindowProcW(hwnd, message, w, l);
        }
    }
    // Nothing unwinds into Windows: a panic in a handler (or in the app's event handler) is
    // counted, and the message gets the default handling.
    let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(hwnd, &context, message, w, l)));
    match handled {
        Ok(Some(result)) => result,
        Ok(None) => {
            // SAFETY: the default procedure with the message's own parameters.
            unsafe { DefWindowProcW(hwnd, message, w, l) }
        }
        Err(_) => {
            context.faults.fetch_add(1, Ordering::SeqCst);
            // SAFETY: as above.
            unsafe { DefWindowProcW(hwnd, message, w, l) }
        }
    }
}

/// Creates the hidden window on this thread. It is top-level (not message-only) so it
/// receives broadcasts, and a tool window so it never shows in the taskbar or Alt+Tab.
fn create_window(context: &Rc<Context>, class_name: &str) -> Result<HWND, PlatformError> {
    let class = wide(class_name);
    // The window's own strong reference; `procedure` releases it at WM_NCDESTROY.
    let raw = Rc::into_raw(context.clone());
    // SAFETY: the class name is NUL-terminated and outlives both calls; `procedure` is the
    // matching window procedure; `raw` stays valid for the window's whole life (see
    // `procedure`). WNDCLASSW is plain data and all-zero is valid for the fields left out.
    let hwnd = unsafe {
        let instance = GetModuleHandleW(ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            ptr::null_mut()
        } else {
            CreateWindowExW(
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
                raw.cast(),
            )
        }
    };
    if hwnd.is_null() {
        let error = os_error("creating the tray window");
        // The window never took its reference (no WM_NCCREATE), or already gave it back
        // (WM_NCDESTROY of a failed creation). Release it here only in the first case.
        if Rc::strong_count(context) > 1 {
            // SAFETY: `raw` is the pointer `Rc::into_raw` returned above and, by the count,
            // was not released by `procedure`.
            unsafe { drop(Rc::from_raw(raw)) };
        }
        return Err(error);
    }
    Ok(hwnd)
}

/// The context and its window, on the calling thread. The harness calls this with its own
/// [`ShellCalls`].
fn build(
    model: Arc<Mutex<TrayModel>>,
    on_event: TrayHandler,
    present: Arc<AtomicBool>,
    faults: Arc<AtomicU32>,
    shell: Box<dyn ShellCalls>,
    class_name: &str,
) -> Result<(HWND, Rc<Context>), PlatformError> {
    let name = wide("TaskbarCreated");
    // SAFETY: the name is NUL-terminated and outlives the call.
    let taskbar_created = unsafe { RegisterWindowMessageW(name.as_ptr()) };
    let context = Rc::new(Context {
        model,
        on_event,
        present,
        shell,
        taskbar_created,
        state: RefCell::new(IconState {
            icon: ptr::null_mut(),
            shown: None,
            step: 0,
            added: false,
            animating: false,
            retrying: false,
            removed_for_quit: false,
            last_key_select: None,
        }),
        refreshing: Cell::new(false),
        refresh_again: Cell::new(false),
        faults,
    });
    let hwnd = create_window(&context, class_name)?;
    Ok((hwnd, context))
}

/// Frees the last icon. After the window is gone.
fn destroy_last_icon(context: &Context) {
    let icon = context.snapshot().icon;
    context.update(|state| state.icon = ptr::null_mut());
    if !icon.is_null() {
        // SAFETY: `icon` came from CreateIcon, the notification icon that used it was deleted
        // at WM_DESTROY, and it was just taken out of the state.
        unsafe { DestroyIcon(icon) };
    }
}

fn tray_thread(
    model: Arc<Mutex<TrayModel>>,
    on_event: TrayHandler,
    present: Arc<AtomicBool>,
    ready: mpsc::SyncSender<Result<isize, PlatformError>>,
) {
    let built = build(
        model,
        on_event,
        present,
        Arc::new(AtomicU32::new(0)),
        Box::new(RealShell),
        AppIdentity::CURRENT.tray_window_class(),
    );
    let (hwnd, context) = match built {
        Ok(built) => built,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    // The first add happens before the app is told the tray exists, so `present()` is the
    // truth by the time `create` returns.
    refresh(hwnd, &context);
    let _ = ready.send(Ok(hwnd as isize));
    // SAFETY: a plain message loop for this thread's window; MSG is plain data filled by
    // GetMessageW; the loop ends with the WM_QUIT that WM_DESTROY posts.
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    destroy_last_icon(&context);
}

struct WindowsTray {
    window: isize,
    model: Arc<Mutex<TrayModel>>,
    present: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
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

    fn present(&self) -> bool {
        self.present.load(Ordering::SeqCst)
    }

    fn remove_for_quit(&self) {
        // SAFETY: posts to the tray thread's window; a window that is gone drops the message.
        unsafe { PostMessageW(self.window as HWND, REMOVE_FOR_QUIT, 0, 0) };
    }
}

impl Drop for WindowsTray {
    fn drop(&mut self) {
        // SAFETY: posts to the tray thread's window, which destroys itself on SHUTDOWN and so
        // ends its loop; a window that is gone drops the message.
        unsafe { PostMessageW(self.window as HWND, SHUTDOWN, 0, 0) };
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Starts the tray thread and waits for its first attempt to add the icon. `Ok` also when
/// that attempt failed: the tray keeps trying (a timer, `TaskbarCreated`) and
/// [`TrayIcon::present`] says the truth meanwhile.
pub(super) fn create(model: &TrayModel, on_event: TrayHandler) -> Result<Box<dyn TrayIcon>, PlatformError> {
    let model = Arc::new(Mutex::new(model.clone()));
    let present = Arc::new(AtomicBool::new(false));
    let (shared, shared_present) = (model.clone(), present.clone());
    let (ready, result) = mpsc::sync_channel(1);
    let worker = std::thread::Builder::new()
        .name("beebeeb-tray".into())
        .spawn(move || tray_thread(shared, on_event, shared_present, ready))
        .map_err(|e| PlatformError::Os(format!("starting the tray thread failed: {e}")))?;
    match result.recv() {
        Ok(Ok(window)) => Ok(Box::new(WindowsTray {
            window,
            model,
            present,
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

#[cfg(test)]
#[path = "windows_tray_tests.rs"]
mod tests;
