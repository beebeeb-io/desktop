//! What the Backups frames (DW-0737, DW-1411, DW-1410, DW-0739, DW-0742, DW-0741) draw beyond
//! the settings kit. Every piece here is a VARIANT the board draws and `settings_*.rs` does not
//! have; nothing here changes an existing component.
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | Card with 12 pt padding (`.kit-card.p-3`) | `card(index, t)` | `card-<N>` |
//! | A card that is one link (`.kit-card.px-3.py-2.5` in a `.kit-go`) | `link_card(index, icon, words, strong, t)` | `card-<N>`, `-icon`, `-text`, `-chev` |
//! | A card line with a machine's value in it | `sub_rich(index, words, mono, t)` | `card-<N>-sub` |
//! | Progress (`.mk-bar`) | `bar(key, fraction, name, t)` | `<key>`, `<key>-fill` |
//! | Toned tile at any size (`.kit-tile`) | `tile(index, TileTone, icon, size, t)` | `tile-<N>`, `tile-<N>-icon` |
//! | Row with a tile or a coloured title | `row_item(list, index, RowItem, t)` | `item-<L>-<I>`, `-icon`, `-title`, `-sub` |
//! | Small button without a border (`.kit-btn-sm.bg-transparent`) | `ghost_button(index, label, t)` | `btn-<N>`, `btn-<N>-label` |
//! | Dialog-size button in four weights (`.kit-btn-md`) | `button_md(index, label, icon, Md, t)` | `btn-<N>`, `-icon`, `-label` |
//! | Alert (`.wn-dlg` with `.wn-dlg-alert` and a stacked foot) | `alert(name, tile, title, sub, buttons, t)` | `scrim`, `dlg`, `dlg-title`, `dlg-sub`, `dlg-foot` |
//! | A dialog paragraph at body size with a machine's value | `para(index, words, mono, t)` | `dlg-text-<I>` |
//! | One-time code, six boxes | `code_boxes(digits, t)` | `secure-code`, `code-label`, `code-<I>`, `code-caret` |

use crate::theme::icons_settings as ic;
use crate::theme::tokens::{Theme, radius, type_};
use rune::tok::Family;
use rune::{Color, El, Icon, Span, TextStyle, col, el, oklch, rich, row, text};

use super::kit;
use super::settings_dialog::{BUTTON_MD, DIALOG_TITLE, scrim_color};
use super::settings_shell::SUB;

/// 13px/400 on a 17px line: body words (`.t-body`).
pub const BODY_13: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
/// 11px/500 on a 15px line: a field's label above it (`.kit-field-outer`).
pub const FIELD_LABEL: TextStyle = TextStyle::new(11.0, 500.0).lh(15.0);
/// 18px mono: one digit of a one-time code.
pub const CODE_DIGIT: TextStyle = TextStyle::new(18.0, 400.0).family(Family::Mono).lh(24.0);

/// `--color-red-text`: red words on paper (a destructive row, a destructive quiet button).
pub fn red_text(t: &Theme) -> Color {
    if t.dark {
        oklch(0.70, 0.19, 25.0)
    } else {
        oklch(0.48, 0.19, 25.0)
    }
}

/// `bg-red`: the fill of a destructive button, the same in both themes.
pub fn red_fill() -> Color {
    oklch(0.55, 0.2, 25.0)
}

/// The panel with 12 pt padding (`.kit-card.p-3.flex.items-center.gap-3`). Key: `card-<N>`.
pub fn card(index: usize, t: &Theme) -> El {
    row()
        .key(format!("card-{index}"))
        .w_full()
        .center_y()
        .padding(12.0)
        .gap(12.0)
        .radius(radius::LG)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .box_shadow(kit::drop(1.0, 2.0, Color::BLACK.alpha(0.04)))
}

/// A card's second line with a machine's value inside it (`mono`: byte ranges in JetBrains
/// Mono). Key: `card-<N>-sub`.
pub fn sub_rich(index: usize, words: &str, mono: &[std::ops::Range<usize>], t: &Theme) -> El {
    let key = format!("card-{index}-sub");
    kit::note_text(&key, &key, words, SUB, t.text_tertiary);
    let spans = mono.iter().map(|r| Span::new(r.start, r.end).mono()).collect();
    rich(words, spans).key(key).font(SUB).color(t.text_tertiary).w_full()
}

