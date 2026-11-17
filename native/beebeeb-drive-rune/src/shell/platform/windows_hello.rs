//! Windows Hello as a gate: is it set up on this PC, and did the person at the keyboard just confirm it is them.
//!
//! Core owns the rules (decision 2495: the lock levels, the offline password check, when a key is loaded and
//! dropped). The app reaches this module through one seam, `engine_host::hello` (`WindowHello`): the lock screen,
//! app lock on Sign-in and security, and the first run's "Lock Beebeeb" ask it; what a confirmed prompt opens is
//! core's answer, asked through the engine.
//!
//! One prompt at a time, app-wide: a [`start`] while another prompt is up takes that one away, and its owner hears
//! `Cancelled`. The owner's own [`HelloRequest::cancel`] is the cancel contract of `engine_host::hello::HelloCancel`
//! (written there, once): after it returns, no answer of that prompt is decided any more; one decided before it may
//! still be on its way, and the owner drops that one by its own check.
//!
//! What it is NOT: no key, password or other secret passes through here, and a `Verified` unlocks nothing by
//! itself. The prompt is Windows' own (`UserConsentVerifier` through
//! `IUserConsentVerifierInterop::RequestVerificationForWindowAsync`), owned by the window that asks; its PIN
//! fallback and its retry limit are Windows', never counted here. Bringing that window to the front first is the
//! caller's: this module moves no window. Nothing is logged: not the reason line, not an OS error text.
//!
//! Threads: [`availability`] and [`verify`] wait for Windows, so they are for a worker thread, never the UI
//! thread. [`start`] is the one for UI code: read the [`HelloWindow`] on the UI thread, hand it over, and the
//! answer arrives on the prompt's own thread.
//!
//! This file names no other module of the crate, so `examples/hello_probe.rs` (the native rung) includes it as it
//! is. Off Windows everything answers "not here": `Unsupported` and `Unavailable`.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

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
    /// The person closed the prompt, or a newer prompt took its place ([`start`]). The owner's own
    /// [`HelloRequest::cancel`] is answered with nothing at all.
    Cancelled,
    /// Not recognised after Windows' own retries, the sensor is in use, or Windows refused the request.
    Failed,
    /// No prompt was shown: Windows Hello is not set up, not present or disabled ([`availability`] says which).
    Unavailable,
}

/// Windows' numbers for `UserConsentVerifierAvailability`, `UserConsentVerificationResult` and the prompt
/// operation's `AsyncStatus`. On Windows the build fails when one of them differs from the `windows` crate's
/// (`imp`, below).
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
    // `AsyncStatus`: how the prompt's operation ended (a different enum from the result's `CANCELED` above).
    pub const STATUS_STARTED: i32 = 0;
    pub const STATUS_COMPLETED: i32 = 1;
    pub const STATUS_CANCELED: i32 = 2;
    pub const STATUS_ERROR: i32 = 3;
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

/// How the prompt ended, from what its operation reported: Windows' `AsyncStatus`, and the
/// `UserConsentVerificationResult` read from it, which is read only when the status is `Completed` (`None`: not
/// read, or it could not be). `Verified` is the result's zero value, so the status decides first: an operation that
/// did not complete (`Canceled`, `Error`, still `Started`, or a status Windows may add later) is never `Verified`,
/// whatever number its result holds. This is the one place a prompt's `Verified` comes from.
fn outcome_of(status: i32, result: Option<i32>) -> HelloResult {
    match status {
        raw::STATUS_COMPLETED => result.map_or(HelloResult::Failed, result_from_raw),
        raw::STATUS_CANCELED => HelloResult::Cancelled,
        _ => HelloResult::Failed,
    }
}

/// What the platform half saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    /// No prompt was shown.
    NoPrompt,
    /// The prompt's operation ended with this `AsyncStatus` and, when it completed, this result ([`outcome_of`]).
    Ended { status: i32, result: Option<i32> },
    /// Windows refused the request or the wait for it.
    Refused,
}

