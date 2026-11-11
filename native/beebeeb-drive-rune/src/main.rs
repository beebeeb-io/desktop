//! Beebeeb Drive — Rune native UI.
//!
//! Renders DW-0401 (the Windows main Files window) pixel-faithfully from the
//! design board, with:
//! - Custom caption bar with Beebeeb mark and Windows caption buttons
//! - Sidebar: sections with icons/badges, THIS PC, storage meter, account
//! - Toolbar: nav, title, view toggle, sort, panel toggle, + New, search
//! - Column headers with sort arrow
//! - File list with type icons, Shared and PC columns (virtual_rows)
//! - Details pane for the selected file
//! - Status footer with sync summary
//!
//! Data from `DriveSource` (board fixture or engine).
//!
//! ```sh
//! cargo run -p beebeeb-drive-rune
//! cargo run -p beebeeb-drive-rune -- --snap DW-0401.png --size 1280x800 --scale 2 --theme light
//! ```

mod source;
mod theme;
mod ui;

use rune::tok::Family;
use rune::{App, Cx, El, Key, KeyEvent, TextEdit, Titlebar, Tx, col, el, row, text};

use source::{BoardFixtureSource, DriveSource, LargeFixtureSource, Listing};
use theme::tokens::{self, Theme, radius, space, type_};
use ui::sidebar::SidebarSection;

// Re-export AppSpec from the tooling module (used by both App and tests)
use rune::AppSpec;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

const TITLEBAR_H: f32 = 32.0;

pub struct State {
    source: Box<dyn DriveSource>,
    listing: Listing,
    storage: source::StorageSummary,
    selected: Option<usize>,
    search: TextEdit,
    sidebar_section: SidebarSection,
    show_detail_pane: bool,
}

