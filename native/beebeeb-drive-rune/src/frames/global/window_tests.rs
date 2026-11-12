//! Tests of the grid (DW-0119) and of the banner on the other views of the window: what a tile
//! shows before and after its thumbnail arrives, the view switch and the arrows from the keyboard
//! (scripted input), the pointer, roles and names; the banner on Shared and Trash, and at the
//! window's smallest size.

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::grid::{BoardThumbs, NoThumbs, ThumbSource, sub_of, thumb_of, tile_name, tile_size};
use super::tests::{NONE, find, focused, info, press, scene, settle, tab_to};
use crate::ui::grid_tiles::{Thumb, columns};
use crate::{State, init, view};

fn names(rt: &Runtime<State>) -> Vec<String> {
    rt.state.listing.items.iter().map(|i| i.name.clone()).collect()
}

fn click(rt: &mut Runtime<State>, key: &str, mods: Mods) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.physical_input(rune::PhysicalInput::Modifiers(mods));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt, 100.0);
    rt.physical_input(rune::PhysicalInput::Modifiers(NONE));
}

/// The board's frame: Lisbon trip as a grid in the board's order, every name and size there,
/// the pictures plain, the video with its length.
#[test]
fn the_grid_has_names_and_sizes_before_any_thumbnail() {
    let rt = scene("DW-0119");
    assert_eq!(
        names(&rt),
        [
            "Day 1",
            "Day 2",
            "Tickets.pdf",
            "Alfama.heic",
            "Belem.heic",
            "Tram 28.mov",
            "Sunset.heic"
        ]
    );
    let subs: Vec<String> = (0..7)
        .map(|i| info(&rt, &format!("grid-{i}-sub")).text.unwrap_or_default())
        .collect();
    assert_eq!(
        subs,
        ["48 items", "61 items", "220 KB", "3.4 MB", "4.1 MB", "82 MB", "3.8 MB"]
    );
    // A folder and a document show their glyph; a picture or a video shows nothing yet.
    for i in 0..3 {
        assert!(find(&rt, &format!("grid-{i}-icon")).is_some(), "tile {i}");
    }
    for i in 3..7 {
        assert!(find(&rt, &format!("grid-{i}-icon")).is_none(), "tile {i}");
    }
    assert_eq!(info(&rt, "grid-5-badge-text").text.as_deref(), Some("0:41"));
    assert!(find(&rt, "grid-6-badge").is_none());
    // No spinner, no loading state, and no list under the grid.
    assert!(find(&rt, "banner").is_none() && find(&rt, "column-headers").is_none());
    assert!(find(&rt, "row-0").is_none());
}

/// To assistive technology the grid is a list box named after the folder; a tile says its
/// name, what it is and its figures.
#[test]
fn the_grid_reads_as_a_list_box_of_named_tiles() {
    let rt = scene("DW-0119");
    let rows = info(&rt, "rows");
    assert_eq!((rows.role, rows.label.as_deref()), ("listbox", Some("Lisbon trip")));
    assert!(rows.focusable);
    let labels: Vec<String> = (0..7)
        .map(|i| {
            let tile = info(&rt, &format!("grid-{i}"));
            assert_eq!(tile.role, "option");
            tile.label.unwrap_or_default()
        })
        .collect();
    assert_eq!(labels[0], "Day 1, folder, 48 items");
    assert_eq!(labels[2], "Tickets.pdf, 220 KB");
    assert_eq!(labels[5], "Tram 28.mov, video, 0:41, 82 MB");
    let (list, grid) = (info(&rt, "view-list"), info(&rt, "view-grid"));
    assert_eq!((list.role, grid.role), ("radio", "radio"));
    assert_eq!(
        (list.label.as_deref(), grid.label.as_deref()),
        (Some("as List"), Some("as Grid"))
    );
}

/// The view switch from the keyboard: Space on "as List" shows the list, on "as Grid" the grid
/// again, and the keyboard lands in the folder's items each time.
#[test]
fn the_view_switch_works_from_the_keyboard() {
    let mut rt = scene("DW-0119");
    tab_to(&mut rt, "view-list");
    press(&mut rt, Key::Text(" ".into()));
    assert!(find(&rt, "grid").is_none(), "the list took the grid's place");
    assert_eq!(info(&rt, "rows").role, "table");
    assert!(find(&rt, "row-0").is_some() && find(&rt, "column-headers").is_some());
    assert_eq!(focused(&rt).as_deref(), Some("rows"));

    tab_to(&mut rt, "view-grid");
    press(&mut rt, Key::Text(" ".into()));
    assert!(find(&rt, "grid-0").is_some() && find(&rt, "row-0").is_none());
    assert_eq!(focused(&rt).as_deref(), Some("rows"));
}

