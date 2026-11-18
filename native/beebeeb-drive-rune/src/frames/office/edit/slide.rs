//! The slide surface's keys and pointer (a later task fills this).
#![allow(dead_code)]

// A press on the stage selects the shape under it (`SelectShape`), or, while a shape's text is being edited, places the
// caret (`hit`); typed text, Backspace, Delete, Enter and the arrows go to the text as `EditOp`s; Escape clears the
// shape. The filmstrip's slide operations live here too (the strip and its menu call them), and so does the drawing of
// the caret, the selection and the selected shape's outline.
//
// The deck's count is kept in step from here (`DeckBody::set_count`) because an `Edited` is drained into `EditUi`, which
// keeps neither its `slides` nor its `dirty`. Pictures an edit made stale are the source's to drop: the stage and the
// strip keep asking through `live.asks`.
//
// Slide points: the helper speaks in slide points from the slide's top-left (0-based slide numbers); the stage draws
// the slide at `DeckView::page_pt` points on `stage_size` layout points, so one is the other times a fixed factor.

use beebeeb_office_protocol::office::{EditOp, HitMode, Mark, Motion, PageRect};
use rune::{Color, Drag, El, Key, KeyEvent, Phase, Tx, el};

use super::EditUi;
use super::clipboard;
use crate::State;
use crate::frames::office::slides::DeckBody;
use crate::frames::viewers::{self, source::Body};

/// The key of the focusable slide on the stage (the editor's keys start with `edit-`).
pub const FOCUS_KEY: &str = "edit-slide";

/// The outline, the handles and the selection's tint: a blue that is neither the amber of the primary action nor the
/// ink of the slide's text.
fn blue() -> Color {
    rune::color::hex("#2f6fed").unwrap_or(Color::BLACK)
}

fn ink() -> Color {
    rune::color::hex("#090909").unwrap_or(Color::BLACK)
}

// ---------------------------------------------------------------------------
// What the surface needs to know of the open deck
// ---------------------------------------------------------------------------

/// The open file as a deck being edited, as the handlers see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Live {
    /// The slide on the stage, 1-based, and how many there are.
    pub current: u32,
    pub count: u32,
    /// The slide's size in points.
    pub page_pt: (u32, u32),
    pub zoom: u16,
    /// A shape's text is being edited (the helper answered a caret).
    pub caret: bool,
    /// Something is selected: a shape, or text.
    pub selected: bool,
    pub presenting: bool,
}

/// `Some` only for a deck open in the live editor (a viewed deck has no `EditUi`).
pub fn live(s: &State) -> Option<Live> {
    let ui = s.viewer.ui.as_ref()?;
    let edit = ui.office.edit.as_ref()?;
    let Body::Deck(deck) = &ui.file.body else { return None };
    Some(Live {
        current: deck.view.current,
        count: deck.view.count,
        page_pt: deck.view.page_pt,
        zoom: deck.view.zoom,
        caret: edit.caret.is_some(),
        selected: edit.caret.is_some() || !edit.selection.is_empty(),
        presenting: deck.presenting,
    })
}

/// Hand one edit to the open file's editor. `false`: there is none or it did not take it.
pub fn send(s: &mut State, op: EditOp) -> bool {
    let Some(ui) = viewers::ui(s) else { return false };
    let (source, id) = ui.edit_source();
    let id = id.to_string();
    source.edit(&id, op)
}

fn send_hit(s: &mut State, page: u32, x_pt: f32, y_pt: f32, mode: HitMode) -> bool {
    let Some(ui) = viewers::ui(s) else { return false };
    let (source, id) = ui.edit_source();
    let id = id.to_string();
    source.hit(&id, page, x_pt, y_pt, mode)
}

// ---------------------------------------------------------------------------
// Slide operations (keys, menu and strip share them)
// ---------------------------------------------------------------------------

/// Show slide `n` (1-based). A shape selected on the slide left is let go, so a key never types into a slide that is
/// not on the stage.
pub fn change_slide(s: &mut State, n: u32) {
    let editing = live(s).is_some();
    let Some(deck) = viewers::deck_mut(s) else { return };
    let before = deck.view.current;
    deck.go(n);
    let moved = deck.view.current != before;
    if editing && moved {
        send(s, EditOp::ClearShape);
    }
}

