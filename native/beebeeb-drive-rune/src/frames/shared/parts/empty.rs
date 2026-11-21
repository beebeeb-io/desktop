//! An empty state: drawn by `ui::view_state` (Empty). Keys: `empty`, `empty-tile`, `empty-icon`,
//! `empty-title`, `empty-text`.

use crate::theme::tokens::Theme;
use crate::ui::view_state::{ViewState, view_state};
use rune::{El, Icon, el};

pub fn empty(ic: &'static Icon, title: &str, sub: &str, action: Option<El>, t: &Theme) -> El {
    let body = view_state(
        ViewState::Empty {
            key: "empty",
            icon: ic,
            title,
            body: sub,
            action,
        },
        t,
    );
    el().key("empty-area").center().child(body)
}
