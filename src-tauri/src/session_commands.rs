//! Windows command admission, independent of whether a sync root/runner exists.
//! The future is created without polling it; admission precedes every credential
//! read. Cancellation drops the entire future (including its client) before the
//! lease is released. Compiled in tests on every host.
use crate::callback_gate::{CallbackGate, CallbackLease};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

pub struct SessionCommands {
    gate: CallbackGate<()>,
    next: AtomicU64,
    pending: AtomicBool,
}
impl SessionCommands {
    pub fn new() -> Self {
        Self {
            gate: CallbackGate::new(),
            next: AtomicU64::new(1),
            pending: AtomicBool::new(false),
        }
    }
    // Call only under SESSION_TRANSITION, after installing the session.
    pub fn open(&self) -> Result<(), String> {
        self.ensure_reactivation_allowed()?;
        if self.gate.acquire(None).is_none() {
            self.gate
                .install(self.next.fetch_add(1, Ordering::SeqCst), Arc::new(()))?;
        }
        Ok(())
    }
    pub fn ensure_reactivation_allowed(&self) -> Result<(), String> {
        if self.pending.load(Ordering::SeqCst) {
            return Err("Vault lock did not finish. Retry locking before unlocking or signing in.".into());
        }
        Ok(())
    }
    pub fn generation(&self) -> Result<u64, String> {
        self.gate
            .acquire(None)
            .map(|l| l.generation_id())
            .ok_or_else(|| "vault is locked or stopping".into())
    }
    pub fn validate_start(&self, generation: u64) -> Result<(), String> {
        self.gate
            .acquire(Some(generation))
            .map(|_| ())
            .ok_or_else(|| "Session changed while starting sync".into())
    }
    pub fn startup_restore_allowed(&self) -> bool {
        self.next.load(Ordering::SeqCst) == 1 && !self.pending.load(Ordering::SeqCst)
    }
    pub fn close(&self) -> Option<crate::callback_gate::Revoked<()>> {
        self.pending.store(true, Ordering::SeqCst);
        self.next.fetch_add(1, Ordering::SeqCst);
        self.gate.revoke()
    }
    pub fn drain(&self, revoked: Option<crate::callback_gate::Revoked<()>>, timeout: Duration) -> Result<(), String> {
        if let Some(revoked) = revoked {
            revoked.drain(timeout)?;
        }
        Ok(())
    }
    /// Call only once the runner and native callbacks also confirmed shutdown.
    pub fn finish_close(&self) {
        self.pending.store(false, Ordering::SeqCst);
    }
    pub async fn run<T>(&self, work: impl std::future::Future<Output = Result<T, String>>) -> Result<T, String> {
        let lease = self.lease()?;
        Self::within(lease, work).await
    }
    /// Work that outlives the command that started it (a task of its own) is bound to the session here, when it is
    /// created: the lease is taken before the work is first polled, so a lock cancels it and waits for it to drop.
    /// Refused once the session is closed; the work then drops without ever being polled (#139 review P1).
    /// The future owns its lease and borrows nothing of `self` (`use<T, W>`), so it can run as a task of its own.
    pub fn bind<T, W>(
        &self,
        work: W,
    ) -> Result<impl std::future::Future<Output = Result<T, String>> + use<T, W>, String>
    where
        W: std::future::Future<Output = Result<T, String>>,
    {
        let lease = self.lease()?;
        Ok(Self::within(lease, work))
    }
    fn lease(&self) -> Result<CallbackLease<()>, String> {
        self.gate
            .acquire(None)
            .ok_or_else(|| "vault is locked or stopping".to_string())
    }
    async fn within<T>(
        lease: CallbackLease<()>,
        work: impl std::future::Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        let result = {
            tokio::pin!(work);
            tokio::select! {
                biased;
                _ = async {
                    while !lease.is_revoked() { tokio::time::sleep(Duration::from_millis(5)).await; }
                } => Err("Vault locked while the command was running".into()),
                result = &mut work => result,
            }
        }; // work and its credentials drop before the lease
        if lease.is_revoked() {
            return Err("Vault locked while the command was running".into());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn held_ipc_response_cancelled_and_credentials_dropped_before_lock_success() {
        let commands = Arc::new(SessionCommands::new());
        commands.open().unwrap();
        let (started, started_rx) = tokio::sync::oneshot::channel();
        let (reply, reply_rx) = tokio::sync::oneshot::channel::<()>();
        let credentials = Arc::new(42);
        let weak = Arc::downgrade(&credentials);
        let task_commands = commands.clone();
        let task = tokio::spawn(async move {
            task_commands
                .run(async move {
                    let _credentials = credentials;
                    started.send(()).unwrap();
                    let _ = reply_rx.await;
                    Ok("plaintext")
                })
                .await
        });
        started_rx.await.unwrap();
        let revoked = commands.close();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .expect("held IPC did not cancel")
                .unwrap()
                .is_err(),
            "held IPC returned plaintext after revocation"
        );
        commands.drain(revoked, Duration::from_secs(1)).unwrap();
        assert!(weak.upgrade().is_none(), "credentials survived successful lock");
        assert!(reply.send(()).is_err(), "HTTP future was not cancelled");
        let reads = AtomicU64::new(0);
        assert!(
            commands
                .run(async {
                    reads.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
                .await
                .is_err()
        );
        assert_eq!(reads.load(Ordering::SeqCst), 0, "locked command copied credentials");
    }
    /// #139 review P1: work that outlives the command that started it (the popover's storage fetch runs as a task of
    /// its own) is bound to the session when it is created. A lock cancels it and drops what it holds before the drain
    /// reports success; bound after the lock, it never runs and drops at once.
    #[tokio::test]
    async fn a_bound_fetch_that_outlives_its_command_is_cancelled_and_drained_with_the_session() {
        let commands = Arc::new(SessionCommands::new());
        commands.open().unwrap();
        let (started, started_rx) = tokio::sync::oneshot::channel();
        let (reply, reply_rx) = tokio::sync::oneshot::channel::<()>();
        let credentials = Arc::new(42);
        let weak = Arc::downgrade(&credentials);
        let bound = commands
            .bind(async move {
                let _credentials = credentials;
                started.send(()).unwrap();
                let _ = reply_rx.await;
                Ok("usage")
            })
            .unwrap();
        // The command that created it has returned; the fetch runs on as its own task.
        let detached = tokio::spawn(bound);
        started_rx.await.unwrap();
        let revoked = commands.close();
        let drained = {
            let commands = commands.clone();
            tokio::task::spawn_blocking(move || commands.drain(revoked, Duration::from_secs(1)))
        };
        assert!(
            tokio::time::timeout(Duration::from_secs(1), detached)
                .await
                .expect("the detached fetch was not cancelled")
                .unwrap()
                .is_err(),
            "the detached fetch answered after the lock"
        );
        drained
            .await
            .unwrap()
            .expect("the drain waited for nothing, or for ever");
        assert!(weak.upgrade().is_none(), "the token outlived the lock");
        assert!(reply.send(()).is_err(), "the HTTP future was not dropped");

        // Bound once the session is closed: refused, never polled, and what it holds drops at once.
        let polled = Arc::new(AtomicBool::new(false));
        let late = Arc::new(7);
        let late_weak = Arc::downgrade(&late);
        let flag = polled.clone();
        assert!(
            commands
                .bind(async move {
                    let _late = late;
                    flag.store(true, Ordering::SeqCst);
                    Ok(())
                })
                .is_err()
        );
        assert!(!polled.load(Ordering::SeqCst));
        assert!(late_weak.upgrade().is_none(), "a refused fetch kept its token");
    }
    #[tokio::test]
    async fn queued_engine_start_revalidates_generation_after_lock_and_relogin() {
        let commands = Arc::new(SessionCommands::new());
        let transition = Arc::new(tokio::sync::Mutex::new(()));
        commands.open().unwrap();
        let old = commands.generation().unwrap();
        let lock = transition.lock().await;
        let c = commands.clone();
        let t = transition.clone();
        let queued = tokio::spawn(async move {
            let _transition = t.lock().await;
            c.validate_start(old)
        });
        commands.drain(commands.close(), Duration::ZERO).unwrap();
        assert!(
            commands.open().is_err(),
            "unfinished native shutdown allowed reactivation"
        );
        commands.finish_close();
        commands.open().unwrap();
        drop(lock);
        assert!(queued.await.unwrap().is_err(), "queued picker spawned old session");
        commands.validate_start(commands.generation().unwrap()).unwrap();
    }
}
