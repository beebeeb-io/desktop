//! Slice A6 (plan40 batch 1): deleting the account from this PC (DW-A58, DW-A80 to DW-A83). Part of
//! [`EngineAccountSource`] (`account.rs`), which hands each deletion method here through `not_wired(…)`.
//!
//! Not bound yet: these are the adapter's bodies of before the scaffold, moved unchanged. Nothing is asked of
//! Beebeeb: no deletion is known, a request is refused as offline (the step-up's confirmation handed back), and a
//! cancel, the link's use and an undo do nothing. "Send again" stays in `account.rs` (the resend route is not
//! deployed). Session-bound like its neighbours once bound: a press stamps the session and account
//! (`stamp.rs`), a late answer under another session is dropped.

use super::*;

/// What slice A6 keeps between a press and its answer: its worker's inbox, what it holds for one session and
/// account. The adapter holds one (`EngineAccountSource::account_deletion`). Empty until A6 binds.
#[derive(Default)]
pub(super) struct DeletionCalls {}

impl<C: CredentialStores + 'static> EngineAccountSource<C> {
    pub(super) fn account_deletion_deletion(&self) -> Deletion {
        Deletion::None
    }

    pub(super) fn account_deletion_request_deletion(
        &mut self,
        confirmation: StepUp,
        _code: &str,
    ) -> Result<(), DeletionRefused> {
        Err(DeletionRefused {
            why: CodeRefused::Offline,
            confirmation: Some(confirmation),
        })
    }

    pub(super) fn account_deletion_cancel_deletion_request(&mut self) {}

    pub(super) fn account_deletion_deletion_link_used(&mut self) -> bool {
        false
    }

    pub(super) fn account_deletion_undo_deletion(&mut self) -> bool {
        false
    }
}
