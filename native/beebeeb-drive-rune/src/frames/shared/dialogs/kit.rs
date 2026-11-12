//! The pieces of the lane's dialogs that `crate::ui` does not have, or has keyed for another
//! group: the card with a head of its own, the alert, a labelled field, the wide switch, a
//! grouped list keyed as this group's keymap keys it, a person's row, the safety number's
//! groups, the code boxes and the plain note.
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | Layer: scrim and card, modal | `layer(name, card, keys, dismiss, under_alert, t)` | `dialog-<name>`, `scrim`, `scrim-2` |
//! | Card with any head (`.wn-dlg`) | `card(name, width, head, body, foot, t)` | `dlg`, `dlg-clip`, `dlg-head`, `dlg-body`, `dlg-foot` |
//! | Alert (`.wn-dlg-alert`), its own scrim | `alert(name, width, tile, title, sub, Foot, over_dialog, keys, dismiss, t)` | `alert-<name>`, `alert-scrim`, `alert`, `alert-title`, `alert-sub`, `alert-foot` |
//! | Field with its label (`.kit-fieldwrap`) | `field(&Field, edit, on_edit, input, t)` | `<key>-wrap`, `<key>-label`, `<key>`, `<key>-ph`, `<key>-caret` |
//! | Wide switch (`.kit-seg-md.w-full`) | `seg(n, name, tabs, t, each)` | `seg-<n>`, `seg-<n>-<words>`, `-label` |
//! | Grouped list (`.kit-list`) | `list(n, name, title, body, foot, t)` | `list-<n>` (the card), `list-<n>-title`, `list-<n>-foot` |
//! | A person's row (`.kit-item` with `.kit-avatar`) | `person(n, m, &Member, link, t)` | `item-<n>-<m>`, `-person`, `-title`, `-sub`, `-chev` |
//! | Safety number (`.grid.font-mono`) | `groups(&Groups, numbers, t)` | `safety`, `safety-<i>` |
//! | Code boxes (`.kit-fieldwrap` > six boxes) | `code_boxes(label, digits, t)` | `code-field`, `code-label`, `secure-code`, `code-<i>`, `code-caret` |
//! | Note with a glyph (`.kit-banner.tone-neutral`) | `note(key, icon, words, t)` | `<key>`, `<key>-icon`, `<key>-text` |
//!
//! Keys a frame's golden has are the board's (`scripts/keymap.js`, the Shared and Trash
//! section); the rest are named here for the tests.

use crate::State;
use crate::frames::shared::parts::controls::slug;
use crate::frames::shared::parts::{self, style};
use crate::frames::shared::source::Member;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::settings_dialog::{DIALOG_TITLE, scrim_color};
use crate::ui::settings_list::{FOOT, LIST_TITLE};
use crate::ui::{field as ui_field, kit};
use rune::tok::Family;
use rune::{Color, El, Icon, KeyEvent, Shadow, TextEdit, TextStyle, Tx, col, el, row, text, text_input};

/// `.kit-field-outer`: a field's label above it.
pub const FIELD_LABEL: TextStyle = TextStyle::new(11.0, 500.0).lh(15.0);
/// `.kit-field-value`, `.kit-field-ph`: what is typed, and the placeholder.
pub const FIELD_VALUE: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
/// `.t-body`: a dialog's sentence that says what will happen.
pub const BODY: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
/// `.kit-seg-md span`: a tab of the wide switch.
pub const SEG_MD: TextStyle = TextStyle::new(12.5, 550.0).lh(17.0);
/// `.kit-banner-text`.
pub const NOTE: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0);
/// A digit of the authenticator code (`font-mono`, 18px).
pub const CODE_DIGIT: TextStyle = TextStyle::new(18.0, 400.0).family(Family::Mono).lh(17.0);
/// The six boxes of the code.
pub const CODE_LEN: usize = 6;

/// The caption bar the scrim leaves free: the window can still be moved and closed (`.mk-scrim`).
const CAPTION_H: f32 = 32.0;

