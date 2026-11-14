//! The per-item answers a source that answers later hands over inside [`super::source::Change`].

use super::source::ActionError;

/// One item of a [`super::source::Change::Trash`]. `left`: `None`: it was made; otherwise why this item was left as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashOutcome {
    pub id: String,
    pub left: Option<ActionError>,
}

/// One item of a [`super::source::Change::Local`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalOutcome {
    pub id: String,
    /// The row's `pc_status` as the engine reports it AFTER the work (`Some(None)`: no mark, a folder that is
    /// not kept). `None`: the engine could not say, and the row stays as it is drawn.
    pub now: Option<Option<String>>,
    /// `None`: the change was made. Otherwise why this item was left; Try again is for these alone.
    pub left: Option<ActionError>,
}
