//! Photos routes: Search and NewInFiles (the box and the New button of Photos' toolbar).

use rune::Tx;

use super::Intent;
use crate::State;
use crate::shell::app::{Event, Route, Section};

/// Takes the intent it can; `Err` hands it back and Photos says "Not available in this version yet".
pub fn handle(s: &mut State, tx: &mut Tx, intent: Intent) -> Result<(), Intent> {
    match intent {
        // The words go to Files' Search, which shows its results over the Files window (DW-0801).
        Intent::Search(words) => {
            if words.trim().is_empty() {
                return Ok(());
            }
            crate::shell::host::dispatch(s, Event::Navigate(Route::section(Section::Files)));
            crate::frames::search::run(s, &words, tx);
            Ok(())
        }
        // New adds to Files: the window goes there with its New menu open, where something can be added (DW-0414).
        Intent::NewInFiles => {
            crate::shell::host::dispatch(s, Event::Navigate(Route::section(Section::Files)));
            if s.can_add() {
                s.actions.menu = None;
                s.toggle_menu(crate::frames::files::MenuKind::New);
            }
            Ok(())
        }
        other => Err(other),
    }
}
