//! Tests of the list offline (DW-1108) and during the first download (DW-1107).

use super::browse_states::ListedSoFar;
use super::source::{BoardDrive, Call};
use super::tests::*;
use super::{Intent, Toast};
use crate::State;
use crate::frames::global::source::board;
use crate::frames::global::{GlobalFixture, Ui};
use crate::init;
use rune::{Key, Runtime};

fn offline() -> (Runtime<State>, Calls) {
    let mut state = init();
    state.global = Ui::new(Box::new(GlobalFixture::of(board::offline())));
    state.show_detail_pane = false;
    let (rt, calls, _) = spied_on(state);
    (rt, calls)
}

fn text(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

#[test]
fn offline_only_files_that_are_not_on_this_pc_are_dimmed_and_say_why() {
    let (rt, _) = offline();
    let items = rt.state.listing.items.clone();
    let mut dimmed = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let row = info(&rt, &format!("row-{i}"));
        let needs = text(&rt, &format!("row-{i}-date")) == "Needs a connection";
        assert_eq!(row.opacity < 0.5, needs, "{}: dimmed and worded together", item.name);
        // True of the row: a file, only online.
        assert_eq!(
            needs,
            !item.is_folder && item.pc_status.as_deref() == Some("online_only"),
            "{}",
            item.name
        );
        if needs {
            dimmed.push(item.name.clone());
        }
    }
    assert_eq!(
        dimmed,
        ["Contract draft.pdf"],
        "the one online-only file of the board's list"
    );
    // Online-only FOLDERS are not dimmed: they are browsed from the list on this PC.
    assert_eq!(items[3].pc_status.as_deref(), Some("online_only"));
    assert!(info(&rt, "row-3").opacity > 0.99);
    assert_eq!(text(&rt, "status-text"), "Offline");
    assert_eq!(
        text(&rt, "status-details"),
        "4 on this PC \u{00B7} 3 available offline \u{00B7} 4 need a connection"
    );
    let count = |status: &str| items.iter().filter(|i| i.pc_status.as_deref() == Some(status)).count();
    assert_eq!(
        (count("on_this_pc"), count("available_offline"), count("online_only")),
        (4, 3, 4),
        "the line counts the rows"
    );
    assert!(
        info(&rt, "banner")
            .label
            .unwrap_or_default()
            .starts_with("You're offline.")
    );
}

#[test]
fn offline_a_dimmed_file_says_it_needs_a_connection_and_the_rest_opens() {
    let (mut rt, calls) = offline();
    // Contract draft.pdf (online only): Enter does not ask the source; the panel says why (DW-0121).
    click(&mut rt, "row-6-date");
    press(&mut rt, Key::Enter, NONE);
    assert!(asked(&calls).is_empty(), "{:?}", asked(&calls));
    assert_eq!(
        rt.state.global.blocked_preview,
        Some(("contract".to_string(), "Contract draft.pdf".to_string()))
    );
    assert!(rt.state.actions.toast.is_none());
    press(&mut rt, Key::Escape, NONE);
    // Harbour.heic is on this PC: it opens.
    click(&mut rt, "row-7-date");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(asked(&calls), [Call::Open(ids(&["harbour"]))]);
    // A folder that is only online is still browsed (the list is on this PC).
    click(&mut rt, "row-3-date");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(rt.state.listing.folder.name, "Lisbon trip");
    assert!(rt.state.global.blocked_preview.is_none());
    // In there, Tickets.pdf is on this PC and the photos are not.
    let inside = rt.state.listing.items.clone();
    let dim: Vec<&str> = (0..inside.len())
        .filter(|i| info(&rt, &format!("row-{i}")).opacity < 0.5)
        .map(|i| inside[i].name.as_str())
        .collect();
    let online_only: Vec<&str> = inside
        .iter()
        .filter(|i| !i.is_folder && i.pc_status.as_deref() == Some("online_only"))
        .map(|i| i.name.as_str())
        .collect();
    assert_eq!(dim, online_only);
    assert_eq!((dim.len(), inside.len()), (4, 7));
    assert!(!dim.contains(&"Tickets.pdf"));
}

#[test]
fn with_a_connection_nothing_is_dimmed() {
    let (rt, _, _) = spied();
    for i in 0..11 {
        assert!(info(&rt, &format!("row-{i}")).opacity > 0.99);
    }
    assert_eq!(text(&rt, "status-text"), "Up to date");
    assert!(text(&rt, "status-details").ends_with("4 online only"));
}

fn first_download() -> (Runtime<State>, Calls) {
    let mut state = init();
    state.source = Box::new(ListedSoFar(BoardDrive::new()));
    (state.listing, state.list_error) = crate::load(state.source.as_ref(), None);
    state.global = Ui::new(Box::new(GlobalFixture::of(board::loading())));
    state.show_detail_pane = false;
    state.clear_selection();
    let (rt, calls, _) = spied_on(state);
    (rt, calls)
}