/// A dialog's layer over the whole window: the scrim under the caption bar, the card in the
/// middle of it. Modal: the window behind takes no press and no key; Esc is `dismiss`; every
/// key the dialog does not use stops at `keys` (the list behind never moves under it). The
/// layer's key is the dialog's own: a dialog opened from another is a new modal, and the
/// keyboard goes to ITS first control.
///
/// `under_alert`: an alert stands over this dialog (DW-0515). The board stacks both washes
/// (`.mk-scrim`) under both cards: the window behind darkens twice, this card not at all, and
/// the alert's shadow falls on it. So the second wash is drawn here, under this card, and the
/// alert draws none of its own (`alert`'s `over_dialog`).
pub fn layer(
    name: &str,
    card: El,
    keys: fn(&mut State, &KeyEvent, &mut Tx) -> bool,
    dismiss: fn(&mut State, &mut Tx),
    under_alert: bool,
    t: &Theme,
) -> El {
    let mut scrim = col()
        .key("scrim")
        .pin(Some(0.0), Some(CAPTION_H), Some(0.0), Some(0.0))
        .center()
        .fill(scrim_color(t));
    if under_alert {
        scrim = scrim.child(
            el().key("scrim-2")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .fill(scrim_color(t)),
        );
    }
    el().key(format!("dialog-{name}"))
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .child(scrim.child(card))
        .on_key(keys)
        .modal_tx(false, dismiss)
}

/// The card (`.wn-dlg`), as `ui::dialog::card` draws it, with a head of the caller's: a back
/// button beside the title (DW-0512), or none (DW-0513b opens on its own heading).
pub fn card(name: &str, width: f32, head: Option<El>, body: Vec<El>, foot: El, t: &Theme) -> El {
    let mut b = col().key("dlg-body").pt(10.0).px(24.0).pb(18.0).gap(12.0).min_h(0.0);
    for child in body {
        b = b.child(child);
    }
    let foot = foot_band(foot, "dlg-foot", t);
    let mut clip = col().key("dlg-clip").w_full().clip_rounded(crate::ui::dialog::RADIUS);
    if let Some(head) = head {
        clip = clip.child(col().key("dlg-head").pt(20.0).px(24.0).pb(4.0).gap(3.0).child(head));
    }
    shadowed(col().key("dlg").role("dialog").label(name), width, t).child(clip.child(b).child(foot))
}

