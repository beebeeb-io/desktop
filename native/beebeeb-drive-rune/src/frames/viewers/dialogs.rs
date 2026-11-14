//! What sits over an open file: the two dialogs of editing (DW-1525 unsaved changes, DW-1526
//! two versions) on their scrim, and the message after a save (DW-1524).
//!
//! The dialogs are the board's `.wn-dlg` with this group's keys (`dlg`, `dlg-head`, `dlg-foot`,
//! `dlg-btn-<n>`); `ui::settings_dialog` draws the same shell with the settings view's keys.

use rune::{Color, El, col, el, row};

use super::source::{Conflict, Keep};
use super::{icons, style, ui};
use crate::State;
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::settings_dialog::{BUTTON_MD, DIALOG_TITLE, scrim_color};
use crate::ui::{au, kit};

/// How a dialog button reads.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Look {
    /// The one Amber action.
    Primary,
    /// Paper with a line.
    Outline,
    /// Words only.
    Quiet,
    /// Words only, in red: it throws something away.
    Danger,
}

fn button(index: usize, label: &str, look: Look, t: &Theme) -> El {
    button_keyed(format!("dlg-btn-{index}"), label, look, t)
}

/// A dialog button under a key of the caller's (`<key>`, `<key>-label`).
pub(super) fn button_keyed(key: String, label: &str, look: Look, t: &Theme) -> El {
    let ink = match look {
        Look::Primary => t.accent_text_on,
        Look::Outline => t.text,
        Look::Quiet => t.text_secondary,
        Look::Danger => au::red_text(t),
    };
    let b = row()
        .key(key.clone())
        .role("button")
        .label(label)
        .h(28.0)
        .px(12.0)
        .center()
        .radius(radius::MD)
        // On the dialog's quarter point, as the dialog is (see `shell`).
        .pixel_snap(false)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer);
    let b = match look {
        Look::Primary => b
            .fill(t.accent)
            .hover(t.accent.mix(Color::BLACK, 0.05))
            .press(t.accent.mix(Color::BLACK, 0.10)),
        Look::Outline => b
            .fill(t.paper)
            .border(1.0, t.card_border)
            .border_layout()
            .hover(t.paper2)
            .press(t.paper3),
        Look::Quiet | Look::Danger => b.fill(Color::TRANSPARENT).hover(t.paper2).press(t.paper3),
    };
    b.child(style::fitted(format!("{key}-label"), label, BUTTON_MD, ink))
}

pub(super) fn foot(t: &Theme) -> El {
    el().key("dlg-foot")
        .w_full()
        .gap(8.0)
        .py(16.0)
        .px(24.0)
        .fill(t.surface)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
}

/// The card on the scrim. The shadow and the hairline ring are outside the card; the clip
/// that rounds the foot's corners would cut them, so it sits one level in.
fn shell(name: &str, role: &'static str, width: f32, head: El, body: Option<El>, foot: El, t: &Theme) -> El {
    // Esc and a press outside are the way back to the edit.
    let keep = |s: &mut State| {
        if let Some(ui) = ui(s) {
            ui.keep_editing();
        }
    };
    shell_closing(name, role, width, head, body, foot, keep, t)
}

