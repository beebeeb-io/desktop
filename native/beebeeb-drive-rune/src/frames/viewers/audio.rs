//! A recording (DW-1503): the artwork from the file's tags (a plain tile when there is none), the
//! title and artist, the scrubber with elapsed and remaining time, previous / pause / next, and
//! the folder's other recordings as the queue.

use rune::tok::Family;
use rune::{Align, Color, El, Icon, TextStyle, col, el, row};

use super::chrome::{Dot, Status};
use super::kinds::clock;
use super::source::{AudioDoc, Picture};
use super::{Intent, Ui, details, media, media_icons, style};
use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::kit;
use crate::ui::toolbar::{ToolState, tool_button};

/// `.t-cap`: 11px/500 on 14px.
const CAP: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);
/// `.kit-list-title`: 10px/600 on 13px, tracked 0.01em.
const LIST_TITLE: TextStyle = TextStyle::new(10.0, 600.0).lh(13.0).tracking(0.1);
/// `.kit-item-value.font-mono`: 12.5px on 17px.
const VALUE: TextStyle = TextStyle::new(12.5, 400.0).family(Family::Mono).lh(17.0);
/// `.kit-item-title`: 13px/500 on 17px.
const ITEM: TextStyle = TextStyle::new(13.0, 500.0).lh(17.0).tracking(-0.06);

/// The row of what is playing: ink at 6 % (light), at 9 % (dark).
fn playing_row(t: &Theme) -> Color {
    if t.dark { style::bar_track(t) } else { style::banner(t) }
}

pub fn tools(t: &Theme) -> Vec<El> {
    vec![
        tool_button("tool-0", "Share", &media_icons::SHARE, 15.0, ToolState::Idle, t).on_press(|s: &mut State| {
            if let Some(ui) = super::ui(s) {
                ui.intent(Intent::Share);
            }
        }),
    ]
}

pub fn status(doc: &AudioDoc) -> Status {
    let at = doc.queue.iter().position(|t| t.current).map_or(0, |i| i + 1);
    Status {
        dot: Dot::Green,
        text: details::DECRYPTED_HERE.into(),
        details: format!(
            "{} \u{00B7} {}, {} of {}",
            if doc.playing { "Playing" } else { "Paused" },
            doc.folder,
            at,
            doc.queue.len()
        ),
    }
}

/// Previous or next: two chevrons (`.mk-tbtn`). Keys: `<key>`, `<key>-icon`, `<key>-icon-2`.
fn skip(key: &'static str, name: &'static str, icon: &'static Icon, delta: isize, can: bool, t: &Theme) -> El {
    // The board draws both in the same ink at either end of the queue; the one that cannot
    // step is not operable.
    let ink = t.text_secondary;
    row()
        .key(key)
        .role("button")
        .label(name)
        .disabled(!can)
        .size(56.0, 28.0)
        .gap(4.0)
        .center()
        .radius(7.0)
        .hover(t.hover)
        .press(t.press)
        .focusable(can)
        .on_press(move |s: &mut State| {
            media::step(s, delta);
        })
        .child(kit::glyph(format!("{key}-icon"), icon, 20.0, ink))
        .child(kit::glyph(format!("{key}-icon-2"), icon, 20.0, ink))
}

