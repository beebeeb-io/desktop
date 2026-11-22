//! A recording (DW-1503): the artwork from the file's tags (a plain tile when there is none), the
//! title and artist, the scrubber with elapsed and remaining time, previous / pause / next, and
//! the folder's other recordings as the queue.

use rune::{Align, Color, El, Icon, TextStyle, col, el, row};

use super::chrome::{Dot, Status};
use super::kinds::clock;
use super::source::{AudioDoc, Picture};
use super::{Intent, Ui, details, media, media_icons, style, video};
use crate::State;
use crate::theme::scale;
use crate::media::player::{PlayState, PlayerHandle};
use crate::theme::tokens::Theme;
use crate::ui::kit;
use crate::ui::toolbar::{self, ToolState, tool_button};

/// `.t-cap`: 11px/500 on 14px.
const CAP: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);
/// `.kit-list-title`: 10px/600 on 13px, tracked 0.01em.
const LIST_TITLE: TextStyle = TextStyle::new(10.0, 600.0).lh(13.0).tracking(0.1);
/// `.kit-item-value.font-mono`: 12.5px on 17px.
const VALUE: TextStyle = crate::theme::scale::text::FIELD_MONO;
/// `.kit-item-title`: 13px/500 on 17px.
const ITEM: TextStyle = TextStyle::new(13.0, 500.0).lh(17.0).tracking(-0.06);

/// The row of what is playing: ink at 6 % (light), at 9 % (dark).
fn playing_row(t: &Theme) -> Color {
    if t.dark { style::bar_track(t) } else { style::banner(t) }
}

pub fn tools(can_play: bool, t: &Theme) -> Vec<El> {
    let mut tools = vec![
        tool_button("tool-0", "Share", &media_icons::SHARE, 15.0, ToolState::Idle, t).on_press(|s: &mut State| {
            if let Some(ui) = super::ui(s) {
                ui.intent(Intent::Share);
            }
        }),
    ];
    // No player in this build: the recording opens in the app the system has for it.
    if !can_play {
        tools.push(
            super::text::tool_action("tool-1", "Open in another app\u{2026}", None, None, t).on_press(
                |s: &mut State| {
                    if let Some(ui) = super::ui(s) {
                        ui.intent(Intent::OpenInAnotherApp);
                    }
                },
            ),
        );
    }
    tools
}

pub fn status(doc: &AudioDoc, can_play: bool) -> Status {
    let at = doc.queue.iter().position(|t| t.current).map_or(0, |i| i + 1);
    Status {
        dot: Dot::Green,
        text: details::DECRYPTED_HERE.into(),
        details: format!(
            "{}{}, {} of {}",
            if !can_play {
                ""
            } else if doc.playing {
                "Playing \u{00B7} "
            } else {
                "Paused \u{00B7} "
            },
            doc.folder,
            at,
            doc.queue.len()
        ),
    }
}

/// Previous or next: the shared tool button with the chevron, pressed to step the queue.
fn skip(key: &'static str, name: &'static str, icon: &'static Icon, delta: isize, can: bool, t: &Theme) -> El {
    // The one that cannot step (an end of the queue) is not operable.
    let button = toolbar::tool(key, name, icon, t).on_press(move |s: &mut State| {
        media::step(s, delta);
    });
    if can { button } else { button.disabled(true).focusable(false) }
}

/// What the player says of THIS recording, when the source hands a player whose current item it is. Another
/// recording's title, times, picture or refusal are never this view's (`PlayerHandle::state_of`).
struct Live {
    handle: PlayerHandle,
    file_id: String,
    state: PlayState,
    art: Option<rune::Pixels>,
}

impl Live {
    fn of(ui: &Ui) -> Option<Live> {
        let handle = ui.source.player()?;
        let state = handle.state_of(&ui.file.id)?;
        let art = handle.art();
        // The picture is read after the state: the player may have moved on in between, and then it is not ours.
        let still = handle.state_of(&ui.file.id).is_some();
        Some(Live {
            handle,
            file_id: ui.file.id.clone(),
            state,
            art: art.filter(|_| still),
        })
    }
}

