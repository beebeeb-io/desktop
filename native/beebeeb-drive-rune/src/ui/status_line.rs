//! The status line with its words from the caller (`.mk-foot`): `status_bar::status_bar` says the
//! DW-0401 sample's; this one says what the folder holds. `state` is the sync state's word ("Up to
//! date"); `None` when nothing syncs here: then there is no dot and no word, never a sample.
//! The dot follows `status_bar::dot_for`.

use crate::theme::tokens::Theme;
use rune::El;

use super::status_bar::{bar, dot_for};

pub use super::status_bar::{
    CHECKING, PAUSED, SIGN_IN_AGAIN, STARTING, SYNC_IS_OFF, SYNC_STOPPED, SYNCING, UP_TO_DATE,
};

pub fn status_line(state: Option<&str>, details: &str, t: &Theme) -> El {
    bar(state.map(dot_for), state, details, t)
}
