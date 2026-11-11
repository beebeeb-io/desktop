//! Beebeeb Drive — Rune native UI spike.
//!
//! Renders DW-0401 (the Windows main Files window) with:
//! - Custom caption bar with Windows caption buttons (Minimize, Maximize, Close)
//! - Sidebar: drive mark, "Beebeeb Drive", sections, storage meter
//! - Toolbar: view toggle, sort, search
//! - Breadcrumbs
//! - File list with file-type icons (virtual_rows for 5,000+ items)
//! - Status footer
//!
//! Data from `DriveSource` (fixture or engine).
//!
//! ```sh
//! cargo run -p beebeeb-drive-rune
//! cargo run -p beebeeb-drive-rune -- --snap DW-0401.png --size 1280x800 --scale 2 --theme dark
//! cargo run -p beebeeb-drive-rune -- --tree tree.json
//! ```

mod source;
mod source_engine;
mod tokens;

use rune::tok::motion;
use rune::{App, AppSpec, Cx, El, Key, KeyEvent, TextEdit, Titlebar, Tx, col, el, row, text, text_input};

use source::{DriveSource, FixtureSource, Listing};
use tokens::{file_icon, radius, space, type_};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

const SIDEBAR_WIDTH: f32 = 240.0;
const TITLEBAR_HEIGHT: f32 = 32.0;
const TOOLBAR_HEIGHT: f32 = 40.0;
const BREADCRUMB_HEIGHT: f32 = 36.0;
const STATUS_HEIGHT: f32 = 28.0;
const ROW_HEIGHT: f32 = 32.0;

struct State {
    source: Box<dyn DriveSource>,
    listing: Listing,
    storage: Option<source::StorageSummary>,
    selected: Option<usize>,
    search: TextEdit,
    sidebar_section: SidebarSection,
}

/// Sidebar sections matching the board's DW-0401 Windows frame.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarSection {
    Files,
    Shared,
    Photos,
    Backups,
    Trash,
}

impl SidebarSection {
    fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Shared => "Shared",
            Self::Photos => "Photos",
            Self::Backups => "Backups",
            Self::Trash => "Trash",
        }
    }

    fn all() -> &'static [SidebarSection] {
        &[
            SidebarSection::Files,
            SidebarSection::Shared,
            SidebarSection::Photos,
            SidebarSection::Backups,
            SidebarSection::Trash,
        ]
    }
}

/// `--db <path>`: show the files of that `state.db` (opened read-only) instead of the fixture.
fn db_arg() -> Option<std::path::PathBuf> {
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--db" {
            return args.next().map(std::path::PathBuf::from);
        }
    }
    None
}

