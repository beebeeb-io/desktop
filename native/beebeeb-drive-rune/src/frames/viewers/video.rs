//! A video in Beebeeb's player (decision D-MED-6: a video opened from a list plays here; the board has no frame for
//! it yet, one is requested; decision D-MED-4: DW-0611's player bar, drawn by the toolkit in the board's geometry and
//! words, since a toolkit window cannot host Windows' own player).
//!
//! The stage is black and shows the player's frame scaled to fit, letterboxed; under it sits the bar (play and
//! pause, elapsed, the scrubber, remaining; times in JetBrains Mono). The bar's drawing is [`bar`], shared with the
//! Photos viewer's stage. The stage's size goes to the player in device pixels (`PlayerHandle::set_view_size`).
//! A source that hands no player draws what a kind without a viewer draws, and nothing is downloaded.

use rune::tok::Family;
use rune::{col, el, row, Color, El, SurfaceFit, TextStyle};

use super::chrome::{Dot, Status};
use super::kinds::clock;
use super::source::{Body, NoPreview};
use super::{media_icons, refusal_text, states, style, Intent, Ui};
use crate::media::player::{NoSound, PlayState, PlayerHandle, Refusal};
use crate::theme::scale;
use crate::theme::tokens::Theme;
use crate::ui::button::{button, keys, ButtonState, Size, Variant};
use crate::ui::kit;
use crate::ui::view_state::{view_state, ViewState};
use crate::State;

#[cfg(test)]
#[path = "player_view_tests.rs"]
mod player_view_tests;

#[cfg(test)]
#[path = "own_file_tests.rs"]
mod own_file_tests;

/// Frames are asked at the stage's size in device pixels; the window's scale is not known where the stage reports
/// its size, so the ask is the drawn points times this (the player caps it at 1920 x 1080 and the stage scales to
/// fit, so a larger screen is never blurrier than a smaller ask would be sharp).
const ASK_SCALE: f32 = 1.5;
/// One step of the scrubber with the arrow keys, in milliseconds.
const SEEK_STEP_MS: u64 = 5_000;
/// The elapsed / remaining times: 12px mono.
/// The no-sound line: 11px, the caption's size.
const CAP: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);
const TIME: TextStyle = TextStyle::new(12.0, 400.0).family(Family::Mono).lh(16.0);

/// The words of a refusal (the table is `refusal_text`).
pub fn refusal_words(why: &Refusal) -> String {
    refusal_text::text(why).sentence
}

/// [`refusal_words`] for a given state of `MEDIA_SYSTEM_DECODER_ON`.
#[cfg(test)]
pub fn refusal_words_with(system_decoder_on: bool, why: &Refusal) -> String {
    refusal_text::text_with(system_decoder_on, why).sentence
}

/// The line for a file whose sound this PC cannot play (decision of 2026-11-15 19:50): the video keeps playing.
/// The name is the FORMAT's (`Codec::name`), never the file's.
pub fn no_sound_words(why: NoSound) -> String {
    refusal_text::no_sound(why).0
}

/// The no-sound line with "Open in another app\u{2026}" one press away (the press of [`refused`]). `prefix` keys it.
pub fn no_sound_line(prefix: &'static str, why: NoSound, ink: Color, t: &Theme) -> El {
    row()
        .key(format!("{prefix}-no-sound"))
        .gap(scale::space::S12)
        .center_y()
        .child(kit::label(
            format!("{prefix}-no-sound-words"),
            no_sound_words(why),
            CAP,
            ink,
        ))
        .child(
            super::text::tool_action("no-sound-open", refusal_text::no_sound(why).1, None, None, t).on_press(
                |s: &mut State| {
                    if let Some(ui) = super::ui(s) {
                        ui.intent(Intent::OpenInAnotherApp);
                    }
                },
            ),
        )
}

