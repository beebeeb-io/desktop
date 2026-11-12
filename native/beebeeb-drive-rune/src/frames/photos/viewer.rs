//! One photo open (DW-0605 to DW-0611): the top bar's two renditions and ⋯, the photo on the
//! dark stage, the strip, and Share · Offline · Details · Trash.

use rune::{Color, El, SurfaceFit, col, el, row};

use super::chrome::{self, Bar, rgb, style};
use super::grid::{length, megabytes, thousands};
use super::source::{Kind, Shape, Viewing};
use super::{Menu, Original, Ui, icons, overlay};
use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::kit;

fn tab(i: usize, words: String, on: bool, t: &Theme) -> El {
    let mut tab = el()
        .key(format!("ph-seg-{i}"))
        .role("tab")
        .label(words.clone())
        .checked(on)
        .h(24.0)
        .px(10.0)
        .radius(8.0)
        .center()
        .focusable(true)
        .child(kit::label(
            format!("ph-seg-{i}-label"),
            words,
            style::SEG,
            if on { t.text } else { t.text_secondary },
        ));
    if on {
        tab = tab
            .fill(t.card)
            .box_shadow(kit::drop(1.0, 2.0, t.shadow_raised))
            .box_shadow(kit::ring(kit::HAIR, t.separator));
    } else {
        tab = tab.hover(t.hover).press(t.press);
    }
    tab
}

pub fn bar(ui: &Ui, viewing: &Viewing, t: &Theme) -> Bar {
    let original = ui.viewer.as_ref().map_or(Original::Preview, |v| v.original);
    let mut middle = row().key("ph-tools").gap(6.0).center_y();
    // A video has one rendition: it streams (DW-0611).
    if viewing.kind == Kind::Photo {
        let whole = megabytes(viewing.original_bytes, true);
        let original_words = match original {
            Original::Downloading { done } => format!("Original \u{00B7} {} of {whole}", megabytes(done, false)),
            _ => format!("Original \u{00B7} {whole}"),
        };
        middle = middle.child(
            row()
                .key("ph-seg")
                .role("tablist")
                .label("Rendition")
                .padding(2.0)
                .radius(10.0)
                .fill(t.field)
                .box_shadow(kit::ring_inset(t.separator))
                .child(
                    tab(
                        0,
                        format!("Preview \u{00B7} {}", viewing.preview_format),
                        original == Original::Preview,
                        t,
                    )
                    .on_press(|s: &mut State| s.photos.show_preview()),
                )
                .child(
                    tab(1, original_words, original != Original::Preview, t)
                        .on_press(|s: &mut State| s.photos.show_original()),
                ),
        );
    }
    middle = middle.child(
        chrome::tbtn("ph-more", "More", &icons::MORE, 17.0, false, t)
            .expanded(ui.menu.is_some_and(|m| m.which == Menu::More))
            .on_press(|s: &mut State| s.photos.toggle_menu(Menu::More)),
    );
    let what = match viewing.kind {
        Kind::Video(seconds) => length(seconds),
        Kind::Photo => viewing.name.clone(),
    };
    Bar {
        title: viewing.taken.clone(),
        sub: if what.is_empty() {
            viewing.device.clone()
        } else {
            format!("{} \u{00B7} {what}", viewing.device)
        },
        back: true,
        details: true,
        middle,
    }
}

/// The status line's words for an open item.
pub fn status(viewing: &Viewing) -> String {
    match viewing.kind {
        Kind::Video(_) => "Streaming \u{00B7} decrypted on this PC as it plays".into(),
        Kind::Photo => format!(
            "Photo {} of {} \u{00B7} \u{2190} \u{2192} for the next one",
            thousands(viewing.position.0),
            thousands(viewing.position.1)
        ),
    }
}

/// The player's bar on a playing video (DW-0611). On Windows these controls are the system
/// player's; this is the board's drawing of them, working: play and pause, the time, the track.
fn player(ui: &Ui, viewing: &Viewing, seconds: u32, t: &Theme) -> El {
    let white = Color::WHITE;
    let playing = ui.viewer.as_ref().is_some_and(|v| v.playing);
    let played = viewing.played_ms as f32 / (seconds.max(1) as f32 * 1000.0);
    let _ = t;
    row()
        .key("ph-player")
        .pin(Some(24.0), None, Some(24.0), Some(20.0))
        .h(46.0)
        .px(16.0)
        .gap(12.0)
        .center_y()
        .radius(12.0)
        .fill(Color::BLACK.alpha(0.45))
        .backdrop_blur(16.0)
        .child(
            kit::glyph(
                "ph-player-icon",
                if playing { &icons::PAUSE } else { &icons::PLAY },
                16.0,
                white,
            )
            .role("button")
            .label(if playing { "Pause" } else { "Play" })
            .shortcut_hint("Space")
            .focusable(true)
            .on_press(|s: &mut State| s.photos.play_pause()),
        )
        .child(kit::label(
            "ph-player-text-0",
            length(viewing.played_ms / 1000),
            style::CAP_MONO,
            white,
        ))
        .child(
            el().key("ph-player-track")
                .role("progressbar")
                .label("Played")
                .range_value(f64::from(played), 0.0, 1.0)
                .grow(1.0)
                .h(4.0)
                .radius(2.0)
                .fill(white.alpha(0.3))
                .child(
                    el().key("ph-player-fill")
                        .w_frac(played.clamp(0.0, 1.0))
                        .h_full()
                        .radius(2.0)
                        .fill(white),
                ),
        )
        .child(kit::label("ph-player-text-1", length(seconds), style::CAP_MONO, white))
}

