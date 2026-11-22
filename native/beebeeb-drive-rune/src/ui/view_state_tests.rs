//! Each state of the one Loading / Empty / Failed view, rendered in a runtime: its keys, its role
//! and name, its words, its live region, and the retry press.

use rune::{Cx, El, ElementInfo, Env, Runtime, row};
use serde_json::Value;

use super::*;
use crate::theme::icons;
use crate::theme::tokens::theme;

#[derive(Clone, Copy, PartialEq)]
enum Which {
    Loading,
    Empty { action: bool },
    Failed { retry: bool },
}

struct Page {
    which: Which,
    retried: u32,
}

fn view(p: &Page, _: &mut Cx<Page>) -> El {
    let t = theme(false);
    let state = match p.which {
        Which::Loading => ViewState::Loading {
            words: "Opening your Drive",
        },
        Which::Empty { action } => ViewState::Empty {
            key: "none",
            icon: &icons::PLUS,
            title: "Nothing here yet",
            body: "Drop files here to add them.",
            action: action.then(|| button::outline_button("none-add", "Add files", None, &t)),
        },
        Which::Failed { retry } => ViewState::Failed {
            key: "oops",
            title: "Could not load this folder",
            why: "The server did not answer.",
            retry: retry.then(|| try_again("oops", &t).on_press(|p: &mut Page| p.retried += 1)),
        },
    };
    row().key("page").w(400.0).child(view_state(state, &t))
}

fn render(which: Which) -> Runtime<Page> {
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(Page { which, retried: 0 }, view, (400.0, 400.0), env);
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    rt
}

fn find(rt: &Runtime<Page>, key: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn info(rt: &Runtime<Page>, key: &str) -> ElementInfo {
    find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn node<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    if v["key"] == key {
        return Some(v);
    }
    v["children"].as_array()?.iter().find_map(|c| node(c, key))
}

fn states(rt: &Runtime<Page>, key: &str) -> Vec<String> {
    let tree = rt.accessibility_json();
    let n = node(&tree, key).unwrap_or_else(|| panic!("no node keyed {key}: {tree:#}"));
    n["states"].as_array().map_or(Vec::new(), |s| {
        s.iter().filter_map(|v| v.as_str().map(String::from)).collect()
    })
}

#[test]
fn loading_is_the_strip_with_its_words_and_a_busy_bar() {
    let rt = render(Which::Loading);
    let root = info(&rt, "loading");
    assert_eq!(root.role, "status");
    assert_eq!(root.label.as_deref(), Some("Opening your Drive"));
    assert_eq!(root.radius, 10.0, "the strip's radius");
    assert_eq!(info(&rt, "loading-words").text.as_deref(), Some("Opening your Drive"));
    let bar = info(&rt, "loading-bar");
    assert_eq!(bar.role, "progressbar");
    assert_eq!(bar.rect[3], 4.0, "the strip's bar height");
    info(&rt, "loading-bar-fill");
    assert!(states(&rt, "loading").iter().any(|s| s == "live-polite"));
}

#[test]
fn empty_has_tile_icon_title_text_and_the_action_only_when_given() {
    for action in [false, true] {
        let rt = render(Which::Empty { action });
        let root = info(&rt, "none");
        assert_eq!(root.role, "group");
        assert_eq!(root.label.as_deref(), Some("Nothing here yet"));
        let tile = info(&rt, "none-tile");
        assert_eq!((tile.rect[2], tile.rect[3], tile.radius), (52.0, 52.0, 15.0));
        info(&rt, "none-icon");
        assert_eq!(info(&rt, "none-title").text.as_deref(), Some("Nothing here yet"));
        assert_eq!(
            info(&rt, "none-text").text.as_deref(),
            Some("Drop files here to add them.")
        );
        assert_eq!(find(&rt, "none-add").is_some(), action, "action drawn: {action}");
        if action {
            assert!(
                info(&rt, "none-add").rect[1] > info(&rt, "none-text").rect[1],
                "the action sits under the text"
            );
        }
    }
}

#[test]
fn empty_state_draws_through_view_state_with_the_same_keys() {
    let mut rt = Runtime::new(
        (),
        |_: &(), _: &mut Cx<()>| {
            super::super::empty::empty_state("e", &icons::PLUS, "Empty", "No files.", &theme(false))
        },
        (400.0, 400.0),
        Env::new(false, 2.0),
    );
    rt.frame(0.0);
    let keys: Vec<String> = rt.elements().into_iter().filter_map(|e| e.key).collect();
    for k in ["e", "e-tile", "e-icon", "e-title", "e-text"] {
        assert!(keys.iter().any(|x| x == k), "{k} missing from {keys:?}");
    }
}

#[test]
fn failed_has_title_reason_try_again_and_a_polite_live_region() {
    let mut rt = render(Which::Failed { retry: true });
    let root = info(&rt, "oops");
    assert_eq!(root.label.as_deref(), Some("Could not load this folder"));
    assert_eq!(
        info(&rt, "oops-title").text.as_deref(),
        Some("Could not load this folder")
    );
    assert_eq!(
        info(&rt, "oops-text").text.as_deref(),
        Some("The server did not answer.")
    );
    let retry = info(&rt, "oops-retry");
    assert_eq!(retry.role, "button");
    assert_eq!(retry.label.as_deref(), Some(TRY_AGAIN));
    assert!(retry.focusable, "Try again takes focus");
    assert_eq!(info(&rt, "oops-retry-label").text.as_deref(), Some(TRY_AGAIN));
    assert!(states(&rt, "oops").iter().any(|s| s == "live-polite"));

    let r = retry.rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    let mut retried = 0;
    rt.update(|p, _| retried = p.retried);
    assert_eq!(retried, 1, "Try again runs the caller's press");

    let rt = render(Which::Failed { retry: false });
    assert!(find(&rt, "oops-retry").is_none());
}

#[test]
fn sized_view_state_takes_the_loading_prefix_and_a_compact_tile() {
    fn v(_: &Page, _: &mut Cx<Page>) -> El {
        let t = theme(false);
        rune::col()
            .key("page")
            .w(400.0)
            .child(view_state_sized(
                ViewState::Loading { words: "Wait" },
                "set-load",
                Size::Sm,
                &t,
            ))
            .child(view_state_sized(
                ViewState::Empty {
                    key: "e",
                    icon: &icons::PLUS,
                    title: "T",
                    body: "B",
                    action: None,
                },
                "x",
                Size::Sm,
                &t,
            ))
    }
    let mut rt = Runtime::new(
        Page {
            which: Which::Loading,
            retried: 0,
        },
        v,
        (400.0, 600.0),
        Env {
            system_dark: false,
            scale: 2.0,
        },
    );
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    for k in ["set-load", "set-load-words", "set-load-bar", "set-load-bar-fill"] {
        assert!(find(&rt, k).is_some(), "{k}");
    }
    assert!(find(&rt, "loading").is_none());
    assert_eq!(find(&rt, "e-tile").expect("tile").rect[2], 40.0);
}
