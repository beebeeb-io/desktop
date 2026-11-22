//! The word ribbon's tools (a later task fills this). Until then the shared ribbon of `viewers/office.rs`.
#![allow(dead_code)]

use beebeeb_office_protocol::office::{EditOp, ListKind, Mark, Marks, ParaStyle, Rgb, TextAlign};
use rune::{Color, El, Key, KeyEvent, TextEdit, TextStyle, Tx, col, el, row, text_input};

use crate::theme::scale::{self as sc, space};
use crate::State;
use crate::frames::viewers::office::{self as vo, Face, Tab, Tool, ico};
use crate::frames::viewers::{self, Intent, Ui};
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;
use crate::ui::menu::{self, Entry};

/// The pop-ups of the Home tab, one open at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pop {
    Style,
    Font,
    Size,
    TextColour,
    Highlight,
    Table,
    Link,
}

/// What the ribbon keeps while a pop-up is open (`OfficeUi.ribbon`).
#[derive(Default)]
pub struct Pops {
    pub open: Option<Pop>,
    /// The highlighted entry of the open menu.
    pub hot: Option<usize>,
    /// The address being typed (the Link pop-up).
    pub link: Option<TextEdit>,
    /// What the Link pop-up says is wrong with the address.
    pub link_wrong: bool,
}

impl Pops {
    fn close(&mut self) {
        self.open = None;
        self.hot = None;
        self.link_wrong = false;
        // An address typed is the person's: not kept after the pop-up.
        if let Some(link) = self.link.as_mut() {
            link.replace_text("");
        }
        self.link = None;
    }
}

const MENU: &str = "rib-menu";
/// Under the ribbon's tool row, in the ribbon's own coordinates.
const MENU_Y: f32 = 68.0;
/// The ribbon's height: the tab row, the tool row, their hairlines.
const RIB_H: f32 = 32.0 + 38.0 + 2.0 * kit::HAIR;
const OUTLINE_W: f32 = 210.0;
const OUTLINE_STRIP_W: f32 = 44.0;
const DETAILS_W: f32 = 280.0;
const PAGE_W: f32 = 520.0;
const FLOAT_W: f32 = 4.0 * 2.0 + 5.0 * 28.0 + 4.0 * 2.0;
const FLOAT_H: f32 = 34.0;
const TOOL_WORD: TextStyle = sc::text::SUB;
const LINK_BODY: TextStyle = sc::text::READ;
const LINK_NOTE: TextStyle = crate::theme::tokens::type_::BODY_SM;

/// A style the pop-up offers, with the words on its button.
const STYLES: [(&str, &str, ParaStyle); 5] = [
    ("normal", "Normal text", ParaStyle::Normal),
    ("title", "Title", ParaStyle::Title),
    ("h1", "Heading 1", ParaStyle::Heading1),
    ("h2", "Heading 2", ParaStyle::Heading2),
    ("h3", "Heading 3", ParaStyle::Heading3),
];
const FONTS: [&str; 6] = ["Inter", "Arial", "Calibri", "Georgia", "Times New Roman", "Courier New"];
const SIZES: [&str; 10] = ["8", "9", "10", "11", "12", "14", "16", "18", "24", "36"];
/// Text colours (ids are the hex).
const INKS: [(&str, &str); 7] = [
    ("0D0D0D", "Black"),
    ("595959", "Dark grey"),
    ("C0392B", "Red"),
    ("D35400", "Orange"),
    ("2E7D32", "Green"),
    ("1F5FBF", "Blue"),
    ("7B3FA0", "Purple"),
];
/// The engine's highlight palette is fixed (it snaps to the nearest of its own): these are its colours.
const MARKERS: [(&str, &str); 4] = [
    ("FFFF00", "Yellow"),
    ("00FF00", "Green"),
    ("00FFFF", "Cyan"),
    ("FF00FF", "Pink"),
];
/// Table sizes: rows x columns.
const TABLES: [(&str, &str, u32, u32); 5] = [
    ("2x2", "2 \u{00D7} 2", 2, 2),
    ("3x3", "3 \u{00D7} 3", 3, 3),
    ("4x4", "4 \u{00D7} 4", 4, 4),
    ("5x3", "5 rows \u{00D7} 3 columns", 5, 3),
    ("6x4", "6 rows \u{00D7} 4 columns", 6, 4),
];