/// While the start buffers: a progress bar and "Buffering 11 MB of 32 MB"; while playing and the download is behind:
/// "Waiting for the network (1.2 MB/s)". Nothing otherwise.
pub fn net_row(state: &PlayState, t: &Theme) -> Option<El> {
    let ink = Color::WHITE;
    let net = &state.net;
    let stalled = net.stall_line();
    let (words, bar) = if state.buffering && state.refused.is_none() && net.total > 0 {
        (net.buffering_line(), Some(net.start_part()))
    } else {
        (stalled?, None)
    };
    let _ = t;
    let mut r = col()
        .key("video-net")
        .w_full()
        .items_center()
        .gap(scale::space::S4)
        .child(kit::label("video-net-words", words, CAP, ink));
    if let Some(part) = bar {
        r = r.child(
            el().key("video-net-bar")
                .role("progressbar")
                .label("Buffering")
                .range_value(f64::from(part), 0.0, 1.0)
                .w(240.0)
                .h(4.0)
                .radius(2.0)
                .fill(ink.alpha(0.3))
                .child(el().key("video-net-fill").w_frac(part).h_full().radius(2.0).fill(ink)),
        );
    }
    Some(r)
}

#[cfg(test)]
pub use super::refusal_text::{OPEN_DEFAULT, OPEN_ELSEWHERE};

/// The button's words for a refusal, as shipped.
#[cfg(test)]
pub fn refusal_action(why: &Refusal) -> &'static str {
    refusal_text::text(why).action
}

/// [`refusal_action`] for a given state of the switch.
#[cfg(test)]
pub fn refusal_action_with(system_decoder_on: bool, why: &Refusal) -> &'static str {
    refusal_text::text_with(system_decoder_on, why).action
}

/// The state of the refusal `why`: its words ([`refusal_words`]) and its button ([`refusal_action`]).
pub fn refused_for(why: &Refusal, t: &Theme) -> El {
    let text = refusal_text::text(why);
    refused_with(&text.sentence, text.action, text.link, t)
}

/// `store`: the Microsoft Store page to offer beside the button, only where the refusal says Windows lacks the
/// extension (`refusal_text::Text::link`).
fn refused_with(words: &str, action: &'static str, store: Option<&'static str>, t: &Theme) -> El {
    // A refusal is final for this file here: it offers the other app, never "Try again".
    let mut buttons = row().key("refused-actions").gap(scale::space::S8).child(
        button(
            keys("refused-open"),
            action,
            None,
            Variant::Secondary,
            Size::Md,
            ButtonState::Enabled,
            t,
        )
        .on_press(|s: &mut State| {
            if let Some(ui) = super::ui(s) {
                ui.intent(Intent::OpenInAnotherApp);
            }
        }),
    );
    if let Some(url) = store {
        buttons = buttons.child(
            button(
                keys("refused-store"),
                "Get HEVC Video Extensions",
                None,
                Variant::Secondary,
                Size::Md,
                ButtonState::Enabled,
                t,
            )
            .on_press(move |s: &mut State| {
                if let Some(ui) = super::ui(s) {
                    ui.intent(Intent::OpenInBrowser(url.to_string()));
                }
            }),
        );
    }
    col()
        .key("viewer-body")
        .grow(1.0)
        .h_full()
        .center()
        .fill(style::stage(t))
        .child(view_state(
            ViewState::Failed {
                key: "refused",
                title: "Can\u{2019}t play this here",
                why: words,
                retry: Some(buttons),
            },
            t,
        ))
}

/// The size a `frame` takes in a `stage`, both in the same unit: scaled to fit, never enlarged past the stage,
/// rounded to whole pixels. A frame of no size takes none.
#[allow(dead_code)] // the toolkit's `Fit` draws the letterbox; this is its rule, kept for the test
pub fn letterbox(frame: (u32, u32), stage: (f32, f32)) -> (f32, f32) {
    if frame.0 == 0 || frame.1 == 0 || stage.0 <= 0.0 || stage.1 <= 0.0 {
        return (0.0, 0.0);
    }
    let k = (stage.0 / frame.0 as f32).min(stage.1 / frame.1 as f32);
    ((frame.0 as f32 * k).round(), (frame.1 as f32 * k).round())
}

