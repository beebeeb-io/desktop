//! The status line without a connection (`.mk-foot` with `.mk-sdot.is-off`): a grey dot and
//! "Offline" where `ui::status_line` draws the green one.

use crate::theme::tokens::Theme;
use rune::El;

use super::status_bar::{Dot, bar};

pub fn status_offline(details: &str, t: &Theme) -> El {
    bar(Some(Dot::Idle), Some("Offline"), details, t)
}
