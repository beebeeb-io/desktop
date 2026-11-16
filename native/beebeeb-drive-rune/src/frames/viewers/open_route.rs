//! Which Opens belong to the viewers: whether a list's Open of an item is this group's.

use super::{State, kinds};

/// Whether a list's Open of `item` is this group's ([`open_item`]).
pub(super) fn is_this_groups(s: &State, item: &crate::source::DriveItem) -> bool {
    // An Office file while its route is on (`kinds::office_route`): the Office viewer, or its no-preview state for a
    // type that is not shown here (`.ppt`, DW-1556), from a source that holds Office files in a renderer
    // (`office_live`: the engine's). With the switches off, or from the board's fixture, it is its own app's.
    let office = kinds::office_route(&item.name).is_some();
    !item.is_folder
        && (kinds::viewer_of(&item.name).is_some() || office)
        // A kind the window's source does not show here stays its own app's. So does a file whose bytes are kept
        // on this PC, when its own app shows it (a document, a picture): the system opens it there with the app
        // the person chose for it, with nothing to download. The viewer is for a file that is NOT on this PC
        // (shown from memory, no copy lands on disk), and for source code, scripts and web pages wherever they
        // are: Enter on one reads it, it never runs it (`kinds::its_app_shows_it`).
        && super::route_source(s).is_some_and(|source| {
            (!office || source.office_live())
                && source.previews(&item.name)
                && !(kinds::its_app_shows_it(&item.name)
                    && source.kept_on_this_pc(&item.id)
                    && !source.opens_here_when_kept(&item.name))
        })
}
