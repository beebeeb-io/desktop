//! The Windows Hello prompt as the lock binding asks for it: one seam, so the source that opens the screen lock
//! (`signin::EngineSignInSource::begin_hello`) is tested without Windows.
//!
//! The real one is [`WindowHello`]: the prompt of `shell::platform::windows_hello`, owned by the app's main
//! window. Its answer is only a statement that Windows verified the person at the keyboard; what that opens is
//! core's rule, asked through the engine. No secret passes through here, and nothing is logged.

use std::sync::{Arc, Mutex};

pub use crate::shell::platform::windows_hello::HelloResult;
use crate::shell::platform::windows_hello::{self, HelloWindow};

/// The line Windows shows in its prompt (DW-0009: "Unlock Beebeeb").
const REASON: &str = "Unlock Beebeeb";
/// The line for the proof that turns app lock off (Sign-in and security).
pub const TURN_APP_LOCK_OFF: &str = "Turn off app lock for Beebeeb";

/// Runs at most once with how the prompt ended, on the prompt's own thread (see [`HelloCancel`] for when it does not).
pub type HelloDone = Box<dyn FnOnce(HelloResult) + Send>;

/// Takes a prompt that is up away: the owner left the screen that asked.
///
/// The cancel contract, the one place it is written (`shell::platform::windows_hello` and every owner follow it):
/// - After the cancel returns, no answer of that prompt is decided any more: its [`HelloDone`] is dropped unrun,
///   whatever the person does in the prompt from then on.
/// - An answer decided BEFORE the cancel may still be on its way (its `done` may be running on the prompt's thread
///   while the cancel returns). The owner drops it with its own check (the attempt, the stamp or the prompt's
///   number it asked under), never by trusting the cancel alone.
/// - It never blocks, it may be called from inside the prompt's own [`HelloDone`], and on a prompt that is over it
///   changes nothing.
pub type HelloCancel = Box<dyn FnOnce() + Send>;

pub trait HelloPrompter: Send {
    /// Ask for the prompt; call it on the window's thread. `Ok`: `done` runs once, later, never inside this call,
    /// unless the returned [`HelloCancel`] was called first. One prompt is up at a time, app-wide: a prompt that is
    /// up for another owner is taken away, and that owner's `done` gets [`HelloResult::Cancelled`]. `Err`: no prompt
    /// can be shown here (no window yet, no thread), and `done` was dropped unrun.
    fn start(&self, done: HelloDone) -> Result<HelloCancel, ()>;
}

/// The prompt owned by the main window. Off Windows, and before the window has a native handle: `Err`.
pub struct WindowHello {
    window: Arc<Mutex<Option<rune::WindowHandle>>>,
    reason: &'static str,
}

impl WindowHello {
    /// `window`: the shell's slot for the main window's native handle (`shell::host::Wiring::window`).
    pub fn new(window: Arc<Mutex<Option<rune::WindowHandle>>>) -> Self {
        Self::for_reason(window, REASON)
    }

    /// The prompt with its own line (`reason`, one of this file's constants).
    pub fn for_reason(window: Arc<Mutex<Option<rune::WindowHandle>>>, reason: &'static str) -> Self {
        Self { window, reason }
    }
}

impl HelloPrompter for WindowHello {
    fn start(&self, done: HelloDone) -> Result<HelloCancel, ()> {
        let window = self
            .window
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .and_then(HelloWindow::of)
            .ok_or(())?;
        let request = windows_hello::start(window, self.reason, done).ok_or(())?;
        Ok(Box::new(move || request.cancel()))
    }
}

/// No prompt at all: a source built without a window (tests of the other steps, tooling).
pub struct NoHello;

impl HelloPrompter for NoHello {
    fn start(&self, _done: HelloDone) -> Result<HelloCancel, ()> {
        Err(())
    }
}