/// The card's surface: its corner, fill, shadow and hairline ring. The foot's hairline makes a
/// dialog's height end in a half point, so centring puts its top on a quarter point; the board
/// draws it there, edges half covered, and so does this (`pixel_snap(false)`).
fn shadowed(card: El, width: f32, t: &Theme) -> El {
    card.pixel_snap(false)
        .w(width)
        .radius(crate::ui::dialog::RADIUS)
        .fill(t.card)
        .box_shadow(kit::drop(24.0, 60.0, Color::BLACK.alpha(0.28)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
}

/// The band of buttons at a card's bottom (`.wn-dlg-foot`).
fn foot_band(foot: El, key: &str, t: &Theme) -> El {
    foot.key(key.to_string())
        .gap(8.0)
        .py(16.0)
        .px(24.0)
        .fill(t.surface)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
}

/// The standard head: the title and the line under it (`.t-title`, `.t-sub`).
pub fn title(words: &str, t: &Theme) -> El {
    kit::label("dlg-title", words, DIALOG_TITLE, t.text)
        .role("heading")
        .level(2)
}

/// How an alert lays out its answers.
pub enum Foot {
    /// Side by side, each as wide as the rest (`.is-even`).
    Even(Vec<El>),
    /// One under the other, each as wide as the alert (`.is-stack`): the labels do not fit side by side.
    Stack(Vec<El>),
}

/// An alert (`.wn-dlg-alert`, `role="alertdialog"`) on a layer of its own, so it can stand over
/// another dialog (DW-0515 over People): the toned tile, the question, what it means, the
/// answers. Its keys are `alert-…`, apart from the dialog's under it. `over_dialog`: the
/// dialog under it draws the alert's wash, under its own card (`layer`'s `under_alert`); the
/// alert's layer still takes every press and key.
#[allow(clippy::too_many_arguments)]
pub fn alert(
    name: &str,
    width: f32,
    tile: El,
    title: &str,
    sub: &str,
    foot: Foot,
    over_dialog: bool,
    keys: fn(&mut State, &KeyEvent, &mut Tx) -> bool,
    dismiss: fn(&mut State, &mut Tx),
    t: &Theme,
) -> El {
    let words = col()
        .key("alert-words")
        .grow(1.0)
        .min_w(0.0)
        .gap(6.0)
        .child(
            kit::label("alert-title", title, DIALOG_TITLE, t.text)
                .role("heading")
                .level(2)
                .w_full(),
        )
        .child(kit::label("alert-sub", sub, style::SUB, t.text_tertiary).wrap());
    let head = row()
        .key("alert-head")
        .pt(20.0)
        .px(24.0)
        .pb(16.0)
        .gap(14.0)
        .items_start()
        .child(tile)
        .child(words);
    let inner = width - 48.0;
    let band = match foot {
        Foot::Even(buttons) => {
            let n = buttons.len().max(1) as f32;
            let each = (inner - 8.0 * (n - 1.0)) / n;
            row().center_y().children(buttons.into_iter().map(|b| b.w(each)))
        }
        Foot::Stack(buttons) => col().children(buttons.into_iter().map(|b| b.w_full())),
    };
    let card = shadowed(col().key("alert").role("alertdialog").label(name), width, t).child(
        col()
            .key("alert-clip")
            .w_full()
            .clip_rounded(crate::ui::dialog::RADIUS)
            .child(head)
            .child(foot_band(band, "alert-foot", t)),
    );
    el().key(format!("alert-{}", slug(name)))
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .child(
            col()
                .key("alert-scrim")
                .pin(Some(0.0), Some(CAPTION_H), Some(0.0), Some(0.0))
                .center()
                .fill(if over_dialog {
                    Color::TRANSPARENT
                } else {
                    scrim_color(t)
                })
                .child(card),
        )
        .on_key(keys)
        .modal_tx(false, dismiss)
}

/// A field: its label above it, the well, the placeholder while it is empty.
pub struct Field<'a> {
    /// The key of the well (`field-<n>`).
    pub key: &'a str,
    /// The label above it, and the field's name.
    pub label: &'a str,
    /// Shown while nothing is typed ("" for none).
    pub placeholder: &'a str,
    /// The text input's own key (tests, `Tx::focus`).
    pub input_key: &'static str,
    /// The width the text has inside the well (the dialog's width less its padding, the well's
    /// padding and its line). Text wider than this scrolls in the input.
    pub inner_w: f32,
}

/// A labelled one-line field (`.kit-fieldwrap` > `.kit-field.kit-field-md`), with the board's
/// two looks: at rest (the recessed well, a hairline) and in focus (the card's paper, the deep
/// Amber line and the Amber ring). Focus is the model's (`TextEdit::focused`, which Rune keeps):
/// a dialog opens with its field in focus by setting it. The caret is drawn as the board draws
/// it (1.5 by 17, 1 pt after the character it follows, before the placeholder of an empty field);
/// Rune's own is left unseen, as `ui::account_form::field` does, while the text fits the well.
/// Text wider than the well scrolls in the input, where only Rune knows the caret's place: then
/// Rune's own caret shows, in the same deep Amber, and the drawn one goes.
pub fn field(
    spec: &Field<'_>,
    edit: &TextEdit,
    on_edit: impl for<'a> Fn(&'a mut State) -> &'a mut TextEdit + 'static,
    input: impl FnOnce(El) -> El,
    t: &Theme,
) -> El {
    let key = spec.key;
    let focused = edit.focused;
    // The drawn caret's room after the last character: 1 pt, and its own 1.5.
    let scrolls = ui_field::advance(&edit.text, FIELD_VALUE) + 2.5 > spec.inner_w;
    let rune_caret = if scrolls { t.accent_deep } else { Color::TRANSPARENT };
    let mut field = text_input(edit, FIELD_VALUE)
        .key(spec.input_key)
        .label(spec.label)
        .single_line()
        .color(t.text)
        .caret(rune_caret, ui_field::selection(t))
        .on_edit(on_edit)
        .grow(1.0)
        .min_w(0.0);
    if !spec.placeholder.is_empty() {
        // Rune's placeholder names it to assistive technology; the drawn one is below.
        field = field.placeholder(spec.placeholder, Color::TRANSPARENT);
    }
    let mut inner = row()
        .key(format!("{key}-main"))
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .center_y()
        .clip_rounded(0.0)
        .child(input(field));
    // `.kit-caret` stands in front of the placeholder: an empty field's words move over by it.
    let caret_room = if focused { 3.5 } else { 0.0 };
    if edit.text.is_empty() && !spec.placeholder.is_empty() {
        inner = inner.child(
            el().key(format!("{key}-ph-box"))
                .a11y_hidden()
                .absolute(caret_room, 0.0)
                .h_full()
                .center_y()
                .child(kit::label(
                    format!("{key}-ph"),
                    spec.placeholder,
                    FIELD_VALUE,
                    t.text_tertiary,
                )),
        );
    }
    if focused && !scrolls {
        let before = &edit.text[..edit.focus.min(edit.text.len())];
        let x = ui_field::advance(before, FIELD_VALUE);
        inner = inner.child(
            el().key(format!("{key}-caret"))
                .a11y_hidden()
                // 5 pt under the well's top: 4 under its 1 pt line.
                .absolute(ui_field::snap(x + 1.0), 4.0)
                .size(1.5, 17.0)
                .radius(1.0)
                .fill(t.accent_deep),
        );
    }
    let well = row()
        .key(key.to_string())
        .w_full()
        .h(28.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(radius::MD)
        .border_layout()
        .child(inner);
    let well = if focused {
        ui_field::ringed(
            well.fill(t.card).border(1.0, t.accent_deep),
            &format!("{key}-ring"),
            3.0,
            radius::MD,
            t.accent.alpha(0.30),
        )
    } else {
        well.fill(t.field)
            .border(1.0, Color::TRANSPARENT)
            .box_shadow(kit::ring_inset(t.separator))
    };
    col()
        .key(format!("{key}-wrap"))
        .w_full()
        .gap(6.0)
        .child(outer_label(&format!("{key}-label"), spec.label, t))
        .child(well)
}

/// The label above a control (`.kit-field-outer`), 2 pt in from the control's edge. It names
/// the control to the eye only: the control carries its own name.
pub fn outer_label(key: &str, words: &str, t: &Theme) -> El {
    kit::label(key.to_string(), words, FIELD_LABEL, t.text_secondary)
        .a11y_hidden()
        .mx(2.0)
}

/// The key of the tab worded `words` of switch `n`.
pub fn tab_key(n: usize, words: &str) -> String {
    format!("seg-{n}-{}", slug(words))
}

/// The wide switch (`.kit-seg.kit-seg-md.w-full`): its tabs share the width. `tabs` are
/// (words, chosen); the caller adds each tab's press through `each`. One Tab stop, the chosen
/// tab; Left and Right move between the tabs (the dialog's keys).
pub fn seg(n: usize, name: &str, tabs: &[(&'static str, bool)], t: &Theme, each: impl Fn(usize, El) -> El) -> El {
    let mut well = row()
        .key(format!("seg-{n}"))
        .role("tablist")
        .label(name)
        .w_full()
        .padding(2.0)
        .radius(10.0)
        .fill(t.field)
        .box_shadow(kit::ring_inset(t.separator));
    for (i, (words, on)) in tabs.iter().enumerate() {
        let k = tab_key(n, words);
        let mut tab = el()
            .key(k.clone())
            .role("tab")
            .label(*words)
            .checked(*on)
            .grow(1.0)
            .h(28.0)
            .px(12.0)
            .center()
            .radius(8.0)
            .focusable(true)
            .tab_index(if *on { 0 } else { -1 })
            .child(kit::label(
                format!("{k}-label"),
                *words,
                SEG_MD,
                if *on { t.text } else { t.text_secondary },
            ));
        tab = if *on {
            tab.fill(t.card)
                .box_shadow(kit::drop(1.0, 2.0, t.shadow_raised))
                .box_shadow(kit::ring(kit::HAIR, t.separator))
        } else {
            tab.hover(t.hover).press(t.press)
        };
        well = well.child(each(i, tab));
    }
    well
}

/// A grouped list in a dialog (`.kit-list`): its heading, the card, its foot note. The card
/// holds `body`: the rows, or a control (DW-0512's switch). Keys as the board's keymap gives
/// them in Shared and Trash: the card is `list-<n>`; the heading and the foot are their whole
/// lines (`list-<n>-title`, `list-<n>-foot`), the words 14 and 16 pt in.
pub fn list(n: usize, name: &str, title: Option<&str>, body: El, foot: Option<&str>, t: &Theme) -> El {
    let mut l = col()
        .key(format!("list-{n}-group"))
        .role("group")
        .label(name)
        .w_full()
        .gap(6.0);
    if let Some(title) = title {
        l = l.child(
            kit::label(format!("list-{n}-title"), title, LIST_TITLE, t.text_tertiary)
                .a11y_hidden()
                .w_full()
                .px(14.0),
        );
    }
    l = l.child(
        col()
            .key(format!("list-{n}"))
            .w_full()
            .radius(radius::LG)
            .fill(t.card)
            .box_shadow(kit::ring(kit::HAIR, t.separator))
            .child(body),
    );
    if let Some(foot) = foot {
        l = l.child(
            kit::label(format!("list-{n}-foot"), foot, FOOT, t.text_tertiary)
                .w_full()
                .px(16.0)
                .wrap(),
        );
    }
    l
}

/// The rows of a list's card (`.kit-list-body`): a list to assistive technology, its corners
/// clipped so a row's tint stays inside.
pub fn rows(n: usize, name: &str, items: Vec<El>) -> El {
    col()
        .key(format!("list-{n}-rows"))
        .role("list")
        .label(name)
        .w_full()
        .clip_rounded(radius::LG)
        .children(items)
}

/// A person's initials on their tint, 30 pt across, ringed in the card's paper
/// (`.kit-avatar`, 11px/650). `kit::avatar` sets 30 pt initials at 10px; the board's are 11px.
pub fn avatar(key: &str, initials: &str, hue: char, t: &Theme) -> El {
    const INITIALS: TextStyle = TextStyle::new(11.0, 650.0);
    kit::note_text(key, key, initials, INITIALS, t.avatar_ink());
    el().key(key.to_string())
        .a11y_hidden()
        .size(30.0, 30.0)
        .radius(15.0)
        .fill(t.avatar_tint(parts::hue(hue)))
        .box_shadow(kit::ring(2.0, t.card))
        .center()
        .child(
            text(initials)
                .key(format!("{key}-initials"))
                .font(INITIALS)
                .color(t.avatar_ink()),
        )
}

/// A person's row (`.kit-item`): their avatar, name and what they may do; a chevron when the
/// row opens their access (`link`). Keys: `item-<n>-<m>`, `-person`, `-title`, `-sub`, `-chev`.
pub fn person(n: usize, m: usize, member: &Member, link: bool, t: &Theme) -> El {
    let k = format!("item-{n}-{m}");
    let line = member.line();
    let words = col()
        .key(format!("{k}-text"))
        .grow(1.0)
        .min_w(0.0)
        .gap(1.0)
        .child(kit::label(format!("{k}-title"), member.name.clone(), type_::ITEM_TITLE, t.text).truncate())
        .child(kit::label(format!("{k}-sub"), line.clone(), style::SUB, t.text_tertiary).truncate());
    let mut r = row()
        .key(k.clone())
        .label(format!("{}, {line}", member.name))
        .w_full()
        .h(40.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .child(avatar(&format!("{k}-person"), &member.initials, member.hue, t))
        .child(words);
    if link {
        r = r
            .role("link")
            .hover(t.hover)
            .press(t.press)
            .focusable(true)
            .child(kit::glyph(
                format!("{k}-chev"),
                &ic::CHEV_RIGHT,
                13.0,
                t.text_quaternary,
            ));
    } else {
        r = r.role("listitem");
    }
    r
}

/// How a safety number's groups are set (the two sizes the board draws).
pub struct Groups {
    pub style: TextStyle,
    pub cell_w: f32,
    pub pad: f32,
    pub row_gap: f32,
}

/// DW-0513: 17px, tracked .04em, in 16 pt of padding.
pub const GROUPS_LARGE: Groups = Groups {
    style: TextStyle::new(17.0, 400.0).family(Family::Mono).lh(17.0).tracking(0.68),
    cell_w: 78.0,
    pad: 16.0,
    row_gap: 10.0,
};
/// DW-0513b: 15px, in 12 pt.
pub const GROUPS_SMALL: Groups = Groups {
    style: TextStyle::new(15.0, 400.0).family(Family::Mono).lh(17.0),
    cell_w: 80.0,
    pad: 12.0,
    row_gap: 8.0,
};

/// A safety number as twelve groups of five digits, four to a line, on the recessed well.
/// What it shows is the caller's: this draws, it never computes. Keys: `safety`, `safety-<i>`.
pub fn groups(g: &Groups, numbers: &[String], t: &Theme) -> El {
    let mut lines = col()
        .key("safety")
        .role("group")
        .label(format!("Safety number: {}", numbers.join(" ")))
        .w_full()
        .padding(g.pad)
        .gap(g.row_gap)
        .radius(12.0)
        .fill(t.field);
    for (l, line) in numbers.chunks(4).enumerate() {
        let mut r = row().key(format!("safety-line-{l}")).a11y_hidden().gap(16.0);
        for (c, group) in line.iter().enumerate() {
            let i = l * 4 + c;
            r = r.child(
                el().key(format!("safety-{i}-cell"))
                    .w(g.cell_w)
                    .center()
                    .child(kit::label(format!("safety-{i}"), group.clone(), g.style, t.text)),
            );
        }
        lines = lines.child(r);
    }
    lines
}

/// An inset line of `width` (CSS `box-shadow: inset 0 0 0 <width>`).
fn inset(width: f32, color: Color) -> Shadow {
    Shadow {
        x: 0.0,
        y: 0.0,
        blur: 0.0,
        spread: width,
        color,
        inset: true,
    }
}

/// The authenticator code as six boxes under its label: the digits typed so far, the next box
/// lit with the caret. ONE control (`secure-code`): it takes the focus and the keys, and its
/// name never holds a digit. Each digit is a secure text inside it, drawn and never copied.
/// The caller adds `.on_key(..)` through `control`.
/// Keys: `code-field`, `code-label`, `secure-code`, `code-<i>`, `code-caret`.
pub fn code_boxes(label: &str, digits: &str, control: impl FnOnce(El) -> El, t: &Theme) -> El {
    // Counted and read where they are: no copy of the code is made to draw it.
    let typed = digits.chars().count().min(CODE_LEN);
    let mut boxes = row()
        .key("secure-code")
        .role("textbox")
        .label(format!("{label}, {CODE_LEN} digits, {typed} entered"))
        .gap(6.0)
        .focusable(true)
        .no_focus_ring()
        .autofocus();
    let mut chars = digits.chars();
    for i in 0..CODE_LEN {
        let mut b = el()
            .key(format!("code-{i}"))
            .pixel_snap(false)
            .size(38.0, 44.0)
            .radius(8.0)
            .fill(t.field)
            .center();
        b = if i == typed {
            b.box_shadow(kit::ring(3.0, t.accent.alpha(0.3)))
                .box_shadow(inset(1.0, t.accent_deep))
                .child(
                    // `margin: 0 1px -3px` in a centred grid cell: 18.25 and 15 into the box.
                    el().key("code-caret")
                        .pixel_snap(false)
                        .pin(Some(18.25), Some(15.0), None, None)
                        .size(1.5, 17.0)
                        .radius(1.0)
                        .fill(t.accent_deep),
                )
        } else {
            b.box_shadow(kit::ring_inset(t.separator))
        };
        if let Some(d) = chars.next().filter(|_| i < typed) {
            // Each digit's own secure text: Rune wipes it when the frame lets it go.
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
        .child(outer_label("code-label", label, t))
        .child(control(boxes))
}

/// A note with a glyph (`.kit-banner.tone-neutral`): what the person should know before they
/// act. Keys: `<key>`, `<key>-icon`, `<key>-text`.
pub fn note(key: &str, icon: &'static Icon, words: &str, t: &Theme) -> El {
    row()
        .key(key.to_string())
        .role("note")
        .label(words)
        .w_full()
        .py(11.0)
        .px(13.0)
        .gap(10.0)
        .items_start()
        .radius(13.0)
        .fill(t.tile)
        .child(kit::glyph(format!("{key}-icon"), icon, 16.0, t.text_secondary).mt(1.0))
        .child(
            kit::label(format!("{key}-text"), words, NOTE, t.text_secondary)
                .a11y_hidden()
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
}
