//! The office editor in the window (the board's "Office on Windows": DW-1507 and its states): the
//! window's own toolbar with the file's path, the ribbon (tabs, command search, Share, Save, the
//! tab's tools), the outline, the page on its stage, and the status line.
//!
//! What is here is the editor's FRAME. No editing engine stands behind it yet: the page is drawn
//! from what [`super::source::ViewerSource::open`] hands over ([`WordDoc`]: the board's sample in
//! a development build), and every command a control stands for is asked of the app around it as
//! [`super::Intent::Office`], which says "Not built yet" until the editor takes it. Nothing is
//! typed, saved or changed by this file.
//!
//! Keys: the window's caption, sidebar and toolbar keep the keys every window has. The editor's
//! own are `rib*` (the ribbon), `rt-<N>` / `rp-<N>` / `rsep-<N>` (its tools, pop-up buttons and
//! hairlines, in order), `ol*` (the outline), `stage`, `page`, `pg-<N>` (the page's lines),
//! `float`, `ft-<N>` (the selection's tools) and `st-<N>` (the status line's parts).

use std::fmt;

use rune::tok::Family;
use rune::{Color, Cx, El, Icon, Span, TextStyle, col, el, rich, row, text_input};
use zeroize::Zeroize;

use super::office_icons as oi;
use super::source::OpenFile;
use super::{Intent, Ui, style};
use crate::State;
use crate::theme::icons;
use crate::theme::tokens::{self, Theme, type_};
use crate::ui::toolbar::{self as tb, Bar, Details, New, Sort, View, press};
use crate::ui::{caption, kit, sidebar};

/// 12px/600 on 15px: the ribbon's tab that is open.
const TAB_ON: TextStyle = TextStyle::new(12.0, 600.0).lh(15.0);
/// 12px/500 on 15px: the other tabs.
const TAB: TextStyle = TextStyle::new(12.0, 500.0).lh(15.0);
/// 11px/500 on 14px (`.t-cap`): the command search's words.
const CAP: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);
/// 12.5px/400 on 17px: a pop-up button's choice.
const POP: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0);
/// 10px mono, 500: a shortcut inside a button.
const HINT: TextStyle = TextStyle::new(10.0, 500.0)
    .family(Family::Mono)
    .lh(14.2857)
    .tracking(-0.06);
/// 10px/600 on 13px, tracked 0.7px, capitals (`.t-label`).
const PANE_LABEL: TextStyle = TextStyle::new(10.0, 600.0).lh(13.0).tracking(0.7);
/// 12px on 15px: an outline line.
const OUTLINE: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
const OUTLINE_TOP: TextStyle = TextStyle::new(12.0, 600.0).lh(15.0);
/// The page's own type: the document's, not the app's.
const DOC_TITLE: TextStyle = TextStyle::new(20.0, 650.0).lh(26.0);
const DOC_BYLINE: TextStyle = TextStyle::new(11.0, 400.0).lh(17.0);
const DOC_HEADING: TextStyle = TextStyle::new(14.0, 650.0).lh(17.0);
const DOC_BODY: TextStyle = TextStyle::new(12.5, 400.0).lh(20.0);
/// 11.5px/600 on 17px: "Encrypted" in the status line.
const STATUS_ENC: TextStyle = TextStyle::new(11.5, 600.0).lh(17.0);
const STATUS_MONO: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono).lh(17.0);
/// The letters of Bold, Italic, Underline and Strikethrough.
const LETTER_HEAVY: TextStyle = TextStyle::new(13.5, 800.0).lh(17.0);
const LETTER: TextStyle = TextStyle::new(13.5, 700.0).lh(17.0);

/// The page's ink: a document is near-black on white in both themes.
fn ink() -> Color {
    rune::color::hex("#0D0D0D").unwrap_or(Color::BLACK)
}

/// A selection in the document, as the editor reports it.
#[derive(Clone, PartialEq)]
pub struct Selection {
    /// Which section's paragraph, and the bytes of it that are selected.
    pub section: usize,
    pub range: std::ops::Range<usize>,
    /// Where the selection's tools go, from the page's top-left corner.
    pub tools_at: (f32, f32),
}

/// One section of a document: its heading, its paragraph, and a table under it if it has one
/// (rows of cells, the first row the headings).
#[derive(Clone, PartialEq, Eq)]
pub struct Section {
    pub heading: String,
    pub text: String,
    pub table: Option<Vec<Vec<String>>>,
}

