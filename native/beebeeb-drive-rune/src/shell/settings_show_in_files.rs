//! Settings' `ShowInFiles(id)`: Files on the folder of that file with the file selected (Insights' largest file).

use rune::Tx;

use crate::State;

pub fn show(s: &mut State, tx: &mut Tx, id: &str) {
    crate::frames::files::reveal::reveal(s, tx, id);
}
