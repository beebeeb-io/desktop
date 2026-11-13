//! Tests of long content and thumbnails (DW-0420b, DW-0421), on the real 212-row virtual list:
//! the scroll, the letters that jump, the names shortened in the middle, and where a stand-in
//! picture may come from. Helpers are `tests.rs`'s.

use super::long_list::{BOARD_FIRST_ROW, LongTrip, NAME_MAX, TripThumbs, shortened, size_words, sort_key};
use super::source::FilesFixture;
use super::tests::*;
use super::{Actions, Intent};
use crate::State;
use crate::frames::global::grid::{self, NoThumbs, ThumbSource, thumb_of};
use crate::ui::grid_tiles::Thumb;
use crate::{init, load};
use rune::{Key, Runtime};

const LONG: &str = "Miradouro de Santa Luzia, the tiled terrace above Alfama at golden hour 0140.heic";

/// Lisbon trip's 212 items, opened by the app's own controls from Files.
fn long_trip() -> Runtime<State> {
    let mut state = init();
    state.source = Box::new(LongTrip::new());
    (state.listing, state.list_error) = load(state.source.as_ref(), None);
    state.actions = Actions::new(Box::new(FilesFixture::default()));
    state.global.grid.thumbs = Box::new(TripThumbs);
    state.show_detail_pane = false;
    state.clear_selection();
    let mut rt = runtime_of(state);
    click(&mut rt, "row-3-date");
    assert_eq!(rt.state.listing.items[3].name, "Lisbon trip");
    press(&mut rt, Key::Enter, NONE);
    settle(&mut rt);
    rt
}

fn in_view(rt: &Runtime<State>, row: usize) -> bool {
    let list = info(rt, "rows").rect;
    find(rt, &format!("row-{row}"))
        .is_some_and(|e| e.rect[1] >= list[1] - 0.5 && e.rect[1] + e.rect[3] <= list[1] + list[3] + 0.5)
}

fn scene(frame: &str) -> State {
    let mut state = init();
    let run = super::long_list::SCENES
        .iter()
        .find(|(id, _)| *id == frame)
        .unwrap_or_else(|| panic!("{frame} is a scene"))
        .1;
    run(&mut state);
    state
}

#[test]
fn a_name_over_58_characters_keeps_its_start_its_number_and_its_extension() {
    assert_eq!(NAME_MAX, 58);
    assert_eq!(
        shortened(LONG).as_deref(),
        Some("Miradouro de Santa Luzia, the ti\u{2026}t golden hour 0140.heic")
    );
    assert_eq!(
        shortened("Museu Nacional do Azulejo \u{2014} opening hours, tickets and the floor plan we used.pdf")
            .as_deref(),
        Some("Museu Nacional do Azulejo \u{2014} open\u{2026} floor plan we used.pdf")
    );
    let at_58 = "a".repeat(54) + ".pdf";
    assert_eq!(at_58.chars().count(), 58);
    assert_eq!(shortened(&at_58), None, "58 characters are written whole");
    assert!(shortened(&("b".to_string() + &at_58)).is_some(), "59 are not");
    // Characters, not bytes: an accented name is not cut inside a letter.
    let accented = "\u{e9}".repeat(60);
    assert_eq!(shortened(&accented).map(|s| s.chars().count()), Some(56));
}

#[test]
fn the_long_folder_lists_212_items_by_name_and_the_boards_rows_start_at_139() {
    let rt = long_trip();
    assert_eq!(rt.state.listing.items.len(), 212);
    assert_eq!(info(&rt, "item-count").text.as_deref(), Some("212 items"));
    assert_eq!(
        info(&rt, "status-details").text.as_deref(),
        Some("12 on this PC \u{00B7} 0 available offline \u{00B7} 200 online only")
    );
    let names = names(&rt);
    let mut sorted = names.clone();
    sorted.sort_by_key(|n| sort_key(n));
    assert_eq!(names, sorted, "sorted by name, accents left out");
    assert_eq!(names[BOARD_FIRST_ROW], "Miradouro da Gra\u{e7}a 0139.heic");
    assert_eq!(names[BOARD_FIRST_ROW + 31], "Wall tiles in Alfama 0169.heic");
    let at = |name: &str| names.iter().position(|n| n == name).unwrap();
    assert!(
        at("S\u{e9} Cathedral 0150.heic") < at("Sintra, Pena Palace 0151.heic"),
        "S\u{e9} stands before Sintra"
    );
    assert_eq!(
        size_words(112 * 1024 * 1024),
        "112 MB",
        "no .0 from 10 up, as the board writes sizes"
    );
    assert_eq!(size_words(4 * 1024 * 1024), "4.0 MB");
    assert_eq!(size_words(23_698_866), "22.6 MB");
}

