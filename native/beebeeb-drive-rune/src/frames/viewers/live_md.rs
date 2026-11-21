//! The live preview of a md file (stub; a later task fills it, its parse kept per source hash).

use rune::El;

use super::super::{Ui, style};
use crate::theme::tokens::Theme;
use crate::ui::kit;

pub fn view(_source: &str, _ui: &Ui, t: &Theme) -> El {
    kit::label("live-md", "Preview", style::BANNER, t.text_secondary).padding(16.0)
}
