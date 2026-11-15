//! Slice B6 (plan40 batch 1): Beebeeb's items in File Explorer's classic menu (DW-1183).
//!
//! Not built yet: [`register`] adds nothing to Windows (today's behaviour). The shell already calls it once the
//! engine is open (`shell::open_engine`); B6 fills it here.

use std::sync::Arc;

use beebeeb_engine::app::{CredentialStores, Engine};

use crate::shell::identity::AppIdentity;

/// The menu's items for this app's files, with what they need from the engine. Nothing yet.
pub(crate) fn register<C: CredentialStores>(_identity: AppIdentity, _engine: &Arc<Engine<C>>) {}
