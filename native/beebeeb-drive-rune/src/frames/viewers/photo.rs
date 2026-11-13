//! A photo (DW-1501, DW-1502): the picture on the dark stage, the gallery strip of its folder,
//! and Share, Offline, Details, Trash under it. A RAW shows the preview the camera stored in the
//! file, and the pill in the toolbar says so. With the details panel open (DW-1502) the stage
//! holds the photo alone.

use rune::{Align, Color, El, Icon, SurfaceFit, TextStyle, col, el, row};

use super::chrome::{Dot, Status};
use super::source::{OpenFile, PhotoDoc, Picture};
use super::{Intent, Ui, details, media, media_icons, style};
use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::kit;

/// `.t-cap`: 11px/500 on 14px.
const CAP: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);
/// `.kit-pill`: 11px/600 on 17px, tracked 0.01em.
const PILL: TextStyle = TextStyle::new(11.0, 600.0).lh(17.0).tracking(0.11);

/// The stage's black: `color-mix(in oklch, black 88%, paper)`.
fn stage_black(t: &Theme) -> Color {
    rune::color::hex(if t.dark { "#000000" } else { "#060505" }).unwrap_or(Color::BLACK)
}

/// A plain tile where a picture is not there: white at 7 % on the stage.
fn plain() -> Color {
    Color::WHITE.alpha(0.07)
}

/// The pill of the toolbar (`.kit-pill`). Keys: `tb-pill`, `tb-pill-label`.
fn pill(words: &str, t: &Theme) -> El {
    let width = rune::text::advance(words, PILL) + 16.0;
    el().key("tb-pill")
        .role("note")
        .label(words)
        .size(width, 20.0)
        .radius(10.0)
        .fill(style::tile_neutral(t))
        .child(
            kit::label("tb-pill-label", words, PILL, t.text_secondary)
                .size(width, 20.0)
                .px(8.0)
                .pt(1.5),
        )
}

pub fn tools(doc: &PhotoDoc, t: &Theme) -> Vec<El> {
    doc.pill.iter().map(|words| pill(words, t)).collect()
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
        .radius(4.0)
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

fn picture(ui: &Ui, file: &OpenFile, width: f32, height: f32) -> El {
    let photo = el()
        .key("photo")
        .role("image")
        .label(file.name.clone())
        .size(width, height)
        .radius(4.0);
    match ui.source.picture(&file.id, Picture::Photo) {
        Some(pixels) => photo.pixels_fit(&pixels, SurfaceFit::Fit),
        None => photo.fill(plain()),
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
        // DW-1502: beside the panel the photo stands alone, smaller.
        return stage.center().child(picture(ui, &ui.file, 500.0, 334.0));
    }
    let mut strip = row()
        .key("gallery")
        .role("list")
        .label("Photos of this folder")
        .w_full()
        .gap(4.0)
        .py(8.0)
        .center();
    for (i, item) in doc.strip.iter().enumerate() {
        let id = item.id.clone();
        let mut thumb = el()
            .key(format!("gallery-{i}"))
            .role("button")
            .label(item.name.clone())
            .checked(item.current)
            .size(if item.current { 52.0 } else { 36.0 }, 52.0)
            .radius(3.0)
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
        "On this PC"
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
                .center()
                .child(picture(ui, &ui.file, 600.0, 400.0)),
        )
        .child(strip)
        .child(actions)
}
