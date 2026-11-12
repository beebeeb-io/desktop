//! A menu (`.wn-menu`): the file menu, the Sort menu, the New menu, the list's background menu.
//!
//! Drawn in the window (the board's own surface, not the system's `TrackPopupMenuEx`, which
//! cannot take the board's look). It is modal: Tab stays inside, Escape and a click outside
//! close it, and focus goes back to what opened it (`El::modal`).
//!
//! Keys, for a menu keyed `menu`: `menu`, per entry that is an item or a heading `menu-item-<i>`
//! with `-label`, `-shortcut`, `-check`; per separator `menu-sep-<j>`.

use crate::State;
use crate::theme::icons_files as glyphs;
use crate::theme::tokens::{Theme, radius};
use rune::{Color, El, TextStyle, Tx, col, el, row};

use super::kit;

/// `.wn-mi .lb`
pub const ITEM: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
/// `.wn-mi.is-head .lb`
pub const HEAD: TextStyle = TextStyle::new(11.0, 600.0).lh(17.0);
/// `.wn-mi .sc` (line-height normal: 14.5 for Inter at 12)
pub const SHORTCUT: TextStyle = TextStyle::new(12.0, 400.0).lh(14.5);

const ITEM_H: f32 = 30.0;
const HEAD_H: f32 = 26.0;
const PAD: f32 = 4.0;

/// One line of a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Item {
        /// What the owner gets back when it is picked.
        id: &'static str,
        label: &'static str,
        shortcut: Option<&'static str>,
        checked: bool,
        /// Destructive (Move to Trash): the label is red.
        danger: bool,
        /// Dimmed (`.wn-mi.is-off`): it is in the menu and cannot be picked now.
        off: bool,
    },
    /// A heading over a group ("Sort by").
    Head(&'static str),
    Sep,
}

impl Entry {
    pub const fn item(id: &'static str, label: &'static str) -> Entry {
        Entry::Item {
            id,
            label,
            shortcut: None,
            checked: false,
            danger: false,
            off: false,
        }
    }

    pub const fn keyed(id: &'static str, label: &'static str, shortcut: &'static str) -> Entry {
        Entry::Item {
            id,
            label,
            shortcut: Some(shortcut),
            checked: false,
            danger: false,
            off: false,
        }
    }

    pub const fn check(id: &'static str, label: &'static str, checked: bool) -> Entry {
        Entry::Item {
            id,
            label,
            shortcut: None,
            checked,
            danger: false,
            off: false,
        }
    }

    pub const fn danger(id: &'static str, label: &'static str, shortcut: &'static str) -> Entry {
        Entry::Item {
            id,
            label,
            shortcut: Some(shortcut),
            checked: false,
            danger: true,
            off: false,
        }
    }

    /// This entry, dimmed when `off`: drawn, read as unavailable, skipped by the arrows, not picked.
    pub const fn dimmed(self, off: bool) -> Entry {
        match self {
            Entry::Item {
                id,
                label,
                shortcut,
                checked,
                danger,
                ..
            } => Entry::Item {
                id,
                label,
                shortcut,
                checked,
                danger,
                off,
            },
            other => other,
        }
    }

    pub fn is_off(&self) -> bool {
        matches!(self, Entry::Item { off: true, .. })
    }

    pub fn id(&self) -> Option<&'static str> {
        match self {
            Entry::Item { id, .. } => Some(id),
            _ => None,
        }
    }
}

/// How tall a menu of these entries is, so its owner can keep it inside the window.
pub fn height(entries: &[Entry]) -> f32 {
    2.0 * PAD
        + entries
            .iter()
            .map(|e| match e {
                Entry::Item { .. } => ITEM_H,
                Entry::Head(_) => HEAD_H,
                Entry::Sep => 2.0 * PAD + kit::HAIR,
            })
            .sum::<f32>()
}

/// The key of the entry `index` of the menu keyed `key` (`<key>-item-<n>`: separators are not
/// counted), so its owner can put keyboard focus on the highlighted item.
pub fn item_key(key: &str, entries: &[Entry], index: usize) -> Option<String> {
    if !matches!(entries.get(index), Some(Entry::Item { .. })) {
        return None;
    }
    let n = entries[..index].iter().filter(|e| !matches!(e, Entry::Sep)).count();
    Some(format!("{key}-item-{n}"))
}

/// The next item from `hot` in direction `delta` (headings, separators and dimmed items are
/// skipped; the ends wrap).
pub fn step(entries: &[Entry], hot: Option<usize>, delta: isize) -> Option<usize> {
    let items: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, Entry::Item { off: false, .. }))
        .map(|(i, _)| i)
        .collect();
    if items.is_empty() {
        return None;
    }
    let at = hot.and_then(|h| items.iter().position(|&i| i == h));
    let next = match at {
        Some(p) => (p as isize + delta).rem_euclid(items.len() as isize) as usize,
        None if delta < 0 => items.len() - 1,
        None => 0,
    };
    Some(items[next])
}

