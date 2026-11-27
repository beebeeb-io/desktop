//! Account inside settings (`.au-set`): the scaffold and the rows of the account, security and
//! sessions panes (DW-A51 to DW-A84a).
//!
//! The board draws these panes with the shared account kit, not with Drive's own settings
//! classes: the sidebar is the shared one (`settings_shell::app_sidebar`), and the column is 520 pt. The lists are the kit's lists, so [`settings_list::list`] holds the rows; the
//! rows here add what an account row has and a settings row does not (an avatar or a toned tile
//! in front, a pill at the end, a destructive title).
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | Pane | `pane(children, t)` | `set-main`, `set-col` |
//! | Heading | `heading(title, sub, trailing, t)` | `set-title`, `set-sub` |
//! | Row | `item(list, index, &Row, extra, t)` | `item-<L>-<I>` + `-icon`, `-avatar`, `-initials`, `-tile`, `-tile-icon`, `-title`, `-sub`, `-value`, `-pill`, `-pill-dot`, `-pill-label`, `-check`, `-chev` |
//! | Pill | `pill(key, label, tone, dot, t)` | `<key>`, `<key>-dot`, `<key>-label` |
//! | Toned tile | `tile(key, icon, size, radius, tone, t)` | `<key>`, `<key>-icon` |
//! | Score ring | `ring(key, d, width, fraction, color, t)` | `<key>` |
//! | Button (22 and 28 pt) | `button(key, label, icon, Look, Size, t)` | `<key>`, `<key>-icon`, `<key>-label` |
//! | Dialog on its scrim | `dialog(name, width, title, words, foot, t)` | `scrim`, `dlg`, `dlg-body`, `dlg-title`, `dlg-text-0`, `dlg-foot` |
//!
//! Every measurement is the board's computed CSS, read from the goldens (`css` on each box).
//! Nothing here knows which panes exist or what a press does: a frame attaches `.on_press`.

use crate::theme::icons_settings as ic;
use crate::theme::tokens::{Theme, type_};
use rune::tok::Family;
use rune::{Color, El, Icon, LineCap, Shape, TextStyle, canvas, col, el, row};

use super::au::{self, Tone};
use super::button;
use super::dialog::{self, Dialog, Foot, Head, scrim};
use super::kit;
use super::settings_dialog::{DIALOG_TITLE};
use super::settings_list::{VALUE, VALUE_MONO};
use super::settings_shell::{PANE_TITLE};

/// A pane's column (`.au-set-col`).
pub const COLUMN_W: f32 = 520.0;

/// 12px/400 on a 15px line: a second line (`.t-sub`).
pub const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
/// 12px/400 mono on a 15px line: a machine's value inside a second line.
pub const SUB_MONO: TextStyle = TextStyle::new(12.0, 400.0).family(Family::Mono).lh(15.0);
/// 11.5px/400 mono on a 15px line: a row's second line when it is a machine's (`.kit-item-sub.font-mono`).
pub const SUB_MONO_ROW: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono).lh(15.0);
/// 12px/650: the initials on a 32 pt avatar.
pub const INITIALS_32: TextStyle = TextStyle::new(12.0, 650.0);
/// 13px/600 mono on a 17px line, tracked -0.006em: the count inside the score ring.
pub const RING_COUNT: TextStyle = TextStyle::new(13.0, 600.0)
    .family(Family::Mono)
    .lh(17.0)
    .tracking(-0.078);

/// `--kit-sel`: the neutral selection of the account kit's sidebar (ink at 6 %, 9 % in dark).
pub fn selection(t: &Theme) -> Color {
    t.text.alpha(if t.dark { 0.09 } else { 0.06 })
}

/// A pane (`.au-set-main` > `.au-set-col`): 22 pt above and 30 pt beside a 520 pt column whose
/// children sit 14 pt apart. A pane taller than the window scrolls.
pub fn pane(children: Vec<El>, t: &Theme) -> El {
    let column = super::settings_shell::flow(children, COLUMN_W);
    col()
        .key("set-main")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .h_full()
        .fill(t.surface)
        .py(22.0)
        .px(30.0)
        .scroll(rune::ScrollSpec { stick_bottom: false })
        .child(column)
}

