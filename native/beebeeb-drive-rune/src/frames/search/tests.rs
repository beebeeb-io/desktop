//! Search: names and roles per frame, the keyboard and the pointer, that no key reaches the Files
//! list behind the results, and that a window that is not a board frame never shows the board's
//! sample as the person's files.

use super::source::{Answer, BoardSearch, Loading, Narrow, SearchSource, Unwired, matched};
use super::{SearchUi, Shows, dw_0801, dw_0802, dw_0802b, dw_0803};
use crate::frames::Intent;
use crate::ui::keys::SEARCH_KEY;
use crate::{State, init, view};
use rune::{Env, Key, KeyEvent, Mods, Runtime};
use serde_json::Value;

const NONE: Mods = Mods {
    shift: false,
    cmd: false,
    alt: false,
    ctrl: false,
    logo: false,
};
const ALT: Mods = Mods { alt: true, ..NONE };
const CTRL: Mods = Mods {
    cmd: true,
    ctrl: true,
    ..NONE
};

/// A window on `state`, wired as `main` wires the app: `AppSpec::on_key` gets the keys that
/// arrive while nothing has focus, and those the focused element leaves.
fn window(state: State) -> Runtime<State> {
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    settle(&mut rt);
    rt
}

fn scene(scene: fn(&mut State)) -> Runtime<State> {
    let mut state = init();
    scene(&mut state);
    window(state)
}

/// Draw, and let what moves or leaves come to rest.
fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
}

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn text(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

/// What a screen reader gets for the element keyed `key`: its name, description and states.
fn node(rt: &Runtime<State>, key: &str) -> Value {
    fn find(v: &Value, key: &str) -> Option<Value> {
        if v["key"] == key {
            return Some(v.clone());
        }
        v["children"].as_array()?.iter().find_map(|c| find(c, key))
    }
    find(&rt.accessibility_json(), key).unwrap_or_else(|| panic!("no accessibility node keyed {key}"))
}

fn states(n: &Value) -> Vec<String> {
    n["states"]
        .as_array()
        .map(|all| all.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

/// The key of the element that has keyboard focus.
fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused && !e.exiting)
        .and_then(|e| e.key)
}

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    settle(rt);
}

fn type_text(rt: &mut Runtime<State>, words: &str) {
    for c in words.chars() {
        press(rt, Key::Text(c.to_string()), NONE);
    }
}

/// What the board's sample would put on screen as the person's: its results, a place, and its
/// recent searches.
const SAMPLE: [&str; 7] = [
    "Contract draft.pdf",
    "Contract signed 2025.pdf",
    "Contractors",
    "Documents \u{203A} Legal",
    "contract",
    "lisbon",
    "invoice 2026",
];

#[test]
fn names_and_roles_of_dw_0801_recent() {
    let rt = scene(dw_0801);
    assert_eq!(info(&rt, "search-box").role, "search");
    assert_eq!(info(&rt, SEARCH_KEY).label.as_deref(), Some("Search names"));
    let toolbar = info(&rt, "toolbar");
    assert_eq!((toolbar.role, toolbar.label.as_deref()), ("toolbar", Some("Search")));
    assert_eq!(text(&rt, "folder-title"), "Search");
    assert_eq!(text(&rt, "item-count"), "Files");
    let list = info(&rt, "list-0");
    assert_eq!((list.role, list.label.as_deref()), ("list", Some("Recent searches")));
    let first = info(&rt, "item-0-0");
    assert_eq!((first.role, first.label.as_deref()), ("button", Some("contract")));
    assert!(first.focusable);
    assert_eq!(node(&rt, "item-0-0")["description"], "Recent search");
    assert_eq!(info(&rt, "item-0-2").label.as_deref(), Some("invoice 2026"));
    let clear = info(&rt, "recent-clear");
    assert_eq!(
        (clear.role, clear.label.as_deref(), clear.focusable),
        ("button", Some("Clear recent searches"), true)
    );
    assert_eq!(text(&rt, "label-recent"), "RECENT");
    assert!(
        states(&node(&rt, "back")).contains(&"disabled".into())
            && states(&node(&rt, "forward")).contains(&"disabled".into()),
        "Esc is the way back: the board draws Back and Forward off"
    );
}

