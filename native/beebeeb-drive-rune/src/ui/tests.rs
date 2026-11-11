//! Tests of the Files window: structure, keyboard, accessibility names.

use crate::{State, init, view};
use rune::{Env, Runtime};

fn runtime(dark: bool) -> Runtime<State> {
    let mut rt = Runtime::new(
        init(),
        view,
        (1278.0, 798.0),
        Env {
            system_dark: dark,
            scale: 2.0,
        },
    );
    rt.frame(0.0);
    rt
}

#[test]
fn the_window_has_its_regions_in_both_themes() {
    for dark in [false, true] {
        let rt = runtime(dark);
        for key in [
            "caption-bar",
            "sidebar",
            "toolbar",
            "column-headers",
            "rows",
            "detail-pane",
            "status-bar",
        ] {
            assert!(rt.find(key).is_some(), "{key} exists (dark: {dark})");
        }
    }
}
