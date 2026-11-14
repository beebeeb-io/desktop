//! What sits over, or in place of, a document in the office editor's frame: the question after a
//! save that found a newer version (DW-1554), the command search (DW-1551), the body when the
//! editor did not start (DW-1556), Insert's tools and a table on the page (DW-1550).
//!
//! As in `office.rs`, no editing engine stands behind these: a save is asked of
//! [`ViewerSource::save_document`], the search of [`ViewerSource::commands`], and both answer
//! "nothing" until an editor takes them. Nothing is drawn as saved, kept or found before the
//! source says so.
//!
//! [`ViewerSource::save_document`]: super::source::ViewerSource::save_document
//! [`ViewerSource::commands`]: super::source::ViewerSource::commands

use std::fmt;

use rune::{Align, Color, El, Icon, Key, KeyEvent, TextEdit, TextStyle, Tx, col, el, row, text_input};

use super::chrome::{ButtonKind, button};
use super::dialogs::{self, Look};
use super::office::{self, WordDoc};
use super::office_icons as oi;
use super::source::{Conflict, Keep, SaveOutcome};
use super::{Intent, Ui, icons, style};
use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::field::caret_mark;
use crate::ui::kit;
use crate::ui::settings_dialog::DIALOG_TITLE;

/// 13px on 17px (`.t-body`): a command's name, and what is typed.
const BODY: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
/// 10px/600 on 13px, tracked 0.7px, capitals (`.t-label`).
const GROUP: TextStyle = TextStyle::new(10.0, 600.0).lh(13.0).tracking(0.7);
/// 11px/500 on 14px (`.t-cap`).
const CAP: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);
/// 12px on 15px: a tool's word.
const TOOL_WORD: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
/// A table's cell on the page.
const CELL: TextStyle = TextStyle::new(11.0, 400.0).lh(17.0);
const CELL_HEAD: TextStyle = TextStyle::new(11.0, 600.0).lh(17.0);

pub const SAVED_FIRST: &str = "Someone else saved first";
pub const NOT_SAVED: &str = "Not saved. Nothing changed; your edit is still here.";
pub const NO_EDITOR_TITLE: &str = "We couldn't prepare the editor on this device.";
pub const NO_EDITOR_WORDS: &str = "Nothing was changed. The file still opens in Preview.";

/// One command the search found.
#[derive(Clone, PartialEq, Eq)]
pub struct Command {
    /// "Heading 1", "Bold", or a file's name.
    pub name: String,
    /// "Ctrl+B".
    pub shortcut: Option<String>,
}

/// The commands of one kind ("Styles", "Format", "Files").
#[derive(Clone, PartialEq, Eq)]
pub struct CommandGroup {
    pub title: String,
    pub commands: Vec<Command>,
}

/// A file's name can be among them: the count only.
impl fmt::Debug for CommandGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CommandGroup({} commands)", self.commands.len())
    }
}

/// The command search while it is open (Ctrl+K).
pub struct Palette {
    pub query: TextEdit,
    /// Which result the arrow keys are on, counted over all groups.
    pub hot: usize,
}

impl Palette {
    fn new() -> Self {
        let mut query = TextEdit::new("");
        query.focused = true;
        Palette { query, hot: 0 }
    }
}

/// What is open over the document.
pub enum Over {
    /// DW-1554: the last save found this newer version in its way.
    Conflict(Conflict),
    /// DW-1551.
    Palette(Palette),
}

impl Ui {
    /// The open document, if the editor's frame shows one.
    fn word(&self) -> Option<&WordDoc> {
        match &self.file.body {
            super::source::Body::Word(doc) => Some(doc),
            _ => None,
        }
    }

    /// Save (the ribbon's Save, Ctrl+S): asked of the source, and drawn as what it answers. A
    /// document with nothing to save asks nothing.
    pub fn office_save(&mut self) {
        if !self.word().is_some_and(|doc| doc.kept == office::Kept::Unsaved) {
            return;
        }
        if let Some(Over::Conflict(_)) = self.office.over {
            return;
        }
        let outcome = self.source.save_document(&self.file.id, self.file.version);
        self.office_outcome(outcome);
    }