/// The stage's size in points to the size to ask the player for, in device pixels.
pub fn ask_size(w: f32, h: f32) -> (u32, u32) {
    (
        (w * ASK_SCALE).round().max(1.0) as u32,
        (h * ASK_SCALE).round().max(1.0) as u32,
    )
}

/// What the bar draws of the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Times {
    pub playing: bool,
    pub at_ms: u64,
    pub length_ms: u64,
    /// How much of the file is fetched, in thousandths: the timeline's buffered range.
    pub buffered_pm: u16,
}

impl Times {
    pub fn of(state: &PlayState) -> Times {
        Times {
            playing: state.playing,
            at_ms: state.at_ms,
            length_ms: state.length_ms,
            buffered_pm: (state.net.buffered_part() * 1000.0) as u16,
        }
    }

    pub fn elapsed(&self) -> String {
        clock(secs(self.at_ms))
    }

    /// "\u{2212}2:50".
    pub fn remaining(&self) -> String {
        format!("\u{2212}{}", clock(secs(self.length_ms.saturating_sub(self.at_ms))))
    }

    /// The slider's value text: "1:12 of 4:02".
    pub fn spoken(&self) -> String {
        format!("{} of {}", self.elapsed(), clock(secs(self.length_ms)))
    }

    pub fn part(&self) -> f32 {
        if self.length_ms == 0 {
            0.0
        } else {
            (self.at_ms as f32 / self.length_ms as f32).clamp(0.0, 1.0)
        }
    }
}

fn secs(ms: u64) -> u32 {
    u32::try_from(ms / 1000).unwrap_or(u32::MAX)
}

/// What the bar's button and Space do for a player in this state: pause what plays, play what does not, nothing
/// before the player has a state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Pause,
    Play,
}

pub fn toggle_for(state: Option<&PlayState>) -> Option<Toggle> {
    match state {
        Some(s) if s.playing => Some(Toggle::Pause),
        Some(_) => Some(Toggle::Play),
        None => None,
    }
}

/// Play or pause whatever the player holds (the Photos stage, which plays the video it opened). A view opened from
/// Files plays or pauses ITS file through the viewer instead (`Ui::toggle_play`).
pub fn toggle(handle: &PlayerHandle) {
    match toggle_for(handle.state().as_ref()) {
        Some(Toggle::Pause) => handle.pause(),
        Some(Toggle::Play) => handle.play(),
        None => {}
    }
}

/// Where a press or drag at `x` on a scrubber `w` wide lands in a file `length_ms` long. `None`: the length is not
/// known yet or the scrubber has no width, so there is nowhere to go.
pub fn seek_target(length_ms: u64, x: f32, w: f32) -> Option<u64> {
    if length_ms == 0 || w.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) || x.is_nan() {
        return None;
    }
    let part = f64::from((x / w).clamp(0.0, 1.0));
    Some((part * length_ms as f64).round() as u64)
}

/// What the player says of the file a control acts for: `Some(id)`, that file's state only (a view opened from Files:
/// another file's position is never this one's to move); `None`, whatever the player holds (the Photos stage).
fn state_for(handle: &PlayerHandle, only: Option<&str>) -> Option<PlayState> {
    match only {
        Some(file_id) => handle.state_of(file_id),
        None => handle.state(),
    }
}

/// The scrubber's drag: the press and the release seek to where the pointer is (the player is not asked on every
/// move); an Esc that cancels the drag seeks nowhere, and so does a drag while the player holds another file than
/// `only`.
pub fn scrub_drag(
    handle: PlayerHandle,
    only: Option<String>,
) -> impl Fn(&mut State, &mut rune::Drag, &mut rune::Tx) + 'static {
    move |_: &mut State, drag: &mut rune::Drag, _: &mut rune::Tx| {
        let lands = match drag.phase {
            rune::Phase::Down => true,
            rune::Phase::Move => false,
            rune::Phase::Up => !drag.cancelled(),
        };
        if !lands {
            return;
        }
        let Some(state) = state_for(&handle, only.as_deref()) else {
            return;
        };
        if let Some(at) = seek_target(state.length_ms, drag.x, drag.w) {
            handle.seek(at);
        }
    }
}

