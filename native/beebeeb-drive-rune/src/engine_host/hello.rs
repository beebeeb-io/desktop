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

/// Runs once with how the prompt ended, on the prompt's own thread.
pub type HelloDone = Box<dyn FnOnce(HelloResult) + Send>;
/// Takes a prompt that is up away. Never called from inside its [`HelloDone`].
pub type HelloCancel = Box<dyn FnOnce() + Send>;

pub trait HelloPrompter: Send {
    /// Ask for the prompt; call it on the window's thread. `Ok`: `done` runs exactly once, later, never inside
    /// this call. `Err`: no prompt can be shown here (no window yet, no thread), and `done` was dropped unrun.
    fn start(&self, done: HelloDone) -> Result<HelloCancel, ()>;
}

/// The prompt owned by the main window. Off Windows, and before the window has a native handle: `Err`.
pub struct WindowHello {
    window: Arc<Mutex<Option<rune::WindowHandle>>>,
}

impl WindowHello {
    /// `window`: the shell's slot for the main window's native handle (`shell::host::Wiring::window`).
    pub fn new(window: Arc<Mutex<Option<rune::WindowHandle>>>) -> Self {
        Self { window }
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
        let request = windows_hello::start(window, REASON, done).ok_or(())?;
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
