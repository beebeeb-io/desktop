//! The Files window, assembled (`.kit-windesktop` > `.mk-win`): caption, sidebar, toolbar,
//! list, details panel, status line.

use crate::State;
use crate::theme::tokens;
use rune::{Cx, El, col, row};

use super::{caption, detail_pane, file_list, sidebar, status_bar, toolbar};

pub fn files_window(s: &State, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());

    let mut body = row().key("list-body").grow(1.0).min_h(0.0).w_full().child(
        col()
            .key("file-list-col")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .child(file_list::column_headers(&t))
            // A folder that could not be read says so. It never draws as an empty folder.
            .child(match s.list_error {
                Some(error) => file_list::list_error(error, &t),
                None => file_list::file_list(s, cx, &t),
            }),
    );
    if s.show_detail_pane {
        let selected = s.selected.and_then(|i| s.listing.items.get(i));
        body = body.child(detail_pane::detail_pane(selected, &t));
    }

    let main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(toolbar::toolbar(s, &t, 0.0))
        .child(body)
        .child(status_bar::status_bar(&t));

    col()
        .key("window")
        .role("window")
        .label("Beebeeb Drive")
        .on_key(super::keys::on_key)
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption::caption_bar(&t, cx))
        .child(
            row()
                .key("body")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .child(sidebar::sidebar(s, &t))
                .child(main),
        )
}