fn rgb(hex: &str) -> Option<Rgb> {
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(Rgb {
        r: (n >> 16) as u8,
        g: (n >> 8) as u8,
        b: n as u8,
    })
}

/// What the ribbon says of the style at the caret.
fn style_words(marks: Option<Marks>) -> &'static str {
    let style = marks.map_or(ParaStyle::Normal, |m| m.style);
    STYLES
        .iter()
        .find(|(_, _, s)| *s == style)
        .map_or("Normal text", |(_, w, _)| w)
}

// ---------------------------------------------------------------------------
// Sending an edit
// ---------------------------------------------------------------------------

/// Hand `op` to the editor through the source. A source that does not take it says "Not built yet", never nothing.
pub fn send(s: &mut State, tx: &mut Tx, op: EditOp, asked: &'static str) {
    tx.instant();
    // The press that reached the tool (or its menu) took the keyboard with it. The tool has acted: the keyboard is
    // the page's again, so the next key is typed into the document and not handed to the tool.
    super::word::focus(tx);
    if let Some(ui) = viewers::ui(s) {
        let (source, id) = ui.edit_source();
        let id = id.to_string();
        if !source.edit(&id, op) {
            ui.intent(Intent::Office(asked));
        }
    }
}

fn ops(op: EditOp, asked: &'static str) -> impl Fn(&mut State, &mut Tx) + 'static {
    move |s: &mut State, tx: &mut Tx| {
        // A tool pressed while a pop-up is open closes it.
        if let Some(ui) = viewers::ui(s) {
            ui.office.ribbon.close();
        }
        send(s, tx, op.clone(), asked);
    }
}

fn open_pop(pop: Pop) -> impl Fn(&mut State, &mut Tx) + 'static {
    move |s: &mut State, tx: &mut Tx| {
        tx.instant();
        if let Some(ui) = viewers::ui(s) {
            let was = ui.office.ribbon.open;
            ui.office.ribbon.close();
            if was != Some(pop) {
                ui.office.ribbon.open = Some(pop);
                if pop == Pop::Link {
                    ui.office.ribbon.link = Some(TextEdit::new(""));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The tools
// ---------------------------------------------------------------------------

/// The Home row as it is built: its tools, and where along the row each pop-up's button starts.
struct Row<'a> {
    t: &'a Theme,
    tools: Vec<El>,
    x: f32,
    n: usize,
    seps: usize,
    at: Vec<(Pop, f32)>,
}

impl<'a> Row<'a> {
    fn new(t: &'a Theme) -> Self {
        Row {
            t,
            tools: Vec::new(),
            // The row's own padding.
            x: 8.0,
            n: 0,
            seps: 0,
            at: Vec::new(),
        }
    }

    fn tool(&mut self, name: &'static str, face: Face, state: Tool, act: impl Fn(&mut State, &mut Tx) + 'static) {
        let key = format!("rt-{}", self.n);
        self.n += 1;
        self.tools.push(vo::tool_with(&key, name, face, state, act, self.t));
        self.x += 28.0 + 2.0;
    }

    /// A tool that opens `pop`: its menu hangs under it.
    fn tool_pop(&mut self, name: &'static str, face: Face, pop: Pop) {
        self.at.push((pop, self.x));
        self.tool(name, face, Tool::Idle, open_pop(pop));
    }

    fn sep(&mut self) {
        self.tools.push(vo::hairline(format!("rsep-{}", self.seps), self.t));
        self.seps += 1;
        self.x += kit::HAIR + 4.0 + 2.0;
    }

    fn pop(&mut self, key: &str, name: &'static str, choice: &str, w: f32, pop: Pop) {
        self.at.push((pop, self.x));
        self.tools
            .push(vo::pop_with(key, name, choice, w, open_pop(pop), self.t));
        self.x += w + 2.0;
    }
}

fn on(lit: bool) -> Tool {
    if lit { Tool::On } else { Tool::Idle }
}

/// Home's tools, lit from what the helper last said of the caret.
fn home_tools(ui: &Ui, t: &Theme) -> (Vec<El>, Vec<(Pop, f32)>) {
    let edit = ui.office.edit.as_ref();
    let marks = edit.and_then(|e| e.marks);
    let (can_undo, can_redo) = edit.map_or((false, false), |e| (e.can_undo, e.can_redo));
    let lit = |f: fn(&Marks) -> bool| marks.as_ref().is_some_and(f);
    let glyph = |icon: &'static rune::Icon| Face::Glyph(icon, 15.0);
    let align = marks.map(|m| m.align);
    let list = marks.map(|m| m.list);
    let mut r = Row::new(t);
    r.tool(
        "Undo",
        glyph(&ico::UNDO),
        if can_undo { Tool::Idle } else { Tool::Off },
        ops(EditOp::Undo, "Undo"),
    );
    r.tool(
        "Redo",
        glyph(&ico::REDO),
        if can_redo { Tool::Idle } else { Tool::Off },
        ops(EditOp::Redo, "Redo"),
    );
    r.sep();
    // The helper reports the style, not the font, size or colour: those buttons say what they are for.
    r.pop("rp-0", "Style", style_words(marks), 112.0, Pop::Style);
    r.pop("rp-1", "Font", "Font", 92.0, Pop::Font);
    r.pop("rp-2", "Size", "Size", 48.0, Pop::Size);
    r.sep();
    for (name, letter, style, mark, is_on) in [
        ("Bold", "B", vo::LETTER_HEAVY, Mark::Bold, lit(|m| m.bold)),
        ("Italic", "I", vo::LETTER, Mark::Italic, lit(|m| m.italic)),
        ("Underline", "U", vo::LETTER, Mark::Underline, lit(|m| m.underline)),
        ("Strikethrough", "S", vo::LETTER, Mark::Strike, lit(|m| m.strike)),
    ] {
        r.tool(
            name,
            Face::Letter(letter, style),
            on(is_on),
            ops(EditOp::Mark { mark, on: !is_on }, name),
        );
    }
    r.tool_pop("Text colour", glyph(&ico::TEXT_COLOUR), Pop::TextColour);
    r.tool_pop("Highlight", glyph(&ico::HIGHLIGHT), Pop::Highlight);
    r.sep();
    for (name, icon, kind) in [
        ("Bulleted list", &ico::LIST_BULLETS, ListKind::Bullet),
        ("Numbered list", &ico::LIST_NUMBERS, ListKind::Number),
    ] {
        let is_on = list == Some(kind);
        // A lit list tool turns the list off.
        let send_kind = if is_on { ListKind::None } else { kind };
        r.tool(name, glyph(icon), on(is_on), ops(EditOp::List(send_kind), name));
    }
    r.sep();
    for (name, icon, to) in [
        ("Align left", &ico::ALIGN_LEFT, TextAlign::Left),
        ("Align centre", &ico::ALIGN_CENTRE, TextAlign::Center),
        ("Align right", &ico::ALIGN_RIGHT, TextAlign::Right),
        ("Justify", &ico::JUSTIFY, TextAlign::Justify),
    ] {
        r.tool(name, glyph(icon), on(align == Some(to)), ops(EditOp::Align(to), name));
    }
    r.sep();
    r.tool(
        "Decrease indent",
        glyph(&ico::OUTDENT),
        Tool::Idle,
        ops(EditOp::Indent(-1), "Decrease indent"),
    );
    r.tool(
        "Increase indent",
        glyph(&ico::INDENT),
        Tool::Idle,
        ops(EditOp::Indent(1), "Increase indent"),
    );
    r.sep();
    r.tool_pop("Link", glyph(&ico::LINK), Pop::Link);
    // The picture is chosen in Insert (its picker is that tab's).
    r.tool("Image", glyph(&ico::IMAGE), Tool::Idle, |s: &mut State, tx: &mut Tx| {
        tx.instant();
        if let Some(ui) = viewers::ui(s) {
            ui.office.ribbon.close();
            ui.office.tab = Tab::Insert;
        }
    });
    r.tool_pop("Table", glyph(&ico::TABLE), Pop::Table);
    (r.tools, r.at)
}

/// A tool with its word (Layout's and Review's): 28 high, the glyph, the word.
fn worded(
    key: &str,
    word: &'static str,
    icon: &'static rune::Icon,
    act: impl Fn(&mut State) + 'static,
    t: &Theme,
) -> El {
    row()
        .key(key.to_string())
        .role("button")
        .label(word)
        .h(28.0)
        .px(space::S6)
        .gap(space::S4)
        .center_y()
        .radius(crate::theme::tokens::radius::CONTROL)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .on_press(act)
        .child(kit::glyph(format!("{key}-icon"), icon, 15.0, t.text_secondary))
        .child(
            kit::label(format!("{key}-label"), word, TOOL_WORD, t.text)
                .w(rune::text::advance(word, TOOL_WORD))
                .a11y_hidden(),
        )
}

/// What the web editor has under Layout: Page layout.
fn layout_tools(t: &Theme) -> Vec<El> {
    vec![worded(
        "rt-0",
        "Page layout\u{2026}",
        &ico::TABLE,
        vo::ask("Page layout\u{2026}"),
        t,
    )]
}

/// What the web editor has under Review: Word count.
fn review_tools(t: &Theme) -> Vec<El> {
    vec![worded("rt-0", "Word count\u{2026}", &ico::LIST_NUMBERS, word_count, t)]
}

/// The words and pages the document has now, said in the message line.
fn word_count(s: &mut State) {
    if let Some(ui) = viewers::ui(s)
        && let crate::frames::viewers::source::Body::Doc(d) = &ui.file.body
    {
        {
            let pages = d.view.pages.max(1);
            ui.toast = Some(format!(
                "{} {} \u{00B7} {} {}",
                d.view.words,
                if d.view.words == 1 { "word" } else { "words" },
                pages,
                if pages == 1 { "page" } else { "pages" },
            ));
        }
    }
}

/// The open tab's tools as one arrow-roving group: Tab stops on one tool, the arrows, Home and End move among
/// them. The ribbon's tabs and its search stay Tab stops of their own, outside the group: an arrow that lands on
/// a tab presses it (Rune's rule for role `tab`), and that would swap the row of tools under the keyboard.
pub fn tool_group(tools: Vec<El>) -> Vec<El> {
    if tools.is_empty() {
        return tools;
    }
    // The row it sits in (`rib-tools`), again: same gap, same centring, all of its room.
    vec![
        row()
            .key("rib-group")
            .role("group")
            .label("Tools")
            .focus_group()
            .w_full()
            .h_full()
            .gap(space::S2)
            .center_y()
            .children(tools),
    ]
}

/// The ribbon of a live text document: the tabs, the open tab's tools and, over everything, the open pop-up and
/// the selection's tools.
pub fn ribbon(ui: &Ui, unsaved: bool, t: &Theme) -> El {
    let mut at = Vec::new();
    let tools = match ui.office.tab {
        Tab::Home => {
            let (tools, places) = home_tools(ui, t);
            at = places;
            tools
        }
        Tab::Insert => {
            let tools = super::word_insert::tools(t);
            if tools.is_empty() {
                viewers::office_over::insert_tools(t)
            } else {
                tools
            }
        }
        Tab::Layout => layout_tools(t),
        Tab::Review => review_tools(t),
        // A workbook's tabs: never on a text document's row.
        Tab::Data | Tab::Formulas => Vec::new(),
    };
    let mut rib = vo::ribbon_of(ui, unsaved, tool_group(tools), t);
    let pops = &ui.office.ribbon;
    if let Some(pop) = pops.open {
        // Opened from the selection's tools while another tab shows: the Home row's places still say where.
        if at.is_empty() {
            at = home_tools(ui, t).1;
        }
        let x = at.iter().find(|(p, _)| *p == pop).map_or(8.0, |(_, x)| *x);
        rib = rib.child(popup(ui, pop, x, t));
    }
    if pops.open.is_none()
        && let Some(float) = floating(ui, t)
    {
        rib = rib.child(float);
    }
    rib
}

// ---------------------------------------------------------------------------
// The pop-ups
// ---------------------------------------------------------------------------

fn entries(pop: Pop, marks: Option<Marks>) -> Vec<Entry> {
    match pop {
        Pop::Style => {
            let now = marks.map_or(ParaStyle::Normal, |m| m.style);
            STYLES
                .iter()
                .map(|(id, words, s)| Entry::check(id, words, *s == now))
                .collect()
        }
        Pop::Font => FONTS.iter().map(|f| Entry::item(f, f)).collect(),
        Pop::Size => SIZES.iter().map(|z| Entry::item(z, z)).collect(),
        Pop::TextColour => INKS.iter().map(|(id, name)| Entry::item(id, name)).collect(),
        Pop::Highlight => {
            let mut all: Vec<Entry> = MARKERS.iter().map(|(id, name)| Entry::item(id, name)).collect();
            all.push(Entry::Sep);
            all.push(Entry::item("none", "No highlight"));
            all
        }
        Pop::Table => TABLES.iter().map(|(id, words, ..)| Entry::item(id, words)).collect(),
        Pop::Link => Vec::new(),
    }
}

/// The edit an entry stands for.
fn op_of(pop: Pop, id: &str) -> Option<(EditOp, &'static str)> {
    Some(match pop {
        Pop::Style => (EditOp::ParaStyle(STYLES.iter().find(|(i, ..)| *i == id)?.2), "Styles"),
        Pop::Font => (
            EditOp::FontFamily(FONTS.iter().find(|f| **f == id)?.to_string()),
            "Fonts",
        ),
        Pop::Size => (EditOp::FontSize(id.parse().ok()?), "Font size"),
        Pop::TextColour => (EditOp::TextColour(rgb(id)?), "Text colour"),
        Pop::Highlight if id == "none" => (EditOp::Highlight(None), "Highlight"),
        Pop::Highlight => (EditOp::Highlight(Some(rgb(id)?)), "Highlight"),
        Pop::Table => {
            let (_, _, rows, cols) = TABLES.iter().find(|(i, ..)| *i == id)?;
            (
                EditOp::Table {
                    rows: *rows,
                    cols: *cols,
                },
                "Table",
            )
        }
        Pop::Link => return None,
    })
}

fn name_of(pop: Pop) -> (&'static str, f32) {
    match pop {
        Pop::Style => ("Style", 160.0),
        Pop::Font => ("Font", 180.0),
        Pop::Size => ("Size", 90.0),
        Pop::TextColour => ("Text colour", 160.0),
        Pop::Highlight => ("Highlight", 160.0),
        Pop::Table => ("Table size", 200.0),
        Pop::Link => ("Link", 280.0),
    }
}

fn marks_of(s: &State) -> Option<Marks> {
    s.viewer.ui.as_ref()?.office.edit.as_ref()?.marks
}

fn pick(s: &mut State, index: usize, tx: &mut Tx) {
    let Some(pop) = s.viewer.ui.as_ref().and_then(|ui| ui.office.ribbon.open) else {
        return;
    };
    let all = entries(pop, marks_of(s));
    let Some(id) = all.get(index).and_then(Entry::id) else {
        return;
    };
    if let Some(ui) = viewers::ui(s) {
        ui.office.ribbon.close();
    }
    if let Some((op, asked)) = op_of(pop, id) {
        send(s, tx, op, asked);
    }
}

fn hover(s: &mut State, index: usize) {
    if let Some(ui) = viewers::ui(s) {
        ui.office.ribbon.hot = Some(index);
    }
}

fn close(s: &mut State, tx: &mut Tx) {
    tx.instant();
    if let Some(ui) = viewers::ui(s) {
        ui.office.ribbon.close();
    }
}

fn menu_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(pop) = s.viewer.ui.as_ref().and_then(|ui| ui.office.ribbon.open) else {
        return false;
    };
    let all = entries(pop, marks_of(s));
    let hot = s.viewer.ui.as_ref().and_then(|ui| ui.office.ribbon.hot);
    let to = match ev.key {
        Key::Down => menu::step(&all, hot, 1),
        Key::Up => menu::step(&all, hot, -1),
        Key::Home => menu::step(&all, None, 1),
        Key::End => menu::step(&all, None, -1),
        Key::Enter => {
            if let Some(hot) = hot {
                pick(s, hot, tx);
            }
            return true;
        }
        Key::Escape => {
            close(s, tx);
            return true;
        }
        // A menu is modal: no other key reaches the page under it.
        _ => return true,
    };
    if let (Some(to), Some(ui)) = (to, viewers::ui(s)) {
        ui.office.ribbon.hot = Some(to);
        if let Some(key) = menu::item_key(MENU, &all, to) {
            tx.focus(key, false);
        }
    }
    true
}

/// The pop-up, hung under its button and over everything else.
fn popup(ui: &Ui, pop: Pop, x: f32, t: &Theme) -> El {
    let (name, w) = name_of(pop);
    let body = if pop == Pop::Link {
        link_box(ui, w, t)
    } else {
        let marks = ui.office.edit.as_ref().and_then(|e| e.marks);
        menu::menu(
            MENU,
            name,
            w,
            &entries(pop, marks),
            ui.office.ribbon.hot,
            t,
            pick,
            hover,
            close,
        )
        .on_key(menu_key)
    };
    body.absolute(x.max(0.0), MENU_Y).raise()
}

/// A web or mail address the engine accepts; the scheme is added to a bare address.
pub fn link_address(typed: &str) -> Option<String> {
    let typed = typed.trim();
    if typed.is_empty() || typed.len() > 4096 || typed.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let lower = typed.to_ascii_lowercase();
    if ["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p)) {
        return Some(typed.to_string());
    }
    // "name@host" is a mail address, "host.tld/path" a web one; anything with another scheme is neither.
    if typed.contains("://") || (typed.contains(':') && !typed.contains('/')) {
        return None;
    }
    if typed.contains('@') && !typed.contains('/') {
        return Some(format!("mailto:{typed}"));
    }
    typed.contains('.').then(|| format!("https://{typed}"))
}

fn link_field(s: &mut State) -> &mut TextEdit {
    match s.viewer.ui.as_mut().and_then(|ui| ui.office.ribbon.link.as_mut()) {
        Some(edit) => edit,
        None => &mut s.viewer.stray,
    }
}

fn link_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    match ev.key {
        Key::Escape => {
            close(s, tx);
            true
        }
        Key::Enter => {
            let typed = s
                .viewer
                .ui
                .as_ref()
                .and_then(|ui| ui.office.ribbon.link.as_ref())
                .map(|l| l.text.clone())
                .unwrap_or_default();
            match link_address(&typed) {
                Some(url) => {
                    close(s, tx);
                    send(
                        s,
                        tx,
                        EditOp::Link {
                            text: String::new(),
                            url,
                        },
                        "Link",
                    );
                }
                None => {
                    if let Some(ui) = viewers::ui(s) {
                        ui.office.ribbon.link_wrong = true;
                    }
                }
            }
            true
        }
        _ => false,
    }
}

fn link_box(ui: &Ui, w: f32, t: &Theme) -> El {
    let pops = &ui.office.ribbon;
    let empty = TextEdit::new("");
    let edit = pops.link.as_ref().unwrap_or(&empty);
    let input = text_input(edit, LINK_BODY)
        .key("rib-link-input")
        .label("Link address")
        .single_line()
        .color(t.text)
        .placeholder("https://", t.text_tertiary)
        .autofocus()
        .on_edit(link_field)
        .on_input_key(|s: &mut State, ev: &KeyEvent, tx: &mut Tx| link_key(s, ev, tx))
        .w_full();
    let mut b = col()
        .key(MENU)
        .role("dialog")
        .label("Link")
        .w(w)
        .padding(space::S10)
        .gap(space::S6)
        .radius(8.0)
        .fill(menu::raised(t))
        .box_shadow(kit::drop(8.0, 28.0, Color::BLACK.alpha(0.20)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .modal_tx(true, close)
        .child(kit::label(
            "rib-link-title",
            "Link address",
            type_::CAPTION,
            t.text_tertiary,
        ))
        .child(input);
    if pops.link_wrong {
        b = b.child(kit::label(
            "rib-link-wrong",
            "Use a web address (https://) or a mail address.",
            LINK_NOTE,
            t.text_secondary,
        ));
    } else {
        b = b.child(kit::label(
            "rib-link-hint",
            "Press Enter to put it at the cursor.",
            LINK_NOTE,
            t.text_tertiary,
        ));
    }
    b
}

// ---------------------------------------------------------------------------
// The selection's tools
// ---------------------------------------------------------------------------

/// Where the selection's tools go, from the page's top-left corner at the stage's zoom: just above the
/// selection's first rectangle (below it when it is at the top of the stage). `None`: nothing is selected, the
/// body is not a live document, or the selection is not on screen.
pub fn float_at(ui: &Ui) -> Option<(f32, f32, f32)> {
    let edit = ui.office.edit.as_ref()?;
    let crate::frames::viewers::source::Body::Doc(d) = &ui.file.body else {
        return None;
    };
    let first = edit.selection.iter().find(|r| r.rect.w > 0.0 && r.rect.h > 0.0)?;
    let page = first.page.checked_add(1)?;
    if page > d.view.pages.max(1) {
        return None;
    }
    let zoom = f32::from(d.view.zoom) / 100.0;
    let w_pt = d
        .view
        .page_sizes_pt
        .get(first.page as usize)
        .map(|s| s.0)
        .filter(|w| w.is_finite() && *w > 0.0)
        .unwrap_or(595.0);
    // A page is the board's width at the zoom; its points scale to that.
    let scale = PAGE_W * zoom / w_pt;
    let page_w = PAGE_W * zoom;
    let top = d.page_top(page) + first.rect.y * scale - d.view_y();
    let (x, y) = (first.rect.x * scale, top - FLOAT_H - 6.0);
    // Scrolled out of view, above or below the stage.
    if top + first.rect.h * scale < 0.0 {
        return None;
    }
    let y = if y < 0.0 { top + first.rect.h * scale + 6.0 } else { y };
    Some((x.clamp(0.0, (page_w - FLOAT_W).max(0.0)), y, page_w))
}

/// The floating tools over a selection: Bold, Italic, Underline, Text colour, Insert link. Raised over the stage,
/// pinned to where the stage is (beside the outline, under the ribbon) and centred like the page.
fn floating(ui: &Ui, t: &Theme) -> Option<El> {
    let (x, y, page_w) = float_at(ui)?;
    let crate::frames::viewers::source::Body::Doc(d) = &ui.file.body else {
        return None;
    };
    let marks = ui.office.edit.as_ref().and_then(|e| e.marks);
    let lit = |f: fn(&Marks) -> bool| marks.as_ref().is_some_and(f);
    let mark = |key: &str, name: &'static str, letter: &'static str, style, mark: Mark, is_on: bool| {
        vo::tool_with(
            key,
            name,
            Face::Letter(letter, style),
            on(is_on),
            ops(EditOp::Mark { mark, on: !is_on }, name),
            t,
        )
    };
    let left = if d.outline_open { OUTLINE_W } else { OUTLINE_STRIP_W };
    let right = if ui.show_details { DETAILS_W } else { 0.0 };
    Some(
        row()
            .key("float-at")
            .pin(Some(left), Some(RIB_H), Some(right), None)
            .h(0.0)
            .justify_center()
            .raise()
            .child(
                el().key("float-page").w(page_w).h(0.0).child(
                    row()
                        .key("float")
                        .role("toolbar")
                        .label("Selection")
                        .absolute(x, y)
                        .h(FLOAT_H)
                        .px(space::S4)
                        .gap(space::S2)
                        .center_y()
                        .radius(sc::radius::PANEL)
                        .fill(t.card.alpha(if t.dark { 0.68 } else { 0.72 }))
                        .border(kit::HAIR, t.separator)
                        .border_layout()
                        .backdrop_blur(22.0)
                        .box_shadow(kit::drop(6.0, 22.0, Color::BLACK.alpha(0.10)))
                        .child(mark("ft-0", "Bold", "B", vo::LETTER_HEAVY, Mark::Bold, lit(|m| m.bold)))
                        .child(mark("ft-1", "Italic", "I", vo::LETTER, Mark::Italic, lit(|m| m.italic)))
                        .child(mark(
                            "ft-2",
                            "Underline",
                            "U",
                            vo::LETTER,
                            Mark::Underline,
                            lit(|m| m.underline),
                        ))
                        .child(vo::tool_with(
                            "ft-3",
                            "Text colour",
                            Face::Glyph(&ico::TEXT_COLOUR, 15.0),
                            Tool::Idle,
                            open_pop(Pop::TextColour),
                            t,
                        ))
                        .child(vo::tool_with(
                            "ft-4",
                            "Insert link",
                            Face::Glyph(&ico::LINK, 15.0),
                            Tool::Idle,
                            open_pop(Pop::Link),
                            t,
                        )),
                ),
            ),
    )
}
