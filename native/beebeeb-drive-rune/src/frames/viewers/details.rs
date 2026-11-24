//! The details panel of an open file (`.mk-det`, Alt+Shift+P): the name, kind and size, the
//! encryption mark, where it was decrypted, the facts, Available offline, Share and Version
//! history. Built from `ui::detail_pane`'s parts; the facts are the source's.

use rune::{col, row, El, Span};

use super::source::OpenFile;
use super::{ui, Intent};
use crate::theme::icons;
use crate::theme::scale;
use crate::theme::tokens::{radius, type_, Theme};
use crate::ui::{detail_pane, kit};
use crate::State;

/// The line every open file's panel carries: the content on screen was decrypted here.
pub const DECRYPTED_HERE: &str = concat!(
    "Decrypted on ",
    beebeeb_engine::device_word!(),
    " \u{00B7} the key never left it"
);

fn intent(intent: Intent) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        if let Some(ui) = ui(s) {
            ui.intent(intent.clone());
        }
    }
}

/// The words of decision D-MED-7: what the player decodes is below full quality on this PC.
pub const REDUCED_QUALITY: &str = concat!("Reduced quality on ", beebeeb_engine::device_word!());

/// Whether the file's player (if it has one) decodes below full quality on this PC: `PlayState::reduced_quality`.
pub fn is_reduced(state: Option<&crate::media::player::PlayState>) -> bool {
    state.is_some_and(|s| s.reduced_quality)
}

/// The details of a file that is not played in the shell's player (the Office viewers): [`panel_with`] without the
/// reduced-quality note.
pub fn panel(file: &OpenFile, offline_known: bool, t: &Theme) -> El {
    panel_with(file, offline_known, false, t)
}

/// The file's details. `offline_known`: the source read whether the file is kept on this PC
/// (`ViewerSource::knows_offline`); when it did not, the panel draws no Available offline switch: a state that was
/// not read is not shown. `reduced_quality` (see [`is_reduced`]): the panel then carries "Reduced quality on this PC"
/// under the decrypted-here note.
pub fn panel_with(file: &OpenFile, offline_known: bool, reduced_quality: bool, t: &Theme) -> El {
    let d = &file.details;
    let kind_line = if d.size.is_empty() {
        file.facts.clone()
    } else {
        format!("{} \u{00B7} {}", d.kind, d.size)
    };
    let mut kind = kit::label("detail-kind", kind_line.clone(), type_::CAPTION, t.text_tertiary).mt(2.0);
    if !d.size.is_empty() {
        // The size is a figure: mono, as every size is.
        kind = kind.spans(vec![Span::new(kind_line.len() - d.size.len(), kind_line.len()).mono()]);
    }
    let mut meta = col().key("detail-meta").role("list").gap(scale::space::S6);
    for (label, value, mono) in &d.facts {
        meta = meta.child(fact_row(fact_key(label), label, value, *mono, t));
    }
    let state = if d.offline {
        "Available offline"
    } else {
        "Online only now"
    };
    // A file that is not kept on this PC takes no room on it: no size after "Online only now".
    let sub = if d.size.is_empty() || !d.offline {
        state.to_string()
    } else {
        format!("{state} \u{00B7} {}", d.size)
    };
    let mut pane = detail_pane::pane(t)
        .child(
            col()
                .key("detail-name-group")
                .child(kit::label("detail-name", file.name.clone(), type_::HEAD, t.text).truncate())
                .child(kind),
        )
        .child(detail_pane::encrypted_badge(t))
        .child(kit::label(
            "detail-note",
            DECRYPTED_HERE,
            type_::CAPTION,
            t.text_tertiary,
        ));
    if reduced_quality {
        pane = pane.child(kit::label(
            "detail-reduced",
            REDUCED_QUALITY,
            type_::CAPTION,
            t.text_tertiary,
        ));
    }
    if !d.facts.is_empty() {
        pane = pane.child(meta);
    }
    if offline_known {
        pane = pane.child(
            switch_card("offline", "Available offline", &sub, d.offline, t).on_press(|s: &mut State| {
                if let Some(ui) = ui(s) {
                    ui.toggle_offline();
                }
            }),
        );
    }
    pane.child(
        row()
            .key("detail-actions")
            .gap(scale::space::S8)
            .child(
                detail_pane::action_button("share", "Share\u{2026}", &icons::SHARE, t).on_press(intent(Intent::Share)),
            )
            .child(
                detail_pane::action_button("versions", "Version history", &icons::VERSION_HISTORY, t)
                    .on_press(intent(Intent::VersionHistory)),
            ),
    )
}

/// The width of a fact's value: the panel's 251.5 less the label (92) and the gap (8).
const VALUE_W: f32 = 151.5;

/// One fact. A value wider than its column wraps onto a second line (the board's Exposure and
/// Preview of a RAW), where `ui::detail_pane::meta_row` would cut it short.
fn fact_row(key: &'static str, label: &'static str, value: &str, mono: bool, t: &Theme) -> El {
    let text_style = if mono { type_::META_MONO } else { type_::BODY };
    if rune::text::advance(value, text_style) <= VALUE_W {
        return detail_pane::meta_row(key, label, value, mono, t);
    }
    row()
        .key(format!("meta-row-{key}"))
        .role("listitem")
        .label(format!("{label}: {value}"))
        .w_full()
        .gap(scale::space::S8)
        .child(kit::label(key, label, type_::BODY, t.text_tertiary).w(92.0))
        .child(
            kit::label(format!("{key}-value"), value, text_style, t.text)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
}

/// The gate key of a fact, from the board's label ("Changed on" is `detail-changed-on`).
fn fact_key(label: &str) -> &'static str {
    match label {
        "Changed" => "detail-changed",
        "Changed on" => "detail-changed-on",
        "Added" => "detail-added",
        "Where" => "detail-where",
        "Version history" => "detail-version-history",
        "Encryption" => "detail-encryption",
        "Key" => "detail-key",
        "Shared" => "detail-shared",
        "Taken" => "detail-taken",
        "From" => "detail-from",
        "Camera" => "detail-camera",
        "Lens" => "detail-lens",
        "Exposure" => "detail-exposure",
        "Focal length" => "detail-focal-length",
        "Size" => "detail-size",
        "Preview" => "detail-preview",
        _ => "detail-fact",
    }
}

/// The card of a switch: title and sub at the left, the shared `ui::switch` at the right. The card is the one
/// control (role `switch`, one focus stop); the drawn switch inside it is its picture. Keys as the shared card's.
fn switch_card(key: &str, title: &str, sub: &str, on: bool, t: &Theme) -> El {
    let switch = crate::ui::switch::switch(&format!("{key}-switch"), "", on, true, t)
        .role("presentation")
        .focusable(false);
    row()
        .key(format!("{key}-card"))
        .role("switch")
        .label(title)
        .checked(on)
        .w_full()
        .min_h(40.0)
        .px(scale::space::S10)
        .gap(scale::space::S8)
        .center_y()
        .radius(radius::LG)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer)
        .child(
            col()
                .key(format!("{key}-text"))
                .grow(1.0)
                .min_w(0.0)
                .gap(scale::space::S2)
                .child(kit::label(format!("{key}-label"), title, type_::ITEM_TITLE, t.text).truncate())
                .child(kit::label(format!("{key}-detail"), sub, type_::BODY.lh(15.0), t.text_tertiary).truncate()),
        )
        .child(switch)
}