/// The answer the caller gets. A prompt that was taken away (a newer one took its place) answers `Cancelled`, also
/// when the person confirmed in the same instant: the screen that asked has a newer prompt now.
fn settle(seen: Seen, taken_away: bool) -> HelloResult {
    if taken_away {
        return HelloResult::Cancelled;
    }
    match seen {
        Seen::NoPrompt => HelloResult::Unavailable,
        Seen::Ended { status, result } => outcome_of(status, result),
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

/// Where one request stands; shared by the prompt's thread and whoever may cancel it.
#[derive(Default)]
struct Progress {
    /// The owner took the prompt away ([`HelloRequest::cancel`]): no answer is handed out from then on.
    cancel_asked: bool,
    /// A newer prompt took this one's place ([`start`]): it goes, and its owner is answered `Cancelled`.
    superseded: bool,
    /// The prompt's operation ended, with this `AsyncStatus`.
    ended: Option<i32>,
    /// The prompt's thread is done with Windows: no prompt was shown, its answer is in, or Windows was asked to close
    /// it (the `Cancel` of a taken-away prompt has returned). [`take_away_for_quit`] waits for this.
    finished: bool,
}

#[derive(Default)]
struct Control {
    progress: Mutex<Progress>,
    changed: Condvar,
}

/// Why the prompt's thread stopped waiting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Woken {
    /// The operation ended, with this `AsyncStatus`.
    PromptEnded(i32),
    /// The owner cancelled, or a newer prompt took this one's place.
    TakenAway,
}

impl Control {
    fn cancel(&self) {
        lock(&self.progress).cancel_asked = true;
        self.changed.notify_all();
    }

    fn supersede(&self) {
        lock(&self.progress).superseded = true;
        self.changed.notify_all();
    }

    /// The completion handler's call: the operation ended with `status` (its `AsyncStatus`).
    fn prompt_ended(&self, status: i32) {
        lock(&self.progress).ended = Some(status);
        self.changed.notify_all();
    }

    fn taken_away(&self) -> bool {
        let progress = lock(&self.progress);
        progress.cancel_asked || progress.superseded
    }

    /// The prompt's thread is done with Windows (its `verify` returned).
    fn finished(&self) {
        lock(&self.progress).finished = true;
        self.changed.notify_all();
    }

    /// Waits until the prompt's thread is done with Windows, at most `limit`. `true`: it is.
    fn wait_finished(&self, limit: Duration) -> bool {
        let until = Instant::now() + limit;
        let mut progress = lock(&self.progress);
        while !progress.finished {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            progress = self
                .changed
                .wait_timeout(progress, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        true
    }

    /// Waits for the prompt to end or to be taken away, whichever is first; taken away wins when both are in. The
    /// wait does not depend on Windows taking the prompt away: a cancel ends it by itself.
    fn wait(&self) -> Woken {
        let mut progress = lock(&self.progress);
        loop {
            if progress.cancel_asked || progress.superseded {
                return Woken::TakenAway;
            }
            if let Some(status) = progress.ended {
                return Woken::PromptEnded(status);
            }
            progress = self
                .changed
                .wait(progress)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// Decides the answer under the lock, then hands it over with the lock released: `on_done` may call
    /// [`HelloRequest::cancel`], which takes the same lock. An owner's cancel that is in before the decision means
    /// no answer at all: `on_done` is dropped unrun (also outside the lock). A cancel after the decision finds the
    /// answer handed over already; the owner's own turn check drops it.
    fn answer(&self, seen: Seen, on_done: impl FnOnce(HelloResult)) {
        let result = {
            let progress = lock(&self.progress);
            (!progress.cancel_asked).then(|| settle(seen, progress.superseded))
        };
        // `None`: `on_done` is dropped unrun when this returns, with the lock already released.
        if let Some(result) = result {
            on_done(result);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The one prompt that is up, app-wide.
struct OneUp {
    up: Mutex<Option<Arc<Control>>>,
}

impl OneUp {
    const fn new() -> Self {
        Self { up: Mutex::new(None) }
    }

    /// `control` is the prompt that is up from now on. The one it replaces (still up, or about to be) is taken
    /// away, and its owner, who did not cancel it, is answered `Cancelled`.
    fn take_place(&self, control: &Arc<Control>) {
        let previous = lock(&self.up).replace(control.clone());
        if let Some(previous) = previous {
            previous.supersede();
        }
    }

    /// The app is quitting: the prompt that is up (whoever asked for it) is taken away and its owner is answered
    /// `Cancelled` (never a yes), and this waits, at most `limit`, until its thread asked Windows to close it.
    /// `true`: no prompt is up, or its thread is done with Windows.
    fn take_away_and_wait(&self, limit: Duration) -> bool {
        let up = lock(&self.up).clone();
        let Some(control) = up else { return true };
        // Not the owner's silent cancel: the owner is answered `Cancelled` (never a yes), so an owner that waits
        // (app lock off, approve) is not left waiting when Windows cancels the shutdown.
        control.supersede();
        control.wait_finished(limit)
    }

    /// `control`'s prompt is over. A newer one that took its place stays.
    fn leave(&self, control: &Arc<Control>) {
        let mut up = lock(&self.up);
        if up.as_ref().is_some_and(|current| Arc::ptr_eq(current, control)) {
            *up = None;
        }
    }
}

static UP: OneUp = OneUp::new();

#[cfg(test)]
thread_local! {
    /// Whether the quit on this test thread reaches [`UP`].
    static QUIT_ON_GLOBAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A prompt that is up, or about to be. Dropping this leaves the prompt alone (a newer [`start`] still takes it
/// away).
pub struct HelloRequest {
    control: Arc<Control>,
}

impl HelloRequest {
    /// The person left the screen that asked: take the prompt away. This is the cancel contract of
    /// `engine_host::hello::HelloCancel`: once this returns, no answer of this prompt is decided any more. `on_done`
    /// has either been handed its answer already (decided before this call, it may still be running: the owner's
    /// own turn check drops it) or it never runs and is dropped, whatever the person does in the prompt from here
    /// on. Never blocks: Windows is asked to close the prompt from the prompt's thread, not from this call. Safe to
    /// call from inside `on_done`, and on a prompt that is over (it then changes nothing).
    pub fn cancel(&self) {
        self.control.cancel();
    }
}

/// The app is quitting (`shell::host::on_quit`): the prompt that is up, whoever asked for it, is taken away, and this
/// waits at most `limit` for its thread to ask Windows to close it. A prompt whose owner is gone stays on screen with
/// nobody to answer, so the quit asks Windows first. Whether and when the prompt leaves the screen is Windows' (the
/// native rung checks it). `true`: no prompt was up, or its thread is done with Windows within `limit`.
pub fn take_away_for_quit(limit: Duration) -> bool {
    // A test thread that runs the quit hook (`host::on_quit`) leaves the process-wide slot, shared with other
    // tests, alone unless it asks for it.
    #[cfg(test)]
    if !QUIT_ON_GLOBAL.with(std::cell::Cell::get) {
        return true;
    }
    UP.take_away_and_wait(limit)
}

/// Is Windows Hello set up here? Waits for Windows (briefly): call it from a worker thread.
pub fn availability() -> HelloAvailability {
    availability_from_raw(imp::availability())
}

/// Shows the prompt, owned by `window`, with `reason` as its line, and waits for the person. Blocks until the
/// prompt is gone: call it from a worker thread ([`start`] does that for UI code).
pub fn verify(window: HelloWindow, reason: &str) -> HelloResult {
    settle(imp::verify(window, reason, &Arc::new(Control::default())), false)
}

/// [`verify`] on a thread of its own. `on_done` runs at most once, on that thread, never inside this call: exactly
/// once, unless [`HelloRequest::cancel`] returned first. Post the answer to the UI loop from it and do nothing else
/// there. A prompt that is up for another screen is taken away first; its owner is answered `Cancelled`. `None`: no
/// thread could be started, so there is no prompt and `on_done` is dropped without running; treat it as
/// [`HelloResult::Failed`].
pub fn start(
    window: HelloWindow,
    reason: &str,
    on_done: impl FnOnce(HelloResult) + Send + 'static,
) -> Option<HelloRequest> {
    let control = Arc::new(Control::default());
    UP.take_place(&control);
    let worker = {
        let (control, reason) = (control.clone(), reason.to_owned());
        move || {
            let seen = imp::verify(window, &reason, &control);
            control.finished();
            UP.leave(&control);
            control.answer(seen, on_done);
        }
    };
    if std::thread::Builder::new()
        .name("windows-hello".into())
        .spawn(worker)
        .is_err()
    {
        UP.leave(&control);
        return None;
    }
    Some(HelloRequest { control })
}

#[cfg(windows)]
mod imp {
    use std::sync::Arc;

    use windows::Foundation::{AsyncOperationCompletedHandler, AsyncStatus, IAsyncOperation};
    use windows::Security::Credentials::UI::{
        UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
    use windows::core::HSTRING;

    use super::{Control, HelloAvailability, HelloWindow, Seen, Woken, availability_from_raw, raw};

    // The mapping above is written (and tested on every platform) against these numbers.
    const _: () = {
        assert!(AsyncStatus::Started.0 == raw::STATUS_STARTED);
        assert!(AsyncStatus::Completed.0 == raw::STATUS_COMPLETED);
        assert!(AsyncStatus::Canceled.0 == raw::STATUS_CANCELED);
        assert!(AsyncStatus::Error.0 == raw::STATUS_ERROR);
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

    pub(super) fn availability() -> Option<i32> {
        let answer = UserConsentVerifier::CheckAvailabilityAsync().and_then(|operation| operation.get());
        answer.ok().map(|availability| availability.0)
    }

    pub(super) fn verify(window: HelloWindow, reason: &str, control: &Arc<Control>) -> Seen {
        // Only a definite "not here" stops before the prompt. When the question itself failed, the request
        // below is the judge: it names a missing or disabled Windows Hello in its own result.
        if availability().is_some_and(|answer| availability_from_raw(Some(answer)) != HelloAvailability::Available) {
            return Seen::NoPrompt;
        }
        // Whoever asked has already left, or a newer prompt took this one's place: no prompt for them. (The owner's
        // cancel hands out no answer; a newer prompt makes this one `Cancelled`.)
        if control.taken_away() {
            return Seen::Refused;
        }
        let Ok(interop) = windows::core::factory::<UserConsentVerifier, IUserConsentVerifierInterop>() else {
            return Seen::Refused;
        };
        // The Credential Dialogs that are already on screen (other prompts, older runs): never ours to close.
        let before = dialogs::all();
        // SAFETY: a COM call on a live interface. The handle is only a number to Windows: one that names no
        // window (it closed in the meantime) is refused with an error, and nothing is read through it here.
        let prompt: windows::core::Result<IAsyncOperation<UserConsentVerificationResult>> =
            unsafe { interop.RequestVerificationForWindowAsync(HWND(window.0 as *mut _), &HSTRING::from(reason)) };
        let Ok(prompt) = prompt else { return Seen::Refused };
        // The handler keeps the operation's status: it decides whether there is a result at all (`outcome_of`).
        let ended = control.clone();
        let on_end = AsyncOperationCompletedHandler::<UserConsentVerificationResult>::new(move |_, status| {
            ended.prompt_ended(status.0);
            Ok(())
        });
        if prompt.SetCompleted(&on_end).is_err() {
            // Nothing would say when it ends: ask Windows to take it away rather than wait for ever.
            let _ = prompt.Cancel();
            dialogs::close_ours(window.0, &before);
            return Seen::Refused;
        }
        match control.wait() {
            Woken::PromptEnded(status) => {
                // Only a completed operation has a result to read. `Verified` is the result's zero value, so
                // nothing is read from one that was cancelled or failed.
                let result = if status == raw::STATUS_COMPLETED {
                    prompt.GetResults().ok().map(|result| result.0)
                } else {
                    None
                };
                Seen::Ended { status, result }
            }
            Woken::TakenAway => {
                // Asks Windows to close the prompt, from this thread. Whether and when it leaves the screen is
                // Windows' to decide; the answer does not wait for it: none for an owner's cancel, `Cancelled`
                // for a prompt a newer one took the place of.
                let _ = prompt.Cancel();
                // `Cancel` on the operation does not remove the "Windows Security" dialog (CredentialUIBroker keeps
                // it on screen, measured on VM 401): close its window itself.
                dialogs::close_ours(window.0, &before);
                Seen::Refused
            }
        }
    }

    /// The "Windows Security" dialog Windows Hello shows lives in CredentialUIBroker.exe, not in this process:
    /// closing the request does not close it, so it is found by window class and sent `WM_CLOSE`.
    mod dialogs {
        use std::time::{Duration, Instant};

        use windows_sys::Win32::Foundation::{HWND, LPARAM, TRUE};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GW_OWNER, GetClassNameW, GetWindow, IsWindowVisible, PostMessageW, WM_CLOSE,
        };

        const CLASS: &str = "Credential Dialog Xaml Host";
        /// The dialog may only appear after the cancel (the request was just made); look this long, no longer.
        const LOOK: Duration = Duration::from_millis(1500);

        fn is_dialog(hwnd: HWND) -> bool {
            let mut name = [0u16; 64];
            // SAFETY: `name` is a live buffer of the length given.
            let n = unsafe { GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32) };
            n > 0 && String::from_utf16_lossy(&name[..n as usize]) == CLASS
        }

        unsafe extern "system" fn collect(hwnd: HWND, out: LPARAM) -> i32 {
            if is_dialog(hwnd) {
                // SAFETY: `out` is the `Vec` `all` passed, alive for the whole `EnumWindows` call.
                unsafe { (*(out as *mut Vec<isize>)).push(hwnd as isize) };
            }
            TRUE
        }

        /// Every Credential Dialog window on the desktop now.
        pub(super) fn all() -> Vec<isize> {
            let mut found: Vec<isize> = Vec::new();
            // SAFETY: the callback only reads window classes and pushes into `found`, which outlives the call.
            unsafe { EnumWindows(Some(collect), &mut found as *mut Vec<isize> as LPARAM) };
            found
        }

        /// Closes the dialog(s) of this request: owned by `owner` (our window), or new since `before`. Keeps
        /// looking for a short while, as the dialog may still be coming up when the cancel is made.
        pub(super) fn close_ours(owner: isize, before: &[isize]) {
            let until = Instant::now() + LOOK;
            let mut closed = false;
            loop {
                for h in all() {
                    // SAFETY: plain window queries on a handle that may have gone: Windows answers with an error.
                    let ours = unsafe { GetWindow(h as HWND, GW_OWNER) } as isize == owner || !before.contains(&h);
                    if ours && unsafe { IsWindowVisible(h as HWND) } != 0 {
                        // SAFETY: posting a message to a window of another process is allowed and non-blocking.
                        unsafe { PostMessageW(h as HWND, WM_CLOSE, 0, 0) };
                        closed = true;
                    }
                }
                if closed || Instant::now() >= until {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::sync::Arc;

    use super::{Control, HelloWindow, Seen};

    pub(super) fn availability() -> Option<i32> {
        None
    }

    pub(super) fn verify(_window: HelloWindow, _reason: &str, _control: &Arc<Control>) -> Seen {
        Seen::NoPrompt
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

    /// A prompt whose operation completed, with this result.
    fn completed(value: i32) -> Seen {
        Seen::Ended {
            status: raw::STATUS_COMPLETED,
            result: Some(value),
        }
    }

    #[test]
    fn what_the_platform_saw_settles_into_the_answer() {
        assert_eq!(settle(Seen::NoPrompt, false), HelloResult::Unavailable);
        assert_eq!(settle(Seen::Refused, false), HelloResult::Failed);
        let passed_on = RESULTS
            .iter()
            .filter(|(value, want)| settle(completed(*value), false) == *want)
            .count();
        assert_eq!(passed_on, 7);
    }

    /// L271: `Verified` is the result's zero value, so an operation that did not complete must never map to it.
    #[test]
    fn non_completed_status_is_never_verified() {
        // Every status but `Completed` (the named three among them), with the zero value first, no result, and
        // every other number Windows names.
        let statuses: Vec<i32> = (-1000..=1000)
            .filter(|status| *status != raw::STATUS_COMPLETED)
            .chain([i32::MIN, i32::MAX])
            .collect();
        let results: Vec<Option<i32>> = [Some(raw::VERIFIED), None]
            .into_iter()
            .chain((1..=7).map(Some))
            .collect();
        let (mut checked, mut verified) = (0, 0);
        for status in &statuses {
            for result in &results {
                checked += 1;
                verified += usize::from(outcome_of(*status, *result) == HelloResult::Verified);
                verified += usize::from(
                    settle(
                        Seen::Ended {
                            status: *status,
                            result: *result,
                        },
                        false,
                    ) == HelloResult::Verified,
                );
            }
        }
        assert_eq!((checked, verified), (2002 * 9, 0));
        // What each named status says: Windows took the prompt away, it failed, or it never ended.
        assert_eq!(
            outcome_of(raw::STATUS_CANCELED, Some(raw::VERIFIED)),
            HelloResult::Cancelled
        );
        assert_eq!(outcome_of(raw::STATUS_ERROR, Some(raw::VERIFIED)), HelloResult::Failed);
        assert_eq!(
            outcome_of(raw::STATUS_STARTED, Some(raw::VERIFIED)),
            HelloResult::Failed
        );
        // Completed: the result decides, and a result that could not be read is a failure, never a yes.
        assert_eq!(
            outcome_of(raw::STATUS_COMPLETED, Some(raw::VERIFIED)),
            HelloResult::Verified
        );
        assert_eq!(outcome_of(raw::STATUS_COMPLETED, None), HelloResult::Failed);
    }

    #[test]
    fn a_superseded_request_is_cancelled_whatever_the_prompt_said() {
        let seen = RESULTS
            .iter()
            .map(|(value, _)| completed(*value))
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
    fn the_wait_ends_on_a_cancel_or_on_the_prompt_and_a_cancel_wins() {
        let waited = |act: fn(&Control)| {
            let control = Arc::new(Control::default());
            let waiter = std::thread::spawn({
                let control = control.clone();
                move || control.wait()
            });
            act(&control);
            waiter.join().unwrap()
        };
        assert_eq!(waited(|control| control.cancel()), Woken::TakenAway);
        assert_eq!(waited(|control| control.supersede()), Woken::TakenAway);
        assert_eq!(
            waited(|control| control.prompt_ended(raw::STATUS_COMPLETED)),
            Woken::PromptEnded(raw::STATUS_COMPLETED)
        );
        assert_eq!(
            waited(|control| control.prompt_ended(raw::STATUS_CANCELED)),
            Woken::PromptEnded(raw::STATUS_CANCELED)
        );
        let both = Control::default();
        both.prompt_ended(raw::STATUS_COMPLETED);
        both.cancel();
        assert_eq!(both.wait(), Woken::TakenAway);
    }

    /// L274: after the owner's cancel no answer is handed out; a prompt a newer one replaced is told `Cancelled`.
    #[test]
    fn the_answer_is_decided_with_the_cancel_that_is_in() {
        let answered = |control: &Control, seen| {
            let mut got = None;
            control.answer(seen, |result| got = Some(result));
            got
        };
        let control = Control::default();
        assert_eq!(answered(&control, completed(0)), Some(HelloResult::Verified));
        control.cancel();
        assert_eq!(
            answered(&control, completed(0)),
            None,
            "an answer after the owner's cancel"
        );
        let superseded = Control::default();
        superseded.supersede();
        assert_eq!(answered(&superseded, completed(0)), Some(HelloResult::Cancelled));
        // A superseded prompt whose owner then cancelled too: nothing.
        superseded.cancel();
        assert_eq!(answered(&superseded, completed(0)), None);
    }

    /// L272: `on_done` runs with the request's lock released, so a cancel from inside it (a screen that closes on the
    /// answer) returns. Run on a thread of its own and bounded: a relock would hang that thread, not the test binary.
    #[test]
    fn cancel_while_on_done_runs_returns() {
        let control = Arc::new(Control::default());
        let request = HelloRequest {
            control: control.clone(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            control.answer(completed(raw::VERIFIED), move |result| {
                request.cancel();
                let _ = tx.send(result);
            });
        });
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)),
            Ok(HelloResult::Verified),
            "on_done ran under the lock that cancel() takes"
        );
    }

    /// L273: one prompt at a time. A newer prompt takes the place of the one that is up: that one's wait ends, its
    /// owner (who did not cancel) is answered `Cancelled` even on a yes, and its end does not free the newer one's
    /// place.
    #[test]
    fn a_newer_prompt_takes_the_place_of_the_one_that_is_up() {
        let one_up = OneUp::new();
        let (first, second) = (Arc::new(Control::default()), Arc::new(Control::default()));
        one_up.take_place(&first);
        assert!(!first.taken_away(), "the first prompt replaces nothing");
        one_up.take_place(&second);
        assert!(first.taken_away());
        assert!(!second.taken_away());
        assert_eq!(first.wait(), Woken::TakenAway);
        let mut got = None;
        first.answer(completed(raw::VERIFIED), |result| got = Some(result));
        assert_eq!(got, Some(HelloResult::Cancelled));

        one_up.leave(&first);
        assert!(lock(&one_up.up).as_ref().is_some_and(|up| Arc::ptr_eq(up, &second)));
        one_up.leave(&second);
        assert!(lock(&one_up.up).is_none());
        // Once the slot is free, a new prompt takes the place of nothing.
        let third = Arc::new(Control::default());
        one_up.take_place(&third);
        assert!(!second.taken_away());
    }

    /// The app quits while a prompt is up (whoever asked for it): the prompt is taken away, its owner hears nothing of
    /// it, also of a yes, and the quit returns only once the prompt's thread asked Windows to close it. A prompt whose
    /// owner is gone stays on screen otherwise, with nobody to answer.
    #[test]
    fn a_quit_takes_the_prompt_away_and_waits_until_its_thread_asked_windows_to_close_it() {
        use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
        let one_up = OneUp::new();
        assert!(
            one_up.take_away_and_wait(Duration::from_millis(1)),
            "no prompt up: nothing to wait for"
        );
        let control = Arc::new(Control::default());
        one_up.take_place(&control);
        // The prompt's thread as `start` runs it: it waits on the prompt; taken away, it asks Windows to close it
        // (here: a while), then it is done with Windows.
        let asked_to_close = Arc::new(AtomicBool::new(false));
        let thread = std::thread::spawn({
            let (control, asked) = (control.clone(), asked_to_close.clone());
            move || {
                assert_eq!(control.wait(), Woken::TakenAway);
                std::thread::sleep(Duration::from_millis(50));
                asked.store(true, SeqCst);
                control.finished();
            }
        });
        assert!(
            one_up.take_away_and_wait(Duration::from_secs(10)),
            "the prompt's thread was woken and finished"
        );
        assert!(
            asked_to_close.load(SeqCst),
            "the quit returned only after the thread asked Windows to close the prompt"
        );
        let mut got = None;
        control.answer(completed(raw::VERIFIED), |result| got = Some(result));
        assert_eq!(
            got,
            Some(HelloResult::Cancelled),
            "its owner is answered Cancelled, also of a yes: it is never left waiting"
        );
        thread.join().unwrap();
    }

    /// Tests that put a control in the process-wide slot ([`UP`]) one at a time.
    #[cfg(not(windows))]
    static GLOBAL_UP: Mutex<()> = Mutex::new(());

    /// The process-wide take-away with a prompt up (off Windows `start` ends at once, so the control is placed by
    /// hand): the quit answers its owner `Cancelled` (a shutdown that is cancelled leaves nobody waiting), never a
    /// yes, and the slot is free after.
    #[cfg(not(windows))]
    #[test]
    fn the_quit_on_the_global_slot_answers_the_owner_cancelled_and_never_a_yes() {
        let _one = lock(&GLOBAL_UP);
        QUIT_ON_GLOBAL.with(|on| on.set(true));
        let control = Arc::new(Control::default());
        UP.take_place(&control);
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn({
            let control = control.clone();
            move || {
                assert_eq!(control.wait(), Woken::TakenAway);
                control.finished();
                UP.leave(&control);
                // Windows said yes in the same instant.
                control.answer(completed(raw::VERIFIED), |result| {
                    let _ = tx.send(result);
                });
            }
        });
        assert!(take_away_for_quit(Duration::from_secs(10)));
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(HelloResult::Cancelled));
        thread.join().unwrap();
        assert!(lock(&UP.up).is_none());
        QUIT_ON_GLOBAL.with(|on| on.set(false));
    }

    /// A prompt's thread that never gets done (Windows does not answer) holds the quit no longer than its limit.
    #[test]
    fn a_quit_waits_no_longer_than_its_limit() {
        let one_up = OneUp::new();
        let control = Arc::new(Control::default());
        one_up.take_place(&control);
        let began = Instant::now();
        assert!(!one_up.take_away_and_wait(Duration::from_millis(100)));
        assert!(began.elapsed() < Duration::from_secs(5));
        assert!(control.taken_away(), "taken away all the same");
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
            let _one = lock(&super::GLOBAL_UP);
            let (tx, rx) = mpsc::channel();
            let caller = std::thread::current().id();
            let window = HelloWindow::from_hwnd(1).unwrap();
            let request = start(window, "Unlock Beebeeb", move |result| {
                let _ = tx.send((result, std::thread::current().id()));
            });
            assert!(request.is_some());
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
        fn a_request_cancelled_before_its_answer_hands_out_none() {
            let control = Arc::new(Control::default());
            control.cancel();
            let seen = imp::verify(HelloWindow::from_hwnd(1).unwrap(), "Unlock Beebeeb", &control);
            let mut got = None;
            control.answer(seen, |result| got = Some(result));
            assert_eq!(got, None);
        }
    }
}
