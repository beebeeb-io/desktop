//! Windows Hello as a gate: is it set up on this PC, and did the person at the keyboard just confirm it is them.
//!
//! Waiting on the core two-level lock (decision 2495): core owns the rules (the lock levels, the offline password
//! check, when a key is loaded and dropped). Nothing calls this module yet; no frame, the sign-in flow and the lock
//! do not use it until core says what a confirmed prompt opens.
//!
//! What it is NOT: no key, password or other secret passes through here, and a `Verified` unlocks nothing by
//! itself. The prompt is Windows' own (`UserConsentVerifier` through
//! `IUserConsentVerifierInterop::RequestVerificationForWindowAsync`), owned by the window that asks so it cannot
//! open behind it; its PIN fallback and its retry limit are Windows', never counted here. Nothing is logged:
//! not the reason line, not an OS error text.
//!
//! Threads: [`availability`] and [`verify`] wait for Windows, so they are for a worker thread, never the UI
//! thread. [`start`] is the one for UI code: read the [`HelloWindow`] on the UI thread, hand it over, and the
//! answer arrives on the prompt's own thread.
//!
//! This file names no other module of the crate, so `examples/hello_probe.rs` (the native rung) includes it as it
//! is. Off Windows everything answers "not here": `Unsupported` and `Unavailable`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Whether a Windows Hello prompt can be shown on this PC, for this person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelloAvailability {
    /// A prompt can be shown (it may still fail, for instance while the sensor is in use).
    Available,
    /// The PC can do it, but this person has set up no PIN, face or fingerprint.
    NotConfigured,
    /// No Windows Hello here: no verifier device, not Windows, or Windows gave no answer.
    Unsupported,
    /// Turned off by the organisation's policy.
    Disabled,
}

/// How one prompt ended. Only `Verified` means the person confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelloResult {
    Verified,
    /// The person closed the prompt, or the app took it away ([`HelloRequest::cancel`]).
    Cancelled,
    /// Not recognised after Windows' own retries, the sensor is in use, or Windows refused the request.
    Failed,
    /// No prompt was shown: Windows Hello is not set up, not present or disabled ([`availability`] says which).
    Unavailable,
}

/// Windows' numbers for `UserConsentVerifierAvailability` and `UserConsentVerificationResult`. On Windows the
/// build fails when one of them differs from the `windows` crate's (`imp`, below).
mod raw {
    pub const AVAILABLE: i32 = 0;
    pub const VERIFIED: i32 = 0;
    // The same three numbers in both enums.
    pub const DEVICE_NOT_PRESENT: i32 = 1;
    pub const NOT_CONFIGURED_FOR_USER: i32 = 2;
    pub const DISABLED_BY_POLICY: i32 = 3;
    pub const DEVICE_BUSY: i32 = 4;
    pub const RETRIES_EXHAUSTED: i32 = 5;
    pub const CANCELED: i32 = 6;
}

/// `None`: Windows gave no answer. A number Windows may add later is `Unsupported`, never `Available`.
fn availability_from_raw(answer: Option<i32>) -> HelloAvailability {
    match answer {
        // A busy sensor is set up: the prompt is offered, and ends `Failed` if the sensor stays busy.
        Some(raw::AVAILABLE | raw::DEVICE_BUSY) => HelloAvailability::Available,
        Some(raw::NOT_CONFIGURED_FOR_USER) => HelloAvailability::NotConfigured,
        Some(raw::DISABLED_BY_POLICY) => HelloAvailability::Disabled,
        Some(_) | None => HelloAvailability::Unsupported,
    }
}

/// One number is `Verified`; a number Windows may add later is `Failed`.
fn result_from_raw(value: i32) -> HelloResult {
    match value {
        raw::VERIFIED => HelloResult::Verified,
        raw::CANCELED => HelloResult::Cancelled,
        raw::DEVICE_NOT_PRESENT | raw::NOT_CONFIGURED_FOR_USER | raw::DISABLED_BY_POLICY => HelloResult::Unavailable,
        _ => HelloResult::Failed,
    }
}

/// What the platform half saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    /// No prompt was shown.
    NoPrompt,
    /// The prompt ended with this `UserConsentVerificationResult`.
    Ended(i32),
    /// Windows refused the request or the wait for it.
    Refused,
}