/// "Each folder remembers its own view": back in Files it is a list, forward again a grid.
#[test]
fn each_folder_remembers_its_own_view() {
    let mut rt = scene("DW-0119");
    tab_to(&mut rt, "rows");
    press(&mut rt, Key::Backspace);
    assert!(rt.state.listing.folder.id.is_none(), "back in Files");
    assert!(find(&rt, "grid").is_none() && find(&rt, "row-0").is_some());
    assert!(!info(&rt, "view-grid").fill.a.gt(&0.0) && info(&rt, "view-list").fill.a > 0.0);
    click(&mut rt, "forward", NONE);
    assert_eq!(rt.state.listing.folder.name, "Lisbon trip");
    assert!(find(&rt, "grid-0").is_some() && find(&rt, "row-0").is_none());
}

/// The arrows: Left and Right one tile, Home and End the ends; with all seven in one row Up and
/// Down stay where they are.
#[test]
fn the_arrows_walk_the_tiles() {
    let mut rt = scene("DW-0119");
    tab_to(&mut rt, "rows");
    assert_eq!(rt.state.selected, None);
    press(&mut rt, Key::Right);
    assert_eq!(rt.state.selected, Some(0));
    press(&mut rt, Key::Right);
    press(&mut rt, Key::Right);
    assert_eq!(rt.state.selected, Some(2));
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Up);
    assert_eq!(rt.state.selected, Some(2), "one row of seven: nothing above or below");
    press(&mut rt, Key::Left);
    assert_eq!(rt.state.selected, Some(1));
    press(&mut rt, Key::End);
    assert_eq!(rt.state.selected, Some(6));
    press(&mut rt, Key::Right);
    assert_eq!(rt.state.selected, Some(6), "it stops at the end");
    press(&mut rt, Key::Home);
    assert_eq!(rt.state.selected, Some(0));
    // The selected tile is the list box's active one, and it is drawn selected.
    assert!(info(&rt, "grid-0").fill.a > 0.0 && info(&rt, "grid-1").fill.a == 0.0);
    // Shift extends the selection.
    rt.key(KeyEvent {
        key: Key::Right,
        mods: Mods { shift: true, ..NONE },
    });
    settle(&mut rt, 400.0);
    assert_eq!(rt.state.selected_rows(), [0, 1]);
}