/// Whether what is on screen is what is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kept {
    /// The version in the status line is what is on screen.
    Saved,
    /// There are changes no version holds yet (DW-1552).
    Unsaved,
}

/// A text document as the editor shows it (DW-1507).
#[derive(Clone, PartialEq)]
pub struct WordDoc {
    pub title: String,
    pub byline: String,
    /// The sections on the page in view.
    pub sections: Vec<Section>,
    /// The document's headings, the title first; `at` is the one the caret is in.
    pub outline: Vec<String>,
    pub at: usize,
    pub pages: u32,
    pub words: u32,
    pub language: String,
    /// When this version was saved ("09:12").
    pub saved_at: String,
    pub selection: Option<Selection>,
    pub kept: Kept,
    /// An older Word file (.doc): it opens and saves in the same format (DW-1511).
    pub older: bool,
}

/// Its counts, never a word of it.
impl fmt::Debug for WordDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WordDoc({} pages, {} words)", self.pages, self.words)
    }
}

impl WordDoc {
    /// Overwrite the decrypted words held here (see [`OpenFile::wipe`]).
    pub fn wipe(&mut self) {
        self.title.zeroize();
        self.byline.zeroize();
        for section in &mut self.sections {
            section.heading.zeroize();
            section.text.zeroize();
            for cell in section.table.iter_mut().flatten().flatten() {
                cell.zeroize();
            }
        }
        for line in &mut self.outline {
            line.zeroize();
        }
        self.selection = None;
    }
}

/// The ribbon's tabs of a text document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Home,
    Insert,
    Layout,
    Review,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Home, Tab::Insert, Tab::Layout, Tab::Review];
    pub fn label(self) -> &'static str {
        match self {
            Tab::Home => "Home",
            Tab::Insert => "Insert",
            Tab::Layout => "Layout",
            Tab::Review => "Review",
        }
    }
}

/// What is open inside the editor's frame.
pub struct OfficeUi {
    pub tab: Tab,
    /// The outline pane shows.
    pub outline: bool,
    /// The question after a save, or the command search (`office_over.rs`).
    pub over: Option<super::office_over::Over>,
}

impl Default for OfficeUi {
    fn default() -> Self {
        OfficeUi {
            tab: Tab::Home,
            outline: true,
            over: None,
        }
    }
}

/// A command a control of the editor stands for. The editor takes it; until it does the press says
/// "Not built yet" ([`Ui::intent`]).
pub(super) fn ask(command: &'static str) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        if let Some(ui) = super::ui(s) {
            ui.intent(Intent::Office(command));
        }
    }
}

/// How a tool of the ribbon shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Idle,
    /// Chosen (the paragraph's alignment).
    On,
    /// Nothing to do now (Redo with nothing undone).
    Off,
}

