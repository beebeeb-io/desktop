//! One photo open (DW-0605 to DW-0611): the top bar's two renditions and ⋯, the photo on the
//! dark stage, the strip, and Share · Offline · Details · Trash.

use crate::theme::scale::space;
use rune::{Color, El, SurfaceFit, col, el, row};

use super::chrome::{self, Bar, rgb, style};
use super::grid::{length, megabytes, thousands};
use super::source::{Kind, Shape, Viewing};
use super::{Intent, Menu, Original, Ui, icons, overlay};
use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::toolbar::tool;
use crate::ui::kit;

/// The largest display scale the cap allows for (a 2x Retina display; 1x gets the same cap, never smaller).
const MAX_SCALE: f32 = 2.0;

/// The size in points a picture of `pixels` is drawn at in a `stage` (contain, aspect kept), never larger than
/// its pixels times the display `scale`. The layout does the same with `aspect_fit` and `max_w`/`max_h`.
pub(crate) fn fit_size(pixels: (u32, u32), stage: (f32, f32), scale: f32) -> (f32, f32) {
    let (pw, ph) = (pixels.0.max(1) as f32, pixels.1.max(1) as f32);
    let k = (stage.0 / pw).min(stage.1 / ph).min(scale);
    (pw * k, ph * k)
}