/// A card that is one link: a red glyph, the words with their strong end, a chevron
/// (DW-0737's "Couldn't back up 3 items"). Keys: `card-<N>`, `-icon`, `-text`, `-chev`.
pub fn link_card(index: usize, icon: &'static Icon, words: &str, strong: &str, t: &Theme) -> El {
    let key = format!("card-{index}");
    let all = format!("{words} {strong}");
    let text_key = format!("{key}-text");
    kit::note_text(&text_key, &text_key, &all, BODY_13, t.text);
    let spans = vec![Span::new(words.len() + 1, all.len()).weight(700.0)];
    row()
        .key(key.clone())
        .role("link")
        .label(all.clone())
        .w_full()
        .center_y()
        .py(10.0)
        .px(12.0)
        .gap(8.0)
        .radius(radius::LG)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .box_shadow(kit::drop(1.0, 2.0, Color::BLACK.alpha(0.04)))
        .focusable(true)
        .child(kit::glyph(format!("{key}-icon"), icon, 16.0, t.red))
        .child(rich(all, spans).key(text_key).font(BODY_13).color(t.text).grow(1.0))
        .child(kit::glyph(format!("{key}-chev"), &ic::CHEV_RIGHT, 13.0, t.text))
}

/// Progress (`.mk-bar`): 4 pt, the done part in Amber (encrypting and uploading: encryption
/// state). `name` says it in words; the fraction is its value. Keys: `<key>`, `<key>-fill`.
pub fn bar(key: &str, fraction: f32, name: &str, t: &Theme) -> El {
    let fraction = fraction.clamp(0.0, 1.0);
    row()
        .key(key.to_string())
        .role("progressbar")
        .label(name)
        .w_full()
        .h(4.0)
        .clip_rounded(2.0)
        .fill(t.text.alpha(0.09))
        .child(
            el().key(format!("{key}-fill"))
                .w_frac(fraction)
                .h(4.0)
                .radius(2.0)
                .fill(t.accent),
        )
}

/// A tile's tone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TileTone {
    /// Done, healthy (`.tone-green`).
    Green,
    /// A thing, no state (`.tone-neutral`).
    Neutral,
}

/// A glyph on its tone's tint (`.kit-tile`), `size` points across, the glyph half of it.
/// Keys: `tile-<N>`, `tile-<N>-icon`.
pub fn tile(index: usize, tone: TileTone, icon: &'static Icon, size: f32, t: &Theme) -> El {
    let (fill, ink) = match tone {
        TileTone::Green => (t.green.alpha(0.14), t.green),
        TileTone::Neutral => (t.text.alpha(0.07), t.text_secondary),
    };
    el().key(format!("tile-{index}"))
        .size(size, size)
        .radius(if size < 32.0 { 8.0 } else { 10.0 })
        .fill(fill)
        .center()
        .child(kit::glyph(format!("tile-{index}-icon"), icon, size / 2.0, ink))
}