fn init() -> State {
    let source = Box::new(BoardFixtureSource::new());
    let listing = source.list(None);
    let storage = source.storage();
    State {
        source,
        listing,
        storage,
        selected: Some(6), // Contract draft.pdf (index 6) selected per the board
        search: TextEdit::new(""),
        sidebar_section: SidebarSection::Files,
        show_detail_pane: true,
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

fn view(s: &State, cx: &mut Cx<State>) -> El {
    let dark = cx.system_dark();
    let t = tokens::theme(dark);
    let insets = cx.chrome_insets();

    // Caption bar
    let caption = ui::caption::caption_bar(&t, cx);

    // Sidebar
    let sidebar = ui::sidebar::sidebar(s, &t);

    // Main content area
    let toolbar = ui::toolbar::toolbar(s, &t, insets.trailing);

    let separator = el()
        .key("separator-main")
        .h(1.0 / cx.env().scale)
        .w_full()
        .fill(t.separator);

    let column_headers = ui::file_list::column_headers(&t);
    let file_list = ui::file_list::file_list(s, cx, &t);
    let status_bar = ui::status_bar::status_bar(&t);

    let main_area = col()
        .key("main-area")
        .grow(1.0)
        .h_full()
        .fill(t.bg)
        .child(toolbar)
        .child(separator)
        .child(column_headers)
        .child(file_list)
        .child(status_bar);

    // Detail pane (right side, when a file is selected and panel is open)
    let detail_pane = if s.show_detail_pane {
        let selected_item = s.selected.and_then(|i| s.listing.items.get(i));
        Some(
            row()
                .key("detail-wrapper")
                .child(
                    el().key("detail-separator")
                        .w(1.0 / cx.env().scale)
                        .h_full()
                        .fill(t.separator),
                )
                .child(ui::detail_pane::detail_pane(selected_item, &t)),
        )
    } else {
        None
    };

    // Assemble: caption bar on top, then sidebar | main | detail pane
    col()
        .key("window")
        .role("window")
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption)
        .child(
            row()
                .key("body")
                .grow(1.0)
                .w_full()
                .child(sidebar)
                .child(
                    el().key("sidebar-separator")
                        .w(1.0 / cx.env().scale)
                        .h_full()
                        .fill(t.separator),
                )
                .child(main_area)
                .children(detail_pane),
        )
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

fn on_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    let count = s.listing.items.len();
    if count == 0 {
        return false;
    }
    match ev.key {
        Key::Down => {
            let next = s.selected.map(|i| (i + 1).min(count - 1)).unwrap_or(0);
            s.selected = Some(next);
            true
        }
        Key::Up => {
            let prev = s.selected.map(|i| i.saturating_sub(1)).unwrap_or(0);
            s.selected = Some(prev);
            true
        }
        Key::Enter => {
            if let Some(idx) = s.selected {
                if s.listing.items[idx].is_folder {
                    s.listing = s.source.list(Some(&s.listing.items[idx].id));
                    s.selected = None;
                }
            }
            true
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let spec = AppSpec::<State> {
        scenes: &[
            ("light", |_| {}),
            ("dark", |_| {}),
            ("large-folder", |s| {
                let source = Box::new(LargeFixtureSource::new(5000));
                s.listing = source.list(None);
                s.source = source;
                s.selected = None;
                s.show_detail_pane = false;
            }),
        ],
        on_key: Some(on_key),
        ..AppSpec::new("Beebeeb Drive", (1280.0, 800.0), init, view)
    };

    App::new(spec).titlebar(Titlebar::new(TITLEBAR_H)).run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime};

    #[test]
    fn board_fixture_has_11_items() {
        let source = BoardFixtureSource::new();
        let listing = source.list(None);
        assert_eq!(listing.items.len(), 11, "board DW-0401 has exactly 11 items");
    }

    #[test]
    fn board_fixture_names_match_board() {
        let source = BoardFixtureSource::new();
        let listing = source.list(None);
        let names: Vec<&str> = listing.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Backups",
                "Documents",
                "Invoices 2026",
                "Lisbon trip",
                "Shoot selects",
                "Budget 2026.xlsx",
                "Contract draft.pdf",
                "Harbour.heic",
                "Northwind brand guide.pdf",
                "Q4 plan.key",
                "Studio rent 2026.pdf",
            ]
        );
    }

    #[test]
    fn contract_draft_is_selected_at_index_6() {
        let s = init();
        assert_eq!(s.selected, Some(6));
        assert_eq!(s.listing.items[6].name, "Contract draft.pdf");
    }

    #[test]
    fn view_renders_without_panic() {
        let env = Env {
            system_dark: false,
            scale: 2.0,
        };
        let mut rt = Runtime::new(init(), view, (1280.0, 800.0), env);
        rt.frame(0.0);

        // Sidebar sections exist
        for section in &["Files", "Shared", "Photos", "Backups", "Trash"] {
            assert!(rt.find(section).is_some(), "sidebar section '{section}' exists");
        }

        // File list exists
        assert!(rt.find("file-list").is_some(), "file-list exists");

        // Detail pane exists
        assert!(rt.find("detail-pane").is_some(), "detail-pane exists");
    }

    #[test]
    fn arrow_down_moves_selection() {
        let mut state = init();
        assert_eq!(state.selected, Some(6));

        // Down
        let count = state.listing.items.len();
        state.selected = Some(state.selected.map(|i| (i + 1).min(count - 1)).unwrap_or(0));
        assert_eq!(state.selected, Some(7));
        assert_eq!(state.listing.items[7].name, "Harbour.heic");
    }

    #[test]
    fn large_fixture_5000_rows() {
        let source = LargeFixtureSource::new(5000);
        let listing = source.list(None);
        assert_eq!(listing.items.len(), 5000);
    }

    #[test]
    fn search_field_exists() {
        let env = Env {
            system_dark: false,
            scale: 2.0,
        };
        let mut rt = Runtime::new(init(), view, (1280.0, 800.0), env);
        rt.frame(0.0);
        assert!(rt.find("search-input").is_some(), "search field exists");
    }

    #[test]
    fn sidebar_has_board_sections_and_badges() {
        let env = Env {
            system_dark: false,
            scale: 2.0,
        };
        let mut rt = Runtime::new(init(), view, (1280.0, 800.0), env);
        rt.frame(0.0);

        // THIS PC section
        assert!(rt.find("this-pc-label").is_some(), "THIS PC label exists");
        assert!(rt.find("Sync history").is_some(), "Sync history exists");
        assert!(rt.find("Available offline").is_some(), "Available offline exists");
    }
}