/// A pane's heading (`.au-h`): the title, at most one control on its line, and a second line.
/// Keys: `set-title`, `set-sub`.
pub fn heading(title: &str, sub: Option<&str>, trailing: Option<El>, t: &Theme) -> El {
    let title_el = kit::label("set-title", title, PANE_TITLE, t.text)
        .role("heading")
        .level(1);
    let first = match trailing {
        Some(trailing) => row()
            .key("set-h-line")
            .w_full()
            .center_y()
            .justify_between()
            .gap(12.0)
            .child(title_el)
            .child(trailing),
        None => title_el.w_full(),
    };
    super::settings_shell::mark_lead();
    let mut h = col().key("set-h").w_full().items_start().gap(6.0).child(first);
    if let Some(sub) = sub {
        h = h.child(kit::label("set-sub", sub, SUB, t.text_secondary).wrap().w_full());
    }
    h
}

/// A pill (`.kit-pill`): a short state, with a dot before it when `dot`. The state is in the
/// words, never only in the colour. Keys: `<key>`, `<key>-dot`, `<key>-label`.
pub fn pill(key: &str, label: &str, tone: Tone, dot: bool, t: &Theme) -> El {
    super::pill::pill(key, label, au::pill_tone(tone), dot, t)
}

/// A toned tile (`.kit-tile`): a glyph half the tile's size on the tone's wash.
/// Keys: `<key>`, `<key>-icon`.
pub fn tile(key: &str, icon: &'static Icon, size: f32, corner: f32, tone: Tone, t: &Theme) -> El {
    let (mark, _, wash) = au::tone(tone, t);
    el().key(key.to_string())
        .a11y_hidden()
        .size(size, size)
        .radius(corner)
        .fill(wash)
        .center()
        .child(kit::glyph(format!("{key}-icon"), icon, size / 2.0, mark))
}

/// A progress ring `d` across and `width` wide (`.kit-ring`): the hairline track and
/// `fraction` of it in `color`, from twelve o'clock, clockwise, round ends. Hidden from
/// assistive technology: the count beside it is text. (`au::ring` is the 3 pt one.)
pub fn ring(key: &str, d: f32, width: f32, fraction: f32, color: Color, t: &Theme) -> El {
    let r = d / 2.0 - width / 2.0;
    let c = d / 2.0;
    let arc = |from: f32, to: f32| -> Vec<[f32; 2]> {
        let steps = ((to - from) * 96.0).ceil().max(1.0) as usize;
        (0..=steps)
            .map(|i| {
                let a = (from + (to - from) * i as f32 / steps as f32) * std::f32::consts::TAU
                    - std::f32::consts::FRAC_PI_2;
                [c + r * a.cos(), c + r * a.sin()]
            })
            .collect()
    };
    let mut shape = Shape::new()
        .view_box(d, d)
        .stroke_with(arc(0.0, 1.0), true, width, LineCap::Butt, t.separator);
    if fraction >= 1.0 {
        shape = shape.stroke_with(arc(0.0, 1.0), true, width, LineCap::Round, color);
    } else {
        // Part of the circle, with round ends: filled as its own outline. (Rune's stroke of an
        // OPEN line with round caps is anti-aliased differently from its closed strokes and
        // its fills, and from the board's: see the PR's Rune notes.)
        let fraction = fraction.max(0.0005);
        let half = width / 2.0;
        let at = |turn: f32, radius: f32| -> [f32; 2] {
            let a = turn * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            [c + radius * a.cos(), c + radius * a.sin()]
        };
        let steps = (fraction * 192.0).ceil().max(1.0) as usize;
        let mut outline: Vec<[f32; 2]> = Vec::with_capacity(2 * steps + 40);
        outline.extend((0..=steps).map(|i| at(fraction * i as f32 / steps as f32, r + half)));
        // The cap at the far end: half a turn around the end of the centre line, outer to inner.
        let cap = |turn: f32, outward_first: bool| -> Vec<[f32; 2]> {
            let centre = at(turn, r);
            let base = turn * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            (1..16)
                .map(|i| {
                    let sweep = std::f32::consts::PI * i as f32 / 16.0;
                    let a = if outward_first {
                        base + sweep
                    } else {
                        base + std::f32::consts::PI + sweep
                    };
                    [centre[0] + half * a.cos(), centre[1] + half * a.sin()]
                })
                .collect()
        };
        outline.extend(cap(fraction, true));
        outline.extend(
            (0..=steps)
                .rev()
                .map(|i| at(fraction * i as f32 / steps as f32, r - half)),
        );
        outline.extend(cap(0.0, false));
        shape = shape.fill(outline, color);
    }
    canvas(shape).key(key.to_string()).a11y_hidden().size(d, d)
}

