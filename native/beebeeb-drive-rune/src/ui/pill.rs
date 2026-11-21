//! The one status pill (stub: filled by drive-complete-020).
#![allow(dead_code, unused_variables)]

use crate::theme::tokens::Theme;
use rune::{El, el};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Good,
    Warn,
    Bad,
    Accent,
}

pub fn pill(key: &str, words: &str, tone: Tone, dot: bool, t: &Theme) -> El {
    el()
}
