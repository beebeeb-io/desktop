//! What the engine asks of the macOS File Provider bridge: one working-set signal, through the shell's bridge gate.
//!
//! The bridge (the Objective-C shim, its one-per-process gate) stays in the shell; the engine names it only through
//! [`FileProviderPort`]. The host hands its port over when the runner starts ([`install_from`]); the runner's
//! signal path calls [`signal_working_set`]. Must not: queue, retry or decide what is owed (the runner's
//! `OwedWorkingSetSignal` does), or name the bridge.

use std::sync::{Arc, Mutex};

use crate::finder_setup::error::{APP_DOMAIN, FpError, app_code};
use crate::host::EngineHost;

/// Result of a best-effort working-set signal (task 1697).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkingSetSignalOutcome {
    /// The system accepted the signal (or the domain is not registered —
    /// nothing to signal is not a failure; sync continues regardless).
    Delivered,
}

/// The shell's File Provider bridge, as far as the engine uses it.
pub trait FileProviderPort: Send + Sync + 'static {
    /// Signal the replica's working set, through the one bridge gate. A busy gate answers at once with the error
    /// [`is_gate_busy`] recognises; any other error is the OS failing. Never blocks on a gate another call holds.
    fn signal_working_set(&self) -> Result<WorkingSetSignalOutcome, FpError>;
}

/// Pure decision core for the runner's signal path: an operation batch that
/// changed items always asks for a working-set signal (Replicated honors ONLY
/// `.workingSet`); anything else never touches the bridge.
pub fn should_signal_working_set(changed_item_ids: &[String]) -> bool {
    !changed_item_ids.is_empty()
}

/// Whether `error` is the gate answering "busy" (an earlier bridge call has not returned), as opposed to
/// the OS failing. The engine's working-set signal owes its ids to the next tick in that case.
pub fn is_gate_busy(error: &FpError) -> bool {
    error.domain == APP_DOMAIN && error.code == app_code::OP_TIMEOUT
}

/// The bridge is one per process (like its gate), so the port is too. The last runner start wins.
static PORT: Mutex<Option<Arc<dyn FileProviderPort>>> = Mutex::new(None);

/// The runner starts: take the host's port. A host without one (a platform with no File Provider) leaves the
/// slot as it was.
pub(crate) fn install_from(host: &dyn EngineHost) {
    if let Some(port) = host.file_provider() {
        *PORT.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(port);
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Test-only: the port this thread's signals go to instead of the installed one.
    static TEST_PORT: std::cell::RefCell<Option<Arc<dyn FileProviderPort>>> = const { std::cell::RefCell::new(None) };
}

/// One working-set signal through the installed port. No port installed is a failure that is not "busy": the
/// caller logs it and owes nothing, as for any other bridge failure.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // the runner's signal path is macOS-only
pub(crate) fn signal_working_set() -> Result<WorkingSetSignalOutcome, FpError> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(port) = TEST_PORT.with(|port| port.borrow().clone()) {
        return port.signal_working_set();
    }
    let port = PORT.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
    match port {
        Some(port) => port.signal_working_set(),
        None => Err(FpError::app(
            app_code::UNEXPECTED_BRIDGE_RETURN,
            "no File Provider port is installed",
        )),
    }
}

/// A port whose gate is always busy and that counts how often it was asked (tests only).
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct BusyGatePort {
    asked: std::sync::atomic::AtomicUsize,
}

#[cfg(any(test, feature = "test-support"))]
impl BusyGatePort {
    pub fn asked(&self) -> usize {
        self.asked.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl FileProviderPort for BusyGatePort {
    fn signal_working_set(&self) -> Result<WorkingSetSignalOutcome, FpError> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(FpError::app(
            app_code::OP_TIMEOUT,
            "an earlier File Provider call has not returned",
        ))
    }
}

/// Run `f` with this thread's signals going to `port` (tests only), as if the shell had installed it.
#[cfg(any(test, feature = "test-support"))]
pub fn with_port<T>(port: Arc<dyn FileProviderPort>, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Arc<dyn FileProviderPort>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let earlier = self.0.take();
            TEST_PORT.with(|slot| *slot.borrow_mut() = earlier);
        }
    }
    let _restore = Restore(TEST_PORT.with(|slot| slot.borrow_mut().replace(port)));
    f()
}

/// Run `f` with the bridge gate busy, as if a bridge call were in flight (tests only): every signal `f` makes
/// meets a busy gate, and no bridge call is made.
#[cfg(any(test, feature = "test-support"))]
pub fn with_busy_gate<T>(f: impl FnOnce() -> T) -> T {
    with_port(Arc::new(BusyGatePort::default()), f)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Answers(Result<WorkingSetSignalOutcome, FpError>);

    impl FileProviderPort for Answers {
        fn signal_working_set(&self) -> Result<WorkingSetSignalOutcome, FpError> {
            self.0.clone()
        }
    }

    #[test]
    fn a_batch_that_changed_items_asks_for_a_signal_and_an_empty_one_does_not() {
        assert!(should_signal_working_set(&["f-1".to_string()]));
        assert!(!should_signal_working_set(&[]));
    }

    #[test]
    fn only_the_gates_own_timeout_is_busy() {
        assert!(is_gate_busy(&FpError::app(app_code::OP_TIMEOUT, "busy")));
        assert!(!is_gate_busy(&FpError::app(app_code::UNEXPECTED_BRIDGE_RETURN, "x")));
        // The same code in another domain is the OS failing, not the gate.
        assert!(!is_gate_busy(&FpError::new(
            crate::finder_setup::error::BRIDGE_DOMAIN,
            app_code::OP_TIMEOUT,
            "x"
        )));
    }

    #[test]
    fn a_signal_goes_to_the_port_and_its_answer_comes_back_unchanged() {
        let busy = Arc::new(BusyGatePort::default());
        let error = with_port(busy.clone(), signal_working_set).expect_err("the busy port refuses");
        assert!(is_gate_busy(&error));
        assert_eq!(busy.asked(), 1);
        assert_eq!(
            with_port(
                Arc::new(Answers(Ok(WorkingSetSignalOutcome::Delivered))),
                signal_working_set
            ),
            Ok(WorkingSetSignalOutcome::Delivered)
        );
        let os_failure = FpError::app(
            app_code::UNEXPECTED_BRIDGE_RETURN,
            "beebeeb_fp_signal_working_set returned 7",
        );
        let answered = with_port(Arc::new(Answers(Err(os_failure.clone()))), signal_working_set).expect_err("fails");
        assert_eq!(answered, os_failure);
        assert!(!is_gate_busy(&answered));
    }

    #[test]
    fn a_nested_port_is_undone_when_its_scope_ends() {
        let outer = Arc::new(BusyGatePort::default());
        with_port(outer.clone(), || {
            with_port(Arc::new(Answers(Ok(WorkingSetSignalOutcome::Delivered))), || {
                assert!(signal_working_set().is_ok());
            });
            assert!(signal_working_set().is_err());
        });
        assert_eq!(outer.asked(), 1);
    }
}