/// The answer the caller gets. Once the app asked for the prompt to go, the answer is `Cancelled`, also when the
/// person confirmed in the same instant: whoever asked has left.
fn settle(seen: Seen, cancel_asked: bool) -> HelloResult {
    if cancel_asked {
        return HelloResult::Cancelled;
    }
    match seen {
        Seen::NoPrompt => HelloResult::Unavailable,
        Seen::Ended(value) => result_from_raw(value),
        Seen::Refused => HelloResult::Failed,
    }
}

/// The top-level window that owns the prompt: a Win32 `HWND` as a number, so it can cross to the worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelloWindow(isize);

impl HelloWindow {
    /// `None` for the null handle.
    pub fn from_hwnd(hwnd: isize) -> Option<Self> {
        (hwnd != 0).then_some(Self(hwnd))
    }

    /// The Rune window's native handle; call this on the UI thread. `None` off Windows, and while the window has
    /// no native handle.
    pub fn of(window: &rune::WindowHandle) -> Option<Self> {
        #[cfg(windows)]
        {
            let hwnd = window.with_raw_handle(|handle| match handle.as_raw() {
                raw_window_handle::RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
                _ => None,
            });
            hwnd.ok().flatten().and_then(Self::from_hwnd)
        }
        #[cfg(not(windows))]
        {
            let _ = window;
            None
        }
    }
}

/// Shared by the prompt's thread and whoever may cancel it.
#[derive(Default)]
struct Control {
    cancel_asked: AtomicBool,
    prompt: Mutex<Option<imp::Prompt>>,
}

impl Control {
    fn cancel(&self) {
        // The flag first, the prompt second; the worker stores the prompt first and reads the flag second. So
        // whichever side comes later sees the other, and the prompt is asked to go by at least one of them.
        self.cancel_asked.store(true, Ordering::SeqCst);
        if let Some(prompt) = lock(&self.prompt).as_ref() {
            imp::dismiss(prompt);
        }
    }

