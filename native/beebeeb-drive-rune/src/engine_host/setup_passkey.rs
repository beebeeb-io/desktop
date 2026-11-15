//! Slice A10 (plan40 batch 1): first-run step 1, "Add a passkey" (DW-0201), with the account password (owner
//! ruling: a passkey change proves the password). Part of [`EngineSetupSource`] (`setup.rs`), which hands
//! `add_passkey` here through `not_wired(…)`.
//!
//! Not bound yet: the adapter's body of before the scaffold, moved unchanged. The step is not offered
//! (`available`), and nothing is added: it stays Settings > Passkeys, where the pane asks. Once bound it takes the
//! same engine path Settings > Passkeys takes (`passkeys.rs`), session-bound like its neighbours.

use super::*;

impl<E: SetupEngine> EngineSetupSource<E> {
    /// The password is borrowed for the call; it is never logged and never kept.
    pub(super) fn setup_passkey_add_passkey(&mut self, _password: Password<'_>) -> bool {
        false
    }
}
