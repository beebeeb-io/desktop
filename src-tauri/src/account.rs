//! The engine's account registry under its old path (`crate::account`), plus the one function the shell's call
//! sites hand the Tauri-managed `AppState` to. Must not: hold account logic (that is the engine's).

pub(crate) use beebeeb_engine::account::*;

/// [`beebeeb_engine::account::synthesize_single_account`] on the shell's own registry. It takes `&AppState`, not
/// the trait, so a call site that holds a `tauri::State<AppState>` passes `&state` as it did before the registry
/// became a trait (a trait bound takes no deref coercion).
pub(crate) fn synthesize_single_account(state: &crate::session_state::AppState, id: AccountId) {
    beebeeb_engine::account::synthesize_single_account(state, id);
}