#[test]
fn the_list_stays_lazy_and_end_and_home_scroll_it() {
    let mut rt = long_trip();
    // Only the rows in view (and a few around them) exist: never all 212.
    let realised = |rt: &Runtime<State>| {
        rt.elements()
            .iter()
            .filter(|e| e.role == "row" && e.key.as_deref().is_some_and(|k| k.starts_with("row-")))
            .count()
    };
    let first = realised(&rt);
    assert!((20..=90).contains(&first), "{first} rows realised of 212");
    click(&mut rt, "row-0-date");
    press(&mut rt, Key::End, NONE);
    settle(&mut rt);
    settle(&mut rt);
    assert_eq!(rt.state.selected, Some(211));
    assert!(in_view(&rt, 211), "End shows the last row");
    assert!(!in_view(&rt, 0));
    let after = realised(&rt);
    assert!((20..=90).contains(&after), "{after} rows realised after End");
    press(&mut rt, Key::Home, NONE);
    settle(&mut rt);
    settle(&mut rt);
    assert!(in_view(&rt, 0), "Home shows the first row");
}

#[test]
fn letters_jump_to_the_first_name_that_starts_with_them() {
    let mut rt = long_trip();
    click(&mut rt, "row-0-date");
    for letter in ["m", "i", "r"] {
        press(&mut rt, Key::Text(letter.into()), NONE);
    }
    settle(&mut rt);
    assert_eq!(rt.state.selected, Some(BOARD_FIRST_ROW), "the first Mir\u{2026}");
    assert!(in_view(&rt, BOARD_FIRST_ROW), "and it is scrolled into view");
    // The shortened name is drawn, and the row still says the whole name.
    let long = rt.state.listing.items.iter().position(|i| i.name == LONG).unwrap();
    assert_eq!(
        info(&rt, &format!("row-{long}-name")).text.as_deref(),
        Some("Miradouro de Santa Luzia, the ti\u{2026}t golden hour 0140.heic")
    );
    assert!(
        info(&rt, &format!("row-{long}"))
            .label
            .unwrap_or_default()
            .starts_with(LONG),
        "a screen reader hears the whole name"
    );
    // An accent is not needed to find a name: "se" finds Sé Cathedral.
    rt.state.actions.typed = Default::default();
    press(&mut rt, Key::Text("s".into()), NONE);
    press(&mut rt, Key::Text("e".into()), NONE);
    let row = rt.state.selected.unwrap();
    assert_eq!(rt.state.listing.items[row].name, "S\u{e9} Cathedral 0149.heic");
    // Letters no name starts with move nothing.
    rt.state.actions.typed = Default::default();
    press(&mut rt, Key::Text("q".into()), NONE);
    assert_eq!(rt.state.selected, Some(row));
    // Space is still Preview, not a letter.
    rt.state.actions.typed = Default::default();
    press(&mut rt, Key::Text(" ".into()), NONE);
    assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame("DW-0408")));
    assert_eq!(rt.state.selected, Some(row));
}

#[test]
fn the_scene_of_the_long_list_is_the_real_folder_drawn_from_row_139() {
    let rt = long_trip();
    let scene = scene("DW-0420b");
    assert_eq!(scene.actions.board_first_row, BOARD_FIRST_ROW);
    let ids = |s: &State| s.listing.items.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&scene), ids(&rt.state), "the same 212 items in the same order");
    assert_eq!(scene.selected.map(|i| scene.listing.items[i].name.as_str()), Some(LONG));
    // In the app the value is 0: every row is reachable.
    assert_eq!(rt.state.actions.board_first_row, 0);
    let mut drawn = runtime_of(scene);
    settle(&mut drawn);
    assert_eq!(
        info(&drawn, "row-0-name").text.as_deref(),
        Some("Miradouro da Gra\u{e7}a 0139.heic")
    );
    assert!(find(&drawn, "row-6-icon").is_some(), "a video row has its mark");
}

// ---- the grid with its thumbnails (DW-0421) ----

