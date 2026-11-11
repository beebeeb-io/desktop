//! A context menu (`.wn-menu`): the board's own menu, drawn in the window at the pointer or
//! under the item it is about, always inside the window. It takes the keyboard while it is
//! open (a modal layer: Escape or a press outside closes it).

use crate::State;
use crate::theme::icons;
use crate::theme::tokens::Theme;
use crate::ui::kit;
use rune::{Align, Color, El, Key, KeyEvent, Side, TextStyle, Tx, col, el, row};

/// `.wn-mi .lb`.
const WORDS: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
/// `.wn-mi .sc`.
const KEYS: TextStyle = TextStyle::new(12.0, 400.0).lh(14.5);

/// What an item shows at its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trail {
    None,
    /// The keys that do the same ("Alt+Enter").
    Keys(&'static str),
    /// It opens a further menu.
    More,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub words: &'static str,
    pub trail: Trail,
    /// Words in red: it removes or ends something.
    pub danger: bool,
    /// A hairline above it.
    pub rule_before: bool,
}

impl Entry {
    pub const fn new(words: &'static str) -> Self {
        Entry {
            words,
            trail: Trail::None,
            danger: false,
            rule_before: false,
        }
    }
    pub const fn keys(mut self, keys: &'static str) -> Self {
        self.trail = Trail::Keys(keys);
        self
    }
    pub const fn more(mut self) -> Self {
        self.trail = Trail::More;
        self
    }
    pub const fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub const fn rule(mut self) -> Self {
        self.rule_before = true;
        self
    }
}

/// Where an open menu is.
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    /// Its top-left corner at the pointer (window points), moved to the other side of the
    /// pointer where it would leave the window.
    At(f32, f32),
    /// Under the element with this key (the keyboard's menu, the More button): Rune flips it
    /// above when it does not fit below, and keeps it inside the window.
    Under(String),
}

/// An open menu: where it is and which item is lit. The lit item is the ONE item the menu
/// acts on: the arrows, Tab and the pointer all move it, and Enter picks it.
#[derive(Clone, Debug, PartialEq)]
pub struct Open {
    pub place: Place,
    pub hot: usize,
}

/// What a key did while a menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Took {
    /// The lit item moved, or the key is not one a menu has: nothing else gets it.
    Key,
    /// Enter or Space on the lit item.
    Pick(usize),
    /// Escape.
    Close,
}

impl Open {
    pub fn at(x: f32, y: f32) -> Self {
        Open {
            place: Place::At(x, y),
            hot: 0,
        }
    }

    pub fn under(key: impl Into<String>) -> Self {
        Open {
            place: Place::Under(key.into()),
            hot: 0,
        }
    }

    pub fn lit(mut self, hot: usize) -> Self {
        self.hot = hot;
        self
    }

    /// A menu is modal: every key is its own while it is open. The arrows move the lit item
    /// and stop at the ends, Home and End jump, Tab and Shift+Tab move it too and go round;
    /// focus goes with it, so the ring and the lit item are never two items.
    pub fn on_key(&mut self, ev: &KeyEvent, entries: &[Entry], tx: &mut Tx) -> Took {
        let count = entries.len();
        if count == 0 {
            return Took::Close;
        }
        match &ev.key {
            Key::Down => self.hot = (self.hot + 1).min(count - 1),
            Key::Up => self.hot = self.hot.saturating_sub(1),
            Key::Home => self.hot = 0,
            Key::End => self.hot = count - 1,
            Key::Tab if ev.mods.shift => self.hot = (self.hot + count - 1) % count,
            Key::Tab => self.hot = (self.hot + 1) % count,
            Key::Enter => return Took::Pick(self.hot),
            Key::Text(t) if t == " " => return Took::Pick(self.hot),
            Key::Escape => return Took::Close,
            _ => return Took::Key,
        }
        tx.focus(item_key(&entries[self.hot]), true);
        Took::Key
    }
}