    /// DW-1554's three ways to keep the edit.
    pub fn office_resolve(&mut self, keep: Keep) {
        if !matches!(self.office.over, Some(Over::Conflict(_))) {
            return;
        }
        let outcome = self.source.resolve_document(&self.file.id, keep);
        self.office_outcome(outcome);
    }

    fn office_outcome(&mut self, outcome: SaveOutcome) {
        match outcome {
            SaveOutcome::Saved { file, .. } => {
                self.toast = Some(format!("Saved \u{00B7} encrypted \u{00B7} version {}", file.version));
                self.office.over = None;
                self.replace(file);
            }
            SaveOutcome::Opened(file) => {
                self.office.over = None;
                self.replace(file);
            }
            SaveOutcome::Conflict(other) => self.office.over = Some(Over::Conflict(other)),
            // Not saved: the edit and the question stay as they are, and the person is told.
            SaveOutcome::Failed => self.toast = Some(NOT_SAVED.to_string()),
        }
    }

    /// Cancel and Esc: back to the document, still not saved.
    pub fn office_close_over(&mut self) {
        self.office.over = None;
        self.focus = Some("rib-find");
    }

    /// Ctrl+K, or a press on the ribbon's search.
    pub fn office_palette(&mut self) {
        if self.word().is_some() && self.office.over.is_none() {
            self.office.over = Some(Over::Palette(Palette::new()));
            self.focus = Some("pal-input");
        }
    }

    /// What the source finds for the typed words. Asked at every draw: a source answers from
    /// what it holds in memory.
    fn office_found(&self) -> Vec<CommandGroup> {
        match &self.office.over {
            Some(Over::Palette(palette)) => self.source.commands(&self.file.id, &palette.query.text),
            _ => Vec::new(),
        }
    }

    fn office_step(&mut self, by: isize) {
        let count: usize = self.office_found().iter().map(|g| g.commands.len()).sum();
        if let Some(Over::Palette(palette)) = &mut self.office.over
            && count > 0
        {
            palette.hot = (palette.hot as isize + by).rem_euclid(count as isize) as usize;
        }
    }

    /// Enter or a press on a result: the editor takes the command; until it does the press says so.
    fn office_run(&mut self) {
        self.office.over = None;
        self.intent(Intent::Office("this command"));
    }
}

/// The keys of what is open over the document. `true`: the key was taken.
pub fn on_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    let Some(ui) = super::ui(s) else { return false };
    // Only the editor's frame has these keys.
    if ui.word().is_none() && ui.office.over.is_none() {
        return false;
    }
    match (&ui.office.over, &ev.key) {
        (Some(_), Key::Escape) => ui.office_close_over(),
        (Some(Over::Palette(_)), Key::Down) => ui.office_step(1),
        (Some(Over::Palette(_)), Key::Up) => ui.office_step(-1),
        (Some(Over::Palette(_)), Key::Enter) => ui.office_run(),
        (None, Key::Command('k')) if !ev.mods.alt && !ev.mods.shift => ui.office_palette(),
        (None, Key::Command('s')) if !ev.mods.alt && !ev.mods.shift && ui.word().is_some() => ui.office_save(),
        _ => return false,
    }
    true
}

fn close(s: &mut State) {
    if let Some(ui) = super::ui(s) {
        ui.office_close_over();
    }
}

