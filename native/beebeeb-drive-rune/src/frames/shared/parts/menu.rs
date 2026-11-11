//! A context menu (`.wn-menu`): the board's own menu, drawn in the window at the pointer. It
//! takes the keyboard while it is open (a modal layer: Escape or a press outside closes it).

use crate::State;
use crate::theme::icons;
use crate::theme::tokens::Theme;
use crate::ui::kit;
use rune::{Color, El, Key, KeyEvent, TextStyle, col, el, row};

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

/// An open menu: where its top-left corner is (window points) and which item is lit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Open {
    pub at: (f32, f32),
    pub hot: usize,
}

impl Open {
    pub fn at(x: f32, y: f32) -> Self {
        Open { at: (x, y), hot: 0 }
    }

    /// The arrows move the lit item and stop at the ends; Home and End jump. `true`: the key was the menu's.
    pub fn on_key(&mut self, ev: &KeyEvent, count: usize) -> bool {
        if count == 0 {
            return false;
        }
        match ev.key {
            Key::Down => self.hot = (self.hot + 1).min(count - 1),
            Key::Up => self.hot = self.hot.saturating_sub(1),
            Key::Home => self.hot = 0,
            Key::End => self.hot = count - 1,
            _ => return false,
        }
        true
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

/// The menu. Keys: `menu`, each item `mi-<its words>` with `-label` and `-sc` or `-sc-icon`,
/// each hairline `msep-<n>`. `pick` gets the item's index; `dismiss` closes the menu.
pub fn menu(
    name: &str,
    entries: &[Entry],
    open: &Open,
    min_w: f32,
    t: &Theme,
    pick: fn(&mut State, usize),
    dismiss: fn(&mut State),
) -> El {
    let mut m = col()
        .key("menu")
        .role("menu")
        .label(name)
        .absolute(open.at.0, open.at.1)
        .min_w(min_w)
        .padding(4.0)
        .radius(8.0)
        .fill(t.card)
        .box_shadow(kit::drop(8.0, 28.0, Color::BLACK.alpha(0.2)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .modal(true, dismiss);
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
        let k = format!("mi-{}", slug(e.words));
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
            .fill(if hot { t.press } else { Color::TRANSPARENT })
            .hover(t.press)
            .focusable(true)
            .focus_selected(hot)
            .on_press(move |s: &mut State| pick(s, i))
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
