//! Slice B4 (plan40 batch 1): Make available offline on a file someone else shares (DW-0505 to DW-0412).
//!
//! Session-bound like its neighbours (`*_for(session)`): every call is asked for the session the host read at its
//! press, and its answer is handed over only while that session is still the one in memory. Empty until B4 binds.

use crate::app::{CredentialStores, Engine};

impl<C: CredentialStores> Engine<C> {}
