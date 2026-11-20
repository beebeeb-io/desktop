//! Windows: the hook that takes Unicode packets' characters off the window thread's message queue
//! (`packet_text.rs` has the rule and why). Compiled only for Windows.
//!
//! A `WH_GETMESSAGE` hook of the window's own thread: it sees each message as the event loop takes it off the
//! queue, before winit does. It reads key-downs (to know whether a packet went down) and turns a packet's
//! `WM_CHAR` into `WM_NULL`, so winit never sees a character it would report without a key; the character goes
//! on [`TAKEN`] and the poll is woken. Only messages of the main window are touched; a peek that leaves the
//! message on the queue is not acted on (it would be counted twice).

use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GA_ROOT, GetAncestor, MSG, PM_REMOVE, SetWindowsHookExW, WH_GETMESSAGE, WM_CHAR, WM_KEYDOWN,
    WM_NULL, WM_SYSKEYDOWN,
};

use super::PacketWake;
use super::packet_text::{KeyMessage, Packets, Verdict};

/// The main window (0: not watched yet).
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
/// The text taken since the poll last asked.
static TAKEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// What wakes the poll.
static WAKE: Mutex<Option<PacketWake>> = Mutex::new(None);

thread_local! {
    /// The window thread's rule state: the hook only ever runs on that thread.
    static PACKETS: RefCell<Packets> = RefCell::new(Packets::default());
}

fn unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Starts the hook on the calling thread (the window's) for `window`. True: watching (also when it already was).
pub(super) fn watch(window: Option<&rune::WindowHandle>, wake: PacketWake) -> bool {
    if MAIN_HWND.load(Ordering::SeqCst) != 0 {
        return true;
    }
    let hwnd = window.and_then(|window| {
        window
            .with_raw_handle(|handle| match handle.as_raw() {
                raw_window_handle::RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
                _ => None,
            })
            .ok()
            .flatten()
    });
    let Some(hwnd) = hwnd else { return false };
    *unpoisoned(&WAKE) = Some(wake);
    // SAFETY: a thread hook of the calling thread with a `extern "system"` procedure that lives as long as the
    // process; no module handle is needed for a hook of the caller's own thread. It is never removed: the thread
    // is the window's and ends with the process.
    let hook = unsafe { SetWindowsHookExW(WH_GETMESSAGE, Some(on_message), std::ptr::null_mut(), GetCurrentThreadId()) };
    if hook.is_null() {
        return false;
    }
    MAIN_HWND.store(hwnd, Ordering::SeqCst);
    true
}

/// The text taken since the last call.
pub(super) fn take() -> Vec<String> {
    std::mem::take(&mut *unpoisoned(&TAKEN))
}

/// The hook: `wparam` says whether the message is being removed, `lparam` points at it.
unsafe extern "system" fn on_message(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam == PM_REMOVE as WPARAM && lparam != 0 {
        // SAFETY: for `WH_GETMESSAGE` with a code of 0 or more, `lparam` is the address of the `MSG` being
        // retrieved, valid and writable for this call (the hook may change it).
        let message = unsafe { &mut *(lparam as *mut MSG) };
        let key = match message.message {
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(KeyMessage::Down(message.wParam as u16)),
            WM_CHAR => Some(KeyMessage::Char(message.wParam as u16)),
            _ => None,
        };
        if let Some(key) = key {
            if is_main(message.hwnd) {
                let verdict = PACKETS.with(|packets| packets.borrow_mut().on(key));
                if let Verdict::Take(text) = verdict {
                    message.message = WM_NULL;
                    if let Some(text) = text {
                        unpoisoned(&TAKEN).push(text);
                        let wake = unpoisoned(&WAKE).clone();
                        if let Some(wake) = wake {
                            wake();
                        }
                    }
                }
            }
        }
    }
    // SAFETY: the next hook in the chain gets what this one got (the first argument is ignored by Windows).
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// The message is the main window's (or a child's of it).
fn is_main(hwnd: HWND) -> bool {
    let main = MAIN_HWND.load(Ordering::SeqCst);
    if main == 0 || hwnd.is_null() {
        return false;
    }
    // SAFETY: `GetAncestor` reads a window handle of this thread's message; a stale one answers null.
    hwnd as isize == main || unsafe { GetAncestor(hwnd, GA_ROOT) } as isize == main
}
