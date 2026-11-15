//! Slice B4 (plan40 batch 1): Make available offline on a file someone else shares (DW-0505 to DW-0412). Part of
//! [`EngineSharedSource`] (`shared.rs`), which hands `keep_offline` here through `not_wired(…)`.
//!
//! Not bound yet: the adapter's body of before the scaffold, moved unchanged. Nothing is sent, and the view says it
//! is not available. Session-bound like its neighbours once bound (`stamp.rs`).

use super::*;

impl<C: CredentialStores> EngineSharedSource<C> {
    pub(super) fn shared_offline_keep_offline(&mut self, _id: &str) -> bool {
        self.refuse(Notice::NotAvailable)
    }
}