fn slug(words: &str) -> String {
    let mut out = String::new();
    for c in words.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// The key of an item: `mi-<its words>`.
pub fn item_key(e: &Entry) -> String {
    format!("mi-{}", slug(e.words))
}

/// The menu's own size in points: its items, hairlines and padding, at least `min_w` wide.
pub fn size(entries: &[Entry], min_w: f32) -> (f32, f32) {
    let rules = entries.iter().filter(|e| e.rule_before).count() as f32;
    let h = 8.0 + 30.0 * entries.len() as f32 + rules * (8.0 + kit::HAIR);
    let widest = entries
        .iter()
        .map(|e| {
            let trail = match e.trail {
                Trail::None => 0.0,
                Trail::Keys(keys) => 10.0 + 24.0 + rune::text::advance(keys, KEYS),
                Trail::More => 10.0 + 24.0 + 11.0,
            };
            8.0 + 14.0 + 10.0 + rune::text::advance(e.words, WORDS) + trail + 12.0
        })
        .fold(0.0, f32::max);
    ((widest + 8.0).max(min_w), h)
}

/// Where a menu of `size` opened at the pointer goes in a window of `window` points: at the
/// pointer, or on its other side where it would cross the right or the bottom edge (as a
/// Windows menu does), and never outside the window.
pub fn fit(at: (f32, f32), size: (f32, f32), window: (f32, f32)) -> (f32, f32) {
    let one = |at: f32, size: f32, room: f32| {
        let at = if at + size > room { at - size } else { at };
        at.min(room - size).max(0.0)
    };
    (one(at.0, size.0, window.0), one(at.1, size.1, window.1))
}

/// The menu. Keys: `menu`, each item `mi-<its words>` with `-label` and `-sc` or `-sc-icon`,
/// each hairline `msep-<n>`. `pick` gets the item's index; `dismiss` closes the menu; `light`
/// lights the item the pointer is on; `keys` is the section's key handler (the window behind a
/// modal is inert, so the menu hands its keys over itself: `Open::on_key` through the section).
/// `window` is the window's size, when it is known.
#[allow(clippy::too_many_arguments)]
pub fn menu(
    name: &str,
    entries: &[Entry],
    open: &Open,
    min_w: f32,
    window: Option<[f32; 2]>,
    t: &Theme,
    pick: fn(&mut State, usize, &mut Tx),
    dismiss: fn(&mut State, &mut Tx),
    light: fn(&mut State, usize),
    keys: fn(&mut State, &KeyEvent, &mut Tx) -> bool,
) -> El {
    let m = col().key("menu").role("menu").label(name);
    let m = match &open.place {
        Place::At(x, y) => {
            let (x, y) = match window {
                Some([w, h]) => fit((*x, *y), size(entries, min_w), (w, h)),
                None => (*x, *y),
            };
            m.absolute(x, y)
        }
        Place::Under(key) => m.anchored(key.clone(), Side::Bottom, Align::Start, 2.0),
    };
    let mut m = m
        .min_w(min_w)
        .padding(4.0)
        .radius(8.0)
        .fill(t.card)
        .box_shadow(kit::drop(8.0, 28.0, Color::BLACK.alpha(0.2)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .on_key(keys)
        .modal_tx(true, dismiss);
    let mut rules = 0;
    for (i, e) in entries.iter().enumerate() {
        if e.rule_before {
            m = m.child(
                el().key(format!("msep-{rules}"))
                    .h(kit::HAIR)
                    .w_full()
                    .my(4.0)
                    .fill(t.separator),
            );
            rules += 1;
        }
        let k = item_key(e);
        let ink = if e.danger { super::red_text(t) } else { t.text };
        let hot = open.hot == i;
        let mut item = row()
            .key(k.clone())
            .role("menuitem")
            .label(e.words)
            .h(30.0)
            .w_full()
            .pl(8.0)
            .pr(12.0)
            .gap(10.0)
            .center_y()
            .radius(4.0)
            // One wash: the lit item's. The pointer lights the item it enters (`light`).
            .fill(if hot { t.press } else { Color::TRANSPARENT })
            .focusable(true)
            .focus_selected(hot)
            .tab_index(if hot { 0 } else { -1 })
            .on_hover(move |s: &mut State| light(s, i))
            .on_press_tx(move |s: &mut State, tx: &mut Tx| pick(s, i, tx))
            // `.ck`: where a tick goes; none of these menus has one.
            .child(el().key(format!("{k}-ck")).w(14.0))
            .child(
                kit::label(format!("{k}-label"), e.words, WORDS, ink)
                    .grow(1.0)
                    .min_w(0.0),
            );
        match e.trail {
            Trail::None => {}
            Trail::Keys(keys) => {
                item = item
                    .shortcut_hint(keys)
                    .child(kit::label(format!("{k}-sc"), keys, KEYS, t.text_tertiary).pl(24.0));
            }
            Trail::More => {
                item = item.child(el().key(format!("{k}-sc-wrap")).pl(24.0).center().child(kit::glyph(
                    format!("{k}-sc-icon"),
                    &icons::CHEVRON_RIGHT,
                    11.0,
                    t.text_tertiary,
                )));
            }
        }
        m = m.child(item);
    }
    m
}
