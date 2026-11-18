//! The input method's text into the editor (a later task fills this).
#![allow(dead_code)]

use rune::{El, Tx};

use crate::State;

pub fn on_ime(_s: &mut State, _tx: &mut Tx, _ime: rune::Ime) {}

/// The element that tells the system where the caret is. `None`: nothing to say.
pub fn caret_el(_s: &State) -> Option<El> {
    None
}