/// The surface a menu, a tooltip-like popup or a drag label sits on: paper, a hairline ring, a drop shadow.
pub fn raised(t: &Theme) -> Color {
    // The board's `--color-card`: paper-2 in dark, the lightest paper in light.
    t.card
}

/// The menu. `hot` is the highlighted entry (pointer or arrows); `pick` runs with the entry's
/// index when it is chosen, `hover` when the pointer comes over it, `close` on Escape or a
/// click outside.
#[allow(clippy::too_many_arguments)]
pub fn menu(
    key: &'static str,
    name: &'static str,
    width: f32,
    entries: &[Entry],
    hot: Option<usize>,
    t: &Theme,
    pick: fn(&mut State, usize, &mut Tx),
    hover: fn(&mut State, usize),
    close: fn(&mut State, &mut Tx),
) -> El {
    let mut surface = col()
        .key(key)
        .role("menu")
        .label(name)
        .w(width)
        .padding(PAD)
        .radius(8.0)
        .fill(raised(t))
        .box_shadow(kit::drop(8.0, 28.0, Color::BLACK.alpha(0.20)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .modal_tx(true, close);
    let (mut n_item, mut n_sep) = (0, 0);
    for (i, entry) in entries.iter().enumerate() {
        match *entry {
            Entry::Sep => {
                surface = surface.child(
                    el().key(format!("{key}-sep-{n_sep}"))
                        .role("separator")
                        .w_full()
                        .h(kit::HAIR)
                        .my(PAD)
                        .fill(t.separator),
                );
                n_sep += 1;
            }
            Entry::Head(label) => {
                let k = format!("{key}-item-{n_item}");
                surface = surface.child(
                    row()
                        .key(k.clone())
                        .role("presentation")
                        .h(HEAD_H)
                        .w_full()
                        .pl(8.0)
                        .pr(12.0)
                        .center_y()
                        .child(kit::label(format!("{k}-label"), label, HEAD, t.text_tertiary).grow(1.0)),
                );
                n_item += 1;
            }
            Entry::Item {
                label,
                shortcut,
                checked,
                danger,
                off,
                ..
            } => {
                let k = format!("{key}-item-{n_item}");
                let is_hot = hot == Some(i) && !off;
                let mut tick = el().key(format!("{k}-tick")).w(14.0).center();
                if checked {
                    tick = tick.child(kit::glyph(format!("{k}-check"), &glyphs::CHECK, 12.0, t.text));
                }
                let mut item = row()
                    .key(k.clone())
                    .role(if checked { "menuitemcheckbox" } else { "menuitem" })
                    .label(label)
                    .h(ITEM_H)
                    .w_full()
                    .pl(8.0)
                    .pr(12.0)
                    .gap(10.0)
                    .center_y()
                    .radius(radius::SM)
                    .fill(if is_hot { t.press } else { Color::TRANSPARENT })
                    .child(tick)
                    .child(
                        kit::label(
                            format!("{k}-label"),
                            label,
                            ITEM,
                            if off {
                                t.text_quaternary
                            } else if danger {
                                danger_ink(t)
                            } else {
                                t.text
                            },
                        )
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                    );
                // A dimmed item is read as unavailable; it takes no focus, no pointer and no pick.
                item = if off {
                    item.disabled(true)
                } else {
                    item.focusable(true)
                        .focus_selected(is_hot || (hot.is_none() && n_item == 0))
                        .no_focus_ring()
                        .on_hover(move |s: &mut State| hover(s, i))
                        .on_press_tx(move |s: &mut State, tx: &mut Tx| pick(s, i, tx))
                };
                if checked {
                    item = item.checked(true);
                }
                if let Some(keys) = shortcut {
                    item = item.shortcut_hint(keys).child(
                        kit::label(
                            format!("{k}-shortcut"),
                            keys,
                            SHORTCUT,
                            if off {
                                t.text_quaternary
                            } else if is_hot {
                                t.text_secondary
                            } else {
                                t.text_tertiary
                            },
                        )
                        .ml(24.0),
                    );
                }
                surface = surface.child(item);
                n_item += 1;
            }
        }
    }
    surface
}

/// `.wn-mi.is-danger .lb`: the board's `--red-deep`.
pub fn danger_ink(t: &Theme) -> Color {
    if t.dark {
        rune::oklch(0.70, 0.19, 25.0)
    } else {
        rune::oklch(0.48, 0.19, 25.0)
    }
}