#[test]
fn names_and_roles_of_dw_0802_results() {
    let rt = scene(dw_0802);
    assert_eq!(text(&rt, "item-count"), "3 results");
    let pills = info(&rt, "seg-0");
    assert_eq!(
        (pills.role, pills.label.as_deref()),
        ("tablist", Some("Narrow the results"))
    );
    let all = info(&rt, "seg-0-all");
    assert_eq!((all.role, all.label.as_deref()), ("tab", Some("All")));
    assert!(states(&node(&rt, "seg-0-all")).contains(&"checked".into()));
    let rows = info(&rt, "rows");
    assert_eq!((rows.role, rows.label.as_deref()), ("list", Some("Results")));
    let row = info(&rt, "row-1");
    assert_eq!(
        (row.role, row.label.as_deref()),
        ("listitem", Some("Contract signed 2025.pdf"))
    );
    assert_eq!(node(&rt, "row-1")["description"], "In Documents \u{203A} Legal");
    assert!(states(&node(&rt, "row-0")).contains(&"selected".into()));
    assert_eq!(info(&rt, "col-pc-cell").label.as_deref(), Some("On this PC"));
    assert_eq!(text(&rt, "col-shared"), "Where");
    assert_eq!(text(&rt, "row-1-dash"), "Documents \u{203A} Legal");
    let pane = info(&rt, "detail-pane");
    assert_eq!((pane.role, pane.label.as_deref()), ("complementary", Some("Details")));
    assert_eq!(
        text(&rt, "detail-name"),
        "Contract draft.pdf",
        "the first result is the selected one"
    );
    let share = info(&rt, "share-box");
    assert_eq!((share.role, share.label.as_deref()), ("button", Some("Share\u{2026}")));
    let clear = info(&rt, "search-clear-btn");
    assert_eq!((clear.role, clear.label.as_deref()), ("button", Some("Clear search")));
    assert!(
        !has(&rt, "kbd-hint"),
        "words in the field: the hint gives way to the clear button"
    );
}

#[test]
fn names_and_roles_of_dw_0802b_loading() {
    let rt = scene(dw_0802b);
    assert_eq!(text(&rt, "item-count"), "3 results so far");
    let note = info(&rt, "loading-note");
    assert_eq!(note.role, "status");
    assert_eq!(
        note.label.as_deref(),
        Some("Still getting your files ready. Results may be incomplete until the list has loaded: 18,240 of 40,112.")
    );
    assert!(has(&rt, "row-2"));
    assert!(
        states(&node(&rt, "panel-toggle-btn")).contains(&"checked".into()) && !has(&rt, "detail-pane"),
        "the board draws no details panel while the list still loads"
    );
}

#[test]
fn names_and_roles_of_dw_0803_no_results() {
    let rt = scene(dw_0803);
    assert_eq!(text(&rt, "item-count"), "No results");
    assert_eq!(text(&rt, "empty-title"), "No results for \u{201C}harbr\u{201D}");
    assert_eq!(
        text(&rt, "empty-text"),
        "Check the spelling, or try fewer letters. Search only looks at names, not inside files."
    );
    assert!(has(&rt, "seg-0") && !has(&rt, "rows") && !has(&rt, "search-scope"));
}

