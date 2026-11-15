//! "Sign in with another device" on the real engine (DW-A07 to DW-A10): the device-request half of
//! [`EngineSignInSource`]. The engine runs the request (`beebeeb_engine::app::bindings::device_request`: the socket, the
//! code's countdown, the opening of the keys and their install, every rule of it); this file starts it, maps its events
//! to the screen's, and makes the QR code of the address the server named.
//!
//! One request at a time: a second press while one runs starts nothing. It runs on a thread of its own (it waits up to
//! the code's five minutes; the sign-in worker keeps serving the other steps), for the session's turn AS READ AT THE
//! PRESS. Its answers carry the attempt of `Wait::Device`: `cancel_device_request` moves that attempt on AND sets the
//! request's stop, so no answer of it reaches the screen after that call (`poll` drops an older attempt, and the thread
//! queues nothing once it sees the move), and the engine ends its wait within its stop poll.
//!
//! A cancel stops what has not been written: the engine looks at the stop under its step lock, right before the
//! install, and after that point a cancel is too late. A request whose install went ahead is NOT signed out again. The
//! screen cancels by itself whenever the gate moves while the request is up (`follow_gate`), and a sign-in that just
//! installed moves it to Ready before its own answer is read; undoing a request that was left would sign out every
//! approval that won that race. The gate then leads the window on (to Files), so DW-A08's screen may not be seen.
//!
//! The countdown is the engine's (`DeviceTick` comes from its `Tick`): no clock of the window's own counts it.
//!
//! How the engine's endings are drawn: approved is DW-A08; denied DW-A09; the code's end DW-A10; a socket that dropped
//! and a server error after the code are a request that was LOST (DW-A10's layout with its own words: the code is spent,
//! a new one is asked for); an approval this PC did not take is said for what it is, "Nothing was signed in": another
//! account's session or local files here (`OtherAccount`), or a PC that moved on while it waited or could not store the
//! session (`NotSignedIn`); a payload that did not open is DW-A08's error state; before any code, offline or the form's
//! "didn't finish". The states without a board frame are flagged in `frames/signin/FRAMES.md`.
//!
//! Secrets: none passes through here. The code is shown and never logged; the payload, the token and the key stay in
//! the engine.
//!
//! Must not: log the code, an address or a name; start a second request while one runs; keep a clock of its own.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::bindings::device_request::{
    DeviceRequestEvent, DeviceRequestRefused, DeviceSignedIn, device_request_name,
};
use beebeeb_engine::app::{CredentialStores, Engine};

use super::{Answer, EngineSignInSource, WAITS, Wait, lock, log};
use crate::frames::signin::source::{DeviceRequest, DeviceStatus, SignInEvent, SignInProblem};

/// This PC's name on the screen when the machine has none (the first frame then carries no name either).
const THIS_COMPUTER: &str = "This computer";

/// The request that runs, if any: shared with its thread, which clears it when it ends (only while it is its own).
#[derive(Default)]
pub(super) struct DeviceSeat(Arc<Mutex<Option<Running>>>);

/// A running request: its attempt of `Wait::Device`, and the stop the engine watches.
struct Running {
    attempt: u64,
    stop: Arc<AtomicBool>,
}

impl Drop for DeviceSeat {
    /// The source is gone (the window closed): its request stops too, rather than waiting out the code.
    fn drop(&mut self) {
        if let Some(running) = lock(&self.0).take() {
            running.stop.store(true, Ordering::SeqCst);
        }
    }
}

impl<C: CredentialStores> EngineSignInSource<C> {
    /// "Sign in with another device", "Ask again" and "Get a new code" (the screen leaves the earlier request first).
    /// A press while a request runs starts nothing.
    pub(super) fn device_begin(&mut self) {
        let mut seat = lock(&self.device.0);
        if seat
            .as_ref()
            .is_some_and(|running| !running.stop.load(Ordering::SeqCst))
        {
            return;
        }
        let attempt = self.next_attempt(Wait::Device);
        let stop = Arc::new(AtomicBool::new(false));
        *seat = Some(Running {
            attempt,
            stop: stop.clone(),
        });
        drop(seat);
        // The session's turn as read at the press: the engine installs nothing for a session that moved on since.
        let turn = self.engine.session_turn();
        let (engine, answers, attempts) = (self.engine.clone(), self.answers.clone(), self.attempts.clone());
        let own_seat = self.device.0.clone();
        let started = thread::Builder::new().name("signin-device".into()).spawn(move || {
            run_request(&engine, &answers, &attempts, attempt, turn, stop);
            let mut current = lock(&own_seat);
            if current.as_ref().is_some_and(|running| running.attempt == attempt) {
                *current = None;
            }
        });
        if started.is_err() {
            log("sign-in: the device request thread did not start");
            *lock(&self.device.0) = None;
            let _ = self.answers.send(Answer {
                wait: Wait::Device,
                attempt,
                event: SignInEvent::DeviceRequested(Err(SignInProblem::Other)),
            });
        }
    }

    /// The person left the request (Cancel, Back, "Get a new code", a gate that moved). From this call on no answer of
    /// it reaches the screen, and the engine ends its wait at its next look at the stop.
    pub(super) fn device_cancel(&mut self) {
        self.next_attempt(Wait::Device);
        if let Some(running) = lock(&self.device.0).take() {
            running.stop.store(true, Ordering::SeqCst);
        }
    }
}