/// The scrubber's arrows: `delta` steps of 5 s from where the file is; nothing while the player holds another file
/// than `only`.
pub fn step_seek(handle: &PlayerHandle, only: Option<&str>, delta: i32) {
    let Some(state) = state_for(handle, only) else {
        return;
    };
    let step = SEEK_STEP_MS.saturating_mul(u64::from(delta.unsigned_abs()));
    handle.seek(if delta < 0 {
        state.at_ms.saturating_sub(step)
    } else {
        state.at_ms.saturating_add(step)
    });
}

/// Whether Space toggles playback: not while a text field has the keys.
#[allow(dead_code)] // the key arm in `media.rs::on_key` for `Body::Video` calls it (the binding slice)
pub fn space_toggles(text_field_has_focus: bool) -> bool {
    !text_field_has_focus
}

/// The player's bar: play and pause, the elapsed time, the scrubber, the time left. `prefix` keys the parts; `ink` is
/// the controls' colour and `track` the scrubber's rail (the stage is dark in both themes). The caller places it.
/// The button plays or pauses whatever the player holds (the Photos stage); a video opened from Files has its own
/// bar ([`files_bar`]).
pub fn bar(prefix: &'static str, handle: &PlayerHandle, times: Times, ink: Color, track: Color, t: &Theme) -> El {
    let toggle_with = handle.clone();
    bar_with(prefix, handle, None, times, ink, track, t, move |_: &mut State| {
        toggle(&toggle_with)
    })
}

/// The bar of a video opened from Files: its button is the viewer's Play (`Ui::toggle_play`, the press Space makes:
/// the first one opens the video through the source for the press's session), and its scrubber moves only this file.
fn files_bar(handle: &PlayerHandle, file_id: &str, times: Times, t: &Theme) -> El {
    let press = |s: &mut State| {
        if let Some(ui) = super::ui(s) {
            ui.toggle_play();
        }
    };
    let track = Color::WHITE.alpha(0.3);
    bar_with(
        "video",
        handle,
        Some(file_id.to_string()),
        times,
        Color::WHITE,
        track,
        t,
        press,
    )
}

/// The bar's drawing; `press` is its button's, and the scrubber acts only while the player holds `only` (`None`:
/// whatever it holds).
#[allow(clippy::too_many_arguments)]
fn bar_with(
    prefix: &'static str,
    handle: &PlayerHandle,
    only: Option<String>,
    times: Times,
    ink: Color,
    track: Color,
    t: &Theme,
    press: impl Fn(&mut State) + 'static,
) -> El {
    let seek_with = handle.clone();
    let seek_only = only.clone();
    let drag_with = handle.clone();
    let part = times.part();
    row()
        .key(format!("{prefix}-bar"))
        .w_full()
        .h(46.0)
        .px(scale::space::S16)
        .gap(scale::space::S12)
        .center_y()
        .child(
            kit::glyph(
                format!("{prefix}-play-icon"),
                if times.playing {
                    &media_icons::PAUSE
                } else {
                    &media_icons::PLAY
                },
                16.0,
                ink,
            )
            .role("button")
            .label(if times.playing { "Pause" } else { "Play" })
            .shortcut_hint("Space")
            .focusable(true)
            .focus_ring(2.0, 2.0, t.accent_deep)
            .cursor(rune::Cursor::Pointer)
            .on_press(press),
        )
        .child(kit::label(format!("{prefix}-elapsed"), times.elapsed(), TIME, ink))
        .child(
            el().key(format!("{prefix}-scrubber"))
                .role("slider")
                .label("Position")
                .a11y_value(times.spoken())
                .range_value(f64::from(part), 0.0, 1.0)
                .on_step(move |_: &mut State, delta: i32, _: &mut rune::Tx| {
                    step_seek(&seek_with, seek_only.as_deref(), delta);
                })
                .on_drag(scrub_drag(drag_with, only))
                .focusable(true)
                .focus_ring(2.0, 2.0, t.accent_deep)
                .grow(1.0)
                .h(4.0)
                .radius(2.0)
                .fill(track)
                .child(
                    el().key(format!("{prefix}-scrub-buffered"))
                        .w_frac(f32::from(times.buffered_pm) / 1000.0)
                        .h_full()
                        .radius(2.0)
                        .fill(ink.alpha(0.35)),
                )
                .child(
                    el().key(format!("{prefix}-scrub-fill"))
                        .w_frac(part)
                        .h_full()
                        .radius(2.0)
                        .fill(ink),
                ),
        )
        .child(kit::label(format!("{prefix}-remaining"), times.remaining(), TIME, ink))
}