fn player(ui: &Ui, doc: &AudioDoc, t: &Theme) -> El {
    let artwork = el()
        .key("artwork")
        .role("image")
        .label(format!("Artwork of {}", doc.title))
        .size(260.0, 260.0)
        .radius(14.0)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 18.0,
            blur: 40.0,
            spread: 0.0,
            color: Color::BLACK.alpha(0.2),
            inset: false,
        });
    let artwork = match ui.source.picture(&ui.file.id, Picture::Artwork) {
        Some(pixels) => artwork.pixels(&pixels),
        // No artwork in the tags: a plain tile.
        None => artwork.fill(style::tile_neutral(t)),
    };
    // The artist's line is as wide as the title's (one centred block).
    let title_w = rune::text::advance(&doc.title, style::TITLE);
    let mut names =
        col()
            .key("audio-names")
            .items_center()
            .child(style::fitted("audio-title", &doc.title, style::TITLE, t.text));
    if !doc.artist.is_empty() {
        names = names.child(
            kit::label("audio-artist", doc.artist.clone(), style::SUB, t.text_tertiary)
                .w(title_w)
                .align(Align::Center),
        );
    }
    let part = if doc.length == 0 {
        0.0
    } else {
        (doc.at as f32 / doc.length as f32).clamp(0.0, 1.0)
    };
    let left = doc.length.saturating_sub(doc.at);
    let scrubber = col()
        .key("scrubber")
        .w_full()
        .gap(4.0)
        .child(
            el().key("scrub-track")
                .role("progressbar")
                .label("Position")
                .range_value(f64::from(part), 0.0, 1.0)
                .w_full()
                .h(4.0)
                .radius(2.0)
                .fill(style::bar_track(t))
                .child(
                    el().key("scrub-fill")
                        .w_frac(part)
                        .h_full()
                        .radius(2.0)
                        .fill(t.text_secondary),
                ),
        )
        .child(
            row()
                .key("times")
                .w_full()
                .justify_between()
                .child(style::fitted(
                    "time-elapsed",
                    &clock(doc.at),
                    style::CAP_MONO,
                    t.text_tertiary,
                ))
                .child(style::fitted(
                    "time-remain",
                    &format!("\u{2212}{}", clock(left)),
                    style::CAP_MONO,
                    t.text_tertiary,
                )),
        );
    let at = doc.queue.iter().position(|t| t.current);
    let (first, last) = (at == Some(0), at.is_none_or(|i| i + 1 >= doc.queue.len()));
    let transport = row()
        .key("transport")
        .center_y()
        .gap(24.0)
        .child(skip(
            "prev",
            "Previous",
            &media_icons::BACK,
            -1,
            !first && at.is_some(),
            t,
        ))
        .child(
            el().key("pause")
                .role("button")
                .label(if doc.playing { "Pause" } else { "Play" })
                .shortcut_hint("Space")
                .size(48.0, 48.0)
                .radius(24.0)
                .center()
                .fill(t.text)
                .focusable(true)
                .focus_ring(2.0, 2.0, t.accent_deep)
                .cursor(rune::Cursor::Pointer)
                .on_press(|s: &mut State| {
                    if let Some(ui) = super::ui(s) {
                        ui.toggle_play();
                    }
                })
                .child(kit::glyph(
                    "pause-icon",
                    if doc.playing {
                        &media_icons::PAUSE
                    } else {
                        &media_icons::PLAY
                    },
                    20.0,
                    t.paper,
                )),
        )
        .child(skip("next", "Next", &media_icons::FWD, 1, !last, t));
    col()
        .key("player")
        .w(360.0)
        .items_center()
        .gap(16.0)
        .child(artwork)
        .child(names)
        .child(scrubber)
        .child(transport)
        .child(
            kit::label(
                "audio-caption",
                "Decrypted on this PC \u{00B7} keeps playing when you leave this view \u{00B7} the media keys work",
                CAP,
                t.text_tertiary,
            )
            .wrap()
            .w(360.0)
            .align(Align::Center),
        )
}

fn queue(doc: &AudioDoc, t: &Theme) -> El {
    let mut list = col()
        .key("queue")
        .role("list")
        .label(format!("Up next, {}", doc.folder))
        .grow(1.0)
        .min_w(0.0)
        .child(
            kit::label(
                "queue-title",
                format!("Up next \u{00B7} {}", doc.folder),
                LIST_TITLE,
                t.text_tertiary,
            )
            .w_full()
            .px(13.0)
            .mb(6.0),
        );
    // `.kit-list-body`: the rows in one outlined box, a hairline between them.
    let mut rows = col()
        .key("queue-body")
        .w_full()
        .radius(10.0)
        .box_shadow(kit::ring(kit::HAIR, t.separator));
    for (i, track) in doc.queue.iter().enumerate() {
        let id = track.id.clone();
        let length = clock(track.length);
        let mut item = row()
            .key(format!("track-{i}"))
            .role("listitem")
            .label(format!("{}, {}", track.title, length))
            .checked(track.current)
            .w_full()
            .h(28.0)
            .px(10.0)
            .gap(8.0)
            .center_y()
            .border_sides([if i == 0 { 0.0 } else { kit::HAIR }, 0.0, 0.0, 0.0], t.separator)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut State| {
                media::switch_to(s, &id);
            })
            .child(
                el().key(format!("track-{i}-icon-box"))
                    .w(30.0)
                    .center()
                    .child(kit::glyph(
                        format!("track-{i}-icon"),
                        if track.current {
                            &media_icons::PLAY
                        } else {
                            &media_icons::FILE_AUDIO
                        },
                        18.0,
                        t.text_secondary,
                    )),
            )
            .child(
                kit::label(format!("track-{i}-title"), track.title.clone(), ITEM, t.text)
                    .grow(1.0)
                    .min_w(0.0)
                    .truncate(),
            )
            .child(style::fitted(
                format!("track-{i}-value"),
                &length,
                VALUE,
                if track.current {
                    t.text_secondary
                } else {
                    t.text_tertiary
                },
            ));
        item = if track.current {
            item.fill(playing_row(t))
        } else {
            item.hover(t.hover).press(t.press)
        };
        rows = rows.child(item);
    }
    list.child(rows)
}

pub fn body(ui: &Ui, doc: &AudioDoc, t: &Theme) -> El {
    row()
        .key("viewer-body")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .px(40.0)
        .py(32.0)
        .gap(32.0)
        .items_start()
        .fill(t.bg)
        .child(player(ui, doc, t))
        .child(queue(doc, t))
}
