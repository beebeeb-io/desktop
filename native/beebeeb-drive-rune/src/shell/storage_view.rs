//! The storage figure the sidebars show (`State::storage`): the signed-in account's used and total bytes.
//!
//! It belongs to ONE account. A sign-out and a change of account clear it at once (the sidebars then say the figure
//! is unavailable, never the earlier account's), and the figure of the account that signs in is read again on a
//! worker (the server answers it; the window never waits for it). A lock of the same account keeps it: the same
//! person comes back to the same figure. An answer is taken only while the account that asked is still the one
//! signed in with an open gate.

use std::sync::{Arc, mpsc};

use super::app::GateState;
use super::state::State;
use crate::source::StorageSummary;

/// Reads the signed-in account's storage (the same source the app start asks); it may block on the network.
pub type StorageReader = Arc<dyn Fn() -> Option<StorageSummary> + Send + Sync>;

/// A read in flight, stamped with the account that asked for it.
pub struct Pending {
    account: String,
    answer: mpsc::Receiver<Option<StorageSummary>>,
}

#[cfg(test)]
impl Pending {
    /// A read in flight for `account`, for a test.
    pub fn stamped(account: &str, answer: mpsc::Receiver<Option<StorageSummary>>) -> Pending {
        Pending {
            account: account.to_owned(),
            answer,
        }
    }
}

/// Forgets the figure: nothing of an account is shown without being that account's.
pub fn clear(s: &mut State) {
    s.storage = StorageSummary {
        used_bytes: 0,
        quota_bytes: 0,
    };
    s.storage_known = false;
    s.storage_split = None;
    s.storage_pending = None;
}

/// Reads the figure again for the account signed in now. Nothing is asked while the gate is closed, there is no
/// account, or no reader was wired (a scene or a `--db` window).
pub fn start_read(s: &mut State) {
    let (Some(reader), Some(account)) = (s.storage_reader.clone(), s.shell.app.account().map(str::to_owned)) else {
        return;
    };
    if s.shell.app.gate() != GateState::Ready {
        return;
    }
    let (send, answer) = mpsc::channel();
    s.storage_pending = Some(Pending { account, answer });
    std::thread::spawn(move || {
        let _ = send.send(reader());
    });
}

/// The session changed from what the last poll saw: `(gate, account)` then and now. Clears for a sign-out and for
/// another account, then reads again when an account has an open gate and no figure of its own. `true`: the
/// window changed.
pub fn session_moved(s: &mut State, was: (GateState, Option<&str>), now: (GateState, Option<&str>)) -> bool {
    let account_left = was.1 != now.1;
    let signed_out = now.0 == GateState::SignedOut;
    let mut changed = false;
    if (account_left || signed_out) && s.storage_reader.is_some() {
        clear(s);
        changed = true;
    }
    if now.0 == GateState::Ready && now.1.is_some() && !s.storage_known && s.storage_pending.is_none() {
        start_read(s);
    }
    changed
}

/// Takes the answer of a read that finished. `true`: the figure changed.
pub fn poll(s: &mut State) -> bool {
    let Some(pending) = &s.storage_pending else {
        return false;
    };
    let answer = match pending.answer.try_recv() {
        Ok(answer) => answer,
        Err(mpsc::TryRecvError::Empty) => return false,
        Err(mpsc::TryRecvError::Disconnected) => None,
    };
    let asked_for = pending.account.clone();
    s.storage_pending = None;
    // Another account, a sign-out or a lock moved on while it was read: the answer is not shown.
    if s.shell.app.account() != Some(asked_for.as_str()) || s.shell.app.gate() != GateState::Ready {
        return false;
    }
    let Some(figure) = answer else {
        return false;
    };
    s.storage = figure;
    s.storage_known = true;
    true
}