    fn cancel_asked(&self) -> bool {
        self.cancel_asked.load(Ordering::SeqCst)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A prompt that is up, or about to be. Dropping this leaves the prompt alone.
pub struct HelloRequest {
    control: Arc<Control>,
}

impl HelloRequest {
    /// The person left the screen that asked: take the prompt away. The request then ends `Cancelled`, whatever
    /// the person did in the prompt; `on_done` still runs, once.
    pub fn cancel(&self) {
        self.control.cancel();
    }
}

/// Is Windows Hello set up here? Waits for Windows (briefly): call it from a worker thread.
pub fn availability() -> HelloAvailability {
    availability_from_raw(imp::availability())
}

/// Shows the prompt, owned by `window`, with `reason` as its line, and waits for the person. Blocks until the
/// prompt is gone: call it from a worker thread ([`start`] does that for UI code).
pub fn verify(window: HelloWindow, reason: &str) -> HelloResult {
    let control = Control::default();
    settle(imp::verify(window, reason, &control), false)
}

/// [`verify`] on a thread of its own. `on_done` runs exactly once, on that thread, never inside this call: post
/// the answer to the UI loop from it and do nothing else there.
pub fn start(window: HelloWindow, reason: &str, on_done: impl FnOnce(HelloResult) + Send + 'static) -> HelloRequest {
    let control = Arc::new(Control::default());
    let on_done: Arc<Mutex<Option<Box<dyn FnOnce(HelloResult) + Send>>>> =
        Arc::new(Mutex::new(Some(Box::new(on_done))));
    let worker = {
        let (control, on_done, reason) = (control.clone(), on_done.clone(), reason.to_owned());
        move || {
            let seen = imp::verify(window, &reason, &control);
            let result = settle(seen, control.cancel_asked());
            let on_done = lock(&on_done).take();
            if let Some(on_done) = on_done {
                on_done(result);
            }
        }
    };
    if std::thread::Builder::new()
        .name("windows-hello".into())
        .spawn(worker)
        .is_err()
    {
        // No thread, so no prompt. Still off this call's stack is not possible without a thread: answered here.
        let on_done = lock(&on_done).take();
        if let Some(on_done) = on_done {
            on_done(HelloResult::Failed);
        }
    }
    HelloRequest { control }
}

#[cfg(windows)]
mod imp {
    use windows::Foundation::IAsyncOperation;
    use windows::Security::Credentials::UI::{
        UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
    use windows::core::HSTRING;

    use super::{Control, HelloAvailability, HelloWindow, Seen, availability_from_raw, lock, raw};

    // The mapping above is written (and tested on every platform) against these numbers.
    const _: () = {
        assert!(UserConsentVerifierAvailability::Available.0 == raw::AVAILABLE);
        assert!(UserConsentVerifierAvailability::DeviceNotPresent.0 == raw::DEVICE_NOT_PRESENT);
        assert!(UserConsentVerifierAvailability::NotConfiguredForUser.0 == raw::NOT_CONFIGURED_FOR_USER);
        assert!(UserConsentVerifierAvailability::DisabledByPolicy.0 == raw::DISABLED_BY_POLICY);
        assert!(UserConsentVerifierAvailability::DeviceBusy.0 == raw::DEVICE_BUSY);
        assert!(UserConsentVerificationResult::Verified.0 == raw::VERIFIED);
        assert!(UserConsentVerificationResult::DeviceNotPresent.0 == raw::DEVICE_NOT_PRESENT);
        assert!(UserConsentVerificationResult::NotConfiguredForUser.0 == raw::NOT_CONFIGURED_FOR_USER);
        assert!(UserConsentVerificationResult::DisabledByPolicy.0 == raw::DISABLED_BY_POLICY);
        assert!(UserConsentVerificationResult::DeviceBusy.0 == raw::DEVICE_BUSY);
        assert!(UserConsentVerificationResult::RetriesExhausted.0 == raw::RETRIES_EXHAUSTED);
        assert!(UserConsentVerificationResult::Canceled.0 == raw::CANCELED);
    };

    pub(super) type Prompt = IAsyncOperation<UserConsentVerificationResult>;

    pub(super) fn availability() -> Option<i32> {
        let answer = UserConsentVerifier::CheckAvailabilityAsync().and_then(|operation| operation.get());
        answer.ok().map(|availability| availability.0)
    }

    pub(super) fn verify(window: HelloWindow, reason: &str, control: &Control) -> Seen {
        if availability_from_raw(availability()) != HelloAvailability::Available {
            return Seen::NoPrompt;
        }
        let Ok(interop) = windows::core::factory::<UserConsentVerifier, IUserConsentVerifierInterop>() else {
            return Seen::Refused;
        };
        // SAFETY: a COM call on a live interface. The handle is only a number to Windows: one that names no
        // window (it closed in the meantime) is refused with an error, and nothing is read through it here.
        let prompt: windows::core::Result<Prompt> =
            unsafe { interop.RequestVerificationForWindowAsync(HWND(window.0 as *mut _), &HSTRING::from(reason)) };
        let Ok(prompt) = prompt else { return Seen::Refused };
        *lock(&control.prompt) = Some(prompt.clone());
        if control.cancel_asked() {
            dismiss(&prompt);
        }
        let ended = prompt.get();
        *lock(&control.prompt) = None;
        match ended {
            Ok(result) => Seen::Ended(result.0),
            Err(_) => Seen::Refused,
        }
    }

    /// Asks Windows to end the prompt. Whether it leaves the screen at once is Windows' to decide; the request's
    /// answer is `Cancelled` either way.
    pub(super) fn dismiss(prompt: &Prompt) {
        let _ = prompt.Cancel();
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Control, HelloWindow, Seen};

    /// Never made off Windows.
    pub(super) enum Prompt {}

    pub(super) fn availability() -> Option<i32> {
        None
    }

    pub(super) fn verify(_window: HelloWindow, _reason: &str, _control: &Control) -> Seen {
        Seen::NoPrompt
    }

    pub(super) fn dismiss(prompt: &Prompt) {
        match *prompt {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AVAILABILITIES: [(i32, HelloAvailability); 5] = [
        (0, HelloAvailability::Available),
        (1, HelloAvailability::Unsupported),
        (2, HelloAvailability::NotConfigured),
        (3, HelloAvailability::Disabled),
        (4, HelloAvailability::Available),
    ];

    const RESULTS: [(i32, HelloResult); 7] = [
        (0, HelloResult::Verified),
        (1, HelloResult::Unavailable),
        (2, HelloResult::Unavailable),
        (3, HelloResult::Unavailable),
        (4, HelloResult::Failed),
        (5, HelloResult::Failed),
        (6, HelloResult::Cancelled),
    ];

    #[test]
    fn each_windows_availability_maps_to_its_own_answer() {
        let right = AVAILABILITIES
            .iter()
            .filter(|(value, want)| availability_from_raw(Some(*value)) == *want)
            .count();
        assert_eq!(right, 5);
    }

    #[test]
    fn no_answer_and_an_unknown_availability_are_never_available() {
        assert_eq!(availability_from_raw(None), HelloAvailability::Unsupported);
        let unknown = (-1000..=1000)
            .filter(|value| !(0..=4).contains(value))
            .chain([i32::MIN, i32::MAX]);
        let (mut checked, mut unsupported) = (0, 0);
        for value in unknown {
            checked += 1;
            unsupported += usize::from(availability_from_raw(Some(value)) == HelloAvailability::Unsupported);
        }
        assert_eq!((checked, unsupported), (1998, 1998));
    }

    #[test]
    fn each_windows_result_maps_to_its_own_answer() {
        let right = RESULTS
            .iter()
            .filter(|(value, want)| result_from_raw(*value) == *want)
            .count();
        assert_eq!(right, 7);
    }

    #[test]
    fn only_the_one_verified_number_verifies() {
        let values = (-1000..=1000).chain([i32::MIN, i32::MAX]);
        let verified: Vec<i32> = values
            .filter(|value| result_from_raw(*value) == HelloResult::Verified)
            .collect();
        assert_eq!(verified, [0]);
        // A number Windows adds later is a failure, not "no Hello here" and not a cancel.
        assert_eq!(result_from_raw(7), HelloResult::Failed);
        assert_eq!(result_from_raw(-1), HelloResult::Failed);
    }

    #[test]
    fn what_the_platform_saw_settles_into_the_answer() {
        assert_eq!(settle(Seen::NoPrompt, false), HelloResult::Unavailable);
        assert_eq!(settle(Seen::Refused, false), HelloResult::Failed);
        let passed_on = RESULTS
            .iter()
            .filter(|(value, want)| settle(Seen::Ended(*value), false) == *want)
            .count();
        assert_eq!(passed_on, 7);
    }

    #[test]
    fn a_cancelled_request_is_cancelled_whatever_the_prompt_said() {
        let seen = RESULTS
            .iter()
            .map(|(value, _)| Seen::Ended(*value))
            .chain([Seen::NoPrompt, Seen::Refused]);
        let (mut checked, mut cancelled) = (0, 0);
        for seen in seen {
            checked += 1;
            cancelled += usize::from(settle(seen, true) == HelloResult::Cancelled);
        }
        assert_eq!((checked, cancelled), (9, 9));
    }

    #[test]
    fn the_null_handle_is_no_window() {
        assert_eq!(HelloWindow::from_hwnd(0), None);
        assert_eq!(HelloWindow::from_hwnd(0x1234), Some(HelloWindow(0x1234)));
    }

    #[test]
    fn cancel_sets_the_flag_the_worker_reads() {
        let control = Control::default();
        assert!(!control.cancel_asked());
        control.cancel();
        assert!(control.cancel_asked());
    }

    // Off Windows there is nothing to show: the answers below are the stub's. On Windows these would raise a
    // real prompt, which is the native rung's job (`examples/hello_probe.rs`).
    #[cfg(not(windows))]
    mod off_windows {
        use std::sync::mpsc;
        use std::time::Duration;

        use super::super::*;

        #[test]
        fn there_is_no_windows_hello() {
            assert_eq!(availability(), HelloAvailability::Unsupported);
            let window = HelloWindow::from_hwnd(1).unwrap();
            assert_eq!(verify(window, "Unlock Beebeeb"), HelloResult::Unavailable);
        }

        #[test]
        fn start_answers_once_on_its_own_thread() {
            let (tx, rx) = mpsc::channel();
            let caller = std::thread::current().id();
            let window = HelloWindow::from_hwnd(1).unwrap();
            let _request = start(window, "Unlock Beebeeb", move |result| {
                let _ = tx.send((result, std::thread::current().id()));
            });
            let (result, thread) = rx.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(result, HelloResult::Unavailable);
            assert_ne!(thread, caller);
            // The sender went with the one call: a second answer cannot come.
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(10)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            );
        }

        #[test]
        fn a_request_cancelled_before_its_answer_ends_cancelled() {
            let control = Control::default();
            control.cancel();
            let seen = imp::verify(HelloWindow::from_hwnd(1).unwrap(), "Unlock Beebeeb", &control);
            assert_eq!(settle(seen, control.cancel_asked()), HelloResult::Cancelled);
        }
    }
}
