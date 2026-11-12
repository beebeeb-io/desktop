//! The details panel of an open file (`.mk-det`, Alt+Shift+P): the name, kind and size, the
//! encryption mark, where it was decrypted, the facts, Available offline, Share and Version
//! history. Built from `ui::detail_pane`'s parts; the facts are the source's.

use rune::{El, Span, col, el, row};

use super::source::OpenFile;
use super::{Intent, style, ui};
use crate::State;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::{detail_pane, kit};

/// The line every open file's panel carries: the content on screen was decrypted here.
pub const DECRYPTED_HERE: &str = "Decrypted on this PC \u{00B7} the key never left it";

fn intent(intent: Intent) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        if let Some(ui) = ui(s) {
            ui.intent(intent.clone());
        }
    }
}

pub fn panel(file: &OpenFile, t: &Theme) -> El {
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
    let mut meta = col().key("detail-meta").role("list").gap(6.0);
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
    if !d.facts.is_empty() {
        pane = pane.child(meta);
    }
    pane.child(
        switch_card("offline", "Available offline", &sub, d.offline, t).on_press(|s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.toggle_offline();
            }
        }),
    )
    .child(
        row()
            .key("detail-actions")
            .gap(8.0)
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
        .gap(8.0)
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

/// `ui::detail_pane::switch_card` with the board's ON knob: the knob of a switch that is on is
/// the paper's colour (light on ink, dark on light ink), where the shared switch keeps white (a
/// request to the design-system lane in the PR body). Keys as the shared card's.
fn switch_card(key: &str, title: &str, sub: &str, on: bool, t: &Theme) -> El {
    let switch = row()
        .key(format!("{key}-switch"))
        .size(36.0, 20.0)
        .radius(10.0)
        .fill(if on { t.text } else { t.paper3 })
        .border(1.0, if on { t.text } else { t.separator_strong })
        .border_layout()
        .center_y()
        .child(
            el().key(format!("{key}-knob"))
                .size(14.0, 14.0)
                .radius(7.0)
                .fill(if on { t.paper } else { t.white })
                // `shadow-1`: the ink at 4 % and 6 %, the same in both themes.
                .box_shadow(kit::drop(1.0, 3.0, style::knob_shadow(0.04)))
                .box_shadow(kit::drop(1.0, 2.0, style::knob_shadow(0.06)))
                .ml(if on { 18.0 } else { 2.0 }),
        );
    row()
        .key(format!("{key}-card"))
        .role("switch")
        .label(title)
        .checked(on)
        .w_full()
        .min_h(40.0)
        .px(10.0)
        .gap(8.0)
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
                .gap(1.0)
                .child(kit::label(format!("{key}-label"), title, type_::ITEM_TITLE, t.text).truncate())
                .child(kit::label(format!("{key}-detail"), sub, type_::BODY.lh(15.0), t.text_tertiary).truncate()),
        )
        .child(switch)
}
