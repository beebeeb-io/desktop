//! Which Opens belong to the viewers: whether a list's Open of an item is this group's.

use super::{State, kinds};

/// Whether a list's Open of `item` is this group's ([`open_item`]).
pub(super) fn is_this_groups(s: &State, item: &crate::source::DriveItem) -> bool {
    !item.is_folder
        && kinds::viewer_of(&item.name).is_some()
        // A kind the window's source does not show here stays its own app's. So does a file whose bytes are kept
        // on this PC, when its own app shows it (a document, a picture): the system opens it there with the app
        // the person chose for it, with nothing to download. The viewer is for a file that is NOT on this PC
        // (shown from memory, no copy lands on disk), and for source code, scripts and web pages wherever they
        // are: Enter on one reads it, it never runs it (`kinds::its_app_shows_it`).
        && super::route_source(s).is_some_and(|source| {
            source.previews(&item.name)
                && !(kinds::its_app_shows_it(&item.name)
                    && source.kept_on_this_pc(&item.id)
                    && !source.opens_here_when_kept(&item.name))
        })
}
