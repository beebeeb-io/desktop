//! The word surface's keys and pointer (a later task fills this).
#![allow(dead_code)]

use rune::{KeyEvent, Tx};

use crate::State;

/// A key for the surface. `false`: not taken.
pub fn on_key(_s: &mut State, _tx: &mut Tx, _ev: &KeyEvent) -> bool {
    false
}
