//! The word surface's keys and pointer (a later task fills this).
#![allow(dead_code)]

use std::cell::Cell;

use beebeeb_office_protocol::check::MAX_SELECTION_RECTS;
use beebeeb_office_protocol::office::{EditKind, EditOp, HitMode, Mark, Marks, Motion};
use rune::{Color, Cursor, Cycle, Drag, El, Key, KeyEvent, Phase, Transition, Tx, WheelEvent, col, el};

use super::{EditUi, clipboard};
use crate::State;
use crate::frames::office::docs::{DocBody, scroll};
use crate::frames::viewers::{self, source::ViewerSource};

/// The key of the focusable page area (the editor's surfaces start with `edit-`: `EditUi::has_keyboard`).
pub const SURFACE: &str = "edit-surface";
/// The caret's key; a piece of the selection is `sel-{n}`.
pub const CARET: &str = "caret";
/// The room kept between the caret and the stage's edge when it is scrolled into view, in points.
const REVEAL_MARGIN: f32 = 24.0;
/// The caret is at least this wide, in points.
const CARET_MIN_W: f32 = 1.5;
/// One blink: on for half of it, off for the other half (the window wakes ten times a second, no more, and not at
/// all while the caret is hidden or the window is).
const BLINK: Cycle = Cycle {
    period: 1.06,
    phase: 0.0,
    stops: &[(0.0, 1.0), (0.499, 1.0), (0.5, 0.0), (0.999, 0.0), (1.0, 1.0)],
    still: 1.0,
    fps: 10.0,
};
/// A caret while the editor does not have the keyboard.
const CARET_IDLE: f32 = 0.35;
/// The paper is white in both themes: the caret and the selection are ink on it.
const SELECTION_ALPHA: f32 = 0.2;
/// Points of Ctrl + wheel for one zoom step.
const ZOOM_WHEEL: f32 = 30.0;

thread_local! {
    /// Where the last hit of this drag was sent (page, x, y in points): a move under a point of it asks nothing.
    static LAST_HIT: Cell<Option<(u32, f32, f32)>> = const { Cell::new(None) };
    /// The next page area drawn before its editor's first answer takes the keyboard (once per opening).
    static OPEN_FOCUS: Cell<bool> = const { Cell::new(true) };
    /// Ctrl + wheel not yet worth a zoom step.
    static WHEEL: Cell<f32> = const { Cell::new(0.0) };
}

/// A document was opened for editing: its page area takes the keyboard when it first appears.
pub fn arm_open_focus() {
    OPEN_FOCUS.set(true);
}

/// Move the keyboard to the page area (a ribbon tool gives it back with this after it acted).
pub fn focus(tx: &mut Tx) {
    tx.focus(SURFACE, false);
}

/// Whether a Word document is open with its editor running: only then the surface takes anything.
fn editing(s: &mut State) -> bool {
    viewers::ui(s).is_some_and(|ui| ui.office.edit.as_ref().is_some_and(|e| !e.stopped && e.kind == EditKind::Docx))
}

/// Hand `f` the source and the open file's id.
fn with_source<R>(s: &mut State, f: impl FnOnce(&mut dyn ViewerSource, &str) -> R) -> Option<R> {
    let ui = viewers::ui(s)?;
    let (source, id) = ui.edit_source();
    Some(f(source, id))
}

/// Ask the stage to show the caret once the editor has moved it: the caret's place now is where it must leave.
fn arm_reveal(s: &mut State) {
    let at = viewers::ui(s)
        .and_then(|ui| ui.office.edit.as_ref())
        .and_then(|e| e.caret)
        .map_or((u32::MAX, 0.0, 0.0), |c| (c.page, c.rect.x, c.rect.y));
    if let Some(doc) = viewers::doc_mut(s) {
        doc.reveal = Some(at);
    }
}

/// One edit to the editor. `true`: it took it.
fn send(s: &mut State, op: EditOp) -> bool {
    let taken = with_source(s, |source, id| source.edit(id, op)).unwrap_or(false);
    if taken {
        arm_reveal(s);
    }
    taken
}