/// [`shell`] with the caller's way back (Esc, a press outside).
#[allow(clippy::too_many_arguments)]
pub(super) fn shell_closing(
    name: &str,
    role: &'static str,
    width: f32,
    head: El,
    body: Option<El>,
    foot: El,
    close: impl Fn(&mut State) + 'static,
    t: &Theme,
) -> El {
    let mut inner = col().key("dlg-clip").w_full().clip_rounded(8.0).child(head);
    if let Some(body) = body {
        inner = inner.child(body);
    }
    let dlg = col()
        .key("dlg")
        .role(role)
        .label(name)
        // The foot's hairline makes the height end in a half point, so centring puts the top on
        // a quarter point. The board draws it there; so does this.
        .pixel_snap(false)
        .w(width)
        .radius(8.0)
        .fill(t.card)
        .box_shadow(kit::drop(24.0, 60.0, Color::BLACK.alpha(0.28)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(inner.child(foot));
    col()
        .key("scrim")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .fill(scrim_color(t))
        .child(dlg)
        .modal(false, close)
}

/// The widths of buttons that share a row evenly but never get narrower than their words
/// (`.wn-dlg-foot.is-even`: `flex: 1 1 0`).
fn even(natural: &[f32], room: f32) -> Vec<f32> {
    let mut width = vec![0.0; natural.len()];
    let mut fixed = vec![false; natural.len()];
    loop {
        let free = room
            - natural
                .iter()
                .zip(&fixed)
                .filter(|(_, f)| **f)
                .map(|(w, _)| w)
                .sum::<f32>();
        let open = fixed.iter().filter(|f| !**f).count();
        if open == 0 {
            break;
        }
        let share = free / open as f32;
        let mut again = false;
        for i in 0..natural.len() {
            if !fixed[i] && natural[i] > share {
                fixed[i] = true;
                again = true;
            }
        }
        if !again {
            for i in 0..natural.len() {
                width[i] = if fixed[i] { natural[i] } else { share };
            }
            return width;
        }
    }
    natural.to_vec()
}

/// DW-1525: leaving with changes asks first.
pub fn unsaved(file_name: &str, t: &Theme) -> El {
    let words =
        format!("Your edits to {file_name} aren't saved. Save them as a new, encrypted version, or discard them.");
    let head = row()
        .key("dlg-head")
        .w_full()
        .pt(20.0)
        .px(24.0)
        .pb(16.0)
        .gap(14.0)
        .items_start()
        .child(
            el().key("dlg-tile")
                .size(36.0, 36.0)
                .radius(10.0)
                .fill(style::tile_neutral(t))
                .center()
                .child(kit::glyph("dlg-icon", &icons::EDIT, 18.0, t.text_secondary)),
        )
        .child(
            col()
                .key("dlg-words")
                .grow(1.0)
                .min_w(0.0)
                .gap(6.0)
                .child(
                    kit::label("dlg-title", "You have unsaved changes", DIALOG_TITLE, t.text)
                        .role("heading")
                        .level(2)
                        .w_full(),
                )
                .child(
                    kit::label("dlg-sub", words, style::SUB, t.text_tertiary)
                        .wrap()
                        .w_full(),
                ),
        );
    let labels = [
        ("Save", Look::Primary),
        ("Discard changes", Look::Danger),
        ("Keep editing", Look::Quiet),
    ];
    let natural: Vec<f32> = labels
        .iter()
        .map(|(label, _)| rune::text::advance(label, BUTTON_MD) + 24.0)
        .collect();
    let widths = even(&natural, 380.0 - 48.0 - 16.0);
    let buttons = row()
        .key("dlg-buttons")
        .w_full()
        .gap(8.0)
        .child(
            button(0, labels[0].0, labels[0].1, t)
                .w(widths[0])
                .on_press(|s: &mut State| {
                    if let Some(ui) = ui(s) {
                        ui.save();
                    }
                }),
        )
        .child(
            button(1, labels[1].0, labels[1].1, t)
                .w(widths[1])
                .on_press(|s: &mut State| {
                    if let Some(ui) = ui(s) {
                        ui.discard();
                    }
                }),
        )
        .child(
            button(2, labels[2].0, labels[2].1, t)
                .w(widths[2])
                .on_press(|s: &mut State| {
                    if let Some(ui) = ui(s) {
                        ui.keep_editing();
                    }
                }),
        );
    shell(
        "You have unsaved changes",
        "alertdialog",
        380.0,
        head,
        None,
        foot(t).child(buttons),
        t,
    )
}

fn side(index: usize, icon: &'static rune::Icon, title: &str, sub: &str, t: &Theme) -> El {
    let key = format!("dlg-card-{index}");
    col()
        .key(key.clone())
        // Two of them share the dialog's 492 with 12 between.
        .w(240.0)
        .padding(12.0)
        .gap(8.0)
        .radius(10.0)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .box_shadow(kit::drop(1.0, 2.0, Color::BLACK.alpha(0.04)))
        .child(
            row()
                .key(format!("{key}-head"))
                .center_y()
                .gap(8.0)
                .child(kit::glyph(format!("{key}-icon"), icon, 15.0, t.text))
                .child(kit::label(format!("{key}-title"), title, type_::HEAD, t.text)),
        )
        .child(kit::label(format!("{key}-sub"), sub, style::CAP_MONO, t.text_tertiary).w_full())
}

/// DW-1526: someone saved the same file elsewhere while it was being edited.
pub fn conflict(file_name: &str, mine: &str, other: &Conflict, t: &Theme) -> El {
    let head = col()
        .key("dlg-head")
        .w_full()
        .pt(20.0)
        .px(24.0)
        .pb(4.0)
        .gap(3.0)
        .child(
            kit::label("dlg-title", "Two versions", DIALOG_TITLE, t.text)
                .role("heading")
                .level(2)
                .w_full(),
        )
        .child(
            kit::label(
                "dlg-sub",
                format!("{file_name} \u{00B7} changed on {} while you were editing", other.from),
                style::SUB,
                t.text_tertiary,
            )
            .wrap()
            .w_full(),
        );
    let body = col()
        .key("dlg-body")
        .w_full()
        .pt(10.0)
        .px(24.0)
        .pb(18.0)
        .gap(12.0)
        .child(
            row()
                .key("dlg-sides")
                .w_full()
                .gap(12.0)
                .child(side(
                    0,
                    &icons::LAPTOP,
                    "Your edit",
                    &format!("this PC \u{00B7} not saved \u{00B7} {mine}"),
                    t,
                ))
                .child(side(
                    1,
                    &icons::PHONE,
                    &other.from,
                    &format!("{} \u{00B7} version {}", other.at, other.version),
                    t,
                )),
        )
        .child(kit::label(
            "dlg-note",
            "Either way, the other version stays in version history.",
            type_::CAPTION,
            t.text_tertiary,
        ));
    let keep = |keep: Keep| {
        move |s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.resolve(keep);
            }
        }
    };
    // The default is first and apart from the rest (`.is-stack`: the gap is doubled under it).
    let buttons = col()
        .key("dlg-buttons")
        .w_full()
        .gap(8.0)
        .child(
            button(0, "Keep both", Look::Primary, t)
                .w_full()
                .mb(8.0)
                .on_press(keep(Keep::Both)),
        )
        .child(
            button(1, "Keep this PC's", Look::Outline, t)
                .w_full()
                .on_press(keep(Keep::Mine)),
        )
        .child(
            button(2, &format!("Keep {}'s", other.from), Look::Outline, t)
                .w_full()
                .on_press(keep(Keep::Theirs)),
        )
        .child(button(3, "Cancel", Look::Quiet, t).w_full().on_press(|s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.keep_editing();
            }
        }));
    shell(
        "Two versions",
        "dialog",
        540.0,
        head,
        Some(body),
        foot(t).child(buttons),
        t,
    )
}

