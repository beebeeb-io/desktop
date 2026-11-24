//! Lets reads from Windows' own threads reach the app only while the helper answers one of the app's messages.
//!
//! The pipe is one conversation: a reader sends `ReadAt` and takes the next message as its `Data`. Media Foundation
//! reads from its own threads, also after the helper replied and while the app is about to send its next request; a
//! read then would take that request off the pipe. So the session opens the gate when a message arrives and closes it
//! before it replies, and every read of the system decoder's byte stream runs inside [`Gate::through`]: one at a
//! time, only while the gate is open. Closing waits for a read in progress. Once the session ends the gate is shut and
//! every read fails at once, so Windows' threads are not left waiting when the decoder is released.

use std::sync::{Condvar, Mutex, MutexGuard};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Closed,
    Open,
    Shut,
}

/// See the module documentation.
#[derive(Debug)]
pub struct Gate {
    state: Mutex<State>,
    changed: Condvar,
}

impl Default for Gate {
    fn default() -> Self {
        Gate::new()
    }
}

impl Gate {
    /// A closed gate.
    pub fn new() -> Gate {
        Gate {
            state: Mutex::new(State::Closed),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set(&self, to: State) {
        let mut state = self.lock();
        if *state != State::Shut {
            *state = to;
        }
        drop(state);
        self.changed.notify_all();
    }

    /// Reads may run (the session is answering a message).
    pub fn open(&self) {
        self.set(State::Open);
    }

    /// Reads wait from now on; returns once a read in progress has finished.
    pub fn close(&self) {
        self.set(State::Closed);
    }

    /// Every read from now on fails at once, and so does every read that was waiting. For good.
    pub fn shut(&self) {
        *self.lock() = State::Shut;
        self.changed.notify_all();
    }

    /// Runs `read` once the gate is open, holding it so that no other read runs and the gate does not close
    /// meanwhile. `None` when the gate is shut.
    pub fn through<T>(&self, read: impl FnOnce() -> T) -> Option<T> {
        let mut state = self.lock();
        while *state == State::Closed {
            state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        if *state == State::Shut {
            return None;
        }
        let out = read();
        drop(state);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_read_waits_while_the_gate_is_closed_and_runs_once_it_opens() {
        let gate = Arc::new(Gate::new());
        let ran = Arc::new(AtomicU32::new(0));
        let reader = {
            let (gate, ran) = (gate.clone(), ran.clone());
            std::thread::spawn(move || gate.through(|| ran.fetch_add(1, Ordering::SeqCst)))
        };
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(ran.load(Ordering::SeqCst), 0, "closed: the read has not run");
        gate.open();
        assert_eq!(reader.join().unwrap(), Some(0));
        assert_eq!(ran.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn closing_waits_for_the_read_in_progress() {
        let gate = Arc::new(Gate::new());
        gate.open();
        let done = Arc::new(AtomicU32::new(0));
        let (started, in_read) = std::sync::mpsc::channel();
        let reader = {
            let (gate, done) = (gate.clone(), done.clone());
            std::thread::spawn(move || {
                gate.through(|| {
                    started.send(()).unwrap();
                    std::thread::sleep(Duration::from_millis(150));
                    done.store(1, Ordering::SeqCst);
                })
            })
        };
        in_read.recv().unwrap();
        gate.close();
        assert_eq!(
            done.load(Ordering::SeqCst),
            1,
            "close returned only after the read finished"
        );
        reader.join().unwrap();
    }

    #[test]
    fn a_shut_gate_fails_waiting_and_later_reads_and_stays_shut() {
        let gate = Arc::new(Gate::new());
        let waiting = {
            let gate = gate.clone();
            std::thread::spawn(move || gate.through(|| 7))
        };
        std::thread::sleep(Duration::from_millis(50));
        gate.shut();
        assert_eq!(waiting.join().unwrap(), None);
        gate.open();
        assert_eq!(gate.through(|| 7), None, "opening does not undo a shut gate");
    }
}