/// What a press does to the selection: Shift extends, a double click takes the word, a triple click the paragraph.
pub fn hit_mode(clicks: u32, shift: bool) -> HitMode {
    match clicks {
        0 | 1 if shift => HitMode::Extend,
        0 | 1 => HitMode::Place,
        2 => HitMode::Word,
        _ => HitMode::Paragraph,
    }
}

/// The page, and the point of it, under (`x`, `y`) of the surface (layout points from its top-left).
fn page_point(s: &mut State, x: f32, y: f32) -> Option<(u32, f32, f32)> {
    viewers::doc_mut(s).map(|doc| doc.page_point(x, y))
}

/// Ask the editor what is under `at`.
fn hit(s: &mut State, at: (u32, f32, f32), mode: HitMode) {
    LAST_HIT.with(|last| last.set(Some(at)));
    let (page, x_pt, y_pt) = at;
    if with_source(s, |source, id| source.hit(id, page, x_pt, y_pt, mode)).unwrap_or(false) {
        arm_reveal(s);
    }
}

/// Whether `at` is another point than the last hit's (a point of the page apart, or another page).
fn moved(at: (u32, f32, f32)) -> bool {
    LAST_HIT.with(|last| {
        last.get()
            .is_none_or(|(p, x, y)| p != at.0 || (x - at.1).abs() >= 1.0 || (y - at.2).abs() >= 1.0)
    })
}

/// The pointer on the page area: a press places the caret (Shift extends, a double click takes a word, a triple click
/// a paragraph) and a drag extends the selection, asked once for each point of the page the pointer reaches.
fn on_drag(s: &mut State, d: &mut Drag, tx: &mut Tx) {
    tx.instant();
    if !editing(s) {
        return;
    }
    let Some(at) = page_point(s, d.x, d.y) else { return };
    match d.phase {
        Phase::Down => hit(s, at, hit_mode(d.clicks, tx.mods().shift)),
        Phase::Move => {
            if moved(at) {
                hit(s, at, HitMode::Extend);
            }
        }
        Phase::Up => {
            if !d.cancelled() && moved(at) {
                hit(s, at, HitMode::Extend);
            }
            LAST_HIT.with(|last| last.set(None));
        }
    }
}

/// Ctrl + wheel zooms; a plain wheel scrolls (not taken here) and drops a caret reveal that is still waited for, so a
/// scroll away from the caret is never taken back.
fn on_wheel(s: &mut State, e: WheelEvent, tx: &mut Tx) -> bool {
    if !editing(s) {
        return false;
    }
    if !(e.mods.cmd || e.mods.ctrl) {
        if let Some(doc) = viewers::doc_mut(s) {
            doc.reveal = None;
        }
        return false;
    }
    tx.instant();
    if e.momentum {
        return true;
    }
    let total = WHEEL.with(|w| {
        w.set(w.get() + e.dy);
        w.get()
    });
    if total.abs() >= ZOOM_WHEEL {
        WHEEL.with(|w| w.set(0.0));
        if let Some(doc) = viewers::doc_mut(s) {
            if total > 0.0 {
                doc.zoom_in();
            } else {
                doc.zoom_out();
            }
            scroll::show_page(doc, tx);
        }
    }
    true
}

/// The motion a navigation key makes; `ctrl`: the word or document variant.
fn motion(key: &Key, ctrl: bool) -> Option<Motion> {
    Some(match (key, ctrl) {
        (Key::Left, false) => Motion::Left,
        (Key::Right, false) => Motion::Right,
        (Key::Up, false) => Motion::Up,
        (Key::Down, false) => Motion::Down,
        (Key::Left, true) => Motion::WordLeft,
        (Key::Right, true) => Motion::WordRight,
        (Key::Home, false) => Motion::LineStart,
        (Key::End, false) => Motion::LineEnd,
        (Key::Home, true) => Motion::DocStart,
        (Key::End, true) => Motion::DocEnd,
        (Key::PageUp, false) => Motion::PageUp,
        (Key::PageDown, false) => Motion::PageDown,
        _ => return None,
    })
}