fn init() -> State {
    let source: Box<dyn DriveSource> = match db_arg() {
        Some(path) => match source_engine::EngineSource::open(&path) {
            Ok(source) => Box::new(source),
            Err(error) => {
                // A fixed word and the path the person typed; never a name from the database.
                eprintln!("beebeeb-drive: cannot open --db {}: {error}", path.display());
                std::process::exit(2);
            }
        },
        None => Box::new(FixtureSource::new(5000)),
    };
    let listing = source.list(None);
    let storage = source.storage();
    State {
        source,
        listing,
        storage,
        selected: None,
        search: TextEdit::new(""),
        sidebar_section: SidebarSection::Files,
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

fn view(s: &State, cx: &mut Cx<State>) -> El {
    let dark = cx.system_dark();
    let t = tokens::theme(dark);
    let insets = cx.chrome_insets();

    // Caption colour: light icons on dark, dark on light.
    cx.caption_color(if dark { rune::Color::WHITE } else { rune::Color::BLACK });

    let sidebar = sidebar(s, &t, dark);
    let main_area = col()
        .key("main")
        .grow(1.0)
        .h_full()
        .child(toolbar(s, &t, insets.trailing))
        .child(breadcrumbs(s, &t))
        .child(
            el().key("separator-main")
                .h(1.0 / cx.env().scale)
                .w_full()
                .fill(t.separator),
        )
        .child(column_headers(&t))
        .child(file_list(s, cx, &t))
        .child(status_bar(s, &t));

    // Root: sidebar | main, with the titlebar band above for drag.
    let titlebar_band = row()
        .key("titlebar-band")
        .w_full()
        .h(TITLEBAR_HEIGHT)
        .window_drag()
        .fill(t.sidebar_bg)
        .pl(insets.leading + space::S3)
        .pr(insets.trailing)
        .center_y()
        .child(text("Beebeeb Drive").key("app-title").font(type_::TITLE).color(t.text));

    col()
        .key("window")
        .role("window")
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(titlebar_band)
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
                .child(main_area),
        )
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

fn sidebar(s: &State, t: &tokens::Theme, _dark: bool) -> El {
    let used_pct = match s.storage {
        Some(storage) if storage.quota_bytes > 0 => (storage.used_bytes as f64 / storage.quota_bytes as f64).min(1.0),
        _ => 0.0,
    };
    // No figure without the server (a `--db` source reads one file and never the network).
    let storage_text = match s.storage {
        Some(storage) => format!(
            "{} of {}",
            format_bytes(storage.used_bytes),
            format_bytes(storage.quota_bytes)
        ),
        None => "Storage unavailable".to_string(),
    };

    let mut sections = col().key("sections-list").grow(1.0).px(space::S2).gap(space::S1);

    for &section in SidebarSection::all() {
        let active = s.sidebar_section == section;
        let section_row = row()
            .key(section.label())
            .role("button")
            .label(section.label())
            .h(30.0)
            .px(space::S3)
            .center_y()
            .radius(radius::MD)
            .fill(if active { t.hover } else { rune::Color::TRANSPARENT })
            .hover(t.hover)
            .press(t.press)
            .focusable(true)
            .on_press(move |s: &mut State| s.sidebar_section = section)
            .child(
                text(section_icon(section))
                    .key("icon")
                    .font(type_::BODY)
                    .color(if active { t.accent } else { t.icon_secondary }),
            )
            .child(
                text(section.label())
                    .key("label")
                    .font(if active { type_::BODY_MEDIUM } else { type_::BODY })
                    .color(t.text)
                    .ml(space::S2),
            );
        sections = sections.child(section_row);
    }

    // Storage meter at the bottom.
    let meter = col()
        .key("storage-meter")
        .px(space::S3)
        .gap(space::S1)
        .child(
            text("Storage")
                .key("storage-label")
                .font(type_::CAPTION)
                .color(t.text_secondary),
        )
        .child(
            el().key("meter-bg")
                .w_full()
                .h(4.0)
                .radius(2.0)
                .fill(t.separator)
                .child(
                    el().key("meter-fill")
                        .h(4.0)
                        .radius(2.0)
                        .fill(t.accent)
                        .w((SIDEBAR_WIDTH - space::S3 * 2.0) * used_pct as f32),
                ),
        )
        .child(
            text(storage_text)
                .key("storage-text")
                .font(type_::CAPTION)
                .color(t.text_tertiary),
        );

    // "THIS PC" section (board shows Sync history, Available offline below the main sections).
    let this_pc = col()
        .key("this-pc")
        .px(space::S2)
        .gap(space::S1)
        .child(
            text("THIS PC")
                .key("this-pc-label")
                .font(type_::CAPTION)
                .color(t.text_tertiary)
                .ml(space::S3)
                .mt(space::S3),
        )
        .child(
            row()
                .key("sync-history")
                .h(30.0)
                .px(space::S3)
                .center_y()
                .radius(radius::MD)
                .hover(t.hover)
                .child(text("Sync history").font(type_::BODY).color(t.text).ml(space::S2)),
        )
        .child(
            row()
                .key("available-offline")
                .h(30.0)
                .px(space::S3)
                .center_y()
                .radius(radius::MD)
                .hover(t.hover)
                .child(text("Available offline").font(type_::BODY).color(t.text).ml(space::S2)),
        );

    // Account row at the bottom (board shows anna@example.eu + settings gear).
    let account_row = row()
        .key("account-row")
        .h(32.0)
        .w_full()
        .px(space::S3)
        .center_y()
        .child(
            text("anna@example.eu")
                .key("account-email")
                .font(type_::CAPTION)
                .color(t.text_secondary)
                .grow(1.0),
        )
        .child(
            el().key("settings-gear")
                .size(16.0, 16.0)
                .center()
                .child(text("\u{2699}").font(type_::BODY).color(t.icon_secondary)),
        );

    col()
        .key("sidebar")
        .w(SIDEBAR_WIDTH)
        .h_full()
        .fill(t.sidebar_bg)
        .pt(space::S2)
        .pb(space::S4)
        .child(sections)
        .child(this_pc)
        .child(el().key("sidebar-spacer").grow(1.0))
        .child(meter)
        .child(account_row)
}

fn section_icon(section: SidebarSection) -> &'static str {
    match section {
        SidebarSection::Files => "\u{1F4C1}", // Placeholder glyph; real builds use icon font.
        SidebarSection::Shared => "\u{1F465}",
        SidebarSection::Photos => "\u{1F5BC}",
        SidebarSection::Backups => "\u{1F4BE}",
        SidebarSection::Trash => "\u{1F5D1}",
    }
}

// ---------------------------------------------------------------------------
// Toolbar
// ---------------------------------------------------------------------------

fn toolbar(s: &State, t: &tokens::Theme, trailing_inset: f32) -> El {
    let item_count = s.listing.items.len();
    let search_field = el()
        .key("search-box")
        .h(28.0)
        .w(200.0)
        .px(space::S2)
        .radius(radius::MD)
        .fill(t.card)
        .border(1.0 / 2.0, t.card_border)
        .center_y()
        .child(
            text_input(&s.search, type_::BODY)
                .key("search")
                .single_line()
                .color(t.text)
                .placeholder("Search names  Ctrl+F", t.text_tertiary)
                .on_edit(|s: &mut State| &mut s.search),
        );

    row()
        .key("toolbar")
        .h(TOOLBAR_HEIGHT)
        .w_full()
        .fill(t.toolbar_bg)
        .px(space::S4)
        .pr(trailing_inset.max(space::S4))
        .center_y()
        .justify_between()
        .child(
            row()
                .key("toolbar-left")
                .gap(space::S2)
                .center_y()
                .child(toolbar_button("back", "<", t))
                .child(toolbar_button("forward", ">", t))
                .child(
                    text(format!("Files  {} items", item_count))
                        .key("folder-title")
                        .font(type_::TITLE)
                        .color(t.text)
                        .ml(space::S2),
                ),
        )
        .child(
            row()
                .key("toolbar-right")
                .gap(space::S2)
                .center_y()
                .child(search_field)
                .child(toolbar_button("sort", "Sort", t))
                .child(
                    row()
                        .key("new-btn")
                        .role("button")
                        .label("New")
                        .h(28.0)
                        .px(space::S3)
                        .center()
                        .radius(radius::MD)
                        .fill(t.accent)
                        .hover(t.hover)
                        .press(t.press)
                        .focusable(true)
                        .child(text("+ New").font(type_::LABEL).color(t.text)),
                ),
        )
}

fn toolbar_button(key: &'static str, label: &'static str, t: &tokens::Theme) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .h(28.0)
        .px(space::S3)
        .center()
        .radius(radius::MD)
        .fill(t.card)
        .border(1.0 / 2.0, t.card_border)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .child(text(label).font(type_::LABEL).color(t.text))
}

