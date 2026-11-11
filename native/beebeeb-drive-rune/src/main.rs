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

mod frames;
mod source;
mod source_engine;
mod theme;
mod ui;

use rune::{App, Cx, El, KeyEvent, TextEdit, Titlebar, Tx};

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
    /// Why `listing` could not be read, when it could not: the window shows this instead of rows.
    list_error: Option<source::SourceError>,
    storage: source::StorageSummary,
    /// `false`: the source cannot know the account's storage (a `--db` source never asks the server).
    storage_known: bool,
    selected: Option<usize>,
    search: TextEdit,
    sidebar_section: SidebarSection,
    show_detail_pane: bool,
    /// What the Files window keeps beside the listing (`ui::FilesUi`).
    files: ui::FilesUi,
    /// What the Files frames keep: selection, menus, toasts (`frames::files`).
    actions: frames::files::Actions,
}

/// `--db <path>`: show the files of that `state.db` (opened read-only) instead of the fixture.
/// `Err`: `--db` was given with no path after it (the end of the line, or another `--option`).
fn db_arg(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Option<std::path::PathBuf>, &'static str> {
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--db" {
            return match args.next() {
                Some(path) if !path.is_empty() && !path.to_string_lossy().starts_with("--") => {
                    Ok(Some(std::path::PathBuf::from(path)))
                }
                _ => Err("--db needs the path of a state.db"),
            };
        }
    }
    Ok(None)
}

/// The folder as the window holds it: the listing, or an empty one and the reason it could not be read.
fn load(source: &dyn DriveSource, folder_id: Option<&str>) -> (Listing, Option<source::SourceError>) {
    match source.list(folder_id) {
        Ok(listing) => (listing, None),
        Err(error) => {
            let root = source::Crumb {
                id: None,
                name: "Files".into(),
            };
            let unread = Listing {
                folder: root.clone(),
                path: vec![root],
                items: Vec::new(),
            };
            (unread, Some(error))
        }
    }
}

fn init() -> State {
    let db = db_arg(std::env::args_os().skip(1)).unwrap_or_else(|error| {
        eprintln!("beebeeb-drive: {error}");
        std::process::exit(2);
    });
    let from_db = db.is_some();
    let source: Box<dyn DriveSource> = match db {
        Some(path) => match source_engine::EngineSource::open(&path) {
            Ok(source) => Box::new(source),
            Err(error) => {
                // A fixed word and the path the person typed; never a name from the database.
                eprintln!("beebeeb-drive: cannot open --db {}: {error}", path.display());
                std::process::exit(2);
            }
        },
        None => Box::new(BoardFixtureSource::new()),
    };
    let (listing, list_error) = load(source.as_ref(), None);
    let storage = source.storage();
    State {
        source,
        listing,
        list_error,
        storage_known: storage.is_some(),
        storage: storage.unwrap_or(source::StorageSummary {
            used_bytes: 0,
            quota_bytes: 0,
        }),
        // The board fixture: Contract draft.pdf (index 6) selected per the board. A real database: nothing selected.
        selected: if from_db { None } else { Some(6) },
        search: TextEdit::new(""),
        sidebar_section: SidebarSection::Files,
        show_detail_pane: true,
        files: ui::FilesUi::default(),
        actions: frames::files::Actions::default(),
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

fn view(s: &State, cx: &mut Cx<State>) -> El {
    frames::files::view(s, cx)
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    frames::files::keys::on_key(s, ev, tx)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Fonts: Inter (sans) + JetBrains Mono (mono), bundled as TTF
// ---------------------------------------------------------------------------

const FAMILIES: (&str, &str) = ("Inter", "JetBrains Mono");

static FONTS: &[&[u8]] = &[
    // Inter, variable (optical size 14-32, weight 100-900): the build the board draws with, so
    // the wordmark's optical size and the 650 weights are the board's.
    include_bytes!("../assets/fonts/inter-latin-variable-opsz-wght.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono-latin-400-normal.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono-latin-500-normal.ttf"),
    include_bytes!("../assets/fonts/jetbrains-mono-latin-600-normal.ttf"),
];

/// Named states of the window: `--snap out.png --scene DW-0401`, and the frame gate
/// (`scripts/compare.sh DW-0401`). One per board frame, by its frame id.
const SCENES: &[ui::gate::Scene<State>] = &[
    ("light", |_| {}),
    ("dark", |_| {}),
    ("large-folder", |s| {
        let source = Box::new(LargeFixtureSource::new(5000));
        (s.listing, s.list_error) = load(source.as_ref(), None);
        s.source = source;
        s.selected = None;
        s.show_detail_pane = false;
    }),
    ("DW-0401", |_| {}),
    frames::files::SCENES[0],
    frames::files::SCENES[1],
    frames::files::SCENES[2],
    frames::files::SCENES[3],
    frames::files::SCENES[4],
    frames::files::SCENES[5],
    frames::files::SCENES[6],
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
    use rune::{Env, Runtime};

    #[test]
    fn db_option_takes_one_path_and_refuses_to_be_given_none() {
        let args = |list: &[&str]| list.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        assert_eq!(db_arg(args(&[])), Ok(None));
        assert_eq!(db_arg(args(&["--snap", "x.png"])), Ok(None));
        assert_eq!(
            db_arg(args(&["--snap", "x.png", "--db", "a/state.db"])),
            Ok(Some(std::path::PathBuf::from("a/state.db")))
        );
        assert!(db_arg(args(&["--db"])).is_err(), "--db at the end of the line");
        assert!(
            db_arg(args(&["--db", "--snap", "x.png"])).is_err(),
            "--db followed by another option"
        );
        assert!(db_arg(args(&["--db", ""])).is_err(), "--db with an empty path");
    }

    /// A folder that cannot be read draws the reason, and no rows: it never looks like an empty folder.
    #[test]
    fn an_unreadable_folder_draws_its_reason_instead_of_rows() {
        struct Broken;
        impl DriveSource for Broken {
            fn list(&self, _: Option<&str>) -> Result<Listing, source::SourceError> {
                Err(source::SourceError::Unreadable)
            }
            fn storage(&self) -> Option<source::StorageSummary> {
                None
            }
        }
        let env = Env {
            system_dark: false,
            scale: 1.0,
        };
        let mut state = init();
        (state.listing, state.list_error) = load(&Broken, None);
        state.source = Box::new(Broken);
        state.selected = None;
        assert_eq!(state.list_error, Some(source::SourceError::Unreadable));
        let mut rt = Runtime::new(state, view, (1280.0, 800.0), env);
        rt.frame(0.0);
        assert!(rt.find("file-list-error-text").is_some(), "the reason is drawn");
        assert!(rt.find("file-list").is_none(), "and no (empty) list is");
        assert!(
            rt.find("detail-pane").is_none() || rt.find("detail-name").is_none(),
            "nothing is selected"
        );
    }

    #[test]
    fn board_fixture_has_11_items() {
        let source = BoardFixtureSource::new();
        assert_eq!(
            source.list(None).unwrap().items.len(),
            11,
            "board DW-0401 has exactly 11 items"
        );
    }

    #[test]
    fn large_fixture_5000_rows() {
        assert_eq!(LargeFixtureSource::new(5000).list(None).unwrap().items.len(), 5000);
    }
}