/// Ctrl+M: a new slide after the one on the stage, which then shows it. No title is promised.
pub fn new_slide(s: &mut State) {
    let Some(l) = live(s) else { return };
    if !send(s, EditOp::NewSlide { layout: None }) {
        return;
    }
    if let Some(deck) = viewers::deck_mut(s) {
        deck.set_count(l.count + 1);
        deck.go(l.current + 1);
    }
}

/// A copy of slide `n` (1-based) follows it, and shows.
pub fn duplicate_slide(s: &mut State, n: u32) {
    let Some(l) = live(s) else { return };
    if n == 0 || n > l.count || !send(s, EditOp::DuplicateSlide(n - 1)) {
        return;
    }
    if let Some(deck) = viewers::deck_mut(s) {
        deck.set_count(l.count + 1);
        deck.go(n + 1);
    }
}

/// Slide `n` goes. The last slide stays (a deck keeps one).
pub fn delete_slide(s: &mut State, n: u32) {
    let Some(l) = live(s) else { return };
    if n == 0 || n > l.count || l.count <= 1 || !send(s, EditOp::DeleteSlide(n - 1)) {
        return;
    }
    let show = if l.current > n {
        l.current - 1
    } else {
        l.current.min(l.count - 1)
    };
    if let Some(deck) = viewers::deck_mut(s) {
        deck.set_count(l.count - 1);
        deck.go(show);
    }
}