/// What leads a row.
pub enum Leading {
    /// An 18 pt glyph in the list's 30 pt icon column.
    Icon(&'static Icon),
    /// Any element of the caller's (a tile).
    El(El),
}

/// A row the settings list does not have: a tile in front, or a title in another colour.
pub struct RowItem {
    pub leading: Leading,
    pub title: String,
    pub title_color: Option<Color>,
    pub sub: Option<String>,
    /// A hairline above the row, from 10 pt in.
    pub ruled: bool,
    /// The row is a control (a link): it takes the press and the focus.
    pub link: bool,
    /// Controls at the trailing edge.
    pub trailing: Vec<El>,
}

/// One row (`.kit-item`). Keys: `item-<L>-<I>`, `-icon`, `-title`, `-sub`.
pub fn row_item(list: usize, index: usize, it: RowItem, t: &Theme) -> El {
    let key = format!("item-{list}-{index}");
    let name = match &it.sub {
        Some(sub) => format!("{}. {sub}", it.title),
        None => it.title.clone(),
    };
    let mut r = row()
        .key(key.clone())
        .w_full()
        .min_h(if it.sub.is_some() { 40.0 } else { 28.0 })
        .px(10.0)
        .gap(8.0)
        .center_y();
    r = if it.link {
        r.role("link").label(name).hover(t.hover).press(t.press).focusable(true)
    } else {
        r.role("listitem").label(name)
    };
    if it.ruled {
        r = r.child(
            el().key(format!("{key}-rule"))
                .pin(Some(10.0), Some(0.0), Some(0.0), None)
                .h(kit::HAIR)
                .fill(t.separator),
        );
    }
    match it.leading {
        Leading::Icon(icon) => {
            r = r.child(el().key(format!("{key}-icon-box")).w(30.0).center().child(kit::glyph(
                format!("{key}-icon"),
                icon,
                18.0,
                t.text_secondary,
            )));
        }
        Leading::El(lead) => r = r.child(lead),
    }
    let mut words = col().key(format!("{key}-text")).grow(1.0).min_w(0.0).gap(1.0).child(
        kit::label(
            format!("{key}-title"),
            it.title,
            type_::ITEM_TITLE,
            it.title_color.unwrap_or(t.text),
        )
        .truncate(),
    );
    if let Some(sub) = it.sub {
        words = words.child(kit::label(format!("{key}-sub"), sub, SUB, t.text_tertiary).truncate());
    }
    r = r.child(words);
    for child in it.trailing {
        r = r.child(child);
    }
    r
}

/// A small button without fill or border (`.kit-btn-sm.bg-transparent.text-ink-2`): a quiet
/// action in a row ("Remove"). Keys: `btn-<N>`, `btn-<N>-label`.
pub fn ghost_button(index: usize, label: &str, name: &str, t: &Theme) -> El {
    let key = format!("btn-{index}");
    row()
        .key(key.clone())
        .role("button")
        .label(name)
        .h(22.0)
        .px(12.0)
        .center()
        .radius(radius::MD)
        .hover(t.paper2)
        .press(t.paper3)
        .focusable(true)
        .child(kit::label(
            format!("{key}-label"),
            label,
            type_::BUTTON_SM,
            t.text_secondary,
        ))
}

/// How a dialog-size button looks.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Md {
    /// Paper with a drawn border: an action that is not the primary one.
    Plain,
    /// Words only (Cancel in a stack).
    Quiet,
    /// Red words only: the destructive choice of a stack.
    QuietRed,
    /// Red fill, white words: a destructive action behind a check. `false`: not available yet
    /// (the board draws it at half strength).
    Danger(bool),
}

/// A dialog-size button (`.kit-btn.kit-btn-md`, 28 pt). Keys: `btn-<N>`, `-icon`, `-label`.
pub fn button_md(index: usize, label: &str, icon: Option<&'static Icon>, kind: Md, t: &Theme) -> El {
    let key = format!("btn-{index}");
    let ink = match kind {
        Md::Plain => t.text,
        Md::Quiet => t.text_secondary,
        Md::QuietRed => red_text(t),
        Md::Danger(_) => Color::WHITE,
    };
    let mut b = row()
        .key(key.clone())
        .role("button")
        .label(label)
        .h(28.0)
        .px(12.0)
        .gap(7.0)
        .center()
        .radius(radius::MD);
    b = match kind {
        Md::Plain => b
            .fill(t.paper)
            .border(1.0, t.card_border)
            .border_layout()
            .hover(t.paper2)
            .press(t.paper3)
            .focusable(true),
        Md::Quiet | Md::QuietRed => b.hover(t.paper2).press(t.paper3).focusable(true),
        Md::Danger(true) => b
            .fill(red_fill())
            .hover(red_fill().mix(Color::BLACK, 0.05))
            .press(red_fill().mix(Color::BLACK, 0.10))
            .focusable(true),
        // Not available: drawn at half strength, takes no press and no focus.
        Md::Danger(false) => b.fill(red_fill()).opacity(0.5).disabled(true),
    };
    if let Some(icon) = icon {
        b = b.child(kit::glyph(format!("{key}-icon"), icon, 16.0, ink));
    }
    b.child(kit::label(format!("{key}-label"), label, BUTTON_MD, ink))
}

/// A paragraph of a dialog's body at body size (`.wn-dlg-body > p.t-body`), with a machine's
/// value inside it (`mono`: byte ranges). Key: `dlg-text-<I>`.
pub fn para(index: usize, words: &str, mono: &[std::ops::Range<usize>], t: &Theme) -> El {
    let key = format!("dlg-text-{index}");
    kit::note_text(&key, &key, words, BODY_13, t.text_secondary);
    let spans = mono.iter().map(|r| Span::new(r.start, r.end).mono()).collect();
    rich(words, spans)
        .key(key)
        .font(BODY_13)
        .color(t.text_secondary)
        .wrap()
        .w_full()
}

