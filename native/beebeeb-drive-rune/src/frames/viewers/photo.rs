//! A photo (DW-1501, DW-1502): the picture on the dark stage, the gallery strip of its folder,
//! and Share, Offline, Details, Trash under it. A RAW shows the preview the camera stored in the
//! file, and the pill in the toolbar says so. With the details panel open (DW-1502) the stage
//! holds the photo alone.

use rune::{col, el, row, Align, Color, El, Icon, SurfaceFit, TextStyle};

use super::chrome::{Dot, Status};
use super::source::{OpenFile, PhotoDoc, Picture};
use super::{details, media, media_icons, Intent, Ui};
use crate::theme::scale;
use crate::theme::tokens::Theme;
use crate::ui::{kit, pill};
use crate::State;

/// `.t-cap`: 11px/500 on 14px.
const CAP: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);

/// The stage's black: `color-mix(in oklch, black 88%, paper)`.
fn stage_black(t: &Theme) -> Color {
    rune::color::hex(if t.dark { "#000000" } else { "#060505" }).unwrap_or(Color::BLACK)
}

/// A plain tile where a picture is not there: white at 7 % on the stage.
fn plain() -> Color {
    Color::WHITE.alpha(0.07)
}

pub fn tools(doc: &PhotoDoc, t: &Theme) -> Vec<El> {
    doc.pill
        .iter()
        .map(|words| pill::pill("tb-pill", words, pill::Tone::Neutral, false, t))
        .collect()
}

pub fn status(ui: &Ui, doc: &PhotoDoc) -> Status {
    Status {
        dot: Dot::Green,
        text: details::DECRYPTED_HERE.into(),
        details: if ui.show_details || doc.note.is_empty() {
            "Space for Preview".into()
        } else {
            doc.note.clone()
        },
    }
}

/// One of the four actions under the strip: the icon over its word. Keys: `action-<i>`,
/// `action-<i>-icon`, `action-<i>-label` (the word's box is the whole action, as the board's is).
fn action(i: usize, name: &'static str, icon: &'static Icon) -> El {
    let width = rune::text::advance(name, CAP).max(18.0);
    el().key(format!("action-{i}"))
        .role("button")
        .label(name)
        .size(width, 36.0)
        .radius(crate::theme::tokens::radius::SM)
        .focusable(true)
        .focus_ring(2.0, 2.0, Color::WHITE)
        .cursor(rune::Cursor::Pointer)
        .child(
            kit::label(format!("action-{i}-label"), name, CAP, Color::WHITE)
                .size(width, 36.0)
                .pt(22.0)
                .align(Align::Center),
        )
        .child(kit::glyph(format!("action-{i}-icon"), icon, 18.0, Color::WHITE).absolute((width - 18.0) / 2.0, 0.0))
}

/// The size in points a picture of `pixels` is drawn at in an `area` of points (contain, aspect kept): the largest
/// box at the picture's own ratio the area holds, so a large photo fills the width or the height and a small one is
/// scaled up to the area, never past it. Rounded down to whole device pixels of `scale` (the window's real display
/// scale factor). The layout does the same live with `aspect_fit`, so a resize or the details panel recomputes it.
#[allow(dead_code)] // no caller yet in every build; kept with its tests
pub(crate) fn fit_size(pixels: (u32, u32), area: (f32, f32), scale: f32) -> (f32, f32) {
    let (pw, ph) = (pixels.0.max(1) as f32, pixels.1.max(1) as f32);
    let k = (area.0 / pw).min(area.1 / ph).max(0.0);
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let snap = |v: f32| (v * scale).floor() / scale;
    (snap(pw * k), snap(ph * k))
}

/// The photo, drawn in whatever room its area has: it never has a fixed size of its own.
fn picture(ui: &Ui, file: &OpenFile) -> El {
    let photo = el().key("photo").role("image").label(file.name.clone()).radius(crate::theme::tokens::radius::SM);
    match ui.source.picture(&file.id, Picture::Photo) {
        Some(pixels) => {
            let (w, h) = pixels.size();
            photo.aspect_fit(w.max(1) as f32 / h.max(1) as f32).pixels_fit(&pixels, SurfaceFit::Fill)
        }
        None => photo.size(600.0, 400.0).fill(plain()),
    }
}

