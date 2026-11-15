//! Slice B5 (plan40 batch 1): Beebeeb's notices as Windows toasts (DW-0750), honouring the alert switches.
//!
//! Not built yet: [`start`] and [`show`] do nothing, so no notice is shown outside the window (today's behaviour).
//! The shell already calls both: [`start`] once at the app's start, before the engine opens (`shell::open_engine`),
//! and [`show`] for every notice the window drains from the engine (`engine_host::LiveEngine::drain`). B5 fills them
//! here and edits no other file for the wiring.

use beebeeb_engine::host::Notice;

use crate::shell::identity::AppIdentity;

/// What Windows needs before this app can show a toast (its identity). Nothing yet.
pub(crate) fn start(_identity: AppIdentity) {}

/// One notice of the engine, as the window drains it. Nothing yet: no toast.
pub(crate) fn show(_notice: &Notice) {}