fn tab(i: usize, words: String, on: bool, t: &Theme) -> El {
    let mut tab = el()
        .key(format!("ph-seg-{i}"))
        .role("tab")
        .label(words.clone())
        .checked(on)
        .h(24.0)
        .px(space::S10)
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
    let mut middle = row().key("ph-tools").gap(space::S6).center_y();
    // A video has one rendition: it streams (DW-0611).
    if viewing.kind == Kind::Photo {
        let whole = megabytes(viewing.original_bytes, true);
        let original_words = match original {
            Original::Downloading { done } => format!("Original \u{00B7} {} of {whole}", super::grid::sized_like(done, viewing.original_bytes, false)),
            _ => format!("Original \u{00B7} {whole}"),
        };
        middle = middle.child(
            row()
                .key("ph-seg")
                .role("tablist")
                .focus_group()
                .label("Rendition")
                .padding(space::S2)
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
    let open = viewing.id;
    let show_label = crate::frames::show_in_folder::label();
    middle = middle.child(
        tool("ph-show-in-folder", show_label, &crate::theme::icons_settings::FOLDER, t)
            .help(show_label)
            .on_press(move |s: &mut State| s.photos.ask(Intent::ShowInFolder(open))),
    );
    middle = middle.child(
        tool("ph-more", "More", &icons::MORE, t)
            .expanded(ui.menu.is_some_and(|m| m.which == Menu::More))
            .on_press(|s: &mut State| s.photos.toggle_menu(Menu::More)),
    );
    let what = match viewing.kind {
        // A length this PC cannot say (0): the name, as for a photo.
        Kind::Video(seconds) if seconds > 0 => length(seconds),
        _ => viewing.name.clone(),
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
        Kind::Video(_) => concat!("Streaming \u{00B7} decrypted on ", beebeeb_engine::device_word!(), " as it plays").into(),
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
        .px(space::S16)
        .gap(space::S12)
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
        // The board's own note on this bar: the system player draws the real controls on Windows.
        .child(kit::label(
            "ph-player-text-2",
            "Windows draws the player controls",
            style::CAP,
            white.alpha(0.7),
        ))
}

/// The player's handle when the stage shows the video the window holds: `held` is the item this window started and
/// its player, `shown` the item on the stage. Any other item draws its own controls (the board's fixture or the still
/// with the status line), never another item's player.
pub fn live_for(
    shown: super::source::PhotoId,
    held: Option<&(super::source::PhotoId, crate::media::player::PlayerHandle)>,
) -> Option<crate::media::player::PlayerHandle> {
    held.filter(|(id, _)| *id == shown).map(|(_, handle)| handle.clone())
}

#[cfg(test)]
#[path = "live_tests.rs"]
mod live_tests;

/// The stage's picture of a video the player plays: the player's frame, scaled to fit, and the player's bar on it
/// (the drawing of `viewers::video::bar`, shared with a video opened from Files).
fn live_stage(photo: El, handle: &crate::media::player::PlayerHandle, own: Option<&str>, t: &Theme) -> El {
    use crate::frames::viewers::video::{self, Times};
    let ask = handle.clone();
    // The picture takes the whole stage (the fixture's 600 x 400 is the board's drawing): its size is what the
    // player is asked for.
    let mut photo = photo.w_full().h_full().on_layout(move |_: &mut State, [_, _, w, h]| {
        let (w, h) = video::ask_size(w, h);
        ask.set_view_size(w, h);
    });
    // Only this video's own state: while the player holds another file (a song started from Files), the stage keeps
    // its own picture and an idle bar, never that file's frame, times or refusal.
    let state = own.and_then(|file| handle.state_of(file));
    if state.is_some()
        && let Some(frame) = handle.frame()
    {
        photo = photo.pixels_fit(&frame, SurfaceFit::Fit);
    }
    let ink = Color::WHITE;
    let inner = match state.as_ref().and_then(|s| s.refused.as_ref()) {
        Some(why) => kit::label("ph-player-refused", video::refusal_words(why), style::CAP, ink),
        None => {
            let times = state.as_ref().map_or(
                Times {
                    playing: false,
                    at_ms: 0,
                    length_ms: 0,
                    buffered_pm: 0,
                },
                Times::of,
            );
            video::bar("ph-live", handle, times, ink, ink.alpha(0.3), t)
        }
    };
    // The file's sound is not available here: the line above the bar, "Open in another app\u{2026}" one press away.
    // The stage keeps playing.
    if let Some(why) = state.as_ref().and_then(|s| s.no_sound) {
        photo = photo.child(
            row()
                .key("ph-no-sound")
                .pin(Some(24.0), None, Some(24.0), Some(72.0))
                .child(video::no_sound_line("ph", why, ink, t)),
        );
    }
    photo.child(
        row()
            .key("ph-player")
            .pin(Some(24.0), None, Some(24.0), Some(20.0))
            .h(46.0)
            .radius(12.0)
            .center_y()
            .fill(Color::BLACK.alpha(0.45))
            .backdrop_blur(16.0)
            .child(inner),
    )
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

    // w31: a HEIC or RAW original is said, and the system's app is offered for it.
    if let Some(words) = ui.source.original_refused(viewer.id).filter(|_| viewing.kind == Kind::Photo) {
        let id = viewer.id;
        let app = crate::frames::show_in_folder::os_app_label();
        stage = stage.child(
            row()
                .key("ph-viewer-notice")
                .role("status")
                .label(words)
                .w_full()
                .px(space::S20)
                .py(space::S8)
                .gap(space::S12)
                .center_y()
                .child(kit::label("ph-viewer-notice-words", words, style::SUB, white.alpha(0.75)))
                .child(
                    chrome::button("ph-open-in-os-app", &format!("Open in {app}"), None, chrome::Tone::Plain, 28.0, crate::theme::tokens::type_::BUTTON_SM, t)
                        .on_press(move |s: &mut State| s.photos.ask(Intent::OpenInOsApp(id))),
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
        .radius(4.0);
    let whole = if viewer.original == Original::Whole {
        ui.source.original(viewer.id)
    } else {
        None
    };
    photo = match whole.or_else(|| ui.source.thumbnail(viewer.id, Shape::Preview)) {
        Some(pixels) => {
            let (w, h) = pixels.size();
            let (cap_w, cap_h) = (w as f32 * MAX_SCALE, h as f32 * MAX_SCALE);
            // The largest box at the picture's own aspect the stage allows, capped at its pixels times the display scale.
            photo
                .aspect_fit(w.max(1) as f32 / h.max(1) as f32)
                .max_w(cap_w)
                .max_h(cap_h)
                .pixels_fit(&pixels, SurfaceFit::Fill)
        }
        // No picture yet: a placeholder at the board's drawing size.
        None => photo.size(600.0, 400.0).fill(white.alpha(0.07)),
    };
    // A video the shell's player is playing: its frames on the stage, the bar working (Space and the button
    // play and pause), a refusal in the bar's place.
    // Only the video this window started (`Ui::playing`) shows live: never the process-wide player.
    let live = match shown.kind {
        Kind::Video(_) => live_for(shown.id, ui.playing.as_ref()),
        Kind::Photo => None,
    };
    if let Some(handle) = live {
        photo = live_stage(photo, &handle, ui.playing_file.as_deref(), t);
    } else if let Kind::Video(seconds) = shown.kind
        && !ui.source.answers_later()
    {
        // The player's controls of the board's fixture, which answers in the call. On the engine's source with no
        // player the stored picture of a video shows, and the status line says why.
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
        .gap(space::S4)
        .py(space::S8)
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
            .gap(space::S4)
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
                act(1, if kept { concat!("On ", beebeeb_engine::device_word!()) } else { "Offline" }, &icons::OFFLINE)
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

#[cfg(test)]
mod fit_tests {
    use super::fit_size;

    #[test]
    fn portrait_preview_fills_stage_height() {
        for scale in [1.0, 2.0] {
            let (w, h) = fit_size((1290, 1920), (2200.0, 1300.0), scale);
            assert!((h - 1300.0).abs() < 0.01, "scale {scale}: {h}");
            assert!(w < 2200.0 && (w / h - 1290.0 / 1920.0).abs() < 1e-4);
        }
    }

    #[test]
    fn never_larger_than_pixels_times_scale() {
        let (w, h) = fit_size((100, 50), (2200.0, 1300.0), 2.0);
        assert_eq!((w, h), (200.0, 100.0));
    }
}