/// A window on the engine (`state::init_on_engine` sets `SearchUi::default()`), and every window
/// of a build that ships (`first(false)`): it answers no query, and the window says search is not
/// available yet, also when what is typed would match the board's sample.
#[test]
fn a_window_that_ships_or_runs_on_the_engine_never_shows_the_sample_as_results() {
    assert!(!SearchUi::default().source.wired());
    assert!(!SearchUi::first(false).source.wired(), "a build that ships");
    assert!(
        SearchUi::first(true).source.wired(),
        "a development build without an engine"
    );
    // The engine path resets Search with the other areas that are not wired, whatever the build.
    let state_rs = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shell/state.rs"))
        .expect("shell/state.rs");
    let on_engine = state_rs.split("pub fn init_on_engine").nth(1).expect("init_on_engine");
    assert!(
        on_engine.contains("s.frames.find = crate::frames::search::SearchUi::default();"),
        "a window on the engine starts unwired"
    );
    // A source that says nothing about itself is not wired (task 2468).
    struct Silent;
    impl SearchSource for Silent {
        fn recent(&self) -> Vec<String> {
            vec!["contract".into()]
        }
        fn clear_recent(&mut self) {}
        fn remember(&mut self, _: &str) {}
        fn search(&self, _: &str, _: Narrow) -> Answer {
            BoardSearch::new().search("Contr", Narrow::All)
        }
        fn details(&self, _: &str) -> Option<crate::frames::files::source::Details> {
            None
        }
    }
    let silent = SearchUi {
        source: Box::new(Silent),
        ..SearchUi::default()
    };
    assert_eq!(silent.shows("Contr"), Shows::Unavailable);
    assert_eq!(silent.shows(""), Shows::Unavailable);
    for query in ["Contr", "contract", "lisbon", "a", ""] {
        for narrow in Narrow::ALL {
            assert_eq!(
                Unwired.search(query, narrow),
                Answer::Unavailable,
                "{query:?} {narrow:?}"
            );
        }
    }
    assert!(Unwired.recent().is_empty() && Unwired.details("board-search-1").is_none());

    // From Files, as the person does it: Ctrl+F, then the words.
    let mut state = init();
    state.frames.find = SearchUi::first(false);
    let mut rt = window(state);
    press(&mut rt, Key::Command('f'), CTRL);
    assert!(rt.state.frames.find.open, "Ctrl+F opens search");
    assert_eq!(focused(&rt).as_deref(), Some(SEARCH_KEY));
    assert_eq!(
        text(&rt, "empty-title"),
        "Search isn't available yet",
        "no recent searches either"
    );
    type_text(&mut rt, "Contr");
    assert_eq!(rt.state.search.text, "Contr");
    assert_eq!(text(&rt, "empty-title"), "Search isn't available yet");
    assert_eq!(text(&rt, "item-count"), "Not available yet");
    assert!(!has(&rt, "row-0") && !has(&rt, "list-0") && !has(&rt, "seg-0") && !has(&rt, "detail-pane"));
    let drawn: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.text)
        .collect();
    let heard = rt.accessibility_json().to_string();
    for sample in SAMPLE {
        assert!(!drawn.iter().any(|t| t == sample), "{sample:?} is drawn");
        assert!(!heard.contains(sample), "{sample:?} is in the accessibility tree");
    }
}

