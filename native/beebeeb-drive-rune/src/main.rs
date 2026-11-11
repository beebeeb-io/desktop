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

use rune::{App, Cx, El, Key, KeyEvent, TextEdit, Titlebar, Tx};

use source::{BoardFixtureSource, DriveSource, LargeFixtureSource, Listing};
use ui::sidebar::SidebarSection;

// Re-export AppSpec from the tooling module (used by both App and tests)
use rune::AppSpec;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Titlebar height in points — the single source of truth used by both
/// `App::titlebar` and `ui::caption::caption_bar`.
/// Board: `.kit-wincap { height: 32px }`.
pub const TITLEBAR_H: f32 = 32.0;

pub struct State {
    source: Box<dyn DriveSource>,
    listing: Listing,
    storage: source::StorageSummary,
    selected: Option<usize>,
    search: TextEdit,
    sidebar_section: SidebarSection,
    show_detail_pane: bool,
    /// Menus, multi-selection and inline rename of the Files window (`ui::FilesUi`).
    files: ui::FilesUi,
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
        files: ui::FilesUi::default(),
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

fn view(s: &State, cx: &mut Cx<State>) -> El {
    ui::window::files_window(s, cx)
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

// ---------------------------------------------------------------------------
// Fonts: Inter (sans) + JetBrains Mono (mono), bundled as TTF
// ---------------------------------------------------------------------------

const FAMILIES: (&str, &str) = ("Inter", "JetBrains Mono");

static FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/inter-latin-400-normal.ttf"),
    include_bytes!("../assets/fonts/inter-latin-500-normal.ttf"),
    include_bytes!("../assets/fonts/inter-latin-600-normal.ttf"),
    include_bytes!("../assets/fonts/inter-latin-700-normal.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono-latin-400-normal.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono-latin-500-normal.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono-latin-600-normal.ttf"),
];

/// Named states of the window: `--snap out.png --scene DW-0401`, and the frame gate
/// (`scripts/compare.sh DW-0401`). One per board frame, by its frame id.
const SCENES: &[(&str, fn(&mut State))] = &[
    ("light", |_| {}),
    ("dark", |_| {}),
    ("large-folder", |s| {
        let source = Box::new(LargeFixtureSource::new(5000));
        s.listing = source.list(None);
        s.source = source;
        s.selected = None;
        s.show_detail_pane = false;
    }),
    ("DW-0401", |_| {}),
];

fn main() {
    // `--gate-export out.json --scene DW-0401 [--theme dark] [--size 1278x798]`: the frame gate's native half.
    if let Some(code) = ui::gate::run_from_args(SCENES, init, view, FONTS, FAMILIES) {
        std::process::exit(code);
    }
    let spec = AppSpec::<State> {
        fonts: FONTS,
        families: FAMILIES,
        scenes: SCENES,
        on_key: Some(on_key),
        ..AppSpec::new("Beebeeb Drive", (1280.0, 800.0), init, view)
    };

    App::new(spec).titlebar(Titlebar::new(TITLEBAR_H)).run();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_fixture_has_11_items() {
        let source = BoardFixtureSource::new();
        assert_eq!(source.list(None).items.len(), 11, "board DW-0401 has exactly 11 items");
    }

    #[test]
    fn large_fixture_5000_rows() {
        assert_eq!(LargeFixtureSource::new(5000).list(None).items.len(), 5000);
    }
}