#[test]
fn the_view_switch_shows_lisbon_trip_as_a_grid_with_its_pictures() {
    let mut state = init();
    state.global.grid.thumbs = Box::new(TripThumbs);
    let mut rt = runtime_of(state);
    click(&mut rt, "row-3-date");
    press(&mut rt, Key::Enter, NONE);
    settle(&mut rt);
    assert!(find(&rt, "grid-0").is_none(), "a list first");
    // The list rows of the photos carry their pictures too, the video its mark.
    assert!(find(&rt, "row-6-icon").is_some(), "Tram 28.mov has the play mark");
    click(&mut rt, "view-grid");
    settle(&mut rt);
    assert!(grid::on(&rt.state));
    let names: Vec<String> = (0..7)
        .map(|i| info(&rt, &format!("grid-{i}-name")).text.unwrap_or_default())
        .collect();
    assert_eq!(
        names,
        [
            "Day 1",
            "Day 2",
            "Alfama.heic",
            "Belem.heic",
            "Sunset.heic",
            "Tickets.pdf",
            "Tram 28.mov"
        ],
        "folders first, then by name"
    );
    assert_eq!(info(&rt, "grid-6-badge-text").text.as_deref(), Some("0:41"));
    assert_eq!(
        info(&rt, "grid-6").label.as_deref(),
        Some("Tram 28.mov, video, 0:41, 82 MB")
    );
    // A click selects, Space previews, a right-click opens the file menu, Enter opens.
    click(&mut rt, "grid-2");
    assert_eq!(rt.state.selected, Some(2));
    press(&mut rt, Key::Text(" ".into()), NONE);
    assert_eq!(rt.state.actions.intents.last(), Some(&Intent::Frame("DW-0408")));
    // The preview is over the window (DW-0408; `actions_tests.rs`): Esc closes it.
    press(&mut rt, Key::Escape, NONE);
    right_click(&mut rt, "grid-4");
    assert!(rt.state.actions.menu.is_some(), "the file menu");
    press(&mut rt, Key::Escape, NONE);
    // The folder remembers its view; Files stays a list.
    press(&mut rt, Key::Left, ALT);
    settle(&mut rt);
    assert!(!grid::on(&rt.state) && find(&rt, "row-0").is_some());
    // The scene is this state with Alfama selected.
    let scene = scene("DW-0421");
    assert!(grid::on(&scene));
    assert_eq!(
        scene.selected.map(|i| scene.listing.items[i].id.as_str()),
        Some("alfama")
    );
    assert_eq!(scene.listing.items.len(), 7);
}

#[test]
fn a_stand_in_picture_comes_only_from_a_fixture_source() {
    let state = init();
    let trip = super::source::BoardDrive::new();
    let items = crate::source::DriveSource::list(&trip, Some("f-lisbon")).unwrap().items;
    let photo = items.iter().find(|i| i.id == "alfama").unwrap();
    // The default body: a source that does not write `sample` names none.
    struct Arrived;
    impl ThumbSource for Arrived {
        fn arrived(&self, _: &str) -> bool {
            true
        }
        fn video_length(&self, _: &str) -> Option<String> {
            None
        }
    }
    assert_eq!(Arrived.sample("alfama"), None);
    assert!(matches!(thumb_of(photo, &Arrived), Thumb::Picture));
    assert_eq!(NoThumbs.sample("alfama"), None);
    assert!(matches!(thumb_of(photo, &NoThumbs), Thumb::Plain));
    // The window starts on a source without samples (what a shipped build keeps): no row and
    // no tile of it is a stand-in picture.
    assert_eq!(state.global.grid.thumbs.sample("alfama"), None);
    assert_eq!(crate::frames::global::Ui::default().grid.thumbs.sample("alfama"), None);
    for item in &items {
        assert!(!matches!(
            thumb_of(item, state.global.grid.thumbs.as_ref()),
            Thumb::Sample(_)
        ));
    }
    // Only the fixture names one.
    assert!(matches!(thumb_of(photo, &TripThumbs), Thumb::Sample(1)));
    let samples: Vec<Option<u8>> = ["alfama", "belem", "tram28", "sunset", "tickets"]
        .iter()
        .map(|id| TripThumbs.sample(id))
        .collect();
    assert_eq!(samples, [Some(1), Some(2), Some(3), Some(4), None]);
}

#[test]
fn letters_jump_in_a_grid_too() {
    let mut rt = long_trip();
    rt.update(|s, _| grid::set(s, true));
    settle(&mut rt);
    click(&mut rt, "grid-0");
    let tile = format!("grid-{BOARD_FIRST_ROW}");
    let seen = |rt: &Runtime<State>| {
        find(rt, &tile).is_some_and(|e| e.rect[1] >= 0.0 && e.rect[1] + e.rect[3] <= 798.0 - 28.0)
    };
    assert!(!seen(&rt), "the tile starts out of view");
    for letter in ["m", "i", "r"] {
        press(&mut rt, Key::Text(letter.into()), NONE);
    }
    settle(&mut rt);
    assert_eq!(rt.state.selected, Some(BOARD_FIRST_ROW), "the first Mir\u{2026}");
    assert!(seen(&rt), "and its tile is scrolled into view");
}