/// What a row starts with.
#[derive(Clone, Debug)]
pub enum Lead {
    /// An 18 pt glyph in a 30 pt column (`.kit-item-icon`).
    Icon(&'static Icon),
    /// A person's initials, 32 pt across (`.kit-avatar`); `hue` is the tint's hue.
    Avatar { initials: String, hue: f32 },
    /// A 28 pt toned tile (`.kit-tile`).
    Tile(&'static Icon, Tone),
    /// Nothing: the title starts at the row's edge (DW-A71's "Added").
    None,
}

/// What sits at a row's trailing edge, in order.
#[derive(Clone, Debug)]
pub enum Tail {
    Value(String),
    Mono(String),
    Pill {
        label: String,
        tone: Tone,
        dot: bool,
    },
    /// Done: the plain checkmark in ink.
    Check,
    Chevron,
}

/// One row of an account list.
#[derive(Clone, Debug)]
pub struct Row {
    pub lead: Lead,
    pub title: String,
    pub sub: Option<String>,
    pub tails: Vec<Tail>,
    /// A destructive row: its title in red text, and announced as destructive.
    pub danger: bool,
    /// The row itself takes the press (it opens a pane or a dialog).
    pub link: bool,
    /// What assistive technology reads instead of the words put together from the row.
    pub name: Option<String>,
    /// The second line is a machine's (a time, a country, an address): in mono.
    pub sub_mono: bool,
    /// The row a menu or a confirm is about: the neutral tint (never Amber).
    pub selected: bool,
}

impl Row {
    pub fn new(lead: Lead, title: impl Into<String>) -> Self {
        Self {
            lead,
            title: title.into(),
            sub: None,
            tails: Vec::new(),
            danger: false,
            link: false,
            name: None,
            sub_mono: false,
            selected: false,
        }
    }
    pub fn sub_mono(mut self, sub: impl Into<String>) -> Self {
        self.sub = Some(sub.into());
        self.sub_mono = true;
        self
    }
    pub fn selected(mut self, on: bool) -> Self {
        self.selected = on;
        self
    }
    pub fn sub(mut self, sub: impl Into<String>) -> Self {
        self.sub = Some(sub.into());
        self
    }
    pub fn tail(mut self, tail: Tail) -> Self {
        self.tails.push(tail);
        self
    }
    /// The row takes the press; with `chevron`, it shows one at its end.
    pub fn link(mut self, chevron: bool) -> Self {
        self.link = true;
        if chevron {
            self.tails.push(Tail::Chevron);
        }
        self
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// The row in words: the title, what it says, then its state.
    pub fn says(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        let mut name = self.title.clone();
        if let Some(sub) = &self.sub {
            name = format!("{name}, {sub}");
        }
        for tail in &self.tails {
            match tail {
                Tail::Value(v) | Tail::Mono(v) => name = format!("{name}, {v}"),
                Tail::Pill { label, .. } => name = format!("{name}, {label}"),
                Tail::Check => name = format!("{name}, done"),
                Tail::Chevron => {}
            }
        }
        if self.danger {
            name = format!("{name}, destructive");
        }
        name
    }
}

/// One row (`.kit-item`): 28 pt, 40 pt with a second line, as tall as its avatar. `extra` are
/// the caller's controls at the trailing edge (a button, a pop-up button, a switch), after the
/// row's own. Keys: `item-<L>-<I>` and the pieces named in the module's table.
pub fn item(list: usize, index: usize, it: &Row, extra: Vec<El>, t: &Theme) -> El {
    let key = format!("item-{list}-{index}");
    let mut r = row()
        .key(key.clone())
        .w_full()
        .min_h(if it.sub.is_some() { 40.0 } else { 28.0 })
        .px(10.0)
        .gap(8.0)
        .center_y()
        .label(it.says());
    // A link or selectable row is itself the focusable item, so a `roving_list` can hold it;
    // a row with buttons of its own stays text around them.
    r = if it.link {
        r.role("link").hover(t.hover).press(t.press).focusable(true).focus_selected(it.selected)
    } else if it.selected && extra.is_empty() {
        r.role("listitem").focusable(true).focus_selected(true)
    } else {
        r.role("listitem")
    };
    // The board rules a row off from the one above it (`.kit-item + .kit-item::before`), from 10 pt in.
    if index > 0 {
        r = r.child(
            el().key(format!("{key}-rule"))
                .pin(Some(10.0), Some(0.0), Some(0.0), None)
                .h(kit::HAIR)
                .fill(t.separator),
        );
    }
    r = match &it.lead {
        Lead::Icon(icon) => r.child(el().key(format!("{key}-icon-box")).w(30.0).center().child(kit::glyph(
            format!("{key}-icon"),
            icon,
            18.0,
            t.text_secondary,
        ))),
        Lead::Avatar { initials, hue } => r.child(
            el().key(format!("{key}-avatar"))
                .a11y_hidden()
                .size(32.0, 32.0)
                .radius(16.0)
                .fill(t.avatar_tint(rune::oklch(0.88, 0.05, *hue)))
                .box_shadow(kit::ring(2.0, t.card))
                .center()
                .child(kit::label(
                    format!("{key}-initials"),
                    initials.clone(),
                    INITIALS_32,
                    t.avatar_ink(),
                )),
        ),
        Lead::Tile(icon, tone) => r.child(tile(&format!("{key}-tile"), icon, 28.0, 8.0, *tone, t)),
        Lead::None => r,
    };
    if it.selected {
        r = r.fill(selection(t));
    }
    let ink = if it.danger { au::red_text(t) } else { t.text };
    let mut words = col()
        .key(format!("{key}-text"))
        .grow(1.0)
        .min_w(0.0)
        .gap(1.0)
        .child(kit::label(format!("{key}-title"), it.title.clone(), type_::ITEM_TITLE, ink).truncate());
    if let Some(sub) = &it.sub {
        let (style, ink) = match (it.sub_mono, it.selected) {
            (true, _) => (SUB_MONO_ROW, t.text_tertiary),
            // The board's selected row says its second line in the stronger ink.
            (false, true) => (SUB, t.text_secondary),
            (false, false) => (SUB, t.text_tertiary),
        };
        words = words.child(kit::label(format!("{key}-sub"), sub.clone(), style, ink).truncate());
    }
    r = r.child(words);
    for tail in &it.tails {
        r = r.child(match tail {
            Tail::Value(v) => kit::label(format!("{key}-value"), v.clone(), VALUE, t.text_tertiary),
            Tail::Mono(v) => kit::label(format!("{key}-value"), v.clone(), VALUE_MONO, t.text_tertiary),
            Tail::Pill { label, tone, dot } => pill(&format!("{key}-pill"), label, *tone, *dot, t),
            Tail::Check => kit::glyph(format!("{key}-check"), &ic::CHECK, 16.0, t.text),
            Tail::Chevron => kit::glyph(format!("{key}-chev"), &ic::CHEV_RIGHT, 14.0, t.text_quaternary),
        });
    }
    for child in extra {
        r = r.child(child);
    }
    r
}

/// How a button looks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Look {
    /// The one primary action of a view: Amber.
    Primary,
    /// Any other action: paper with a drawn border.
    Plain,
    /// Words only, a tint under the pointer (a row's own small action).
    Quiet,
    /// A destructive action, in a confirm: red.
    Danger,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Size {
    /// 22 pt (`.kit-btn-sm`).
    Small,
    /// 28 pt (`.kit-btn-md`).
    Medium,
}

/// A button (`.kit-btn`), as wide as its words. `dimmed`: drawn at half strength and inert
/// (the caller attaches no press). Keys: `<key>`, `<key>-icon`, `<key>-label`.
pub fn button(
    key: &str,
    label: &str,
    icon: Option<&'static Icon>,
    look: Look,
    size: Size,
    dimmed: bool,
    t: &Theme,
) -> El {
    let variant = match look {
        Look::Primary => button::Variant::Primary,
        Look::Plain => button::Variant::Secondary,
        Look::Quiet => button::Variant::Ghost,
        Look::Danger => button::Variant::Danger,
    };
    let size = match size {
        Size::Small => button::Size::Sm,
        Size::Medium => button::Size::Md,
    };
    let state = if dimmed { button::ButtonState::Disabled } else { button::ButtonState::Enabled };
    button::button(button::keys(key), label, icon, variant, size, state, t)
}

/// A confirm over a pane (`.au-scrim` + `.wn-dlg`): a title, what happens in one paragraph,
/// and the buttons side by side, each half the foot (`.wn-dlg-foot.is-even`). The caller makes
/// it modal and names the default button (the safe choice).
///
/// `foot`: each button, and whether it has a drawn border. The board shares the foot's free
/// width evenly AFTER each button's own padding and border (CSS `flex: 1` on content boxes), so
/// a bordered button comes out 2 pt wider than a borderless one: the widths are set to that.
pub fn dialog(
    name: &str,
    width: f32,
    title: &str,
    words: &str,
    extra: Vec<El>,
    foot: Vec<(El, bool)>,
    t: &Theme,
) -> El {
    dialog_enter(name, width, title, words, extra, foot, None, t)
}

/// [`dialog`] where Enter presses foot button `enter` (never a destructive one).
#[allow(clippy::too_many_arguments)] // one slot per board element; the callers are outside these files
pub fn dialog_enter(
    name: &str,
    width: f32,
    title: &str,
    words: &str,
    extra: Vec<El>,
    foot: Vec<(El, bool)>,
    enter: Option<usize>,
    t: &Theme,
) -> El {
    let paragraph = kit::label("dlg-text-0", words, SUB, t.text_secondary).wrap().w_full();
    dialog_with_enter(name, width, title, paragraph, extra, foot, enter, t)
}

/// [`dialog`] with the caller's own paragraph under the title (one whose lead is bold:
/// `account_form::lead_paragraph`).
pub fn dialog_with(
    name: &str,
    width: f32,
    title: &str,
    paragraph: El,
    extra: Vec<El>,
    foot: Vec<(El, bool)>,
    t: &Theme,
) -> El {
    dialog_with_enter(name, width, title, paragraph, extra, foot, None, t)
}

/// [`dialog_with`] where Enter presses foot button `enter` (never a destructive one).
#[allow(clippy::too_many_arguments)] // one slot per board element; the callers are outside these files
pub fn dialog_with_enter(
    name: &str,
    width: f32,
    title: &str,
    paragraph: El,
    extra: Vec<El>,
    foot: Vec<(El, bool)>,
    enter: Option<usize>,
    t: &Theme,
) -> El {
    let mut body = vec![
        col()
            .key("dlg-h")
            .w_full()
            .gap(6.0)
            .child(
                kit::label("dlg-title", title, DIALOG_TITLE, t.text)
                    .role("heading")
                    .level(2)
                    .wrap()
                    .w_full(),
            )
            .child(paragraph),
    ];
    body.extend(extra);
    let count = foot.len().max(1) as f32;
    let fixed = |bordered: bool| 24.0 + if bordered { 2.0 } else { 0.0 };
    let free = width - 48.0 - 8.0 * (count - 1.0) - foot.iter().map(|(_, bordered)| fixed(*bordered)).sum::<f32>();
    let buttons = foot
        .into_iter()
        .map(|(child, bordered)| child.w(free / count + fixed(bordered)))
        .collect();
    scrim(
        0.0,
        dialog::dialog(
            Dialog {
                head: Head::None,
                body,
                body_top: Some(22.0),
                foot: Foot::Row(buttons),
                enter,
                ..Dialog::new("dlg", name, width)
            },
            t,
        ),
        t,
    )
}