/// An alert in the middle of the body, on its scrim (`.wn-dlg` with `.wn-dlg-alert`): a tile,
/// the question and what it means, then the choices stacked, the safest first. 420 pt wide.
/// Keys: `scrim`, `dlg`, `dlg-title`, `dlg-sub`, `dlg-foot`.
pub fn alert(name: &str, tile: El, title: &str, sub: &str, buttons: Vec<El>, t: &Theme) -> El {
    let head = col().key("dlg-head").pt(20.0).px(24.0).pb(16.0).child(
        row()
            .key("dlg-alert")
            .w_full()
            .items_start()
            .gap(14.0)
            .child(tile)
            .child(
                col()
                    .key("dlg-words")
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(6.0)
                    .child(
                        kit::label("dlg-title", title, DIALOG_TITLE, t.text)
                            .role("heading")
                            .level(2)
                            .w_full(),
                    )
                    .child(kit::label("dlg-sub", sub, SUB, t.text_tertiary).wrap().w_full()),
            ),
    );
    let mut foot = col()
        .key("dlg-foot")
        .w_full()
        .gap(8.0)
        .py(16.0)
        .px(24.0)
        .fill(t.surface)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout();
    for b in buttons {
        foot = foot.child(b.w_full());
    }
    let dlg = col()
        .key("dlg")
        .role("alertdialog")
        .label(name)
        // The foot's hairline ends the height in a half point (`settings_dialog::dialog`).
        .pixel_snap(false)
        .w(420.0)
        .radius(8.0)
        .fill(t.card)
        .box_shadow(kit::drop(24.0, 60.0, Color::BLACK.alpha(0.28)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(col().key("dlg-clip").w_full().clip_rounded(8.0).child(head).child(foot));
    col()
        .key("scrim")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .fill(scrim_color(t))
        .child(dlg)
}

/// How many digits a one-time code has.
pub const CODE_LEN: usize = 6;

/// A one-time code as six boxes under its label (`.kit-fieldwrap`): the digits typed so far,
/// the next box lit with the caret. ONE control: it takes the focus and the keys, and its
/// name never holds a digit. Everything that can show a digit sits inside `secure-code`
/// (`src/feedback/README.md`), each digit a secure text.
/// Keys: `secure-code`, `code-label`, `code-<I>`, `code-caret`.
pub fn code_boxes(label: &str, digits: &str, t: &Theme) -> El {
    let typed: Vec<char> = digits.chars().take(CODE_LEN).collect();
    let mut boxes = row()
        .key("secure-code")
        .role("textbox")
        .label(format!("{label}, {CODE_LEN} digits, {} entered", typed.len()))
        .gap(6.0)
        .focusable(true)
        .autofocus();
    for i in 0..CODE_LEN {
        let mut b = el()
            .key(format!("code-{i}"))
            // The dialog is centred on a quarter point (`settings_dialog::dialog`); the boxes'
            // strong edges are drawn where they fall, as the board draws them.
            .pixel_snap(false)
            .size(36.0, 42.0)
            .radius(8.0)
            .fill(t.field)
            .center();
        b = if i == typed.len() {
            b.box_shadow(kit::ring(3.0, t.accent.alpha(0.3)))
                .box_shadow(rune::Shadow {
                    x: 0.0,
                    y: 0.0,
                    blur: 0.0,
                    spread: 1.0,
                    color: t.accent_deep,
                    inset: true,
                })
                .child(
                    // `margin: 0 1px -3px` in a centred grid cell puts the caret at 17.25, 14 in
                    // the box; the board paints its sides on whole pixels (17.5 at 200 %).
                    el().key("code-caret")
                        .pixel_snap(false)
                        .pin(Some(17.5), Some(14.0), None, None)
                        .size(1.5, 17.0)
                        .radius(1.0)
                        .fill(t.accent_deep),
                )
        } else {
            b.box_shadow(kit::ring_inset(t.separator))
        };
        if let Some(d) = typed.get(i) {
            b = b.child(
                text(d.to_string())
                    .key(format!("code-{i}-digit"))
                    .font(CODE_DIGIT)
                    .color(t.text)
                    .secure()
                    .revealed(true),
            );
        }
        boxes = boxes.child(b);
    }
    col()
        .key("code-field")
        .w_full()
        .gap(6.0)
        .child(
            el().key("code-label-box")
                .px(2.0)
                .w_full()
                .child(kit::label("code-label", label, FIELD_LABEL, t.text_secondary).w_full()),
        )
        .child(boxes)
}