/// Slide `n` moves one place: up (`-1`) or down (`1`). At an end nothing moves.
pub fn move_slide(s: &mut State, n: u32, by: i32) {
    let Some(l) = live(s) else { return };
    let to = i64::from(n) + i64::from(by);
    if n == 0 || n > l.count || to < 1 || to > i64::from(l.count) || by == 0 {
        return;
    }
    let to = to as u32;
    if !send(
        s,
        EditOp::MoveSlide {
            from: n - 1,
            to: to - 1,
        },
    ) {
        return;
    }
    // The stage follows its slide: the moved one, or the one it swapped with.
    let show = if l.current == n {
        to
    } else if l.current == to {
        n
    } else {
        l.current
    };
    if let Some(deck) = viewers::deck_mut(s) {
        deck.go(show);
    }
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// The text motion a navigation key stands for, when it moves the caret.
fn motion(key: &Key, word: bool) -> Option<Motion> {
    Some(match key {
        Key::Left if word => Motion::WordLeft,
        Key::Right if word => Motion::WordRight,
        Key::Left => Motion::Left,
        Key::Right => Motion::Right,
        Key::Up => Motion::Up,
        Key::Down => Motion::Down,
        Key::Home if word => Motion::DocStart,
        Key::End if word => Motion::DocEnd,
        Key::Home => Motion::LineStart,
        Key::End => Motion::LineEnd,
        Key::PageUp => Motion::PageUp,
        Key::PageDown => Motion::PageDown,
        _ => return None,
    })
}

/// A key for the surface. `false`: not taken (Present's keys, Ctrl+Enter and F5, are never taken here).
pub fn on_key(s: &mut State, tx: &mut Tx, ev: &KeyEvent) -> bool {
    if clipboard::on_clip_key(s, tx, ev) {
        return true;
    }
    let Some(l) = live(s) else { return false };
    if l.presenting {
        return false;
    }
    tx.instant();
    let m = ev.mods;
    if (m.cmd || m.ctrl) && !m.alt {
        return command_key(s, ev, l);
    }
    match &ev.key {
        Key::Text(t) if l.caret && (!m.cmd || m.alt) => {
            if t.is_empty() || t.chars().any(char::is_control) {
                return false;
            }
            send(s, EditOp::InsertText(t.clone()));
            true
        }
        Key::Enter if l.caret && !m.alt => {
            send(
                s,
                if m.shift {
                    EditOp::InsertLineBreak
                } else {
                    EditOp::InsertParagraph
                },
            );
            true
        }
        Key::Backspace if l.caret => {
            send(
                s,
                EditOp::Delete {
                    back: true,
                    word: false,
                },
            );
            true
        }
        Key::Delete if l.caret => {
            send(
                s,
                EditOp::Delete {
                    back: false,
                    word: false,
                },
            );
            true
        }
        Key::Escape if l.selected => {
            send(s, EditOp::ClearShape);
            true
        }
        key @ (Key::Left | Key::Right | Key::Up | Key::Down | Key::Home | Key::End | Key::PageUp | Key::PageDown)
            if !m.alt =>
        {
            if l.caret {
                if let Some(to) = motion(key, false) {
                    send(s, EditOp::Move { to, extend: m.shift });
                }
                return true;
            }
            // No text in hand: the arrows step through the slides, as in the viewer.
            if m.shift {
                return false;
            }
            match key {
                Key::Left | Key::Up | Key::PageUp => change_slide(s, l.current.saturating_sub(1).max(1)),
                Key::Right | Key::Down | Key::PageDown => change_slide(s, l.current.saturating_add(1)),
                _ => return false,
            }
            true
        }
        _ => false,
    }
}

/// Ctrl chords: new slide, undo and redo, select all, the marks, the word-wise motions and deletions.
fn command_key(s: &mut State, ev: &KeyEvent, l: Live) -> bool {
    let shift = ev.mods.shift;
    if ev.is_command('m') {
        new_slide(s);
        return true;
    }
    if ev.is_command('z') {
        send(s, if shift { EditOp::Redo } else { EditOp::Undo });
        return true;
    }
    if ev.is_command('y') {
        send(s, EditOp::Redo);
        return true;
    }
    if !l.caret {
        return false;
    }
    if matches!(ev.key, Key::Left | Key::Right | Key::Home | Key::End) {
        if let Some(to) = motion(&ev.key, true) {
            send(s, EditOp::Move { to, extend: shift });
            return true;
        }
    }
    match &ev.key {
        Key::Backspace => {
            send(s, EditOp::Delete { back: true, word: true });
            return true;
        }
        Key::Delete => {
            send(
                s,
                EditOp::Delete {
                    back: false,
                    word: true,
                },
            );
            return true;
        }
        _ => {}
    }
    if ev.is_command('a') {
        send(s, EditOp::SelectAll);
        return true;
    }
    let mark = [('b', Mark::Bold), ('i', Mark::Italic), ('u', Mark::Underline)]
        .into_iter()
        .find(|(c, _)| ev.is_command(*c))
        .map(|(_, mark)| mark);
    let Some(mark) = mark else { return false };
    let was_on = viewers::ui(s)
        .and_then(|ui| ui.office.edit.as_ref().and_then(|e| e.marks))
        .is_some_and(|m| match mark {
            Mark::Bold => m.bold,
            Mark::Italic => m.italic,
            Mark::Underline => m.underline,
            Mark::Strike => m.strike,
        });
    send(s, EditOp::Mark { mark, on: !was_on });
    true
}

// ---------------------------------------------------------------------------
// The pointer
// ---------------------------------------------------------------------------

/// A point of the slide element as slide points, kept inside the slide.
fn to_slide_pt(d: &Drag, page_pt: (u32, u32)) -> (f32, f32) {
    let (w, h) = (d.w.max(1.0), d.h.max(1.0));
    let x = (d.x / w * page_pt.0 as f32).clamp(0.0, page_pt.0 as f32);
    let y = (d.y / h * page_pt.1 as f32).clamp(0.0, page_pt.1 as f32);
    (x, y)
}

/// The press on the slide: a shape's text being edited takes the click as a click in the text (a second press picks the
/// word, a third the paragraph, Shift extends); elsewhere it selects what is under it. A drag that began in the text
/// extends the selection.
pub fn on_drag(s: &mut State, d: &mut Drag, tx: &mut Tx) {
    tx.instant();
    let Some(l) = live(s) else { return };
    if l.presenting {
        return;
    }
    let (x, y) = to_slide_pt(d, l.page_pt);
    let page = l.current - 1;
    match d.phase {
        Phase::Down => {
            tx.focus(FOCUS_KEY, false);
            if let Some(deck) = viewers::deck_mut(s) {
                deck.last_hit = None;
            }
            if l.caret {
                let mode = match d.clicks {
                    0 | 1 if tx.mods().shift => HitMode::Extend,
                    0 | 1 => HitMode::Place,
                    2 => HitMode::Word,
                    _ => HitMode::Paragraph,
                };
                d.data = 1.0;
                send_hit(s, page, x, y, mode);
            } else {
                d.data = 0.0;
                send(
                    s,
                    EditOp::SelectShape {
                        slide: page,
                        x_pt: x,
                        y_pt: y,
                    },
                );
            }
        }
        Phase::Move if d.data >= 1.0 => {
            // A whole point at a time: the helper is not asked twice for the same spot.
            let at = (x.round() as i32, y.round() as i32);
            let Some(deck) = viewers::deck_mut(s) else { return };
            if deck.last_hit == Some(at) {
                return;
            }
            deck.last_hit = Some(at);
            send_hit(s, page, x, y, HitMode::Extend);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// What is drawn on the slide
// ---------------------------------------------------------------------------

/// Layout points per slide point, across and down, on a stage of `stage` points.
pub fn k_of(stage: (f32, f32), page_pt: (u32, u32)) -> (f32, f32) {
    (stage.0 / page_pt.0.max(1) as f32, stage.1 / page_pt.1.max(1) as f32)
}

fn rect_el(key: String, r: &PageRect, k: (f32, f32)) -> El {
    el().key(key)
        .absolute(r.rect.x * k.0, r.rect.y * k.1)
        .size((r.rect.w * k.0).max(0.0), (r.rect.h * k.1).max(0.0))
}

const HANDLE: f32 = 7.0;
const CARET_W: f32 = 1.5;
/// The eight handles' places on a box, as fractions of its width and height.
const HANDLES: [(f32, f32); 8] = [
    (0.0, 0.0),
    (0.5, 0.0),
    (1.0, 0.0),
    (1.0, 0.5),
    (1.0, 1.0),
    (0.5, 1.0),
    (0.0, 1.0),
    (0.0, 0.5),
];

/// The caret, the selection and the selected shape on the slide on the stage, as children of the slide's element
/// (absolutely placed). Rectangles of another slide are not drawn. While a shape's text is being edited the helper says
/// the caret and the selected text; a shape selected without its text in hand comes as its box (no caret).
pub fn overlay(deck: &DeckBody, edit: &EditUi, stage: (f32, f32)) -> Vec<El> {
    let k = k_of(stage, deck.view.page_pt);
    let page = deck.view.current.saturating_sub(1);
    let mut out = Vec::new();
    let on_slide = |r: &&PageRect| r.page == page;
    if edit.caret.is_none() {
        for (i, r) in edit.selection.iter().filter(on_slide).enumerate() {
            out.push(
                rect_el(format!("edit-shape-{i}"), r, k)
                    .border(1.5, blue())
                    .border_layout()
                    .label("Selected shape"),
            );
            for (h, (fx, fy)) in HANDLES.iter().enumerate() {
                let (x, y) = ((r.rect.x + fx * r.rect.w) * k.0, (r.rect.y + fy * r.rect.h) * k.1);
                out.push(
                    el().key(format!("edit-shape-{i}-h{h}"))
                        .absolute(x - HANDLE / 2.0, y - HANDLE / 2.0)
                        .size(HANDLE, HANDLE)
                        .fill(Color::WHITE)
                        .border(1.5, blue())
                        .border_layout(),
                );
            }
        }
        return out;
    }
    for (i, r) in edit.selection.iter().filter(on_slide).enumerate() {
        out.push(rect_el(format!("edit-sel-{i}"), r, k).fill(blue().alpha(0.28)));
    }
    if let Some(c) = edit.caret.as_ref().filter(on_slide) {
        out.push(
            rect_el("edit-caret".into(), c, k)
                .size(CARET_W, (c.rect.h * k.1).max(6.0))
                .fill(ink())
                .label("Caret"),
        );
    }
    out
}

#[cfg(test)]
#[path = "slide_tests.rs"]
mod tests;
