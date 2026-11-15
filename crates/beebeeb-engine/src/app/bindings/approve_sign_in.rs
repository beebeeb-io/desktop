//! Slice A5 (plan40 batch 1): approving a sign-in from another device (DW-A12 to DW-A17): the request a code names, the
//! approval that seals the keys to the other device, and the denial.
//!
//! Session-bound like its neighbours (`*_for(session)`): every call is asked for the session the host read at its
//! press, and its answer is handed over only while that session is still the one in memory. Empty until A5 binds.

use crate::app::{CredentialStores, Engine};

impl<C: CredentialStores> Engine<C> {}
