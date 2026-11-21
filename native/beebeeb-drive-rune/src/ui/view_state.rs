//! Loading, empty and failed states in one shape (stub: filled by drive-complete-019).
#![allow(dead_code, unused_variables)]

use crate::theme::tokens::Theme;
use rune::{El, el};

pub enum ViewState<'a> {
    Loading {
        words: &'a str,
    },
    Empty {
        key: &'a str,
        icon: &'static rune::Icon,
        title: &'a str,
        body: &'a str,
        action: Option<El>,
    },
    Failed {
        key: &'a str,
        title: &'a str,
        why: &'a str,
        retry: Option<El>,
    },
}

pub fn view_state(state: ViewState, t: &Theme) -> El {
    el()
}