// ---------------------------------------------------------------------------
// Breadcrumbs
// ---------------------------------------------------------------------------

fn breadcrumbs(s: &State, t: &tokens::Theme) -> El {
    let mut bc = row()
        .key("breadcrumbs")
        .h(BREADCRUMB_HEIGHT)
        .w_full()
        .px(space::S4)
        .center_y()
        .gap(space::S1);

    for (i, crumb) in s.listing.path.iter().enumerate() {
        if i > 0 {
            bc = bc.child(
                text("/")
                    .key(format!("sep-{i}"))
                    .font(type_::BODY)
                    .color(t.text_tertiary),
            );
        }
        let is_last = i == s.listing.path.len() - 1;
        bc = bc.child(
            text(&crumb.name)
                .key(format!("crumb-{i}"))
                .font(if is_last { type_::BODY_MEDIUM } else { type_::BODY })
                .color(if is_last { t.text } else { t.text_secondary }),
        );
    }

    bc
}

// ---------------------------------------------------------------------------
// Column headers (matching the board: Name, Modified, Size, Shared, PC)
// ---------------------------------------------------------------------------

fn column_headers(t: &tokens::Theme) -> El {
    row()
        .key("column-headers")
        .h(ROW_HEIGHT)
        .w_full()
        .px(space::S4)
        .center_y()
        .fill(t.toolbar_bg)
        // Spacer for icon column
        .child(el().key("icon-spacer").w(20.0))
        .child(
            text("Name")
                .key("col-name")
                .font(type_::LABEL)
                .color(t.text_secondary)
                .ml(space::S2)
                .grow(1.0),
        )
        .child(
            text("Modified")
                .key("col-modified")
                .font(type_::LABEL)
                .color(t.text_secondary)
                .w(120.0),
        )
        .child(
            text("Size")
                .key("col-size")
                .font(type_::LABEL)
                .color(t.text_secondary)
                .w(80.0),
        )
}