/// The message after a save (`.mk-toast > .kit-toast`): a pill, centred over the content
/// area, 42 above the window's foot. `ui::settings_notice::toast` is the same pill with a
/// green check; a save shows the lock.
pub fn toast(words: &str, t: &Theme) -> El {
    let glass = if t.dark {
        t.paper2.alpha(0.68)
    } else {
        t.paper.alpha(0.72)
    };
    row()
        .key("toast-line")
        .pin(Some(220.0), None, Some(0.0), Some(42.0))
        .justify_center()
        .child(
            row()
                .key("toast")
                .role("status")
                .label(words)
                .h(34.0)
                .pl(14.0)
                .pr(8.0)
                .gap(10.0)
                .center_y()
                .radius(17.0)
                .fill(glass)
                .border(kit::HAIR, t.separator)
                .border_layout()
                .box_shadow(kit::drop(6.0, 22.0, Color::BLACK.alpha(0.10)))
                .box_shadow(rune::Shadow {
                    x: 0.0,
                    y: kit::HAIR,
                    blur: 0.0,
                    spread: 0.0,
                    color: Color::WHITE.alpha(if t.dark { 0.08 } else { 0.70 }),
                    inset: true,
                })
                .child(kit::glyph("toast-icon", &icons::LOCK, 15.0, t.text_secondary))
                .child(kit::label("toast-text", words, style::TOAST, t.text)),
        )
}
