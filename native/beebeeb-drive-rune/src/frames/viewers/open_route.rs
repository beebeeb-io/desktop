//! Which Opens belong to the viewers: whether a list's Open of an item is this group's.

use super::State;

/// Whether a list's Open of `item` is this group's ([`open_item`]): every file, while the window has a source to
/// show it from. A kind the viewer shows is shown; any other kind (an unknown one, an Office type that ships no
/// preview) gets the viewer's "No preview" frame, whose "Open in another app" button is the only way to the
/// system's app (w29: nothing opens outside Beebeeb Drive without that press, kept on this PC or not).
pub(super) fn is_this_groups(s: &State, item: &crate::source::DriveItem) -> bool {
    !item.is_folder && super::route_source(s).is_some()
}
