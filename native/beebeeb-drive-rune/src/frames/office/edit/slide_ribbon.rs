//! The slide ribbon's tools: Home (Undo, Redo, font, Bold, Italic, Underline, Align centre, New slide, Duplicate
//! slide, Delete slide, Insert image, Present), the layouts the open file has (the shared ribbon's third tab) and,
//! under its fourth, the words that transitions are not carried.
#![allow(dead_code)]

// The shared ribbon's tab row says Home, Insert, Layout, Review; the deck's Design tab is its third (`Tab::Layout`)
// and Transitions its fourth (`Tab::Review`). Their words on the tab row are `viewers/office.rs`'s, not this file's.
//
// Which layout a slide has is not something the helper reports: the ribbon marks the one it last applied to the
// slide on the stage (and Title slide on a deck's first slide until then). The layouts' names come from the helper
// (`ViewerSource::layouts`, answered `EditAnswer::Layouts`); until they are in, Design offers to ask for them.

use std::cell::Cell;

use beebeeb_office_protocol::office::{EditOp, Mark, Marks, TextAlign};
use rune::{El, Key, KeyEvent, Tx, row};

use super::slide;
use crate::State;
use crate::frames::viewers::office::{self as vo, Face, Tab, Tool, ico};
use crate::frames::viewers::office_over::Insert;
use crate::frames::viewers::{self, Intent, Ui};
use crate::theme::tokens::Theme;
use crate::ui::kit;
use crate::ui::menu::{self, Entry};

use super::word_ribbon::Pop;

const MENU: &str = "rib-menu";
/// Under the ribbon's tool row, in the ribbon's own coordinates.
const MENU_Y: f32 = 68.0;
const FONTS: [&str; 6] = ["Inter", "Arial", "Calibri", "Georgia", "Times New Roman", "Courier New"];
/// What a new deck's layouts are called, in the order its helper lists them.
pub const NEW_DECK_LAYOUTS: [&str; 5] = [
    "Title slide",
    "Title, content",
    "Title, two content",
    "Title only",
    "Blank",
];
pub const NO_TRANSITIONS: &str = "Transitions are not kept in a deck edited here. Nothing to change.";

thread_local! {
    /// The layout last applied, and the slide (1-based) it was applied to.
    static APPLIED: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
}

fn on(lit: bool) -> Tool {
    if lit { Tool::On } else { Tool::Idle }
}

/// Hand `op` to the editor. A source that does not take it says "Not built yet", never nothing.
fn send(s: &mut State, tx: &mut Tx, op: EditOp, asked: &'static str) {
    tx.instant();
    if let Some(ui) = viewers::ui(s) {
        ui.office.ribbon.open = None;
        ui.office.ribbon.hot = None;
    }
    if !slide::send(s, op) {
        if let Some(ui) = viewers::ui(s) {
            ui.intent(Intent::Office(asked));
        }
    }
}

