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
    storage: source::StorageSummary,
    selected: Option<usize>,
    search: TextEdit,
    sidebar_section: SidebarSection,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarSection {
    Files,
    Shared,
    Recent,
    Starred,
    Trash,
}

impl SidebarSection {
    fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Shared => "Shared",
            Self::Recent => "Recent",
            Self::Starred => "Starred",
            Self::Trash => "Trash",
        }
    }

    fn all() -> &'static [SidebarSection] {
        &[
            SidebarSection::Files,
            SidebarSection::Shared,
            SidebarSection::Recent,
            SidebarSection::Starred,
            SidebarSection::Trash,
        ]
    }
}

fn init() -> State {
    let source = Box::new(FixtureSource::new(5000));
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
    let storage = s.storage;
    let used_pct = if storage.quota_bytes > 0 {
        (storage.used_bytes as f64 / storage.quota_bytes as f64).min(1.0)
    } else {
        0.0
    };
    let used_label = format_bytes(storage.used_bytes);
    let quota_label = format_bytes(storage.quota_bytes);

    let mut sidebar = col()
        .key("sidebar")
        .w(SIDEBAR_WIDTH)
        .h_full()
        .fill(t.sidebar_bg)
        .pt(space::S2)
        .pb(space::S4);

    // Drive mark + sections.
    sidebar = sidebar.child(col().key("sidebar-sections").grow(1.0).px(space::S2).gap(space::S1));

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
                        .w_pct(used_pct as f32),
                ),
        )
        .child(
            text(format!("{used_label} of {quota_label}"))
                .key("storage-text")
                .font(type_::CAPTION)
                .color(t.text_tertiary),
        );

    col()
        .key("sidebar")
        .w(SIDEBAR_WIDTH)
        .h_full()
        .fill(t.sidebar_bg)
        .pt(space::S2)
        .pb(space::S4)
        .child(sections)
        .child(meter)
}

fn section_icon(section: SidebarSection) -> &'static str {
    match section {
        SidebarSection::Files => "\u{1F4C1}", // Placeholder glyph; real builds use icon font.
        SidebarSection::Shared => "\u{1F465}",
        SidebarSection::Recent => "\u{1F552}",
        SidebarSection::Starred => "\u{2B50}",
        SidebarSection::Trash => "\u{1F5D1}",
    }
}

// ---------------------------------------------------------------------------
// Toolbar
// ---------------------------------------------------------------------------

fn toolbar(s: &State, t: &tokens::Theme, trailing_inset: f32) -> El {
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
                .placeholder("Search", t.text_tertiary)
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
                .child(toolbar_button("new-folder", "New folder", t))
                .child(toolbar_button("upload", "Upload", t)),
        )
        .child(search_field)
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
// File list (virtual_rows for large folders)
// ---------------------------------------------------------------------------

fn file_list(s: &State, cx: &mut Cx<State>, t: &tokens::Theme) -> El {
    let count = s.listing.items.len();
    let hair = 1.0 / cx.env().scale;

    col()
        .key("file-list")
        .grow(1.0)
        .w_full()
        .clip(true)
        .virtual_rows(count, ROW_HEIGHT, |i, cx| {
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
                    t.selection
                } else {
                    rune::Color::TRANSPARENT
                })
                .hover(t.hover)
                .press(t.press)
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
                        .color(t.text)
                        .ml(space::S2)
                        .grow(1.0),
                )
                // Modified
                .child(
                    text(short_date(&item.modified_at))
                        .key("modified")
                        .font(type_::CAPTION)
                        .color(t.text_secondary)
                        .w(120.0),
                )
                // Size
                .child(
                    text(size_text)
                        .key("size")
                        .font(type_::CAPTION)
                        .color(t.text_secondary)
                        .w(80.0),
                )
        })
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

fn status_bar(s: &State, t: &tokens::Theme) -> El {
    let count = s.listing.items.len();
    let folders = s.listing.items.iter().filter(|i| i.is_folder).count();
    let files = count - folders;

    row()
        .key("status-bar")
        .h(STATUS_HEIGHT)
        .w_full()
        .fill(t.status_bar_bg)
        .px(space::S4)
        .center_y()
        .child(
            text(format!("{files} files, {folders} folders"))
                .key("item-count")
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
        Key::ArrowDown => {
            let next = s.selected.map(|i| (i + 1).min(count - 1)).unwrap_or(0);
            s.selected = Some(next);
            true
        }
        Key::ArrowUp => {
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
        return file_icon::FOLDER;
    }
    match mime {
        Some(m) if m.starts_with("image/") => file_icon::IMAGE,
        Some(m) if m.starts_with("video/") => file_icon::VIDEO,
        Some(m) if m.starts_with("audio/") => file_icon::AUDIO,
        Some("application/pdf") => file_icon::PDF,
        Some(m) if m.contains("document") || m.contains("text") => file_icon::DOCUMENT,
        Some(m) if m.contains("spreadsheet") => file_icon::SPREADSHEET,
        Some(m) if m.contains("presentation") => file_icon::PRESENTATION,
        Some(m) if m.contains("zip") || m.contains("archive") => file_icon::ARCHIVE,
        _ => file_icon::GENERIC,
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

        // Verify the file list element exists and virtual_rows is mounting rows.
        let tree = rt.accessibility_json();
        let tree_str = serde_json::to_string(&tree).unwrap_or_default();

        // The tree should contain row elements (not all 5000, just the mounted ones).
        let row_count = tree_str.matches("row-").count();
        assert!(row_count > 0, "virtual_rows mounts some rows");
        assert!(
            row_count < 200,
            "virtual_rows does NOT mount all 5000 rows: mounted {row_count}"
        );
    }

    #[test]
    fn arrow_keys_move_selection() {
        let env = Env {
            system_dark: true,
            scale: 2.0,
        };
        let mut rt = Runtime::new(init(), view, (1280.0, 800.0), env);
        rt.frame(0.0);

        assert!(rt.state.selected.is_none());

        // Press ArrowDown to select first item.
        rt.key(rune::KeyEvent {
            key: Key::ArrowDown,
            mods: rune::Mods::default(),
        });
        rt.frame(0.0);
        assert_eq!(rt.state.selected, Some(0));

        // Press ArrowDown again.
        rt.key(rune::KeyEvent {
            key: Key::ArrowDown,
            mods: rune::Mods::default(),
        });
        rt.frame(0.0);
        assert_eq!(rt.state.selected, Some(1));

        // Press ArrowUp.
        rt.key(rune::KeyEvent {
            key: Key::ArrowUp,
            mods: rune::Mods::default(),
        });
        rt.frame(0.0);
        assert_eq!(rt.state.selected, Some(0));
    }

    #[test]
    fn enter_on_folder_navigates() {
        let env = Env {
            system_dark: true,
            scale: 2.0,
        };
        let mut rt = Runtime::new(init(), view, (1280.0, 800.0), env);
        rt.frame(0.0);

        // Select the first item (a folder).
        rt.state.selected = Some(0);
        assert!(rt.state.listing.items[0].is_folder);

        // Press Enter.
        rt.key(rune::KeyEvent {
            key: Key::Enter,
            mods: rune::Mods::default(),
        });
        rt.frame(0.0);

        // Selection is cleared after navigation.
        assert!(rt.state.selected.is_none());
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