/// The edit a key stands for. `marks`: what the caret is in now, so Ctrl+B turns bold off where it is on.
/// `None`: not the surface's key.
pub fn op_for(ev: &KeyEvent, marks: Option<&Marks>) -> Option<EditOp> {
    let m = ev.mods;
    let ctrl = m.cmd || m.ctrl;
    match &ev.key {
        // Text typed with Ctrl held is a shortcut, not text (Ctrl + Alt is AltGr, which types).
        Key::Text(_) if ctrl && (cfg!(target_os = "macos") || !m.alt) => None,
        Key::Text(t) if t.is_empty() || t.chars().any(char::is_control) => None,
        Key::Text(t) => Some(EditOp::InsertText(t.clone())),
        _ if m.alt => None,
        Key::Enter if m.shift && !ctrl => Some(EditOp::InsertLineBreak),
        Key::Enter if !ctrl => Some(EditOp::InsertParagraph),
        Key::Backspace => Some(EditOp::Delete { back: true, word: ctrl }),
        Key::Delete if !m.shift => Some(EditOp::Delete { back: false, word: ctrl }),
        Key::Left | Key::Right | Key::Home | Key::End => {
            motion(&ev.key, ctrl).map(|to| EditOp::Move { to, extend: m.shift })
        }
        // Page Up and Down by a screen with Ctrl, and Up and Down by a paragraph, are not the editor's.
        Key::Up | Key::Down | Key::PageUp | Key::PageDown if !ctrl => {
            motion(&ev.key, false).map(|to| EditOp::Move { to, extend: m.shift })
        }
        Key::Command(_) if ctrl => command(ev, marks),
        _ => None,
    }
}

/// A Ctrl chord: select all, undo, redo, bold, italic, underline.
fn command(ev: &KeyEvent, marks: Option<&Marks>) -> Option<EditOp> {
    let shift = ev.mods.shift;
    let toggle = |mark: Mark, now: Option<bool>| (!shift).then(|| EditOp::Mark { mark, on: !now.unwrap_or(false) });
    if ev.is_command('a') && !shift {
        Some(EditOp::SelectAll)
    } else if ev.is_command('z') {
        Some(if shift { EditOp::Redo } else { EditOp::Undo })
    } else if ev.is_command('y') && !shift {
        Some(EditOp::Redo)
    } else if ev.is_command('b') {
        toggle(Mark::Bold, marks.map(|m| m.bold))
    } else if ev.is_command('i') {
        toggle(Mark::Italic, marks.map(|m| m.italic))
    } else if ev.is_command('u') {
        toggle(Mark::Underline, marks.map(|m| m.underline))
    } else {
        None
    }
}

/// A key for the surface. `false`: not taken.
pub fn on_key(s: &mut State, tx: &mut Tx, ev: &KeyEvent) -> bool {
    tx.instant();
    if !editing(s) {
        return false;
    }
    if clipboard::on_clip_key(s, tx, ev) {
        return true;
    }
    let marks = viewers::ui(s).and_then(|ui| ui.office.edit.as_ref()).and_then(|e| e.marks);
    let Some(op) = op_for(ev, marks.as_ref()) else {
        return false;
    };
    if !send(s, op) {
        return false;
    }
    // A page of the caret's moves is a page of the stage's.
    match ev.key {
        Key::PageDown => tx.scroll_page(scroll::STAGE, 1),
        Key::PageUp => tx.scroll_page(scroll::STAGE, -1),
        _ => {}
    }
    true
}

