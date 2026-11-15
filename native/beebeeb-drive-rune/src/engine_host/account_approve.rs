//! Slice A5 (plan40 batch 1): approving a sign-in from another device (DW-A12 to DW-A17). Part of
//! [`EngineAccountSource`] (`account.rs`), which hands each approval method here through `not_wired(…)`.
//!
//! Not bound yet: these are the trait's own "nothing waits" bodies, moved unchanged. No request is found or waits,
//! an approval begins nothing and never answers, a denial answers that the request expired, and no request is gone.
//! Session-bound like its neighbours once bound: a press stamps the session and account (`stamp.rs`), a late
//! answer under another session is dropped.

use super::*;

/// What slice A5 keeps between a press and its answer: its worker's inbox, what it holds for one session and
/// account. The adapter holds one (`EngineAccountSource::account_approve`). Empty until A5 binds.
#[derive(Default)]
pub(super) struct ApproveCalls {}

impl<C: CredentialStores + 'static> EngineAccountSource<C> {
    pub(super) fn account_approve_find_sign_in_request(&mut self, _code: &str) -> Lookup {
        Lookup::NotFound
    }

    pub(super) fn account_approve_waiting_sign_in_request(&self, _request: &str) -> Option<SignInRequest> {
        None
    }

    /// The password is borrowed for the call; it is never logged and never kept.
    pub(super) fn account_approve_begin_approval(&mut self, _request: &str, _code: &str, _password: Password<'_>) {}

    pub(super) fn account_approve_approval_answer(&mut self) -> Option<(String, Approval)> {
        None
    }

    pub(super) fn account_approve_cancel_approval(&mut self, _request: &str) {}

    pub(super) fn account_approve_deny_sign_in(&mut self, _request: &str) -> Denial {
        Denial::Expired
    }

    pub(super) fn account_approve_sign_in_request_gone(&mut self, _request: &str) -> Option<Denial> {
        None
    }
}