/// One request, on its own thread: the engine's events go to the screen while it waits, then its ending.
fn run_request<C: CredentialStores>(
    engine: &Arc<Engine<C>>,
    answers: &mpsc::Sender<Answer>,
    attempts: &Arc<[AtomicU64; WAITS]>,
    attempt: u64,
    turn: SessionGeneration,
    stop: Arc<AtomicBool>,
) {
    let shown = Arc::new(AtomicBool::new(false));
    let tell = {
        let (engine, answers, attempts, shown) = (engine.clone(), answers.clone(), attempts.clone(), shown.clone());
        let asking_as = asking_as();
        move |event: DeviceRequestEvent| {
            let event = on_event(event, &asking_as, &shown);
            // An event of a request the person left is not even queued.
            if attempts[Wait::Device as usize].load(Ordering::SeqCst) != attempt {
                return;
            }
            let _ = answers.send(Answer {
                wait: Wait::Device,
                attempt,
                event,
            });
            engine.wake_host();
        }
    };
    let result = std::panic::catch_unwind(AssertUnwindSafe(move || engine.device_request_for(turn, tell, stop)))
        .unwrap_or(Err(DeviceRequestRefused::Failed));
    if attempts[Wait::Device as usize].load(Ordering::SeqCst) != attempt {
        // Left while it ran: its ending is not said. An install that went ahead stays (the module's doc): the gate
        // already says signed in, and it leads the window on.
        engine.wake_host();
        return;
    }
    if let Some(event) = ending(result, shown.load(Ordering::SeqCst)) {
        let _ = answers.send(Answer {
            wait: Wait::Device,
            attempt,
            event,
        });
    }
    engine.wake_host();
}

/// This PC's name as the other device sees it: the name the engine's first frame sends.
fn asking_as() -> String {
    device_request_name().unwrap_or_else(|| THIS_COMPUTER.to_string())
}

/// An engine event as the screen reads it. The code comes with the QR code of the address (never of the code).
fn on_event(event: DeviceRequestEvent, asking_as: &str, shown: &AtomicBool) -> SignInEvent {
    match event {
        DeviceRequestEvent::Code {
            code,
            verification_uri,
            expires_in,
        } => {
            shown.store(true, Ordering::SeqCst);
            SignInEvent::DeviceRequested(Ok(DeviceRequest {
                code,
                qr: qr_of(&verification_uri).unwrap_or_default(),
                seconds_left: u32::try_from(expires_in).unwrap_or(u32::MAX),
                asking_as: asking_as.to_string(),
            }))
        }
        DeviceRequestEvent::Tick { seconds_left } => SignInEvent::DeviceTick { seconds_left },
        DeviceRequestEvent::Connection(connected) => SignInEvent::DeviceConnection { connected },
    }
}

/// How a request's end is drawn (the module's doc). `shown`: a code was on the screen, which then reads only the
/// `Device` endings. `None`: the request was stopped, and nothing is said.
fn ending(result: Result<DeviceSignedIn, DeviceRequestRefused>, shown: bool) -> Option<SignInEvent> {
    use DeviceRequestRefused as Refused;
    Some(match result {
        // The gate decides what follows: Files, or (a key the store refused) the twelve words.
        Ok(signed_in) => SignInEvent::Device(DeviceStatus::Approved { by: signed_in.by }),
        Err(Refused::Stopped) => return None,
        Err(Refused::Denied { by }) => SignInEvent::Device(DeviceStatus::Denied { by }),
        Err(Refused::KeysNotReceived { by }) => SignInEvent::Device(DeviceStatus::KeysNotReceived { by }),
        Err(Refused::Expired) if shown => SignInEvent::Device(DeviceStatus::Expired),
        // The code is spent either way: the connection cannot be joined again (a new one is a new code), or Beebeeb
        // ended the request with an error.
        Err(Refused::ConnectionLost | Refused::Failed | Refused::Offline) if shown => {
            SignInEvent::Device(DeviceStatus::Lost)
        }
        // The keys arrived; this PC did not take them.
        Err(Refused::AccountMismatch) if shown => SignInEvent::Device(DeviceStatus::OtherAccount),
        Err(Refused::Overtaken | Refused::NotStored) if shown => SignInEvent::Device(DeviceStatus::NotSignedIn),
        Err(Refused::Offline | Refused::ConnectionLost) => SignInEvent::DeviceRequested(Err(SignInProblem::Offline)),
        Err(_) => SignInEvent::DeviceRequested(Err(SignInProblem::Other)),
    })
}

/// The QR code of the address the server named: dark modules, row by row. It opens the request on the other device and
/// holds no code (task 1734; the engine refuses an address that holds it). `None`: the address does not fit a code.
fn qr_of(address: &str) -> Option<Vec<Vec<bool>>> {
    let code = qrcode::QrCode::new(address.as_bytes()).ok()?;
    let width = code.width();
    Some(
        code.to_colors()
            .chunks(width)
            .map(|row| row.iter().map(|module| *module == qrcode::Color::Dark).collect())
            .collect(),
    )
}

#[cfg(test)]
#[path = "signin_device_tests.rs"]
mod tests;