enum Face {
    Glyph(&'static Icon, f32),
    Letter(&'static str, TextStyle),
}

/// A tool button (`.mk-tbtn`): 28 square, its glyph or its letter. Keys: `<key>`, `-icon` or `-letter`.
fn tool(key: &str, name: &'static str, face: Face, state: Tool, command: &'static str, t: &Theme) -> El {
    let ink = match state {
        Tool::Idle => t.text_secondary,
        Tool::On => t.text,
        Tool::Off => t.text_quaternary,
    };
    let mut b = row()
        .key(key.to_string())
        .role("button")
        .label(name)
        .size(28.0, 28.0)
        .center()
        .radius(7.0)
        .fill(if state == Tool::On {
            t.selection
        } else {
            Color::TRANSPARENT
        });
    b = if state == Tool::Off {
        b.disabled(true)
    } else {
        b.hover(t.hover).press(t.press).focusable(true).on_press(ask(command))
    };
    if state == Tool::On {
        b = b.checked(true);
    }
    b.child(match face {
        Face::Glyph(icon, size) => kit::glyph(format!("{key}-icon"), icon, size, ink),
        Face::Letter(letter, text_style) => {
            // Underline is drawn underlined and Italic slanted, as the board's letters are.
            let key = format!("{key}-letter");
            let span = match letter {
                "U" => vec![Span::new(0, 1).underline(ink)],
                "I" => vec![Span::new(0, 1).italic()],
                _ => Vec::new(),
            };
            kit::note_text(&key, &key, letter, text_style, ink);
            rich(letter, span)
                .key(key)
                .font(text_style)
                .color(ink)
                .w(rune::text::advance(letter, text_style))
                .a11y_hidden()
        }
    })
}

/// A hairline between groups of tools (`.mk-sep`). Key: `<key>`.
pub(super) fn hairline(key: String, t: &Theme) -> El {
    el().key(key).w(kit::HAIR).h(18.0).mx(2.0).fill(t.separator)
}

/// The raised look of a pop-up button and of the command search (`.mk-pop2`).
fn raised(e: El, t: &Theme) -> El {
    e.fill(t.card)
        .box_shadow(kit::drop(0.5, 1.5, Color::BLACK.alpha(0.18)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
}

/// A pop-up button of the ribbon (`.mk-pop2`): the choice, a caret. Keys: `<key>`, `-label`, `-caret`.
fn pop(key: &str, name: &'static str, choice: &str, w: f32, command: &'static str, t: &Theme) -> El {
    raised(
        row()
            .key(key.to_string())
            .role("combobox")
            .label(format!("{name}, {choice}"))
            .w(w)
            .h(24.0)
            .pl(9.0)
            .pr(6.0)
            .gap(4.0)
            .center_y()
            .radius(6.0),
        t,
    )
    .focusable(true)
    .on_press(ask(command))
    .child(style::fitted(format!("{key}-label"), choice, POP, t.text).a11y_hidden())
    .child(el().key(format!("{key}-gap")).grow(1.0))
    .child(kit::glyph(format!("{key}-caret"), &oi::CARET, 11.0, t.text_tertiary))
}

/// A small button of the ribbon's first row (`.kit-btn-sm`). Keys: `<key>`, `-icon`, `-label`, `-hint`.
fn small_button(key: &str, label: &'static str, icon: &'static Icon, hint: Option<&'static str>, t: &Theme) -> El {
    let mut b = row()
        .key(key.to_string())
        .role("button")
        .label(label)
        .h(22.0)
        .px(12.0)
        .gap(7.0)
        .center()
        .radius(6.0)
        .fill(t.paper)
        .border(1.0, t.card_border)
        .border_layout()
        .child(kit::glyph(format!("{key}-icon"), icon, 14.0, t.text))
        .child(style::fitted(format!("{key}-label"), label, style::BUTTON_SM, t.text));
    if let Some(hint) = hint {
        b = b.shortcut_hint(hint).child(
            el().key(format!("{key}-hint-box"))
                .ml(-3.0)
                .opacity(0.6)
                .child(style::fitted(format!("{key}-hint"), hint, HINT, t.text).a11y_hidden()),
        );
    }
    b
}

/// The ribbon's first row: the tabs, the command search, Share, Save.
fn tabs_row(ui: &Ui, unsaved: bool, t: &Theme) -> El {
    let mut r = row().key("rib-tabs").w_full().h(32.0).px(12.0).gap(4.0).center_y();
    for (i, tab) in Tab::ALL.into_iter().enumerate() {
        let on = ui.office.tab == tab;
        let mut cell = el()
            .key(format!("rib-tab-{i}-cell"))
            .role("tab")
            .label(tab.label())
            .checked(on)
            .h_full()
            .px(10.0)
            .center()
            .focusable(true)
            .on_press(move |s: &mut State| {
                if let Some(ui) = super::ui(s) {
                    ui.office.tab = tab;
                }
            })
            .child(
                style::fitted(
                    format!("rib-tab-{i}"),
                    tab.label(),
                    if on { TAB_ON } else { TAB },
                    if on { t.text } else { t.text_tertiary },
                )
                .a11y_hidden(),
            );
        if on {
            // Ink, not Amber: selection in the window is neutral.
            cell = cell.child(
                el().key("rib-tab-on")
                    .pin(Some(8.0), None, Some(8.0), Some(0.0))
                    .h(2.0)
                    .corners([4.0, 4.0, 0.0, 0.0])
                    .fill(t.text),
            );
        }
        r = r.child(cell);
    }
    let find = raised(
        row()
            .key("rib-find")
            .role("button")
            .label("Search commands, styles, files")
            .shortcut_hint("Ctrl+K")
            .h(24.0)
            .pl(9.0)
            .pr(6.0)
            .gap(6.0)
            .center_y()
            .radius(12.0),
        t,
    )
    .focusable(true)
    .on_press(|s: &mut State| {
        if let Some(ui) = super::ui(s) {
            ui.office_palette();
        }
    })
    .child(kit::glyph("rib-find-icon", &oi::FIND, 12.0, t.text_tertiary))
    .child(style::fitted(
        "rib-find-text",
        "Search commands, styles, files\u{2026}",
        CAP,
        t.text_tertiary,
    ))
    // `.kit-kbd`, as wide as its letters (the toolbar's own rounds up to half a point).
    .child(
        el().key("rib-find-kbd-box")
            .h(20.0)
            .px(6.0)
            .radius(6.0)
            .fill(t.card)
            .box_shadow(kit::ring_inset(t.separator))
            .box_shadow(kit::drop(1.0, 0.0, t.separator))
            .center()
            .child(style::fitted("rib-find-kbd", "Ctrl+K", type_::KBD, t.text_tertiary)),
    );
    let share = small_button("rib-btn-0", "Share", &oi::SHARE, None, t)
        .focusable(true)
        .on_press(|s: &mut State| {
            if let Some(ui) = super::ui(s) {
                ui.intent(Intent::Share);
            }
        });
    let save = small_button("rib-btn-1", "Save", &oi::PADLOCK, Some("Ctrl+S"), t);
    // Save is off, at 45 %, while there is nothing to save.
    let save = if unsaved {
        el().key("rib-save")
            .child(save.focusable(true).on_press(|s: &mut State| {
                if let Some(ui) = super::ui(s) {
                    ui.office_save();
                }
            }))
    } else {
        el().key("rib-save").opacity(0.45).child(save.disabled(true))
    };
    r.child(el().key("rib-tabs-gap").grow(1.0))
        .child(find)
        .child(share)
        .child(save)
}

/// Home's tools, in the board's order.
fn home_tools(t: &Theme) -> Vec<El> {
    let glyph = |icon: &'static Icon| Face::Glyph(icon, 15.0);
    let mut n = 0;
    let mut seps = 0;
    let mut next = |name: &'static str, face: Face, state: Tool, command: &'static str| {
        let key = format!("rt-{n}");
        n += 1;
        tool(&key, name, face, state, command, t)
    };
    let mut sep = || {
        seps += 1;
        hairline(format!("rsep-{}", seps - 1), t)
    };
    vec![
        next("Undo", glyph(&oi::UNDO), Tool::Idle, "Undo"),
        next("Redo", glyph(&oi::REDO), Tool::Off, "Redo"),
        sep(),
        pop("rp-0", "Style", "Normal text", 112.0, "Styles", t),
        pop("rp-1", "Font", "Inter", 92.0, "Fonts", t),
        pop("rp-2", "Size", "11", 48.0, "Font size", t),
        sep(),
        next("Bold", Face::Letter("B", LETTER_HEAVY), Tool::Idle, "Bold"),
        next("Italic", Face::Letter("I", LETTER), Tool::Idle, "Italic"),
        next("Underline", Face::Letter("U", LETTER), Tool::Idle, "Underline"),
        next("Strikethrough", Face::Letter("S", LETTER), Tool::Idle, "Strikethrough"),
        next("Text colour", glyph(&oi::TEXT_COLOUR), Tool::Idle, "Text colour"),
        next("Highlight", glyph(&oi::HIGHLIGHT), Tool::Idle, "Highlight"),
        sep(),
        next("Bulleted list", glyph(&oi::LIST_BULLETS), Tool::Idle, "Bulleted list"),
        next("Numbered list", glyph(&oi::LIST_NUMBERS), Tool::Idle, "Numbered list"),
        sep(),
        next("Align left", glyph(&oi::ALIGN_LEFT), Tool::On, "Align left"),
        next("Align centre", glyph(&oi::ALIGN_CENTRE), Tool::Idle, "Align centre"),
        next("Align right", glyph(&oi::ALIGN_RIGHT), Tool::Idle, "Align right"),
        next("Justify", glyph(&oi::JUSTIFY), Tool::Idle, "Justify"),
        sep(),
        next("Decrease indent", glyph(&oi::OUTDENT), Tool::Idle, "Decrease indent"),
        next("Increase indent", glyph(&oi::INDENT), Tool::Idle, "Increase indent"),
        sep(),
        next("Link", glyph(&oi::LINK), Tool::Idle, "Link"),
        next("Image", glyph(&oi::IMAGE), Tool::Idle, "Image"),
        next("Table", glyph(&oi::TABLE), Tool::Idle, "Table"),
    ]
}

/// The ribbon (the tabs' row over the open tab's tools).
fn ribbon(ui: &Ui, unsaved: bool, t: &Theme) -> El {
    let tools = match ui.office.tab {
        Tab::Home => home_tools(t),
        Tab::Insert => super::office_over::insert_tools(t),
        // Layout's and Review's tools are on no board frame: not drawn.
        Tab::Layout | Tab::Review => Vec::new(),
    };
    col()
        .key("rib")
        .role("toolbar")
        .label("Editor")
        .w_full()
        .fill(t.surface)
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(tabs_row(ui, unsaved, t))
        .child(
            row()
                .key("rib-tools")
                .w_full()
                .h(38.0)
                .px(8.0)
                .gap(2.0)
                .center_y()
                .fill(t.card)
                .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
                .border_layout()
                .clip_rounded(0.0)
                .children(tools),
        )
}

/// The outline pane: the document's headings, the one the caret is in marked.
fn outline(doc: &WordDoc, t: &Theme) -> El {
    let close = tool(
        "ol-close",
        "Hide the outline",
        Face::Glyph(&oi::CLOSE_PANE, 13.0),
        Tool::Idle,
        "",
        t,
    )
    .on_press(|s: &mut State| {
        if let Some(ui) = super::ui(s) {
            ui.office.outline = false;
        }
    });
    let mut pane = col()
        .key("ol")
        .role("navigation")
        .label("Outline")
        .w(210.0)
        .h_full()
        .py(12.0)
        .px(8.0)
        .gap(4.0)
        .fill(t.surface)
        .border_sides([0.0, kit::HAIR, 0.0, 0.0], t.separator)
        .border_layout()
        .clip_rounded(0.0)
        .child(
            row()
                .key("ol-head")
                .w_full()
                .px(8.0)
                .pb(4.0)
                .center_y()
                .child(kit::label("ol-title", "OUTLINE", PANE_LABEL, t.text_tertiary))
                .child(el().key("ol-head-gap").grow(1.0))
                .child(close),
        );
    for (i, heading) in doc.outline.iter().enumerate() {
        let here = i == doc.at;
        let (text_style, ink, indent) = match (i, here) {
            (0, _) => (OUTLINE_TOP, t.text_secondary, 8.0),
            (_, true) => (OUTLINE, t.text, 20.0),
            (_, false) => (OUTLINE, t.text_secondary, 20.0),
        };
        pane = pane.child(
            row()
                .key(format!("ol-{i}-row"))
                .role("link")
                .label(heading.clone())
                .w_full()
                .h(23.0)
                .pl(indent)
                .center_y()
                .radius(6.0)
                .fill(if here { t.selection } else { Color::TRANSPARENT })
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .on_press(ask("Go to a heading"))
                .child(
                    kit::label(format!("ol-{i}"), heading.clone(), text_style, ink)
                        .truncate()
                        .a11y_hidden(),
                ),
        );
    }
    pane
}

/// The page on its stage, and the selection's tools over it.
fn stage(doc: &WordDoc, t: &Theme) -> El {
    let mut page = col()
        .key("page")
        .role("document")
        .label("Page 1")
        .w(520.0)
        .min_h(700.0)
        .px(48.0)
        .py(44.0)
        .gap(10.0)
        .radius(2.0)
        .fill(Color::WHITE)
        .box_shadow(kit::drop(4.0, 18.0, Color::BLACK.alpha(0.16)))
        .child(kit::label("pg-0", doc.title.clone(), DOC_TITLE, ink()).w_full())
        .child(
            el().key("pg-1-box")
                .w_full()
                .opacity(0.6)
                .child(kit::label("pg-1", doc.byline.clone(), DOC_BYLINE, ink()).w_full()),
        );
    for (i, section) in doc.sections.iter().enumerate() {
        let key = format!("pg-{}", 3 + 2 * i);
        let selected: Vec<Span> = doc
            .selection
            .iter()
            .filter(|sel| sel.section == i)
            .map(|sel| Span::new(sel.range.start, sel.range.end).bg(Color::BLACK.alpha(0.14), 0.0))
            .collect();
        kit::note_text(&key, &key, &section.text, DOC_BODY, ink());
        page = page
            .child(
                el().key(format!("pg-{}-box", 2 + 2 * i)).w_full().pt(8.0).child(
                    kit::label(format!("pg-{}", 2 + 2 * i), section.heading.clone(), DOC_HEADING, ink())
                        .role("heading")
                        .level(2)
                        .w_full(),
                ),
            )
            .child(
                rich(&section.text, selected)
                    .key(key)
                    .font(DOC_BODY)
                    .color(ink())
                    .wrap()
                    .w_full(),
            );
        if let Some(rows) = &section.table {
            page = page.child(super::office_over::table(rows, 424.0, ink()));
        }
    }
    let mut stage = row()
        .key("stage")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .pt(24.0)
        .justify_center()
        .items_start()
        .fill(style::stage(t))
        .clip_rounded(0.0)
        .child(page);
    if let Some(selection) = &doc.selection {
        let letter = |key: &str, name: &'static str, letter: &'static str, text_style| {
            tool(key, name, Face::Letter(letter, text_style), Tool::Idle, name, t)
        };
        // 520 wide, centred: the page's left edge on the stage.
        stage = stage.child(
            row()
                .key("float-at")
                .pin(Some(0.0), Some(24.0), Some(0.0), None)
                .h(0.0)
                .justify_center()
                .child(
                    el().key("float-page").w(520.0).h(0.0).child(
                        row()
                            .key("float")
                            .role("toolbar")
                            .label("Selection")
                            .absolute(selection.tools_at.0, selection.tools_at.1)
                            .h(34.0)
                            .px(4.0)
                            .gap(2.0)
                            .center_y()
                            .radius(10.0)
                            .fill(t.card.alpha(if t.dark { 0.68 } else { 0.72 }))
                            .border(kit::HAIR, t.separator)
                            .border_layout()
                            .backdrop_blur(22.0)
                            .box_shadow(kit::drop(6.0, 22.0, Color::BLACK.alpha(0.10)))
                            .child(letter("ft-0", "Bold", "B", LETTER_HEAVY))
                            .child(letter("ft-1", "Italic", "I", LETTER))
                            .child(letter("ft-2", "Underline", "U", LETTER))
                            .child(tool(
                                "ft-3",
                                "Text colour",
                                Face::Glyph(&oi::TEXT_COLOUR, 15.0),
                                Tool::Idle,
                                "Text colour",
                                t,
                            ))
                            .child(tool(
                                "ft-4",
                                "Insert link",
                                Face::Glyph(&oi::LINK, 15.0),
                                Tool::Idle,
                                "Link",
                                t,
                            )),
                    ),
                ),
        );
    }
    stage
}

/// How the document is kept, as the status line and Save show it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Keeping {
    Saved,
    Unsaved,
    /// The last save found a newer version; the question is open (DW-1554).
    Conflict,
    /// The editor did not start: the file is shown by nothing here (DW-1556).
    NoEditor,
}

/// 11.5px/500 on 17px: a state's words at the start of the status line.
const STATUS_STATE: TextStyle = TextStyle::new(11.5, 500.0).lh(17.0);
/// The board's words in the status line of DW-1556.
pub const PREPARING: &str = "Preparing the editor on this PC\u{2026}";
pub const OLDER_FORMAT: &str = "An older Word file (.doc). It opens and saves in the same format.";

/// The status line: what the document is, how it is kept, the zoom, About. Its parts are keyed
/// `st-<N>` in order.
fn status(file: &OpenFile, doc: Option<&WordDoc>, keeping: Keeping, t: &Theme) -> El {
    let plain = |key: String, words: &str| style::fitted(key, words, type_::STATUS, t.text_tertiary);
    let sep = |key: &str| el().key(key.to_string()).w(kit::HAIR).h(18.0).mx(2.0).fill(t.separator);
    let mut n = 0;
    let mut next = || {
        n += 1;
        format!("st-{}", n - 1)
    };
    let mut bar = row()
        .key("status-bar")
        .role("status")
        .w_full()
        .h(28.0)
        .px(16.0)
        .gap(7.0)
        .center_y()
        .fill(t.status_bar_bg)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout();
    let what = match doc {
        Some(doc) => {
            let pages = format!("Page 1 of {}", doc.pages.max(1));
            let words = format!("{} words", thousands(doc.words));
            bar = bar
                .child(plain(next(), &pages))
                .child(plain(next(), "\u{00B7}"))
                .child(plain(next(), &words))
                .child(plain(next(), "\u{00B7}"))
                .child(plain(next(), &doc.language));
            format!("{pages}, {words}, {}", doc.language)
        }
        None => {
            bar = bar
                .child(
                    el().key("status-dot")
                        .size(7.0, 7.0)
                        .radius(3.5)
                        .pixel_snap(false)
                        .fill(style::warn(t)),
                )
                .child(kit::label("status-text", PREPARING, STATUS_STATE, t.text_secondary));
            PREPARING.to_string()
        }
    };
    let kept = match (keeping, doc) {
        (Keeping::Saved, Some(doc)) => format!("saved as version {} \u{00B7} {}", file.version, doc.saved_at),
        (Keeping::Unsaved, _) => "unsaved changes".to_string(),
        (Keeping::Conflict, _) => "not saved \u{00B7} conflict".to_string(),
        (Keeping::NoEditor, _) | (Keeping::Saved, None) => "read-only for now".to_string(),
    };
    let enc = next();
    let dot = next();
    let kept_key = next();
    let kept_el = match (keeping, doc) {
        // The time is a figure: mono.
        (Keeping::Saved, Some(doc)) => {
            let at = kept.len() - doc.saved_at.len();
            kit::note_text(&kept_key, &kept_key, &kept, type_::STATUS, t.text_tertiary);
            rich(&kept, vec![Span::new(at, kept.len()).mono()])
                .key(kept_key)
                .font(type_::STATUS)
                .color(t.text_tertiary)
                .w(rune::text::advance(&kept[..at], type_::STATUS) + rune::text::advance(&doc.saved_at, STATUS_MONO))
        }
        _ => plain(kept_key, &kept),
    };
    let zoom = next();
    let about = next();
    bar.label(format!("{what}. Encrypted, {kept}"))
        .child(el().key("st-gap").grow(1.0))
        .child(
            row()
                .key(format!("{enc}-box"))
                .gap(4.0)
                .center_y()
                .child(kit::glyph(format!("{enc}-icon"), &oi::PADLOCK, 11.0, t.accent_deep))
                .child(style::fitted(enc, "Encrypted", STATUS_ENC, t.accent_deep)),
        )
        .child(plain(dot, "\u{00B7}"))
        .child(kept_el)
        .child(sep("st-sep-0"))
        .child(
            row()
                .key(zoom.clone())
                .role("group")
                .label("Zoom, 100 per cent")
                .gap(4.0)
                .center_y()
                .child(kit::glyph(format!("{zoom}-less"), &oi::LESS, 12.0, t.text_tertiary))
                .child(style::fitted(
                    format!("{zoom}-zoom"),
                    "100%",
                    STATUS_MONO,
                    t.text_tertiary,
                ))
                .child(kit::glyph(format!("{zoom}-more"), &oi::MORE, 12.0, t.text_tertiary)),
        )
        .child(sep("st-sep-1"))
        .child(
            el().key(format!("{about}-box"))
                .role("link")
                .label("About this editor")
                .focusable(true)
                .on_press(ask("About this editor"))
                .child({
                    kit::note_text(&about, &about, "About", type_::STATUS, t.text_tertiary);
                    rich("About", vec![Span::new(0, 5).underline(t.text_tertiary)])
                        .key(about)
                        .font(type_::STATUS)
                        .color(t.text_tertiary)
                        .w(rune::text::advance("About", type_::STATUS))
                }),
        )
}

/// `1284` as "1,284".
fn thousands(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The window's toolbar with the document's path where a folder's title is (`ui::toolbar::bar`).
fn toolbar(s: &State, ui: &Ui, unsaved: bool, t: &Theme) -> El {
    let file = &ui.file;
    let mut crumbs = row()
        .key("crumbs")
        .role("navigation")
        .label("Path")
        .center_y()
        .gap(4.0)
        .min_w(0.0);
    for (i, folder) in file.path.iter().enumerate() {
        crumbs = crumbs
            .child(kit::label(
                format!("crumb-{i}"),
                folder.clone(),
                style::CRUMB,
                t.text_tertiary,
            ))
            .child(kit::glyph(
                format!("crumb-sep-{i}"),
                &icons::CHEVRON_RIGHT,
                12.0,
                t.text_tertiary,
            ));
    }
    let name = kit::label(
        if unsaved { "crumb-name" } else { "crumb-here" },
        file.name.clone(),
        style::CRUMB_NAME,
        t.text_tertiary,
    )
    .truncate()
    .min_w(0.0);
    crumbs = crumbs.child(if unsaved {
        // The dot after the name: changes no version holds yet.
        kit::note_text(
            "crumb-here",
            "crumb-here-row",
            &file.name,
            style::CRUMB_NAME,
            t.text_tertiary,
        );
        row()
            .key("crumb-here-row")
            .gap(6.0)
            .center_y()
            .min_w(0.0)
            .child(name)
            .child(
                el().key("crumb-dot")
                    .role("img")
                    .label("Unsaved changes")
                    .size(7.0, 7.0)
                    .radius(3.5)
                    // After the name, wherever its last letter ends.
                    .pixel_snap(false)
                    .fill(t.text),
            )
    } else {
        name
    });
    let title = col()
        .key("folder-info")
        .role("group")
        .label(format!("{}, {}", file.name, file.facts))
        .px(4.0)
        .min_w(0.0)
        .child(crumbs)
        .child(kit::label(
            "item-count",
            file.facts.clone(),
            type_::TITLE_SUB,
            t.text_tertiary,
        ));
    const SEARCH_PLACEHOLDER: &str = "Search names";
    let search = tb::search_box(
        "search-input",
        SEARCH_PLACEHOLDER,
        text_input(&s.search, type_::SEARCH)
            .key("search-input")
            .label("Search names")
            .shortcut_hint("Ctrl+F")
            .single_line()
            .color(t.text)
            .placeholder(SEARCH_PLACEHOLDER, t.text_tertiary)
            .on_edit(|s: &mut State| &mut s.search)
            .grow(1.0),
        t,
    );
    tb::bar(
        Bar {
            name: file.name.clone(),
            back: (true, press(|s, _| super::close(s))),
            forward: (false, None),
            title,
            slot: None,
            slot_sep: false,
            // A document has one layout and nothing to sort: drawn dimmed, never left out.
            view: View::NotHere,
            on_view: None,
            sort: Sort::NotHere,
            on_sort: None,
            details: if ui.show_details {
                Details::Open
            } else {
                Details::Closed
            },
            on_details: press(|s, _| {
                if let Some(ui) = super::ui(s) {
                    ui.toggle_details();
                }
            }),
            inert: false,
            on_explorer: Box::new(tb::open_beebeeb_location),
            new: New::Idle,
            on_new: press(|s, _| {
                if let Some(ui) = super::ui(s) {
                    ui.intent(Intent::NewMenu);
                }
            }),
            search,
            trailing_inset: 12.0,
        },
        t,
    )
}

/// The window with the editor's frame: a text document (`doc`), or what is shown when the editor
/// did not start (`None`, DW-1556).
pub fn window(s: &State, cx: &mut Cx<State>, ui: &Ui, doc: Option<&WordDoc>) -> El {
    let t = tokens::theme(cx.system_dark());
    let keeping = match (doc, &ui.office.over) {
        (None, _) => Keeping::NoEditor,
        (Some(_), Some(super::office_over::Over::Conflict(_))) => Keeping::Conflict,
        (Some(doc), _) if doc.kept == Kept::Unsaved => Keeping::Unsaved,
        (Some(_), _) => Keeping::Saved,
    };
    let unsaved = matches!(keeping, Keeping::Unsaved | Keeping::Conflict);
    let mut body = row().key("viewer-row").grow(1.0).min_h(0.0).w_full();
    body = match doc {
        Some(doc) => {
            if ui.office.outline {
                body = body.child(outline(doc, &t));
            }
            body.child(stage(doc, &t))
        }
        None => body.child(super::office_over::no_editor(&t)),
    };
    if ui.show_details {
        body = body.child(super::details::panel(&ui.file, ui.knows_offline(), &t));
    }
    let mut main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(toolbar(s, ui, unsaved, &t));
    if doc.is_some_and(|doc| doc.older) {
        main = main.child(super::text::banner_el(&super::icons::INFO, OLDER_FORMAT, None, &t));
    }
    let ribbon = ribbon(ui, unsaved, &t);
    main = main
        .child(match doc {
            Some(_) => ribbon,
            // No editor: its controls are drawn at 45 % and take no press and no key.
            None => el().key("rib-dim").w_full().opacity(0.45).inert().child(ribbon),
        })
        .child(body)
        .child(status(&ui.file, doc, keeping, &t));
    let mut under = el().key("under-caption").grow(1.0).min_h(0.0).w_full().child(
        row()
            .key("body")
            .w_full()
            .h_full()
            .child(sidebar::sidebar(s, &t))
            .child(main),
    );
    if let Some(words) = &ui.toast {
        under = under.child(super::dialogs::toast(words, &t));
    }
    if let Some(over) = super::office_over::over(ui, &t) {
        under = under.child(over);
    }
    col()
        .key("window")
        .role("window")
        .label(s.identity.main_window_title())
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption::caption_bar(s.identity.main_window_title(), &t, cx))
        .child(under)
}

/// The board's sample documents, for the fixture source.
pub use super::office_board as board;