/// In a narrower window the tiles wrap, and Up and Down move by a row of tiles.
#[test]
fn up_and_down_move_by_a_row_of_tiles() {
    assert_eq!((columns(1058.0), columns(480.0), columns(100.0)), (7, 3, 1));
    let mut state = init();
    super::grid::scene(&mut state);
    state.global.board_crop_bottom = false;
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    // 700 wide: the sidebar's 220 leaves 480, which holds three tiles side by side.
    let mut rt = Runtime::new(state, view, (700.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.frame(0.0);
    settle(&mut rt, 0.0);
    let (first, fourth) = (info(&rt, "grid-0").rect, info(&rt, "grid-3").rect);
    assert_eq!(first[0], fourth[0], "the fourth tile starts the second row");
    assert_eq!(fourth[1] - first[1], 160.0, "150 high, 10 between the rows");
    tab_to(&mut rt, "rows");
    press(&mut rt, Key::Right);
    press(&mut rt, Key::Down);
    assert_eq!(rt.state.selected, Some(3));
    press(&mut rt, Key::Down);
    assert_eq!(rt.state.selected, Some(6));
    press(&mut rt, Key::Down);
    assert_eq!(rt.state.selected, Some(6), "no row below");
    press(&mut rt, Key::Up);
    press(&mut rt, Key::Right);
    assert_eq!(rt.state.selected, Some(4));
}

/// The pointer: a click selects, Ctrl+click adds, a click on the switch keeps working.
#[test]
fn a_click_selects_a_tile() {
    let mut rt = scene("DW-0119");
    click(&mut rt, "grid-3-thumb", NONE);
    assert_eq!(rt.state.selected, Some(3));
    assert_eq!(focused(&rt).as_deref(), Some("rows"));
    click(
        &mut rt,
        "grid-5-thumb",
        Mods {
            cmd: true,
            ctrl: true,
            ..NONE
        },
    );
    assert_eq!(rt.state.selected_rows(), [3, 5]);
    // Space previews the selected file, as in the list (the Files window's key).
    click(&mut rt, "grid-2-thumb", NONE);
    assert_eq!(rt.state.selected_rows(), [2]);
    click(&mut rt, "view-list", NONE);
    assert!(find(&rt, "grid").is_none() && find(&rt, "row-2").is_some());
    assert_eq!(rt.state.selected, Some(2), "the selection stays with the item");
}

/// Thumbnails arrive one by one: only the tile whose thumbnail came in changes.
#[test]
fn a_tile_is_plain_until_its_thumbnail_arrives() {
    let rt = scene("DW-0119");
    let items = &rt.state.listing.items;
    let kind = |thumbs: &dyn ThumbSource, i: usize| match thumb_of(&items[i], thumbs) {
        Thumb::Glyph(_, size) => format!("glyph {size}"),
        Thumb::Plain => "plain".to_string(),
        Thumb::Picture => "picture".to_string(),
    };
    let none = BoardThumbs::default();
    let all: Vec<String> = (0..7).map(|i| kind(&none, i)).collect();
    assert_eq!(
        all,
        ["glyph 44", "glyph 44", "glyph 34", "plain", "plain", "plain", "plain"]
    );
    let one = BoardThumbs {
        arrived: vec!["belem".to_string()],
    };
    let all: Vec<String> = (0..7).map(|i| kind(&one, i)).collect();
    assert_eq!(
        all,
        ["glyph 44", "glyph 44", "glyph 34", "plain", "picture", "plain", "plain"]
    );
    // A thumbnail never replaces a folder's or a document's glyph.
    let odd = BoardThumbs {
        arrived: vec!["f-day1".to_string(), "tickets".to_string()],
    };
    assert_eq!(
        (kind(&odd, 0), kind(&odd, 2)),
        ("glyph 44".to_string(), "glyph 34".to_string())
    );
    // With no source of thumbnails every picture stays plain, and no length is invented.
    assert_eq!(kind(&NoThumbs, 5), "plain");
    assert_eq!(NoThumbs.video_length("tram28"), None);
    assert_eq!(none.video_length("tram28").as_deref(), Some("0:41"));
    assert_eq!(none.video_length("alfama"), None);
}

#[test]
fn the_figures_under_a_name() {
    assert_eq!(tile_size(220 * 1024), "220 KB");
    assert_eq!(tile_size(82 * 1024 * 1024), "82 MB");
    assert_eq!(tile_size((3.4 * 1024.0 * 1024.0) as i64), "3.4 MB");
    assert_eq!(tile_size(0), "0 B");
    let rt = scene("DW-0119");
    let items = &rt.state.listing.items;
    assert_eq!(sub_of(&items[0]), "48 items");
    let mut one = items[0].clone();
    one.child_count = Some(1);
    assert_eq!(sub_of(&one), "1 item");
    one.child_count = None;
    assert_eq!(sub_of(&one), "");
    assert_eq!(tile_name(&one, "", None), "Day 1, folder");
}

/// "The banner sits under the toolbar on every view": Shared and Trash show it too, its button
/// works from the keyboard there, and Trash's own note has the place while Trash empties.
#[test]
fn the_banner_shows_on_shared_and_trash_too() {
    use super::source::board;
    use super::{GlobalFixture, Intent, Ui};
    use crate::shell::app::Section;
    for section in [Section::Shared, Section::Trash] {
        let mut state = init();
        state.global = Ui::new(Box::new(GlobalFixture::of(board::sync_problem())));
        crate::frames::go(&mut state, section);
        let mut rt = super::tests::runtime_of(state);
        let banner = info(&rt, "banner");
        assert_eq!(banner.role, "status", "{section:?}");
        let toolbar = info(&rt, "toolbar").rect;
        assert!(banner.rect[1] >= toolbar[1] + toolbar[3], "under the toolbar");
        tab_to(&mut rt, "banner-action");
        press(&mut rt, Key::Enter);
        assert_eq!(rt.state.global.intents, [Intent::Frame("DW-0428")], "{section:?}");
    }
    // A quiet window draws none there.
    let mut state = init();
    crate::frames::go(&mut state, Section::Shared);
    assert!(find(&super::tests::runtime_of(state), "banner").is_none());
    // Trash while it empties (DW-0749b) keeps its own note: one at a time.
    let mut state = init();
    crate::frames::trash::scenes::dw_0749b(&mut state);
    state.global = Ui::new(Box::new(GlobalFixture::of(board::sync_problem())));
    let rt = super::tests::runtime_of(state);
    assert_eq!(info(&rt, "banner").label.as_deref(), Some("Emptying Trash"));
    assert!(find(&rt, "banner-action").is_none());
}

/// At the window's smallest size (960 x 640) every banner's sentence still fits on its line
/// beside its button: nothing is cut and nothing overlaps.
#[test]
fn every_sentence_fits_the_smallest_window() {
    use super::source::board;
    use super::{GlobalFixture, Ui};
    assert_eq!(crate::shell::surfaces::WindowKind::Main.min_size(), (960.0, 640.0));
    // The longest figures the sentences can carry beside the board's samples.
    let mut many = board::sync_problem();
    many.sync_problems = 1_000_000;
    for status in [
        board::offline(),
        board::sync_problem(),
        board::storage_almost_full(),
        board::storage_full(),
        board::signed_out(),
        board::backup_problem(),
        many,
    ] {
        let mut state = init();
        state.global = Ui::new(Box::new(GlobalFixture::of(status.clone())));
        let env = Env {
            system_dark: false,
            scale: 2.0,
        };
        let mut rt = Runtime::new(state, view, (960.0, 640.0), env);
        rt.frame(0.0);
        settle(&mut rt, 0.0);
        let banner = info(&rt, "banner").rect;
        let text = info(&rt, "banner-text-box").rect;
        let last = rt
            .elements()
            .into_iter()
            .filter(|e| {
                e.key
                    .as_deref()
                    .is_some_and(|k| k.starts_with("banner-text-run-") || k.starts_with("banner-figure-"))
            })
            .map(|e| e.rect[0] + e.rect[2])
            .fold(0.0_f32, f32::max);
        assert!(last > text[0], "{status:?}: the sentence is drawn");
        assert!(
            last <= text[0] + text[2] + 0.5,
            "{status:?}: the sentence ends at {last}, its box at {}",
            text[0] + text[2]
        );
        let limit = find(&rt, "banner-action")
            .or_else(|| find(&rt, "banner-dismiss"))
            .map_or(banner[0] + banner[2] - 10.0, |next| next.rect[0] - 10.0);
        assert!(
            last <= limit + 0.5,
            "{status:?}: the sentence ends at {last}, what follows starts at {limit}"
        );
    }
}

/// Space on a file that is only online, while there is no connection: the panel of DW-0121, not
/// the viewer. A file that is on this PC still opens in the viewer, and so does any file once
/// the connection is back.
#[test]
fn a_preview_without_a_connection_opens_the_panel() {
    use super::source::board;
    use super::{GlobalFixture, Ui};
    use crate::frames::files::Intent as FilesIntent;
    let offline_window = || {
        let mut state = init();
        state.global = Ui::new(Box::new(GlobalFixture::of(board::offline())));
        super::tests::runtime_of(state)
    };
    let row_of = |rt: &Runtime<State>, status: &str| {
        rt.state
            .listing
            .items
            .iter()
            .position(|i| !i.is_folder && i.pc_status.as_deref() == Some(status))
            .unwrap_or_else(|| panic!("the board has a file that is {status}"))
    };
    let mut rt = offline_window();
    let online_only = row_of(&rt, "online_only");
    let name = rt.state.listing.items[online_only].name.clone();
    click(&mut rt, &format!("row-{online_only}-date"), NONE);
    press(&mut rt, Key::Text(" ".into()));
    assert_eq!(
        info(&rt, "preview-card").label,
        Some(format!("{name}: Needs a connection"))
    );
    assert!(rt.state.actions.intents.is_empty(), "the viewer was not asked for");
    press(&mut rt, Key::Escape);
    assert!(find(&rt, "preview-card").is_none());

    let mut rt = offline_window();
    let here = row_of(&rt, "on_this_pc");
    click(&mut rt, &format!("row-{here}-date"), NONE);
    press(&mut rt, Key::Text(" ".into()));
    assert!(find(&rt, "preview-card").is_none(), "it is on this PC");
    assert_eq!(rt.state.actions.intents, [FilesIntent::Frame("DW-0408")]);

    let mut rt = super::tests::runtime_of(init());
    click(&mut rt, &format!("row-{online_only}-date"), NONE);
    press(&mut rt, Key::Text(" ".into()));
    assert!(find(&rt, "preview-card").is_none(), "there is a connection");
    assert_eq!(rt.state.actions.intents, [FilesIntent::Frame("DW-0408")]);
}