/// DW-1554: saved elsewhere while this document was edited.
fn conflict(opened: u32, other: &Conflict, t: &Theme) -> El {
    let words = format!(
        "This file was saved as version {} while you were editing (you opened version {opened}). Office documents can't be diffed line by line \u{2014} choose how to keep your edit.",
        other.version
    );
    let head = row()
        .key("dlg-head")
        .w_full()
        .pt(20.0)
        .px(24.0)
        .pb(16.0)
        .gap(14.0)
        .items_start()
        .child(
            el().key("dlg-tile")
                .size(36.0, 36.0)
                .radius(10.0)
                .fill(style::tile_red(t))
                .center()
                .pixel_snap(false)
                .child(kit::glyph("dlg-tile-icon", &icons::REFRESH, 18.0, style::red(t))),
        )
        .child(
            col()
                .key("dlg-words")
                .grow(1.0)
                .min_w(0.0)
                .gap(6.0)
                .child(
                    kit::label("dlg-title", SAVED_FIRST, DIALOG_TITLE, t.text)
                        .role("heading")
                        .level(1)
                        .w_full(),
                )
                .child(
                    kit::label("dlg-sub", words, style::SUB, t.text_tertiary)
                        .wrap()
                        .w_full(),
                ),
        );
    let keep = |keep: Keep| {
        move |s: &mut State| {
            if let Some(ui) = super::ui(s) {
                ui.office_resolve(keep);
            }
        }
    };
    let wide = |key: &str, label: String, look: Look| dialogs::button_keyed(key.to_string(), &label, look, t).w_full();
    let foot = dialogs::foot(t).child(
        col()
            .key("dlg-stack")
            .w_full()
            .gap(8.0)
            .child(wide("btn-0", "Save as a new version anyway".into(), Look::Primary).on_press(keep(Keep::Mine)))
            .child(
                wide(
                    "btn-1",
                    "Keep both \u{2014} save mine as a separate file".into(),
                    Look::Outline,
                )
                .on_press(keep(Keep::Both)),
            )
            .child(
                wide(
                    "btn-2",
                    format!("Discard my changes and reopen version {}", other.version),
                    Look::Danger,
                )
                .on_press(keep(Keep::Theirs)),
            )
            .child(wide("btn-3", "Cancel".into(), Look::Quiet).on_press(close)),
    );
    dialogs::shell_closing(SAVED_FIRST, "alertdialog", 420.0, head, None, foot, close, t)
}

/// A `.kit-kbd` as wide as its letters. Keys: `<key>-box`, `<key>`.
pub(super) fn kbd(key: &str, words: &str, t: &Theme) -> El {
    el().key(format!("{key}-box"))
        .h(20.0)
        .px(6.0)
        .radius(6.0)
        .fill(t.card)
        .box_shadow(kit::ring_inset(t.separator))
        .box_shadow(kit::drop(1.0, 0.0, t.separator))
        .center()
        .child(style::fitted(key.to_string(), words, type_::KBD, t.text_tertiary))
}

