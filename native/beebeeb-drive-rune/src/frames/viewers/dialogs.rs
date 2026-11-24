//! What sits over an open file: the two dialogs of editing (DW-1525 unsaved changes, DW-1526
//! two versions) on their scrim, and the message after a save (DW-1524).
//!
//! The dialogs are the board's `.wn-dlg` with this group's keys (`dlg`, `dlg-head`, `dlg-foot`,
//! `dlg-btn-<n>`); `ui::settings_dialog` draws the same shell with the settings view's keys.

use rune::{Color, El, col, el, row};

use super::source::{Conflict, Keep};
use super::{icons, style, ui};
use crate::State;
use crate::theme::scale;
use crate::theme::tokens::{Theme, type_};
use crate::ui::button::{ButtonState, Size, Variant, button as ui_button, keys};
use crate::ui::dialog::{self, Dialog, Foot, Head};
use crate::ui::toast::{Lead, toast_with};
use crate::ui::settings_dialog::{BUTTON_MD, DIALOG_TITLE};
use crate::ui::kit;

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

impl Look {
    /// The component's variant for this look.
    pub(super) fn variant(self) -> Variant {
        match self {
            Look::Primary => Variant::Primary,
            Look::Outline => Variant::Secondary,
            Look::Quiet => Variant::Ghost,
            Look::Danger => Variant::Danger,
        }
    }
}

fn button(index: usize, label: &str, look: Look, t: &Theme) -> El {
    // On the dialog's quarter point, as the dialog is.
    ui_button(keys(&format!("dlg-btn-{index}")), label, None, look.variant(), Size::Md, ButtonState::Enabled, t)
        .pixel_snap(false)
}

/// The card on the scrim: Esc and a press outside are the way back to the edit. Both editing questions are asked
/// from the editor (leaving with changes, a save that met another version); an answer may take the editor away
/// (Discard, Keep theirs), so focus goes back to the editor when it is still there, else into the toolbar
/// (`ui::dialog::modal_back`).
fn shell(name: &str, role: &'static str, width: f32, head: El, body: Vec<El>, foot: El, t: &Theme) -> El {
    let keep = |s: &mut State| {
        if let Some(ui) = ui(s) {
            ui.keep_editing();
        }
    };
    dialog::modal_back(
        0.0,
        card(name, role, width, head, body, foot, t),
        super::EDITOR_KEY,
        super::chrome::PANE_TOOLBAR,
        keep,
        t,
    )
}

/// [`shell`] with the caller's way back (Esc, a press outside). `foot`: the buttons row.
#[allow(clippy::too_many_arguments)]
pub(super) fn shell_closing(
    name: &str,
    role: &'static str,
    width: f32,
    head: El,
    body: Vec<El>,
    foot: El,
    close: impl Fn(&mut State) + 'static,
    t: &Theme,
) -> El {
    dialog::modal(0.0, card(name, role, width, head, body, foot, t), close, t)
}

/// The dialog's card, without its scrim.
fn card(name: &str, role: &'static str, width: f32, head: El, body: Vec<El>, foot: El, t: &Theme) -> El {
    dialog::dialog(
        Dialog {
            alert: role == "alertdialog",
            head: Head::Raw(head),
            body,
            foot: Foot::Custom(el().child(foot)),
            ..Dialog::new("dlg", name, width)
        },
        t,
    )
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
        .px(scale::space::S24)
        .pb(16.0)
        .gap(scale::space::S14)
        .items_start()
        .child(
            el().key("dlg-tile")
                .size(36.0, 36.0)
                .radius(scale::radius::PANEL)
                .fill(style::tile_neutral(t))
                .center()
                .child(kit::glyph("dlg-icon", &icons::EDIT, 18.0, t.text_secondary)),
        )
        .child(
            col()
                .key("dlg-words")
                .grow(1.0)
                .min_w(0.0)
                .gap(scale::space::S6)
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
        .gap(scale::space::S8)
        .child(
            button(0, labels[0].0, labels[0].1, t)
                .w(widths[0])
                .default_button() // Enter saves; Discard never takes Enter (ux-53)
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
        Vec::new(),
        buttons,
        t,
    )
}

fn side(index: usize, icon: &'static rune::Icon, title: &str, sub: &str, t: &Theme) -> El {
    let key = format!("dlg-card-{index}");
    col()
        .key(key.clone())
        // Two of them share the dialog's 492 with 12 between.
        .w(240.0)
        .padding(scale::space::S12)
        .gap(scale::space::S8)
        .radius(scale::radius::PANEL)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .box_shadow(kit::drop(1.0, 2.0, Color::BLACK.alpha(0.04)))
        .child(
            row()
                .key(format!("{key}-head"))
                .center_y()
                .gap(scale::space::S8)
                .child(kit::glyph(format!("{key}-icon"), icon, 15.0, t.text))
                .child(kit::label(format!("{key}-title"), title, type_::HEAD, t.text)),
        )
        .child(kit::label(format!("{key}-sub"), sub, style::CAP_MONO, t.text_tertiary).w_full())
}

/// DW-1526: someone saved the same file elsewhere while it was being edited. While nobody can say where the other
/// version was saved (`Conflict::from` empty: the engine does not know which device, or whether it was this one),
/// the words stay neutral: changed since it was opened, "the other version" (delta check of desktop#432, round 3).
pub fn conflict(file_name: &str, mine: &str, other: &Conflict, t: &Theme) -> El {
    let known = !other.from.trim().is_empty();
    let sub = if known {
        format!("{file_name} \u{00B7} changed on {} while you were editing", other.from)
    } else {
        format!("{file_name} \u{00B7} changed since you opened it")
    };
    let (other_icon, other_title, keep_other) = if known {
        (&icons::PHONE, other.from.clone(), format!("Keep {}'s", other.from))
    } else {
        (
            &icons::REFRESH,
            "The other version".to_string(),
            "Keep the other version".to_string(),
        )
    };
    let head = col()
        .key("dlg-head")
        .w_full()
        .pt(20.0)
        .px(scale::space::S24)
        .pb(4.0)
        .gap(scale::space::S4)
        .child(
            kit::label("dlg-title", "Two versions", DIALOG_TITLE, t.text)
                .role("heading")
                .level(2)
                .w_full(),
        )
        .child(kit::label("dlg-sub", sub, style::SUB, t.text_tertiary).wrap().w_full());
    let body = vec![
        row()
                .key("dlg-sides")
                .w_full()
                .gap(scale::space::S12)
                .child(side(
                    0,
                    &icons::LAPTOP,
                    "Your edit",
                    &format!("this PC \u{00B7} not saved \u{00B7} {mine}"),
                    t,
                ))
                .child(side(
                    1,
                    other_icon,
                    &other_title,
                    &format!("{} \u{00B7} version {}", other.at, other.version),
                    t,
                )),
        kit::label(
            "dlg-note",
            "Either way, the other version stays in version history.",
            type_::CAPTION,
            t.text_tertiary,
        ),
    ];
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
        .gap(scale::space::S8)
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
            button(2, &keep_other, Look::Outline, t)
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
        body,
        buttons,
        t,
    )
}

/// The message after a save, over the content area with the lock: the shared toast.
pub fn toast(words: &str, t: &Theme) -> El {
    crate::ui::toast::layer(toast_with("toast", words, Lead::Lock, None, t))
}