fn ops(op: EditOp, asked: &'static str) -> impl Fn(&mut State, &mut Tx) + 'static {
    move |s: &mut State, tx: &mut Tx| send(s, tx, op.clone(), asked)
}

fn live(ui: &Ui) -> bool {
    ui.office.edit.as_ref().is_some_and(|e| !e.stopped)
}

// ---------------------------------------------------------------------------
// Home
// ---------------------------------------------------------------------------

/// A tool with its word: 28 high, the glyph, the word.
fn worded(
    key: &str,
    word: &'static str,
    icon: &'static rune::Icon,
    state: Tool,
    act: impl Fn(&mut State, &mut Tx) + 'static,
    t: &Theme,
) -> El {
    let ink = if state == Tool::Off { t.text_quaternary } else { t.text };
    let style = rune::TextStyle::new(12.0, 400.0).lh(15.0);
    let mut b = row()
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
        .child(kit::glyph(format!("{key}-icon"), icon, 15.0, ink))
        .child(
            kit::label(format!("{key}-label"), word, style, ink)
                .w(rune::text::advance(word, style))
                .a11y_hidden(),
        );
    if state == Tool::Off {
        b = b.disabled(true);
    } else {
        b = b.focusable(true).on_press(move |s: &mut State, tx: &mut Tx| act(s, tx));
    }
    b
}

fn open_font(s: &mut State, tx: &mut Tx) {
    tx.instant();
    if let Some(ui) = viewers::ui(s) {
        let was = ui.office.ribbon.open;
        ui.office.ribbon.open = (was != Some(Pop::Font)).then_some(Pop::Font);
        ui.office.ribbon.hot = None;
    }
}

fn pick_image(s: &mut State, tx: &mut Tx) {
    tx.instant();
    if let Some(ui) = viewers::ui(s) {
        ui.office.ribbon.open = None;
        ui.office_insert(Insert::Image);
    }
}

fn present(s: &mut State, tx: &mut Tx) {
    tx.instant();
    if let Some(deck) = viewers::deck_mut(s) {
        deck.present();
    }
}

fn home_tools(ui: &Ui, t: &Theme) -> Vec<El> {
    let edit = ui.office.edit.as_ref();
    let marks = edit.and_then(|e| e.marks);
    let (can_undo, can_redo) = edit.map_or((false, false), |e| (e.can_undo, e.can_redo));
    let lit = |f: fn(&Marks) -> bool| marks.as_ref().is_some_and(f);
    let glyph = |icon: &'static rune::Icon| Face::Glyph(icon, 15.0);
    let (current, count) = match &ui.file.body {
        viewers::source::Body::Deck(d) => (d.view.current, d.view.count),
        _ => (1, 1),
    };
    let mut tools = Vec::new();
    let mut n = 0;
    let mut seps = 0;
    let mut tool = |tools: &mut Vec<El>, name: &'static str, face: Face, state: Tool, act: Box<dyn Fn(&mut State, &mut Tx)>| {
        tools.push(vo::tool_with(&format!("rt-{n}"), name, face, state, act, t));
        n += 1;
    };
    let mut sep = |tools: &mut Vec<El>| {
        tools.push(vo::hairline(format!("rsep-{seps}"), t));
        seps += 1;
    };
    tool(
        &mut tools,
        "Undo",
        glyph(&ico::UNDO),
        if can_undo { Tool::Idle } else { Tool::Off },
        Box::new(ops(EditOp::Undo, "Undo")),
    );
    tool(
        &mut tools,
        "Redo",
        glyph(&ico::REDO),
        if can_redo { Tool::Idle } else { Tool::Off },
        Box::new(ops(EditOp::Redo, "Redo")),
    );
    sep(&mut tools);
    tools.push(vo::pop_with("rp-0", "Font", "Font", 92.0, open_font, t));
    sep(&mut tools);
    for (name, letter, style, mark, is_on) in [
        ("Bold", "B", vo::LETTER_HEAVY, Mark::Bold, lit(|m| m.bold)),
        ("Italic", "I", vo::LETTER, Mark::Italic, lit(|m| m.italic)),
        ("Underline", "U", vo::LETTER, Mark::Underline, lit(|m| m.underline)),
    ] {
        tool(
            &mut tools,
            name,
            Face::Letter(letter, style),
            on(is_on),
            Box::new(ops(EditOp::Mark { mark, on: !is_on }, name)),
        );
    }
    let centred = marks.map(|m| m.align) == Some(TextAlign::Center);
    tool(
        &mut tools,
        "Align centre",
        glyph(&ico::ALIGN_CENTRE),
        on(centred),
        Box::new(ops(EditOp::Align(TextAlign::Center), "Align centre")),
    );
    sep(&mut tools);
    let is_live = live(ui);
    let off = |ok: bool| if ok { Tool::Idle } else { Tool::Off };
    tools.push(worded("rw-0", "New slide", &ico::MORE, off(is_live), |s, tx| {
        tx.instant();
        slide::new_slide(s);
    }, t));
    tools.push(worded("rw-1", "Duplicate slide", &ico::TABLE, off(is_live), move |s, tx| {
        tx.instant();
        if let Some(l) = slide::live(s) {
            slide::duplicate_slide(s, l.current);
        }
    }, t));
    // A deck keeps one slide: Delete is off on the last.
    tools.push(worded("rw-2", "Delete slide", &ico::LESS, off(is_live && count > 1), move |s, tx| {
        tx.instant();
        if let Some(l) = slide::live(s) {
            slide::delete_slide(s, l.current);
        }
    }, t));
    let _ = current;
    sep(&mut tools);
    tool(&mut tools, "Insert image", glyph(&ico::IMAGE), off(is_live), Box::new(pick_image));
    tools.push(worded("rw-3", "Present", &ico::CARET, Tool::Idle, present, t));
    tools
}

// ---------------------------------------------------------------------------
// Design: the file's layouts
// ---------------------------------------------------------------------------

/// The layout the slide on the stage is taken to have.
fn current_layout(ui: &Ui) -> Option<u32> {
    let slide = match &ui.file.body {
        viewers::source::Body::Deck(d) => d.view.current,
        _ => return None,
    };
    if let Some((on, layout)) = APPLIED.with(Cell::get)
        && on == slide
    {
        return Some(layout);
    }
    // A new deck's first slide is its Title slide.
    if slide == 1 {
        let names = &ui.office.edit.as_ref()?.layouts;
        return names.iter().position(|n| n == NEW_DECK_LAYOUTS[0]).map(|i| i as u32);
    }
    None
}

fn apply_layout(layout: u32) -> impl Fn(&mut State, &mut Tx) + 'static {
    move |s: &mut State, tx: &mut Tx| {
        let Some(l) = slide::live(s) else { return };
        send(
            s,
            tx,
            EditOp::SetLayout {
                slide: l.current - 1,
                layout,
            },
            "Layouts",
        );
        APPLIED.with(|a| a.set(Some((l.current, layout))));
    }
}

