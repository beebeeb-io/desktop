//! Copy, cut and paste keys (a later task fills this).
#![allow(dead_code)]

use rune::{KeyEvent, Tx};

use crate::State;

/// A clipboard key. `false`: not taken. Called first by each surface's `on_key`.
pub fn on_clip_key(_s: &mut State, _tx: &mut Tx, _ev: &KeyEvent) -> bool {
    false
}
