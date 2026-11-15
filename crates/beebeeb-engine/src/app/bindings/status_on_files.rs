//! Slice B3 (plan40 batch 1): "Show sync status on files" (DW-0746): the setting, honoured by File Explorer on Windows.
//!
//! Session-bound like its neighbours (`*_for(session)`): every call is asked for the session the host read at its
//! press, and its answer is handed over only while that session is still the one in memory. Empty until B3 binds.

use crate::app::{CredentialStores, Engine};

impl<C: CredentialStores> Engine<C> {}