/// The pages' column as the editor's surface: focusable, with the caret and the selection drawn over the pages from
/// what the editor last answered (its points converted to the column's, never read back from a layout). `stage`: the
/// stage's rectangle in the window as the last frame laid it out. `ime`: the input method's caret element, put at
/// the caret while the editor has the keyboard.
pub fn surface(pages: El, doc: &DocBody, edit: &EditUi, stage: Option<[f32; 4]>, ime: Option<El>) -> El {
    let mut surface = col()
        .key(SURFACE)
        .role("textbox")
        .label("Document text")
        .focusable(true)
        .no_focus_ring()
        .cursor(Cursor::Text)
        // Typing and moving are direct manipulation: the caret and the pictures land at once, never glide.
        .instant_changes()
        .on_key(|s: &mut State, ev: &KeyEvent, tx: &mut Tx| on_key(s, tx, ev))
        .on_drag(on_drag)
        .on_wheel(on_wheel)
        .child(pages);
    // The keyboard is on the page as the document opens: typing starts at once. Only until the editor's first
    // answer, so a pop-up or a ribbon tool that took the keyboard later keeps it.
    if edit.revision == 0 && edit.caret.is_none() && !edit.has_keyboard && OPEN_FOCUS.replace(false) {
        surface = surface.autofocus();
    }
    for (i, rect) in edit.selection.iter().take(MAX_SELECTION_RECTS).enumerate() {
        let [x, y, w, h] = doc.page_rect(rect);
        surface = surface.child(
            el().key(format!("sel-{i}"))
                .a11y_hidden()
                .transition(Transition::none())
                .absolute(x, y)
                .size(w, h)
                .fill(Color::BLACK.alpha(SELECTION_ALPHA)),
        );
    }
    // A range selection has no caret.
    let Some(caret) = edit.caret.filter(|_| edit.selection.is_empty()) else {
        return surface;
    };
    let [x, y, w, h] = doc.page_rect(&caret);
    let size = (w.max(CARET_MIN_W), h);
    let mut mark = el()
        .key(CARET)
        .a11y_hidden()
        .transition(Transition::none())
        .absolute(x, y)
        .size(size.0, size.1)
        .fill(Color::BLACK);
    mark = if edit.has_keyboard {
        mark.cycle(BLINK)
    } else {
        mark.opacity(CARET_IDLE)
    };
    if let (Some(from), Some(stage)) = (doc.reveal, stage) {
        let at = (caret.page, caret.rect.x, caret.rect.y);
        mark = mark.on_layout_tx(move |s: &mut State, r: [f32; 4], tx: &mut Tx| reveal(s, r, [x, y], stage, from, at, tx));
    }
    surface = surface.child(mark);
    if edit.has_keyboard
        && let Some(input) = ime
    {
        surface = surface.child(input.absolute(x, y).size(size.0, size.1));
    }
    surface
}

/// The caret was laid out at `r` (window points). If it moved since the editor was asked to show it (`from`: where
/// it stood then, `at`: where it stands now), scroll the stage the least that brings it into view and stop waiting.
/// `content`: its place in the pages' column; `stage`: the stage's rectangle.
fn reveal(
    s: &mut State,
    r: [f32; 4],
    content: [f32; 2],
    stage: [f32; 4],
    from: (u32, f32, f32),
    at: (u32, f32, f32),
    tx: &mut Tx,
) {
    if from == at {
        // Not moved yet: this is the stage scrolling under the caret, which changes nothing drawn.
        tx.live_only();
        return;
    }
    if let Some(doc) = viewers::doc_mut(s) {
        doc.reveal = None;
    }
    tx.instant();
    let [sx, sy, sw, sh] = stage;
    // Where the stage stands now: the column's top-left in the window is the caret's less its place in the column.
    let (offset_x, offset_y) = (sx + content[0] - r[0], sy + content[1] - r[1]);
    let to = |lo: f32, hi: f32, edge: f32, size: f32, offset: f32| {
        let (top, bottom) = (edge + REVEAL_MARGIN, edge + size - REVEAL_MARGIN);
        if lo < top {
            Some((offset - (top - lo)).max(0.0))
        } else if hi > bottom {
            Some(offset + (hi - bottom))
        } else {
            None
        }
    };
    if let Some(y) = to(r[1], r[1] + r[3], sy, sh, offset_y) {
        tx.scroll_to_y(scroll::STAGE, y);
    }
    if let Some(x) = to(r[0], r[0] + r[2], sx, sw, offset_x) {
        tx.scroll_to_x(scroll::STAGE, x);
    }
}

#[cfg(test)]
#[path = "word_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "budget_tests.rs"]
mod budget_tests;