fn player(ui: &Ui, doc: &AudioDoc, live: Option<&Live>, t: &Theme) -> El {
    // The tags' title and artist once the player has read them; until then (and with no player) the file's.
    let title = live
        .map(|l| l.state.title.as_str())
        .filter(|w| !w.is_empty())
        .unwrap_or(&doc.title);
    let artist = live.map_or(doc.artist.as_str(), |l| l.state.artist.as_str());
    let (at, length, playing) = match live {
        Some(l) => {
            let times = video::Times::of(&l.state);
            (secs(times.at_ms), secs(times.length_ms), times.playing)
        }
        None => (doc.at, doc.length, doc.playing),
    };
    let artwork = el()
        .key("artwork")
        .role("image")
        .label(format!("Artwork of {title}"))
        .size(260.0, 260.0)
        .radius(crate::theme::tokens::radius::XL)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 18.0,
            blur: 40.0,
            spread: 0.0,
            color: Color::BLACK.alpha(0.2),
            inset: false,
        });
    let tags_art = match live {
        Some(l) => l.art.clone(),
        None => ui.source.picture(&ui.file.id, Picture::Artwork),
    };
    let artwork = match tags_art {
        Some(pixels) => artwork.pixels(&pixels),
        // No artwork in the tags: a plain tile.
        None => artwork.fill(style::tile_neutral(t)),
    };
    // The artist's line is as wide as the title's (one centred block).
    let title_w = rune::text::advance(title, style::TITLE);
    let mut names =
        col()
            .key("audio-names")
            .items_center()
            .child(style::fitted("audio-title", title, style::TITLE, t.text));
    if !artist.is_empty() {
        names = names.child(
            kit::label("audio-artist", artist.to_string(), style::SUB, t.text_tertiary)
                .w(title_w)
                .align(Align::Center),
        );
    }
    let part = if length == 0 {
        0.0
    } else {
        (at as f32 / length as f32).clamp(0.0, 1.0)
    };
    let left = length.saturating_sub(at);
    let mut track = el()
        .key("scrub-track")
        .role("progressbar")
        .label("Position")
        .range_value(f64::from(part), 0.0, 1.0);
    // The player's scrubber is a slider: "1:12 of 4:02", and the arrows seek.
    if let Some(l) = live {
        let handle = l.handle.clone();
        let step_only = l.file_id.clone();
        let drag_with = l.handle.clone();
        track = track
            .role("slider")
            .a11y_value(video::Times::of(&l.state).spoken())
            .on_step(move |_: &mut State, delta: i32, _: &mut rune::Tx| {
                video::step_seek(&handle, Some(&step_only), delta);
            })
            .on_drag(video::scrub_drag(drag_with, Some(l.file_id.clone())))
            .focusable(true)
            .focus_ring(2.0, 2.0, t.accent_deep);
    }
    let scrubber = col()
        .key("scrubber")
        .w_full()
        .gap(scale::space::S4)
        .child(
            track.w_full().h(4.0).radius(2.0).fill(style::bar_track(t)).child(
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
                    &clock(at),
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
    let mut transport = row().key("transport").center_y().gap(scale::space::S24).child(skip(
        "prev",
        "Previous",
        &media_icons::BACK,
        -1,
        !first && at.is_some(),
        t,
    ));
    // No Play where the source cannot play: a button that does nothing is not drawn. It is the press Space makes
    // (`Ui::toggle_play`): this recording plays or pauses, whatever else the player holds.
    if ui.can_play() {
        transport = transport.child(
            el().key("pause")
                .role("button")
                .label(if playing { "Pause" } else { "Play" })
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
                    if playing {
                        &media_icons::PAUSE
                    } else {
                        &media_icons::PLAY
                    },
                    20.0,
                    t.paper,
                )),
        );
    }
    let transport = transport.child(skip("next", "Next", &media_icons::FWD, 1, !last, t));
    let caption = kit::label(
        "audio-caption",
        if ui.can_play() {
            "Decrypted on this PC \u{00B7} keeps playing when you leave this view \u{00B7} the media keys work"
        } else {
            "Decrypted on this PC \u{00B7} playing needs another app: use Open in another app"
        },
        CAP,
        t.text_tertiary,
    )
    .wrap()
    .w(360.0)
    .align(Align::Center);
    let mut shown = col()
        .key("player")
        .w(360.0)
        .items_center()
        .gap(scale::space::S16)
        .child(artwork)
        .child(names)
        .child(scrubber)
        .child(transport)
        .child(caption);
    // The file's sound is not available on this PC: the line, with "Open in another app" one press away. The
    // stage keeps playing. ("Reduced quality on this PC", D-MED-7, is the details panel's line.)
    if let Some(why) = live.and_then(|l| l.state.no_sound) {
        shown = shown.child(video::no_sound_line("audio", why, t.text_tertiary, t));
    }
    shown
}

fn queue(doc: &AudioDoc, t: &Theme) -> El {
    let list = col()
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
        .radius(scale::radius::PANEL)
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
            .px(scale::space::S10)
            .gap(scale::space::S8)
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

fn secs(ms: u64) -> u32 {
    u32::try_from(ms / 1000).unwrap_or(u32::MAX)
}

pub fn body(ui: &Ui, doc: &AudioDoc, t: &Theme) -> El {
    let live = Live::of(ui);
    // A refusal: the words, and "Open in another app\u{2026}" one press away.
    if let Some(why) = live.as_ref().and_then(|l| l.state.refused.as_ref()) {
        return video::refused_for(why, t);
    }
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
        .child(player(ui, doc, live.as_ref(), t))
        .child(queue(doc, t))
}