pub fn body(ui: &Ui, doc: &PhotoDoc, t: &Theme) -> El {
    let stage = col()
        .key("viewer-body")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(stage_black(t));
    if ui.show_details {
        // DW-1502: beside the panel the photo stands alone, in the room the panel leaves.
        return stage.center().child(el().key("photo-area").grow(1.0).min_h(0.0).w_full().padding(scale::space::S20).center().child(picture(ui, &ui.file)));
    }
    let mut strip = row()
        .key("gallery")
        .role("list")
        .label("Photos of this folder")
        .focus_group()
        .w_full()
        .gap(scale::space::S4)
        .py(scale::space::S8)
        .center();
    for (i, item) in doc.strip.iter().enumerate() {
        let id = item.id.clone();
        let mut thumb = el()
            .key(format!("gallery-{i}"))
            .role("button")
            .label(item.name.clone())
            .checked(item.current)
            .size(if item.current { 52.0 } else { 36.0 }, 52.0)
            .radius(crate::theme::tokens::radius::SM)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut State| {
                media::switch_to(s, &id);
            });
        thumb = match ui.source.picture(&ui.file.id, Picture::Strip(i)) {
            Some(pixels) => thumb.pixels(&pixels),
            None => thumb.fill(plain()),
        };
        strip = strip.child(if item.current {
            thumb.box_shadow(kit::ring(2.0, Color::WHITE))
        } else {
            thumb.opacity(0.55)
        });
    }
    let offline = if ui.file.details.offline {
        concat!("On ", beebeeb_engine::device_word!())
    } else {
        "Offline"
    };
    let actions = row()
        .key("actions")
        .w_full()
        .gap(32.0)
        .pt(4.0)
        .pb(12.0)
        .center()
        .child(action(0, "Share", &media_icons::SHARE).on_press(|s: &mut State| {
            if let Some(ui) = super::ui(s) {
                ui.intent(Intent::Share);
            }
        }))
        // Only where the source read whether the file is kept on this PC: a state that was not read is not shown.
        .children(ui.knows_offline().then(|| {
            action(1, offline, &media_icons::OFFLINE).on_press(|s: &mut State| {
                if let Some(ui) = super::ui(s) {
                    ui.toggle_offline();
                }
            })
        }))
        .child(action(2, "Details", &super::icons::INFO).on_press(|s: &mut State| {
            if let Some(ui) = super::ui(s) {
                ui.show_details = true;
            }
        }))
        .child(action(3, "Trash", &media_icons::TRASH).on_press(|s: &mut State| {
            if let Some(ui) = super::ui(s) {
                ui.intent(Intent::Trash);
            }
        }));
    stage
        .child(
            el().key("photo-area")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .padding(scale::space::S20)
                .center()
                .child(picture(ui, &ui.file)),
        )
        .child(strip)
        .child(actions)
}

#[cfg(test)]
mod fit_tests {
    use super::fit_size;

    #[test]
    fn six_thousand_by_four_thousand_fills_the_width_at_scale_two() {
        let (w, h) = fit_size((6000, 4000), (1900.0, 1350.0), 2.0);
        assert!((w - 1900.0).abs() < 0.51 && h <= 1350.0 && (w / h - 1.5).abs() < 0.01, "{w}x{h}");
    }

    #[test]
    fn a_portrait_photo_fills_the_height() {
        let (w, h) = fit_size((4000, 6000), (1900.0, 1350.0), 2.0);
        assert!((h - 1350.0).abs() < 0.51 && w < 1900.0, "{w}x{h}");
    }

    #[test]
    fn a_resize_recomputes_the_fit() {
        let big = fit_size((6000, 4000), (1900.0, 1350.0), 2.0);
        let small = fit_size((6000, 4000), (900.0, 700.0), 2.0);
        assert!(small.0 < big.0 && (small.0 - 900.0).abs() < 0.51);
    }

    #[test]
    fn a_small_image_is_not_blown_past_the_area() {
        let (w, h) = fit_size((300, 200), (1900.0, 1350.0), 2.0);
        assert!(w <= 1900.0 && h <= 1350.0, "{w}x{h}");
    }

    #[test]
    fn it_snaps_to_whole_device_pixels() {
        let (w, _) = fit_size((6000, 4000), (1001.0, 1350.0), 2.0);
        assert_eq!(w * 2.0, (w * 2.0).round());
    }
}
