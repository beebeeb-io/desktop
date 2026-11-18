//! The deck's filmstrip as a scrolling window (P3-8): its offset, the wheel over it, and which slides it draws and
//! asks. The offset lives in the deck, so a key that moves the current slide ([`DeckBody::go`]) keeps that slide's
//! line in view without a viewport to command.

use rune::{El, Key, KeyEvent, Tx};

use super::{DeckBody, STRIP_GAP, STRIP_PITCH, THUMB_H};
use crate::State;
use crate::frames::office::edit::slide as edit_slide;
use crate::frames::viewers;
use crate::theme::tokens::Theme;
use crate::ui::menu::{self, Entry};

impl DeckBody {
    /// The filmstrip's content height: every slide's row and the gaps between them.
    pub(super) fn strip_content(&self) -> u32 {
        (self.view.count * STRIP_PITCH).saturating_sub(STRIP_GAP)
    }

    /// Where the strip can scroll to at most.
    pub(super) fn strip_max(&self) -> u32 {
        self.strip_content().saturating_sub(self.strip_h)
    }

    /// The strip scrolls by `dy` points (toward you: down), kept inside the deck.
    pub fn strip_scroll_by(&mut self, dy: i32) {
        let y = i64::from(self.strip_y) + i64::from(dy);
        self.strip_y = y.clamp(0, i64::from(self.strip_max())) as u32;
    }

    /// The wheel over the filmstrip: it scrolls the strip and leaves the slide. A sideways delta is not the strip's.
    pub fn strip_wheel(&mut self, ev: &rune::WheelEvent) -> bool {
        if ev.dy.abs() < ev.dx.abs() {
            return false;
        }
        self.strip_scroll_by(-(ev.dy.round() as i32));
        true
    }

    /// The strip's height was laid out as `h` points. `true`: it changed.
    pub fn set_strip_h(&mut self, h: f32) -> bool {
        let h = if h.is_finite() { h.max(0.0).round() as u32 } else { 0 };
        if h == self.strip_h {
            return false;
        }
        self.strip_h = h;
        self.reveal_current();
        true
    }

    /// Scroll the strip the least that shows the current slide's whole line.
    pub(super) fn reveal_current(&mut self) {
        let top = self.view.current.saturating_sub(1).saturating_mul(STRIP_PITCH);
        let bottom = top + THUMB_H as u32;
        if top < self.strip_y {
            self.strip_y = top;
        } else if bottom > self.strip_y + self.strip_h {
            self.strip_y = bottom.saturating_sub(self.strip_h);
        }
        self.strip_scroll_by(0);
    }

    /// The slides the strip draws and asks: those in view and one row each side. 0-based indexes, first to last.
    pub fn strip_rows(&self) -> std::ops::RangeInclusive<usize> {
        let last_slide = self.view.slides.len().saturating_sub(1);
        let first = (self.strip_y.saturating_sub(STRIP_PITCH) / STRIP_PITCH) as usize;
        let last = ((self.strip_y + self.strip_h + STRIP_PITCH) / STRIP_PITCH) as usize;
        first.min(last_slide)..=last.min(last_slide)
    }
}

// ---------------------------------------------------------------------------
// The filmstrip's right-click menu (the editor's)
// ---------------------------------------------------------------------------

/// The open slide menu: the slide it is for (1-based), where its corner is (window points, whole) and the line the
/// keyboard is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StripMenu {
    pub slide: u32,
    pub x: i32,
    pub y: i32,
    pub hot: Option<usize>,
}

pub const MENU_W: f32 = 190.0;
pub const DUPLICATE: &str = "Duplicate slide";
pub const DELETE: &str = "Delete slide";
pub const MOVE_UP: &str = "Move slide up";
pub const MOVE_DOWN: &str = "Move slide down";

/// The lines of the menu for slide `slide` of `count`: Delete is dimmed on the only slide, Move up on the first and
/// Move down on the last.
pub fn entries(slide: u32, count: u32) -> Vec<Entry> {
    vec![
        Entry::item("duplicate", DUPLICATE),
        Entry::item("delete", DELETE).dimmed(count <= 1),
        Entry::Sep,
        Entry::item("up", MOVE_UP).dimmed(slide <= 1),
        Entry::item("down", MOVE_DOWN).dimmed(slide >= count),
    ]
}

