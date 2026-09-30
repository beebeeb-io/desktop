//! Revocable ownership for native callbacks. No credential-bearing Arc escapes a
//! lease: teardown cancels work, denies admission, and waits for every lease to
//! release its value before returning. Kept portable for Linux/Windows CI.
use std::ops::Deref;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
type Slot<T> = Arc<Mutex<Option<(Arc<T>, Arc<Generation>)>>>;

pub struct CallbackGate<T> {
    current: Slot<T>,
}
struct Generation {
    id: u64,
    state: Mutex<LeaseState>,
    drained: Condvar,
}
#[derive(Default)]
struct LeaseState {
    revoked: bool,
    active: usize,
}
pub struct CallbackLease<T> {
    value: Option<Arc<T>>,
    generation: Arc<Generation>,
}
pub struct Revoked<T>(Arc<T>, Arc<Generation>, Slot<T>);

impl<T> CallbackGate<T> {
    pub fn new() -> Self {
        Self {
            current: Arc::new(Mutex::new(None)),
        }
    }

    /// Activation is only legal after the previous generation has been drained.
    pub fn install(&self, id: u64, value: Arc<T>) -> Result<(), &'static str> {
        let mut slot = self.current.lock().unwrap();
        if slot.is_some() {
            return Err("previous generation has not drained");
        }
        *slot = Some((
            value,
            Arc::new(Generation {
                id,
                state: Mutex::new(LeaseState::default()),
                drained: Condvar::new(),
            }),
        ));
        Ok(())
    }

    pub fn acquire(&self, id: Option<u64>) -> Option<CallbackLease<T>> {
        let slot = self.current.lock().unwrap();
        let (value, generation) = slot.as_ref()?;
        if id.is_some_and(|id| generation.id != id) {
            return None;
        }
        let mut state = generation.state.lock().unwrap();
        if state.revoked {
            return None;
        }
        state.active += 1;
        Some(CallbackLease {
            value: Some(value.clone()),
            generation: generation.clone(),
        })
    }

    pub fn revoke(&self) -> Option<Revoked<T>> {
        let slot = self.current.lock().unwrap();
        let (value, generation) = slot.as_ref()?;
        generation.state.lock().unwrap().revoked = true;
        Some(Revoked(value.clone(), generation.clone(), self.current.clone()))
    }
}
impl<T> Revoked<T> {
    /// Failure retains the revoked slot, including its owner and active count.
    /// A retry must drain this same generation before installation can succeed.
    pub fn drain(self, timeout: Duration) -> Result<(), &'static str> {
        let state = self.1.state.lock().unwrap();
        let (state, _) = self
            .1
            .drained
            .wait_timeout_while(state, timeout, |s| s.active != 0)
            .unwrap();
        if state.active != 0 {
            return Err(
                "Vault lock failed: work is still stopping. Retry locking; if it persists, restart Beebeeb. The vault is not locked.",
            );
        }
        drop(state);
        let mut slot = self.2.lock().unwrap();
        if slot
            .as_ref()
            .is_some_and(|(_, generation)| Arc::ptr_eq(generation, &self.1))
        {
            slot.take();
        }
        drop(self.0);
        Ok(())
    }
}
impl<T> CallbackLease<T> {
    pub fn generation_id(&self) -> u64 {
        self.generation.id
    }
    pub fn is_revoked(&self) -> bool {
        self.generation.state.lock().unwrap().revoked
    }
}
impl<T> Deref for CallbackLease<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value.as_deref().unwrap()
    }
}
impl<T> Drop for CallbackLease<T> {
    fn drop(&mut self) {
        drop(self.value.take());
        let mut state = self.generation.state.lock().unwrap();
        state.active -= 1;
        self.generation.drained.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn revoke_denies_new_work_and_cancels_existing_work() {
        let gate = CallbackGate::new();
        gate.install(10, Arc::new(42)).unwrap();
        let lease = gate.acquire(Some(10)).unwrap();
        assert!(!lease.is_revoked());
        let revoked = gate.revoke().unwrap();
        assert!(gate.acquire(None).is_none());
        assert!(lease.is_revoked());
        drop(lease);
        revoked.drain(Duration::from_secs(2)).unwrap();
    }

    #[test]
    fn drain_waits_for_transfer_and_releases_credentials() {
        let gate = CallbackGate::new();
        let owner = Arc::new(42);
        let weak = Arc::downgrade(&owner);
        gate.install(10, owner).unwrap();
        let lease = gate.acquire(None).unwrap();
        let revoked = gate.revoke().unwrap();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            revoked.drain(Duration::from_secs(2)).unwrap();
            tx.send(()).unwrap();
        });
        assert!(
            rx.recv_timeout(Duration::from_millis(30)).is_err(),
            "lock completed during transfer"
        );
        drop(lease);
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        assert!(weak.upgrade().is_none(), "credential owner survived drain");
    }

    #[test]
    fn twice_relogin_rejects_old_connection_and_uses_current_account() {
        let gate = CallbackGate::new();
        for id in 1..=3 {
            gate.install(id, Arc::new(id)).unwrap();
            assert!(gate.acquire(Some(id - 1)).is_none());
            assert_eq!(*gate.acquire(Some(id)).unwrap(), id);
            gate.revoke().unwrap().drain(Duration::from_secs(2)).unwrap();
        }
        assert!(gate.revoke().is_none());
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;
    #[test]
    fn stalled_native_lease_returns_failed_lock_and_retains_revocation_for_retry() {
        let gate = CallbackGate::new();
        gate.install(1, Arc::new(42)).unwrap();
        let lease = gate.acquire(None).unwrap();
        let start = std::time::Instant::now();
        assert!(gate.revoke().unwrap().drain(Duration::from_millis(20)).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(gate.acquire(None).is_none());
        assert!(gate.install(2, Arc::new(43)).is_err(), "timeout must deny reactivation");
        assert!(
            gate.revoke().unwrap().drain(Duration::ZERO).is_err(),
            "retry must retain outstanding lease"
        );
        drop(lease);
        gate.revoke().unwrap().drain(Duration::ZERO).unwrap();
        gate.install(2, Arc::new(43)).unwrap();
    }
}