fn ask_layouts(s: &mut State, tx: &mut Tx) {
    tx.instant();
    if let Some(ui) = viewers::ui(s) {
        let (source, id) = ui.edit_source();
        let id = id.to_string();
        if !source.layouts(&id) {
            ui.intent(Intent::Office("Layouts"));
        }
    }
}

fn design_tools(ui: &Ui, t: &Theme) -> Vec<El> {
    let layouts: &[String] = ui.office.edit.as_ref().map_or(&[], |e| e.layouts.as_slice());
    if layouts.is_empty() {
        return vec![worded("rw-ask", "Show layouts", &ico::TABLE, Tool::Idle, ask_layouts, t)];
    }
    let now = current_layout(ui);
    layouts
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let key = format!("rl-{i}");
            let is_on = now == Some(i as u32);
            let style = rune::TextStyle::new(12.0, if is_on { 600.0 } else { 400.0 }).lh(15.0);
            let ink = if is_on { t.text } else { t.text_secondary };
            let act = apply_layout(i as u32);
            let mut b = row()
                .key(key.clone())
                .role("button")
                .label(name.clone())
                .checked(is_on)
                .h(28.0)
                .px(8.0)
                .center_y()
                .radius(7.0)
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .on_press(move |s: &mut State, tx: &mut Tx| act(s, tx))
                .child(
                    kit::label(format!("{key}-label"), name.clone(), style, ink)
                        .w(rune::text::advance(name, style))
                        .a11y_hidden(),
                );
            if is_on {
                b = b.fill(t.press);
            }
            b
        })
        .collect()
}

fn transitions_tools(t: &Theme) -> Vec<El> {
    vec![kit::label(
        "rt-none",
        NO_TRANSITIONS,
        rune::TextStyle::new(12.0, 400.0).lh(15.0),
        t.text_tertiary,
    )]
}

/// DW-1560: the deck in the live editor with Design open and the layouts of a new deck, Title slide marked.
pub fn scene_design(s: &mut State) {
    super::super::slides::scene_deck(s);
    if let Some(ui) = viewers::ui(s) {
        let mut edit = super::EditUi::new(beebeeb_office_protocol::office::EditKind::Pptx);
        edit.layouts = NEW_DECK_LAYOUTS.iter().map(|n| (*n).to_string()).collect();
        ui.office.edit = Some(edit);
        ui.office.tab = Tab::Layout;
    }
    if let Some(deck) = viewers::deck_mut(s) {
        deck.go(1);
    }
}

// ---------------------------------------------------------------------------
// The ribbon
// ---------------------------------------------------------------------------

/// The ribbon of a live deck: the tabs, the open tab's tools and, over everything, the Font pop-up.
pub fn ribbon(ui: &Ui, unsaved: bool, t: &Theme) -> El {
    let tools = match ui.office.tab {
        Tab::Home => home_tools(ui, t),
        Tab::Insert => vec![vo::tool_with(
            "rt-0",
            "Insert image",
            Face::Glyph(&ico::IMAGE, 15.0),
            if live(ui) { Tool::Idle } else { Tool::Off },
            pick_image,
            t,
        )],
        Tab::Layout => design_tools(ui, t),
        Tab::Review => transitions_tools(t),
    };
    let mut rib = vo::ribbon_of(ui, unsaved, tools, t);
    if ui.office.ribbon.open == Some(Pop::Font) && ui.office.tab == Tab::Home {
        rib = rib.child(
            menu::menu(
                MENU,
                "Font",
                180.0,
                &entries(),
                ui.office.ribbon.hot,
                t,
                pick,
                hover,
                close,
            )
            .on_key(menu_key)
            .absolute(font_x(), MENU_Y)
            .raise(),
        );
    }
    rib
}

/// Where the Font button starts along the Home row: the row's padding, Undo, Redo, a separator.
fn font_x() -> f32 {
    8.0 + 2.0 * 30.0 + kit::HAIR + 6.0
}

fn entries() -> Vec<Entry> {
    FONTS.iter().map(|f| Entry::item(f, f)).collect()
}

fn pick(s: &mut State, index: usize, tx: &mut Tx) {
    if let Some(font) = FONTS.get(index) {
        send(s, tx, EditOp::FontFamily((*font).to_string()), "Fonts");
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
        ui.office.ribbon.open = None;
        ui.office.ribbon.hot = None;
    }
}

fn menu_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let all = entries();
    let hot = s.viewer.ui.as_ref().and_then(|ui| ui.office.ribbon.hot);
    let to = match ev.key {
        Key::Down => menu::step(&all, hot, 1),
        Key::Up => menu::step(&all, hot, -1),
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

#[cfg(test)]
#[path = "slide_ribbon_tests.rs"]
mod tests;
