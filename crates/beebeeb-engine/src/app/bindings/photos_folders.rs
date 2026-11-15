//! Slice B1 (plan40 batch 1): Photo sources' folders (DW-0604) and the devices' backup line (DW-0601).
//!
//! Session-bound like its neighbours (`*_for(session)`): every call is asked for the session the host read at its
//! press, and its answer is handed over only while that session is still the one in memory. Empty until B1 binds.

use crate::app::{CredentialStores, Engine};

impl<C: CredentialStores> Engine<C> {}