/// Only a board frame's scene makes the sample: no source file outside this group names it, so
/// no path of the running app (`state::init_on_engine`, `shell::run`) can put it in a window.
#[test]
fn only_this_groups_scenes_name_the_sample() {
    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("a source folder") {
            let path = entry.expect("a source entry").path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let ours = src.join("frames").join("search");
    let mut files = Vec::new();
    sources(&src, &mut files);
    assert!(
        files.len() > 100,
        "the walk reached the crate's sources: {} files",
        files.len()
    );
    assert!(files.iter().any(|p| p.ends_with("shell/state.rs")));
    let naming: Vec<_> = files
        .iter()
        .filter(|p| !p.starts_with(&ours))
        .filter(|p| {
            std::fs::read_to_string(p)
                .expect("a source file")
                .contains("BoardSearch")
        })
        .collect();
    assert!(naming.is_empty(), "the sample is named outside Search: {naming:?}");
}

#[test]
fn ctrl_f_opens_search_and_esc_takes_the_window_back_to_files() {
    let mut rt = window(init());
    assert!(has(&rt, "file-list-col"));
    press(&mut rt, Key::Command('f'), CTRL);
    assert_eq!(text(&rt, "folder-title"), "Search");
    assert!(!has(&rt, "file-list-col"), "search replaces the list");
    press(&mut rt, Key::Escape, NONE);
    assert!(!rt.state.frames.find.open && rt.state.search.text.is_empty());
    assert!(has(&rt, "file-list-col"), "Files' list is back");
    assert_ne!(text(&rt, "folder-title"), "Search");

    // The × in the field does the same.
    let mut rt = scene(dw_0802);
    click(&mut rt, "search-clear-btn");
    assert!(!super::is_open(&rt.state) && rt.state.search.text.is_empty());
    assert!(has(&rt, "file-list-col"));
}

/// The board: "Type: shows results". A word typed in the Files toolbar's field opens search, and
/// the field keeps the keyboard across the switch (both windows key it on the same path).
#[test]
fn typing_in_the_files_field_opens_search_and_keeps_the_keyboard() {
    let mut state = init();
    state.frames.find.source = Box::new(BoardSearch::new());
    let mut rt = window(state);
    click(&mut rt, SEARCH_KEY);
    assert!(has(&rt, "file-list-col"), "a click alone leaves the list");
    type_text(&mut rt, "contr");
    assert_eq!(
        focused(&rt).as_deref(),
        Some(SEARCH_KEY),
        "the keyboard stays in the field"
    );
    assert_eq!(rt.state.search.text, "contr", "every letter reached the field");
    assert_eq!(text(&rt, "item-count"), "3 results");
    assert_eq!(text(&rt, "row-2-name"), "Contractors");
}

#[test]
fn down_moves_into_the_results_and_enter_opens_the_selected_one() {
    let mut rt = scene(dw_0802);
    click(&mut rt, SEARCH_KEY);
    press(&mut rt, Key::Down, NONE);
    assert_eq!(focused(&rt).as_deref(), Some("row-0"));
    press(&mut rt, Key::Down, NONE);
    assert_eq!(focused(&rt).as_deref(), Some("row-1"));
    assert_eq!(rt.state.frames.find.selected.as_deref(), Some("board-search-2"));
    assert_eq!(text(&rt, "detail-name"), "Contract signed 2025.pdf");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(
        rt.state.shell.app.route().item.as_deref(),
        Some("board-search-2"),
        "Enter opens the selected result (DW-0408)"
    );
    assert!(!has(&rt, "results"), "the open item shows, not the results");
    press(&mut rt, Key::Left, ALT);
    assert_eq!(rt.state.shell.app.route().item, None, "Back leaves the item");
    assert_eq!(text(&rt, "item-count"), "3 results", "and the results are as they were");
    assert_eq!(rt.state.search.text, "Contr");
    assert_eq!(
        rt.state.frames.find.source.recent().first().map(String::as_str),
        Some("Contr"),
        "an opened result keeps its search as a recent one"
    );
    // Up from the first result goes back to the field.
    press(&mut rt, Key::Home, NONE);
    assert_eq!(focused(&rt).as_deref(), Some("row-0"));
    press(&mut rt, Key::Up, NONE);
    assert_eq!(focused(&rt).as_deref(), Some(SEARCH_KEY));
}

#[test]
fn the_pills_narrow_the_results() {
    let mut rt = scene(dw_0802);
    click(&mut rt, "seg-0-folders");
    assert_eq!(rt.state.frames.find.narrow, Narrow::Folders);
    assert_eq!(text(&rt, "item-count"), "1 result");
    assert_eq!(text(&rt, "row-0-name"), "Contractors");
    assert!(!has(&rt, "row-1"));
    // The keyboard: Right to Photos (nothing), Right to Documents.
    press(&mut rt, Key::Right, NONE);
    assert_eq!(rt.state.frames.find.narrow, Narrow::Photos);
    assert_eq!(focused(&rt).as_deref(), Some("seg-0-photos"));
    assert_eq!(text(&rt, "empty-title"), "No results for \u{201C}Contr\u{201D}");
    press(&mut rt, Key::Right, NONE);
    assert_eq!(rt.state.frames.find.narrow, Narrow::Documents);
    assert_eq!(text(&rt, "item-count"), "2 results");
}

#[test]
fn a_recent_search_runs_and_clear_removes_them() {
    let mut rt = scene(dw_0801);
    click(&mut rt, "item-0-0");
    assert_eq!(rt.state.search.text, "contract");
    assert_eq!(text(&rt, "item-count"), "3 results");
    assert_eq!(focused(&rt).as_deref(), Some(SEARCH_KEY));

    // From the field: Down into the list, Down to the second, Enter runs it.
    let mut rt = scene(dw_0801);
    click(&mut rt, SEARCH_KEY);
    press(&mut rt, Key::Down, NONE);
    assert_eq!(focused(&rt).as_deref(), Some("item-0-0"));
    press(&mut rt, Key::Down, NONE);
    assert_eq!(focused(&rt).as_deref(), Some("item-0-1"));
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(rt.state.search.text, "lisbon");
    assert_eq!(
        rt.state.frames.find.source.recent().first().map(String::as_str),
        Some("lisbon"),
        "a search that was run goes to the top"
    );

    let mut rt = scene(dw_0801);
    click(&mut rt, "recent-clear");
    assert!(rt.state.frames.find.source.recent().is_empty());
    assert!(!has(&rt, "list-0") && !has(&rt, "label-recent"));
    assert!(has(&rt, "search-covers"), "what search covers stays");
}

/// While the results show, no key reaches the Files list behind them: Delete there would move
/// Files' selected row to Trash, out of sight.
#[test]
fn no_key_reaches_the_files_list_while_searching() {
    let mut rt = scene(dw_0802);
    let files_selected = rt.state.selected;
    let files_rows = rt.state.listing.items.len();
    assert!(files_selected.is_some(), "Files has a selected row behind the results");
    // Nothing has focus: the keys go to the app's `on_key` (the registry's Files line).
    assert_eq!(focused(&rt), None);
    press(&mut rt, Key::Delete, NONE);
    press(&mut rt, Key::Down, NONE);
    assert_eq!(rt.state.listing.items.len(), files_rows, "nothing went to Trash");
    assert!(rt.state.actions.undo.is_empty() && rt.state.actions.toast.is_none());
    assert_eq!(rt.state.selected, files_selected, "Files' selection did not move");
    assert_eq!(
        rt.state.frames.find.selected.as_deref(),
        Some("board-search-2"),
        "Down moved the results' selection"
    );
}

#[test]
fn the_details_panel_is_the_selected_results_and_its_controls_ask_for_it() {
    let mut rt = scene(dw_0802);
    click(&mut rt, "row-2-date");
    assert_eq!(text(&rt, "detail-name"), "Contractors");
    click(&mut rt, "share-box");
    click(&mut rt, "versions-box");
    click(&mut rt, "offline-switch");
    assert_eq!(
        rt.state.frames.intents,
        vec![
            Intent::About("DW-0407", "board-search-3".into()),
            Intent::About("DW-0413", "board-search-3".into()),
            Intent::Do("keep-offline", "board-search-3".into()),
        ]
    );
    // Alt+Shift+P hides the panel, as in Files.
    press(
        &mut rt,
        Key::Text("P".into()),
        Mods {
            alt: true,
            shift: true,
            ..NONE
        },
    );
    assert!(!has(&rt, "detail-pane") && !rt.state.show_detail_pane);
}

#[test]
fn the_match_is_found_in_any_case_and_marked_by_its_bytes() {
    assert_eq!(matched("Contract draft.pdf", "contr"), Some((0, 5)));
    assert_eq!(matched("Contractors", "ACTOR"), Some((5, 10)));
    assert_eq!(matched("Übersicht 2026", "über"), Some((0, 5)), "Ü is two bytes");
    assert_eq!(matched("Plan für 2026", "FÜR"), Some((5, 9)));
    assert_eq!(
        matched("Contract draft.pdf", "  draft "),
        Some((9, 14)),
        "the query is trimmed"
    );
    assert_eq!(matched("Contractors", "harbr"), None);
    assert_eq!(matched("Contractors", " "), None);
}

#[test]
fn the_toolbar_says_what_was_found() {
    let Answer::Found { hits, .. } = BoardSearch::new().search("Contr", Narrow::All) else {
        panic!("the sample answers");
    };
    let loading = Some(Loading { listed: 1, total: 2 });
    let words = |shows: Shows| shows.count_words();
    assert_eq!(
        words(Shows::Results {
            hits: hits[..1].to_vec(),
            loading: None
        }),
        "1 result"
    );
    assert_eq!(
        words(Shows::Results {
            hits: hits.clone(),
            loading
        }),
        "3 results so far"
    );
    assert_eq!(words(Shows::Nothing { loading: None }), "No results");
    assert_eq!(words(Shows::Nothing { loading }), "No results so far");
    assert_eq!(words(Shows::Recent(Vec::new())), "Files");
    assert_eq!(words(Shows::Unavailable), "Not available yet");
}

/// The link, in a window nobody set up (a development build without an engine): a word typed in
/// the Files field shows DW-0802, a word with no match DW-0803, the emptied field DW-0801, and
/// Esc Files again.
#[test]
fn typing_in_a_window_goes_through_results_no_results_recent_and_back_to_files() {
    let mut rt = window(init());
    click(&mut rt, SEARCH_KEY);
    type_text(&mut rt, "Contr");
    assert_eq!(text(&rt, "folder-title"), "Search");
    assert_eq!(text(&rt, "item-count"), "3 results");
    assert_eq!(text(&rt, "row-0-name"), "Contract draft.pdf");
    assert_eq!(
        text(&rt, "row-1-dash"),
        "Documents \u{203A} Legal",
        "each row says where it is"
    );
    assert!(has(&rt, "seg-0-all") && !has(&rt, "empty-title"));
    // No match (DW-0803).
    for _ in 0.."Contr".len() {
        press(&mut rt, Key::Backspace, NONE);
    }
    assert_eq!(rt.state.search.text, "");
    // The field emptied by hand closes search, as before it was typed in: Files is back.
    assert!(has(&rt, "file-list-col"));
    type_text(&mut rt, "harbr");
    assert_eq!(text(&rt, "item-count"), "No results");
    assert_eq!(text(&rt, "empty-title"), "No results for \u{201C}harbr\u{201D}");
    assert!(!has(&rt, "row-0"));
    // Ctrl+F keeps search open over an empty field: the recent searches (DW-0801).
    press(&mut rt, Key::Command('f'), CTRL);
    for _ in 0.."harbr".len() {
        press(&mut rt, Key::Backspace, NONE);
    }
    assert_eq!(text(&rt, "item-count"), "Files");
    assert_eq!(text(&rt, "item-0-0-title"), "contract");
    press(&mut rt, Key::Escape, NONE);
    assert!(has(&rt, "file-list-col") && !super::is_open(&rt.state));
}

/// A double-click opens the result (DW-0802: "Double-click a result: opens it").
#[test]
fn a_double_click_opens_the_result() {
    let mut rt = scene(dw_0802);
    let r = info(&rt, "row-2").rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    for _ in 0..2 {
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
    }
    settle(&mut rt);
    assert_eq!(rt.state.shell.app.route().item.as_deref(), Some("board-search-3"));
}

/// The board draws Back and Forward off while searching: Alt+Left is not the window's Back, with
/// focus in the field, on a result or nowhere. Once a result is open, Back is Back again (the
/// test of Enter above).
#[test]
fn alt_left_while_searching_does_not_leave_the_place_behind_the_results() {
    use crate::shell::app::{Event, Section};
    let mut state = init();
    crate::shell::host::dispatch(
        &mut state,
        Event::Navigate(crate::shell::app::Route::section(Section::Trash)),
    );
    crate::shell::host::dispatch(
        &mut state,
        Event::Navigate(crate::shell::app::Route::section(Section::Files)),
    );
    assert!(state.shell.app.can_go_back());
    let mut rt = window(state);
    let before = rt.state.shell.app.route().clone();
    press(&mut rt, Key::Command('f'), CTRL);
    type_text(&mut rt, "Contr");
    for step in ["in the field", "on a result"] {
        press(&mut rt, Key::Left, ALT);
        assert_eq!(rt.state.shell.app.route(), &before, "{step}: Alt+Left");
        assert!(rt.state.shell.app.can_go_back(), "{step}: the history is untouched");
        assert!(!rt.state.shell.app.can_go_forward(), "{step}: nothing went back");
        press(&mut rt, Key::Right, ALT);
        assert_eq!(rt.state.shell.app.route(), &before, "{step}: Alt+Right");
        assert_eq!(text(&rt, "item-count"), "3 results", "{step}");
        press(&mut rt, Key::Down, NONE);
    }
    // Esc ends search, and Back is the window's again.
    press(&mut rt, Key::Escape, NONE);
    assert!(!super::is_open(&rt.state), "Esc ended search");
    // (The field keeps the keyboard after Esc, and Alt+Left in a field is the field's own.)
    click(&mut rt, "row-2");
    press(&mut rt, Key::Left, ALT);
    assert_eq!(rt.state.shell.app.route().section, Section::Trash);
}

/// Ctrl+F with focus inside the Files list opens search with its recent searches (DW-0801), not
/// only the cursor in the field.
#[test]
fn ctrl_f_from_inside_the_files_list_opens_search() {
    let mut rt = window(init());
    click(&mut rt, "row-2");
    assert!(focused(&rt).is_some_and(|k| k != SEARCH_KEY), "focus is in the list");
    press(&mut rt, Key::Command('f'), CTRL);
    assert!(rt.state.frames.find.open);
    assert_eq!(focused(&rt).as_deref(), Some(SEARCH_KEY));
    assert_eq!(text(&rt, "folder-title"), "Search");
    assert_eq!(text(&rt, "item-0-0-title"), "contract");
}

/// The toolbar's Open in File Explorer (the Files toolbar's button, DW-0401) asks the shell for
/// what the b's menu item does.
#[test]
fn the_toolbars_open_in_file_explorer_asks_the_shell() {
    use crate::shell::actions::{FixtureCall, FixtureEngine};
    let engine = std::sync::Arc::new(FixtureEngine::ready());
    let shell = crate::shell::host::Shell::fixture_on(engine.clone(), crate::shell::platform::current());
    let mut state = crate::shell::state::init_with(shell);
    dw_0802(&mut state);
    let mut rt = window(state);
    let button = info(&rt, "open-explorer");
    assert_eq!(button.role, "button");
    assert_eq!(button.label.as_deref(), Some("Open in File Explorer"));
    let before = engine.calls().len();
    click(&mut rt, "open-explorer");
    assert_eq!(&engine.calls()[before..], [FixtureCall::OpenInFileExplorer]);
}