/// DW-1551: the command search over the document.
fn palette(ui: &Ui, palette: &Palette, t: &Theme) -> El {
    let typed = palette.query.text.as_str();
    let typed_w = rune::text::advance(typed, BODY);
    if !typed.is_empty() {
        kit::note_text("pal-query", "pal-query-box", typed, BODY, t.text);
    }
    let input = text_input(&palette.query, BODY)
        .key("pal-input")
        .label("Search commands, styles, files")
        .single_line()
        .color(t.text)
        .caret(Color::TRANSPARENT, t.selection)
        // The palette's own words; the window's search box when it closed under the key.
        .on_edit(
            |s: &mut State| match s.viewer.ui.as_mut().map(|ui| &mut ui.office.over) {
                Some(Some(Over::Palette(palette))) => {
                    palette.hot = 0;
                    &mut palette.query
                }
                _ => &mut s.search,
            },
        )
        .on_input_key(|s: &mut State, ev: &KeyEvent, tx: &mut Tx| {
            matches!(ev.key, Key::Escape | Key::Down | Key::Up | Key::Enter) && on_key(s, ev, tx)
        })
        .w_full();
    let field = el()
        .key("pal-field")
        .grow(1.0)
        .min_w(0.0)
        .h(17.0)
        .child(input)
        // Where the typed words end, for the gate and for the caret.
        .child(el().key("pal-query-box").absolute(0.0, 0.5).w(typed_w).h(15.5))
        .child(
            el().key("pal-caret-at")
                .absolute(typed_w + 1.0, 1.0)
                .child(caret_mark("pal-caret".into(), 15.0, t)),
        );
    let head = row()
        .key("pal-head")
        .w_full()
        .h(48.0)
        .px(16.0)
        .gap(8.0)
        .center_y()
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(kit::glyph("pal-find-icon", &oi::FIND, 15.0, t.text))
        .child(field)
        .child(kbd("pal-esc", "esc", t));
    let mut list = col().key("pal-list").role("listbox").label("Results").w_full().py(8.0);
    let mut n = 0;
    for (g, group) in ui.office_found().iter().enumerate() {
        let title = group.title.to_uppercase();
        kit::note_text(
            &format!("pal-g{g}"),
            &format!("pal-g{g}-box"),
            &title,
            GROUP,
            t.text_tertiary,
        );
        list = list.child(
            el().key(format!("pal-g{g}-box"))
                .w_full()
                .pt(8.0)
                .px(16.0)
                .pb(4.0)
                .child(kit::label(format!("pal-g{g}-t"), title, GROUP, t.text_tertiary)),
        );
        for command in &group.commands {
            let key = format!("pal-{n}");
            let hot = n == palette.hot;
            let mut line = row()
                .key(format!("{key}-row"))
                .role("option")
                .label(command.name.clone())
                .checked(hot)
                .mx(8.0)
                .h(30.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(7.0)
                .fill(if hot { t.selection } else { Color::TRANSPARENT })
                .hover(t.hover)
                .on_press(|s: &mut State| {
                    if let Some(ui) = super::ui(s) {
                        ui.office_run();
                    }
                })
                .child(
                    kit::label(key.clone(), command.name.clone(), BODY, t.text)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0)
                        .a11y_hidden(),
                );
            if let Some(shortcut) = &command.shortcut {
                line = line
                    .shortcut_hint(shortcut.clone())
                    .child(kbd(&format!("{key}-kbd"), shortcut, t));
            }
            list = list.child(line);
            n += 1;
        }
    }
    let foot = row()
        .key("pal-foot")
        .w_full()
        .h(34.0)
        .px(16.0)
        .gap(8.0)
        .center_y()
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .child(kit::glyph("pal-foot-icon", &oi::PADLOCK, 11.0, t.text_tertiary))
        .child(style::fitted(
            "pal-foot-text",
            "Runs entirely on this device",
            CAP,
            t.text_tertiary,
        ));
    let card = col()
        .key("pal")
        .role("dialog")
        .label("Search commands, styles, files")
        .absolute(390.0, 70.0)
        .w(520.0)
        .radius(14.0)
        .fill(t.card)
        .box_shadow(kit::drop(30.0, 80.0, Color::BLACK.alpha(0.30)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            col()
                .key("pal-clip")
                .w_full()
                .clip_rounded(14.0)
                .child(head)
                .child(list)
                .child(foot),
        );
    el().key("scrim")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(crate::ui::settings_dialog::scrim_color(t))
        .child(card)
        .modal(false, close)
}

/// What is open over the document, if anything.
pub fn over(ui: &Ui, t: &Theme) -> Option<El> {
    Some(match ui.office.over.as_ref()? {
        Over::Conflict(other) => conflict(ui.file.version, other, t),
        Over::Palette(p) => palette(ui, p, t),
    })
}

/// DW-1556: the editor did not start. Nothing was changed; the file still opens read-only.
pub fn no_editor(t: &Theme) -> El {
    let width = 30.0 * rune::text::advance("0", style::SUB);
    let content = col()
        .key("empty")
        .role("alert")
        .py(28.0)
        .px(24.0)
        .gap(8.0)
        .items_center()
        .child(
            el().key("empty-tile")
                .size(52.0, 52.0)
                .radius(15.0)
                .fill(style::tile_red(t))
                .center()
                .child(kit::glyph("empty-icon", &icons::ALERT, 26.0, style::red(t))),
        )
        .child(
            kit::label("empty-title", NO_EDITOR_TITLE, style::TITLE, t.text)
                .role("heading")
                .level(1),
        )
        .child(
            kit::label("empty-text", NO_EDITOR_WORDS, style::SUB, t.text_secondary)
                .wrap()
                .w(width)
                .align(Align::Center),
        )
        .child(
            row()
                .key("empty-actions")
                .gap(8.0)
                .mt(8.0)
                .child(
                    button("empty-add", "Try again", ButtonKind::Outline, false, t).on_press(|s: &mut State| {
                        if let Some(ui) = super::ui(s) {
                            ui.retry();
                        }
                    }),
                )
                .child(
                    button(
                        "empty-btn-1",
                        "Open in another app\u{2026}",
                        ButtonKind::Ghost,
                        false,
                        t,
                    )
                    .on_press(|s: &mut State| {
                        if let Some(ui) = super::ui(s) {
                            ui.intent(Intent::OpenInAnotherApp);
                        }
                    }),
                ),
        );
    col()
        .key("stage")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .center()
        .fill(style::stage(t))
        .child(content)
}

/// A tool with its word (Insert's): 28 high, the glyph, the word. Keys: `<key>`, `-icon`, `-label`.
fn worded(key: &str, word: &'static str, icon: &'static Icon, t: &Theme) -> El {
    row()
        .key(key.to_string())
        .role("button")
        .label(word)
        .h(28.0)
        .px(6.0)
        .gap(4.0)
        .center_y()
        .radius(7.0)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .on_press(office::ask(word))
        .child(kit::glyph(format!("{key}-icon"), icon, 15.0, t.text_secondary))
        .child(style::fitted(format!("{key}-label"), word, TOOL_WORD, t.text).a11y_hidden())
}

/// Insert's tools, in the board's order (DW-1550).
pub fn insert_tools(t: &Theme) -> Vec<El> {
    vec![
        worded("rt-0", "Table\u{2026}", &oi::TABLE, t),
        worded("rt-1", "Image\u{2026}", &oi::IMAGE, t),
        worded("rt-2", "Link\u{2026}", &oi::LINK, t),
        office::hairline("rsep-0".into(), t),
        worded("rt-3", "Page break", &super::media_icons::FILE_TEXT, t),
    ]
}

/// A table on the page: the first row is its headings. Keys: `tbl`, `tbl-<row>-<column>`.
pub fn table(rows: &[Vec<String>], width: f32, ink: Color) -> El {
    let mut grid = col()
        .key("tbl")
        .role("table")
        .w_full()
        .box_shadow(kit::ring(1.0, Color::BLACK.alpha(0.30)));
    for (r, cells) in rows.iter().enumerate() {
        let text_style = if r == 0 { CELL_HEAD } else { CELL };
        let mut line = row().key(format!("tbl-{r}")).role("row").w_full();
        for (c, cell) in cells.iter().enumerate() {
            let key = format!("tbl-{r}-{c}");
            kit::note_text(&key, &format!("{key}-cell"), cell, text_style, ink);
            line = line.child(
                el().key(format!("{key}-cell"))
                    .role(if r == 0 { "columnheader" } else { "cell" })
                    .label(cell.clone())
                    .w(width / cells.len().max(1) as f32)
                    .px(8.0)
                    .py(4.0)
                    .pixel_snap(false)
                    .box_shadow(kit::ring(kit::HAIR, Color::BLACK.alpha(0.20)))
                    .child(
                        kit::label(format!("{key}-t"), cell.clone(), text_style, ink)
                            .truncate()
                            .a11y_hidden(),
                    ),
            );
        }
        grid = grid.child(line);
    }
    grid
}
