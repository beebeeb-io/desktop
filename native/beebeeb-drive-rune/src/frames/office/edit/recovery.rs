//! The offer to bring back an unsaved editing (a later task fills this).
#![allow(dead_code)]

use rune::Tx;

use crate::State;

/// The session unlocked: offer what this PC kept.
pub fn on_unlocked(_s: &mut State, _tx: &mut Tx) {}