pub fn stage(ui: &Ui, viewing: &Viewing, t: &Theme) -> El {
    let white = Color::WHITE;
    let Some(viewer) = &ui.viewer else {
        return el().key("ph-stage");
    };
    let mut stage = col()
        .key("ph-stage")
        .fill(if t.dark { rgb(0, 0, 0) } else { rgb(6, 5, 5) });

    if let Original::Downloading { done } = viewer.original {
        let part = done as f32 / viewing.original_bytes.max(1) as f32;
        stage = stage.child(
            el().key("ph-progress")
                .role("progressbar")
                .label("Downloading the original")
                .range_value(f64::from(part), 0.0, 1.0)
                .w_full()
                .h(2.0)
                .fill(white.alpha(0.15))
                .child(
                    el().key("ph-progress-fill")
                        .w_frac(part.clamp(0.0, 1.0))
                        .h_full()
                        .fill(white),
                ),
        );
    }

    // The photo on the stage: the open one, or the next one while a trashed one can come back.
    let shown = ui.source.viewing(viewer.id);
    let shown = shown.as_ref().unwrap_or(viewing);
    let mut photo = el()
        .key("ph-photo")
        .role("image")
        .label(if shown.name.is_empty() {
            shown.taken.clone()
        } else {
            shown.name.clone()
        })
        .size(600.0, 400.0)
        .radius(4.0);
    photo = match ui.source.thumbnail(viewer.id, Shape::Preview) {
        Some(pixels) => photo.pixels_fit(&pixels, SurfaceFit::Fit),
        None => photo.fill(white.alpha(0.07)),
    };
    if let Kind::Video(seconds) = shown.kind {
        photo = photo.child(player(ui, shown, seconds, t));
    }
    stage = stage.child(
        el().key("ph-photo-area")
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .px(40.0)
            .center()
            .child(photo),
    );

    let mut strip = row()
        .key("ph-strip")
        .role("list")
        .label("Photos nearby")
        .w_full()
        .gap(4.0)
        .py(8.0)
        .center();
    for (i, item) in viewing.strip.iter().enumerate() {
        let id = item.id;
        let shape = if item.current { Shape::Square } else { Shape::Strip };
        let mut thumb = el()
            .key(format!("ph-strip-{i}"))
            .role("button")
            .label(format!("Photo {} of {} nearby", i + 1, viewing.strip.len()))
            .size(if item.current { 52.0 } else { 36.0 }, 52.0)
            .radius(3.0)
            .focusable(true)
            .on_press(move |s: &mut State| s.photos.jump(id));
        thumb = match ui.source.thumbnail(id, shape) {
            Some(pixels) => thumb.pixels(&pixels),
            None => thumb.fill(white.alpha(0.07)),
        };
        thumb = if item.current {
            thumb.box_shadow(kit::ring(2.0, white))
        } else {
            thumb.opacity(0.55)
        };
        strip = strip.child(thumb);
    }
    stage = stage.child(strip);

    let kept = ui.source.kept_offline(viewer.id);
    let act = |i: usize, name: &'static str, ic: &'static rune::Icon| {
        col()
            .key(format!("ph-act-{i}"))
            .role("button")
            .label(name)
            .items_center()
            .gap(4.0)
            .focusable(true)
            .child(kit::glyph(format!("ph-act-{i}-icon"), ic, 18.0, white))
            .child(kit::label(format!("ph-act-{i}-label"), name, style::CAP, white))
    };
    stage = stage.child(
        row()
            .key("ph-acts")
            .w_full()
            .gap(32.0)
            .pt(4.0)
            .pb(12.0)
            .center()
            .child(act(0, "Share", &icons::SHARE).on_press(|s: &mut State| s.photos.open_share()))
            .child(
                act(1, if kept { "On this PC" } else { "Offline" }, &icons::OFFLINE)
                    .on_press(|s: &mut State| s.photos.press_offline()),
            )
            .child(act(2, "Details", &icons::INFO).on_press(|s: &mut State| s.photos.open_details()))
            .child(act(3, "Trash", &icons::TRASH).on_press(|s: &mut State| s.photos.trash_open())),
    );

    if let Some(toast) = &ui.toast {
        stage = stage.child(overlay::toast_layer(toast, 118.0, false, t));
    }
    stage
}