/// The strip's key for the slide row it is on: Ctrl+M a new slide, Delete or Backspace the slide.
pub fn row_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if edit_slide::live(s).is_none_or(|l| l.presenting) {
        return false;
    }
    let (m, current) = (ev.mods, edit_slide::live(s).map_or(1, |l| l.current));
    tx.instant();
    if ev.is_command('m') {
        edit_slide::new_slide(s);
        return true;
    }
    if (m.cmd || m.ctrl || m.alt) && !matches!(ev.key, Key::Delete) {
        return false;
    }
    match ev.key {
        Key::Delete | Key::Backspace if !m.shift => {
            edit_slide::delete_slide(s, current);
            true
        }
        _ => false,
    }
}

/// A right-click on slide `n`: the stage shows it and the menu opens at the pointer, kept inside the window.
pub fn open_menu(s: &mut State, n: u32, at: (f32, f32), tx: &mut Tx) {
    tx.instant();
    if edit_slide::live(s).is_none() {
        return;
    }
    edit_slide::change_slide(s, n);
    let Some(count) = edit_slide::live(s).map(|l| l.count) else {
        return;
    };
    let height = menu::height(&entries(n, count));
    let (win_w, win_h) = s.actions.window;
    let x = at.0.min(win_w - MENU_W - 8.0).max(8.0);
    let y = at.1.min(win_h - height - 8.0).max(8.0);
    if let Some(deck) = viewers::deck_mut(s) {
        deck.menu = Some(StripMenu {
            slide: n,
            x: x.round() as i32,
            y: y.round() as i32,
            hot: None,
        });
    }
}

fn close(s: &mut State, tx: &mut Tx) {
    if let Some(deck) = viewers::deck_mut(s) {
        deck.menu = None;
    }
    tx.instant();
}

fn hover(s: &mut State, i: usize) {
    if let Some(open) = viewers::deck_mut(s).and_then(|d| d.menu.as_mut()) {
        open.hot = Some(i);
    }
}

fn pick(s: &mut State, i: usize, tx: &mut Tx) {
    let Some(open) = viewers::deck_mut(s).and_then(|d| d.menu) else {
        return;
    };
    let Some(count) = viewers::deck_mut(s).map(|d| d.view.count) else {
        return;
    };
    let list = entries(open.slide, count);
    let Some(entry) = list.get(i).copied() else { return };
    // A dimmed line is in the menu and does nothing; the menu stays.
    let Some(id) = entry.id().filter(|_| !entry.is_off()) else {
        return;
    };
    close(s, tx);
    match id {
        "duplicate" => edit_slide::duplicate_slide(s, open.slide),
        "delete" => edit_slide::delete_slide(s, open.slide),
        "up" => edit_slide::move_slide(s, open.slide, -1),
        "down" => edit_slide::move_slide(s, open.slide, 1),
        _ => {}
    }
}

/// The menu's own keys: it is modal, so what is outside it hears none.
fn menu_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(open) = viewers::deck_mut(s).and_then(|d| d.menu) else {
        return false;
    };
    let Some(count) = viewers::deck_mut(s).map(|d| d.view.count) else {
        return false;
    };
    let list = entries(open.slide, count);
    let hot = match ev.key {
        Key::Down => menu::step(&list, open.hot, 1),
        Key::Up => menu::step(&list, open.hot, -1),
        Key::Home => menu::step(&list, None, 1),
        Key::End => menu::step(&list, None, -1),
        Key::Enter => {
            if let Some(hot) = open.hot {
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
    if let (Some(hot), Some(open)) = (hot, viewers::deck_mut(s).and_then(|d| d.menu.as_mut())) {
        open.hot = Some(hot);
        if let Some(key) = menu::item_key("slide-menu", &list, hot) {
            tx.focus(key, false);
        }
    }
    true
}

/// The open menu as a layer of the window, at its point. `None`: not open.
pub fn menu_layer(deck: &DeckBody, t: &Theme) -> Option<El> {
    let open = deck.menu?;
    let list = entries(open.slide, deck.view.count);
    Some(
        menu::menu("slide-menu", "Slide", MENU_W, &list, open.hot, t, pick, hover, close)
            .on_key(menu_key)
            .absolute(open.x as f32, open.y as f32),
    )
}
