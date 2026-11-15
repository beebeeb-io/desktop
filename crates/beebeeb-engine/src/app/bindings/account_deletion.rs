//! Slice A6 (plan40 batch 1): deleting the account from this PC (DW-A58, DW-A80 to DW-A83): the deletion's state, the
//! request with the step-up and the second factor, the link's use, the cancel and the undo.
//!
//! Session-bound like its neighbours (`*_for(session)`): every call is asked for the session the host read at its
//! press, and its answer is handed over only while that session is still the one in memory. Empty until A6 binds.

use crate::app::{CredentialStores, Engine};

impl<C: CredentialStores> Engine<C> {}
