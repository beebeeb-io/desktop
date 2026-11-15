//! Slice A7 (plan40 batch 1): Add a folder and Stop backing up (DW-0737, DW-0738, DW-1410): a folder this PC backs up,
//! added or removed.
//!
//! Session-bound like its neighbours (`*_for(session)`): every call is asked for the session the host read at its
//! press, and its answer is handed over only while that session is still the one in memory. Empty until A7 binds.

use crate::app::{CredentialStores, Engine};

impl<C: CredentialStores> Engine<C> {}