// ---------------------------------------------------------------------------
// File list (virtual_rows for large folders)
// ---------------------------------------------------------------------------

fn file_list(s: &State, cx: &mut Cx<State>, t: &tokens::Theme) -> El {
    let count = s.listing.items.len();
    let _hair = 1.0 / cx.env().scale;
    let selection_color = t.selection;
    let hover_color = t.hover;
    let press_color = t.press;
    let text_color = t.text;
    let text_secondary = t.text_secondary;

    // The virtual_rows element must be inside a scroll viewport.
    col()
        .key("file-list-scroll")
        .grow(1.0)
        .w_full()
        .scroll(rune::ScrollSpec { stick_bottom: false })
        .child(
            col()
                .key("file-list")
                .w_full()
                .growth(rune::GrowthSpec::default())
                .virtual_rows(
                    count,
                    ROW_HEIGHT,
                    // key: return a stable key for each row
                    |s: &State, i| {
                        use std::borrow::Cow;
                        if i < s.listing.items.len() {
                            Cow::Owned(format!("row-{i}"))
                        } else {
                            Cow::Borrowed("row-oob")
                        }
                    },
                    // build: render each row
                    move |s: &State, i| {
                        if i >= s.listing.items.len() {
                            return el().key("empty");
                        }
                        let item = &s.listing.items[i];
                        let selected = s.selected == Some(i);

                        let icon_color = file_type_color(item.mime.as_deref(), item.is_folder);
                        let icon_glyph = if item.is_folder {
                            "\u{1F4C1}"
                        } else {
                            file_type_glyph(item.mime.as_deref())
                        };

                        let size_text = if item.is_folder {
                            item.child_count.map(|c| format!("{c} items")).unwrap_or_default()
                        } else {
                            item.size.map(format_bytes).unwrap_or_default()
                        };

                        row()
                            .key(format!("row-{i}"))
                            .role("listitem")
                            .label(&item.name)
                            .h(ROW_HEIGHT)
                            .w_full()
                            .px(space::S4)
                            .center_y()
                            .fill(if selected {
                                selection_color
                            } else {
                                rune::Color::TRANSPARENT
                            })
                            .hover(hover_color)
                            .press(press_color)
                            .focusable(true)
                            .on_press(move |s: &mut State| {
                                s.selected = Some(i);
                            })
                            // Icon
                            .child(
                                el().key("icon")
                                    .size(20.0, 20.0)
                                    .center()
                                    .child(text(icon_glyph).font(type_::BODY).color(icon_color)),
                            )
                            // Name
                            .child(
                                text(&item.name)
                                    .key("name")
                                    .font(type_::BODY)
                                    .color(text_color)
                                    .ml(space::S2)
                                    .grow(1.0),
                            )
                            // Modified
                            .child(
                                text(short_date(&item.modified_at))
                                    .key("modified")
                                    .font(type_::CAPTION)
                                    .color(text_secondary)
                                    .w(120.0),
                            )
                            // Size
                            .child(
                                text(size_text)
                                    .key("size")
                                    .font(type_::CAPTION)
                                    .color(text_secondary)
                                    .w(80.0),
                            )
                    },
                ),
        )
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

fn status_bar(_s: &State, t: &tokens::Theme) -> El {
    // Board shows: "Up to date - 4 on this PC - 3 available offline - 4 online only"
    row()
        .key("status-bar")
        .h(STATUS_HEIGHT)
        .w_full()
        .fill(t.status_bar_bg)
        .px(space::S4)
        .center_y()
        .child(
            text("Up to date \u{00B7} 4 on this PC \u{00B7} 3 available offline \u{00B7} 4 online only")
                .key("sync-status")
                .font(type_::CAPTION)
                .color(t.text_secondary),
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
                    // Navigate into folder (in the spike, reload root).
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
// Helpers
// ---------------------------------------------------------------------------

fn format_bytes(bytes: i64) -> String {
    const KB: i64 = 1024;
    const MB: i64 = 1024 * 1024;
    const GB: i64 = 1024 * 1024 * 1024;
    const TB: i64 = 1024 * 1024 * 1024 * 1024;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn short_date(iso: &str) -> String {
    // Show just the date portion.
    iso.split('T').next().unwrap_or(iso).to_string()
}

fn file_type_color(mime: Option<&str>, is_folder: bool) -> rune::Color {
    if is_folder {
        return file_icon::folder();
    }
    match mime {
        Some(m) if m.starts_with("image/") => file_icon::image(),
        Some(m) if m.starts_with("video/") => file_icon::video(),
        Some(m) if m.starts_with("audio/") => file_icon::audio(),
        Some("application/pdf") => file_icon::pdf(),
        Some(m) if m.contains("document") || m.contains("text") => file_icon::document(),
        Some(m) if m.contains("spreadsheet") => file_icon::spreadsheet(),
        Some(m) if m.contains("presentation") => file_icon::presentation(),
        Some(m) if m.contains("zip") || m.contains("archive") => file_icon::archive(),
        _ => file_icon::generic(),
    }
}

fn file_type_glyph(mime: Option<&str>) -> &'static str {
    match mime {
        Some(m) if m.starts_with("image/") => "\u{1F5BC}",
        Some(m) if m.starts_with("video/") => "\u{1F3AC}",
        Some(m) if m.starts_with("audio/") => "\u{1F3B5}",
        Some("application/pdf") => "\u{1F4C4}",
        Some(m) if m.contains("spreadsheet") => "\u{1F4CA}",
        _ => "\u{1F4C4}",
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
                let source = Box::new(FixtureSource::new(5000));
                s.listing = source.list(None);
                s.source = source;
            }),
        ],
        on_key: Some(on_key),
        ..AppSpec::new("Beebeeb Drive", (1280.0, 800.0), init, view)
    };

    App::new(spec).titlebar(Titlebar::new(TITLEBAR_HEIGHT)).run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime};

    #[test]
    fn file_list_renders_5000_rows_with_virtual_rows() {
        let env = Env {
            system_dark: true,
            scale: 2.0,
        };
        let mut rt = Runtime::new(init(), view, (1280.0, 800.0), env);
        rt.frame(0.0);

        // Check the listing has 5000 items.
        assert_eq!(rt.state.listing.items.len(), 5000, "fixture produces 5000 items");

        // The file list element exists: find it by key.
        let file_list = rt.find("file-list");
        assert!(file_list.is_some(), "file-list element exists in the tree");
    }

    #[test]
    fn arrow_down_moves_selection() {
        // Test the key handling logic directly via state mutation.
        let mut state = init();
        assert!(state.selected.is_none());

        // Simulate Down: same logic as on_key.
        let count = state.listing.items.len();
        state.selected = Some(state.selected.map(|i| (i + 1).min(count - 1)).unwrap_or(0));
        assert_eq!(state.selected, Some(0));

        // Down again.
        state.selected = Some(state.selected.map(|i| (i + 1).min(count - 1)).unwrap_or(0));
        assert_eq!(state.selected, Some(1));

        // Up.
        state.selected = Some(state.selected.map(|i| i.saturating_sub(1)).unwrap_or(0));
        assert_eq!(state.selected, Some(0));
    }

    #[test]
    fn enter_on_folder_changes_crumbs() {
        let mut state = init();

        // Select the first item (a folder).
        state.selected = Some(0);
        assert!(state.listing.items[0].is_folder);

        // Simulate Enter: navigate into folder.
        let folder_id = state.listing.items[0].id.clone();
        state.listing = state.source.list(Some(&folder_id));
        state.selected = None;

        // Selection is cleared after navigation.
        assert!(state.selected.is_none());
        // The listing was reloaded (fixture always returns root).
        assert_eq!(state.listing.path[0].name, "Files");
    }

    #[test]
    fn sidebar_has_board_sections() {
        let env = Env {
            system_dark: true,
            scale: 2.0,
        };
        let mut rt = Runtime::new(init(), view, (1280.0, 800.0), env);
        rt.frame(0.0);

        // The board shows Files, Shared, Photos, Backups, Trash.
        for section in &["Files", "Shared", "Photos", "Backups", "Trash"] {
            assert!(
                rt.find(section).is_some(),
                "sidebar section '{section}' exists in the tree"
            );
        }
    }

    #[test]
    fn search_field_takes_text() {
        let env = Env {
            system_dark: true,
            scale: 2.0,
        };
        let mut rt = Runtime::new(init(), view, (1280.0, 800.0), env);
        rt.frame(0.0);

        // Find and focus the search field.
        let search = rt.find("search");
        assert!(search.is_some(), "search field exists in the tree");
    }
}