#[test]
fn the_first_download_says_what_happens_here_and_the_listed_rows_work() {
    let (mut rt, calls) = first_download();
    assert_eq!(text(&rt, "ready-title"), "Getting your files ready");
    assert_eq!(
        text(&rt, "ready-text"),
        "Your file list is downloading and being decrypted on this PC. You can start using Beebeeb before it's finished."
    );
    assert_eq!(
        [text(&rt, "ready-figure"), text(&rt, "ready-left")],
        ["18,240 of 40,112", " \u{00B7} about 1 min left"]
    );
    assert_eq!(
        [text(&rt, "ready-list-value"), text(&rt, "ready-thumbs-value")],
        ["In progress", "Waiting for the file list"]
    );
    let bar = info(&rt, "ready-bar");
    assert_eq!(
        (bar.role, bar.label.as_deref()),
        ("progressbar", Some("Getting your files ready: 45 percent"))
    );
    let (whole, filled) = (bar.rect[2], info(&rt, "ready-bar-fill").rect[2]);
    assert!((filled / whole - 0.45).abs() < 0.01, "{filled} of {whole}");
    assert_eq!(text(&rt, "status-text"), "Getting your files ready");
    assert_eq!(text(&rt, "status-details"), "18,240 of 40,112 listed");
    assert_eq!(rt.state.listing.items.len(), 7, "the rows listed so far");
    assert!(find(&rt, "banner").is_none(), "the card, not the strip");
    // A listed file works at once; then the card gives way to the strip (DW-0118).
    click(&mut rt, "row-5-date");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(asked(&calls), [Call::Open(ids(&["budget"]))]);
    assert!(rt.state.actions.toast != Some(Toast::LinkCopied));
    assert!(find(&rt, "ready-card").is_none(), "the card is gone");
    assert!(
        info(&rt, "banner")
            .label
            .unwrap_or_default()
            .starts_with("Getting your files ready"),
        "the strip took its place"
    );
    // The status line leads to the b's flyout, which shows the same progress.
    click(&mut rt, "status-bar");
    assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame("DW-1201")));
}

#[test]
fn no_card_when_nothing_loads_and_the_scenes_are_these_states() {
    let (rt, _, _) = spied();
    assert!(find(&rt, "ready-card").is_none());
    let scene = |frame: &str| {
        let mut state = init();
        let run = super::browse_states::SCENES
            .iter()
            .find(|(id, _)| *id == frame)
            .unwrap()
            .1;
        run(&mut state);
        state
    };
    let off = scene("DW-1108");
    assert!(super::browse_states::offline(&off) && off.listing.items.len() == 11);
    let first = scene("DW-1107");
    assert!(first.global.source.status().loading.is_some());
    assert_eq!(first.listing.items.len(), 7);
    assert!(!first.actions.first_load_strip);
    assert_eq!(super::browse_states::SCENES.len(), 3);
}

#[test]
fn a_folder_keeps_its_own_view_and_the_panel_follows_the_selection() {
    use crate::frames::global::grid;
    let mut state = init();
    let scene = super::browse_states::SCENES
        .iter()
        .find(|(id, _)| *id == "DW-1102")
        .unwrap()
        .1;
    scene(&mut state);
    let (mut rt, _, _) = spied_on(state);
    // Lisbon trip: a grid, the panel on the selected photo.
    assert_eq!(rt.state.listing.folder.name, "Lisbon trip");
    assert!(grid::on(&rt.state) && rt.state.show_detail_pane);
    assert!(find(&rt, "row-0-date").is_none(), "no list row in a grid");
    let panel: Vec<String> = rt.elements().into_iter().filter_map(|e| e.text).collect();
    assert_eq!(
        panel.iter().filter(|t| t.as_str() == "Alfama.heic").count(),
        2,
        "the tile and the panel"
    );
    // Up to Files: that folder is still a list. The panel stays open, as the person left it.
    rt.update(|s, _| {
        s.back();
    });
    settle(&mut rt);
    assert_eq!(rt.state.listing.folder.id, None);
    assert!(!grid::on(&rt.state) && rt.state.show_detail_pane);
    assert!(find(&rt, "row-0-date").is_some());
    // Back in: the grid again, without asking.
    rt.update(|s, _| {
        s.forward();
    });
    settle(&mut rt);
    assert!(grid::on(&rt.state));
    assert!(find(&rt, "row-0-date").is_none());
    // Switching this folder to a list leaves the other as it is, and the reverse.
    rt.update(|s, _| grid::set(s, false));
    settle(&mut rt);
    assert!(find(&rt, "row-0-date").is_some());
}