/// The black stage with the player's frame, scaled to fit, when `own` (the player holds this view's file; another
/// file's picture is never shown here). Its size goes to the player when it changes.
fn stage(handle: &PlayerHandle, own: bool) -> El {
    let ask = handle.clone();
    let mut stage = el()
        .key("video-stage")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .center()
        .fill(Color::BLACK)
        .on_layout(move |_: &mut State, [_, _, w, h]| {
            let (w, h) = ask_size(w, h);
            ask.set_view_size(w, h);
        });
    if let Some(frame) = handle.frame().filter(|_| own) {
        // `Fit` scales the picture to the box and centres it: the letterbox.
        stage = stage.child(
            el().key("video-frame")
                .role("image")
                .label("Video")
                .w_full()
                .h_full()
                .pixels_fit(&frame, SurfaceFit::Fit),
        );
    }
    stage
}

/// The video's stage and bar. They draw what the player says of THIS file only (`PlayerHandle::state_of`): with
/// nothing of it held (before its first Play, or while another file plays) the stage is black, the times are 0:00 and
/// the button is Play, which opens this video.
pub fn view(ui: &Ui, t: &Theme) -> El {
    if let Some(handle) = ui.source.player() {
        let state = handle.state_of(&ui.file.id);
        if let Some(why) = state.as_ref().and_then(|s| s.refused.as_ref()) {
            return refused_for(why, t);
        }
        let times = state.as_ref().map_or(
            Times {
                playing: false,
                at_ms: 0,
                length_ms: 0,
                buffered_pm: 0,
            },
            Times::of,
        );
        let mut body = col()
            .key("viewer-body")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .fill(Color::BLACK)
            .child(stage(&handle, state.is_some()));
        // The stage keeps playing; the line sits where "Reduced quality on this PC" sits in the bar's words.
        if let Some(why) = state.as_ref().and_then(|s| s.no_sound) {
            body = body.child(no_sound_line("video", why, Color::WHITE, t));
        }
        if let Some(net) = state.as_ref().and_then(|s| net_row(s, t)) {
            body = body.child(net);
        }
        return body.child(files_bar(&handle, &ui.file.id, times, t));
    }
    // No player: the state of a kind without a viewer, for this file. The copy's name is overwritten when it is drawn.
    let mut shown = ui.file.clone();
    shown.body = Body::NotPreviewable(NoPreview::NoViewer);
    let drawn = states::body(&shown, ui.saves_copies(), ui.origin(), t).unwrap_or_else(el);
    shown.wipe();
    drawn
}

/// The status line under the video.
pub fn status(ui: &Ui) -> Status {
    if ui.source.player().is_some() {
        return Status {
            dot: Dot::Green,
            text: super::details::DECRYPTED_HERE.into(),
            details: concat!(
                "Streaming \u{00B7} decrypted on ",
                beebeeb_engine::device_word!(),
                " as it plays"
            )
            .into(),
        };
    }
    states::status(&Body::NotPreviewable(NoPreview::NoViewer), ui.origin()).unwrap_or(Status {
        dot: Dot::Warn,
        text: String::new(),
        details: String::new(),
    })
}
