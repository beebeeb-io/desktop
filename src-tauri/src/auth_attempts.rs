//! Authentication starts before there is an unlocked command session. Its own
//! revocable leases cover handoff, decrypted credentials and queued installers.
use crate::callback_gate::{CallbackGate, CallbackLease, Revoked};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct AuthAttempts {
    gate: CallbackGate<()>,
    state: Mutex<(u64, bool)>,
}
pub struct Attempt(CallbackLease<()>);
pub struct ReopenOnDrop<'a>(pub &'a AuthAttempts);
impl Drop for ReopenOnDrop<'_> {
    fn drop(&mut self) {
        self.0.finish_close();
    }
}
impl AuthAttempts {
    pub fn new() -> Self {
        Self {
            gate: CallbackGate::new(),
            state: Mutex::new((1, false)),
        }
    }
    pub fn begin(&self) -> Result<Attempt, String> {
        let state = self.state.lock().unwrap();
        if state.1 {
            return Err("Authentication is stopping. Retry after locking finishes.".into());
        }
        if self.gate.acquire(None).is_none() {
            self.gate.install(state.0, Arc::new(()))?;
        }
        self.gate
            .acquire(Some(state.0))
            .map(Attempt)
            .ok_or_else(|| "Sign-in cancelled".into())
    }
    /// Must be checked under SESSION_TRANSITION immediately before installation.
    pub fn validate(&self, attempt: &Attempt) -> Result<(), String> {
        let state = self.state.lock().unwrap();
        if state.1 || state.0 != attempt.0.generation_id() || attempt.0.is_revoked() {
            return Err("Session changed during sign-in. Start sign-in again.".into());
        }
        Ok(())
    }
    pub fn close(&self) -> Option<Revoked<()>> {
        let mut state = self.state.lock().unwrap();
        state.0 += 1;
        state.1 = true;
        self.gate.revoke()
    }
    pub fn finish_close(&self) {
        self.state.lock().unwrap().1 = false;
    }
}
impl Attempt {
    pub async fn run<T>(&self, work: impl std::future::Future<Output = Result<T, String>>) -> Result<T, String> {
        tokio::pin!(work);
        tokio::select! {
            biased;
            _ = async { while !self.0.is_revoked() { tokio::time::sleep(Duration::from_millis(5)).await; } } => Err("Sign-in cancelled by vault lock or sign-out".into()),
            result = &mut work => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn round7_recovery_verification_cancels_and_late_install_is_rejected() {
        for teardown in ["lock", "signout"] {
            let auth = Arc::new(AuthAttempts::new());
            let transition = Arc::new(tokio::sync::Mutex::new(()));
            let writes = Arc::new(AtomicUsize::new(0));
            let runners = Arc::new(AtomicUsize::new(0));
            let attempt = auth.begin().unwrap();
            let late_attempt = auth.begin().unwrap();
            let (started, ready) = tokio::sync::oneshot::channel();
            let (reply, response) = tokio::sync::oneshot::channel();
            let (a, t, w, r) = (auth.clone(), transition.clone(), writes.clone(), runners.clone());
            let task = tokio::spawn(async move {
                attempt
                    .run(async {
                        {
                            let _transition = t.lock().await;
                            a.validate(&attempt)?;
                        }
                        started.send(()).unwrap();
                        let _verified_key = response.await.map_err(|_| "closed")?;
                        let _transition = t.lock().await;
                        a.validate(&attempt)?;
                        w.fetch_add(1, Ordering::SeqCst);
                        r.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                    .await
            });
            ready.await.unwrap();
            tokio::time::timeout(Duration::from_secs(1), async {
                let _transition = transition.lock().await;
                let revoked = auth.close().unwrap();
                assert!(task.await.unwrap().is_err(), "{teardown}");
                // A successful verifier already waiting to install must also fail.
                assert!(auth.validate(&late_attempt).is_err());
                drop(late_attempt);
                revoked.drain(Duration::ZERO).unwrap();
            })
            .await
            .expect("teardown waited for recovery verifier");
            assert!(reply.send([7u8; 32]).is_err());
            assert_eq!((writes.load(Ordering::SeqCst), runners.load(Ordering::SeqCst)), (0, 0));
        }
    }

    #[tokio::test]
    async fn round5_browser_handoff_after_signout_persists_nothing_and_starts_no_runner() {
        let auth = AuthAttempts::new();
        let attempt = auth.begin().unwrap();
        let transition = tokio::sync::Mutex::new(());
        let persisted = AtomicUsize::new(0);
        let runners = AtomicUsize::new(0);
        let lock = transition.lock().await;
        // A decrypted reply is queued behind sign-out. Exercise the final
        // validation independently of cooperative network cancellation.
        let install = async {
            let _transition = transition.lock().await;
            auth.validate(&attempt)?;
            persisted.fetch_add(1, Ordering::SeqCst);
            runners.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>(())
        };
        let revoked = auth.close().unwrap();
        assert!(auth.begin().is_err());
        auth.finish_close(); // clearing pending flags must NEVER revive this attempt
        drop(lock);
        assert!(install.await.is_err());
        assert_eq!(
            (persisted.load(Ordering::SeqCst), runners.load(Ordering::SeqCst)),
            (0, 0)
        );
        drop(attempt);
        revoked.drain(Duration::ZERO).unwrap();
        let fresh = auth.begin().unwrap();
        auth.validate(&fresh).unwrap();
    }

    #[tokio::test]
    async fn round5_lock_cancels_handoff_and_drains_credential_owner() {
        let auth = Arc::new(AuthAttempts::new());
        let attempt = auth.begin().unwrap();
        let secret = Arc::new(zeroize::Zeroizing::new(vec![7u8; 32]));
        let weak = Arc::downgrade(&secret);
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            attempt
                .run(async move {
                    let _secret = secret;
                    started.send(()).unwrap();
                    std::future::pending::<Result<(), String>>().await
                })
                .await
        });
        ready.await.unwrap();
        let revoked = auth.close().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        revoked.drain(Duration::ZERO).unwrap();
        assert!(weak.upgrade().is_none(), "credential owner survived drain");
        auth.finish_close();
        auth.validate(&auth.begin().unwrap()).unwrap();
    }
}
