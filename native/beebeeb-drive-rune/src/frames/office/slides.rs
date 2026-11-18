//! Slice OF3 (plan40 batch 1): the presentation VIEWER of the Office engine, DW-1509 (the deck, read-only) and
//! DW-1561 (Present, full screen). DW-1560 (slide layouts) is a 0.9.1 frame, the editor's: its row stays
//! `placeholder::scene` (owner 2026-11-15 16:29, decision 2539: the Office viewer is in 0.9.0, the editor in 0.9.1).
//!
//! What is drawn: the window's own toolbar (the viewer's, reused from `viewers::office`), a bar with Present, the
//! filmstrip, the current slide on grey, and the status line. Nothing here edits: no ribbon, no New, Duplicate,
//! Delete, Save or Undo. A slide's pixels are asked of the source by number (`SlidePage`); the fixture answers now,
//! the live host answers at O-CP3. A slide the source has no picture of yet is a blank slide, never a sample.
//!
//! Keys: the window's caption, sidebar and toolbar keep the keys every window has. This file's are `vbar`,
//! `present` (the button and its `present-icon` / `present-label` / `present-hint`), `filmstrip`,
//! `fs-<N>` (a thumbnail; `fs-<N>-slide`, `fs-<N>-num`), `stage`, `slide`, and `st-<N>` (the status line).
//! Present's: `pres-stage`, `pres-slide`, `pres-<N>` (the title), `pres-li-<N>` (a point), `pres-image`,
//! `pres-esc-box`, `pres-esc`, `pres-count`.
//!
//! Wiring this file needs from `viewers` (listed in BRANCH-NOTES.md for the checkpoint): a `Body::Deck(DeckBody)`
//! arm, `viewers::deck_mut(&mut State) -> Option<&mut DeckBody>`, `viewers::open_deck(&mut State, OpenFile) -> bool`,
//! the `Ui` window arm calling [`window`] or [`present`], and the key route calling [`DeckBody::on_key`].

use beebeeb_office_protocol::office::EditOp;
use rune::tok::Family;
use rune::{Color, Cx, El, Icon, Key, Mods, Pixels, Span, SurfaceFit, TextStyle, col, el, row};

use super::docs::{ABOUT, KEEP_FIRST, Live, OPEN_ELSEWHERE};
use super::edit::slide as edit_slide;
use super::edit::{EditUi, slide_ribbon};
use super::states;
use crate::State;
use crate::frames::placeholder;
use crate::frames::signin::icons::EYE;
use crate::frames::viewers::kinds::OfficePicture;
use crate::frames::viewers::source::Body;
use crate::frames::viewers::{self, Intent, Ui};
use crate::theme::icons;
use crate::theme::tokens::{self, Theme, type_};
use crate::ui::{caption, kit, sidebar};

#[path = "slides_fixture.rs"]
pub mod slides_fixture;
#[path = "slides_strip.rs"]
mod strip;

pub use strip::{StripMenu, entries as menu_entries};

/// The group's board frames and the scene that shows each (`frames::scenes`). DW-1560 stays a placeholder (0.9.1).
pub const FRAMES: &[crate::ui::gate::Scene<State>] = &[
    ("DW-1509", scene_deck),
    ("DW-1560", placeholder::scene),
    ("DW-1561", scene_present),
];

/// DW-1509: the deck open on slide 2 of 3, in the window.
pub fn scene_deck(s: &mut State) {
    if viewers::open_deck(s, slides_fixture::file()) {
        if let Some(deck) = viewers::deck_mut(s) {
            deck.go(slides_fixture::BOARD_SLIDE);
        }
    }
}

/// DW-1561: the same deck, after a press on Present: slide 2 on black.
pub fn scene_present(s: &mut State) {
    scene_deck(s);
    if let Some(deck) = viewers::deck_mut(s) {
        deck.present();
    }
}

// ---------------------------------------------------------------------------
// What a deck is
// ---------------------------------------------------------------------------

/// One slide of the filmstrip. Its pixels are the source's, asked by `n` (1-based).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlideThumb {
    pub n: u32,
}

/// The deck as the viewer shows it: the filmstrip's slides, the one on the stage, and how many there are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeckView {
    pub slides: Vec<SlideThumb>,
    /// The slide on the stage, 1-based.
    pub current: u32,
    pub count: u32,
    /// The stage's zoom in per cent (the bar's "- 100% +").
    pub zoom: u16,
    /// Some content of a slide could not be drawn: the status bar says so quietly (the live source sets it).
    pub quiet: bool,
    /// The slides' size in points (the editor turns a point on the stage into a point on the slide with it). The
    /// source sets it from the file; until it does a slide is the 16:9 of PowerPoint's own default.
    pub page_pt: (u32, u32),
}

impl DeckView {
    /// A deck of `count` slides, on the first.
    pub fn of(count: u32) -> Self {
        DeckView {
            slides: (1..=count).map(|n| SlideThumb { n }).collect(),
            current: 1,
            count,
            zoom: 100,
            quiet: false,
            page_pt: PAGE_PT,
        }
    }
}

/// What the source says of one slide: the picture for the filmstrip and the stage, and the words for Present.
/// Either may be missing while the host renders it.
#[derive(Clone, Default)]
pub struct SlidePage {
    pub pixels: Option<Pixels>,
    pub words: Option<SlideWords>,
}

/// The slide's words, as Present draws them: a title, its points and whether a picture follows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlideWords {
    pub title: String,
    /// The line under the title ("" when the slide has none).
    pub sub: String,
    pub points: Vec<String>,
    pub picture: bool,
}

/// What `viewers` holds for an open deck: the view and whether Present has the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeckBody {
    pub view: DeckView,
    pub presenting: bool,
    /// A touchpad's travel toward the next step of the wheel, in points ([`DeckBody::wheel_step`]).
    pub wheel: i32,
    /// The filmstrip's scroll offset in points from the first slide's top, and its height as last laid out.
    pub strip_y: u32,
    pub strip_h: u32,
    /// The filmstrip's right-click menu, while it is open (the editor's).
    pub menu: Option<StripMenu>,
    /// The last spot a drag asked the editor about, in whole slide points (the editor's).
    pub last_hit: Option<(i32, i32)>,
}

/// Points of touchpad travel that step one slide, and of a mouse wheel's (a notch is 40 points; a smooth wheel sends it
/// in parts).
const SWIPE: i32 = 120;
const NOTCH: i32 = 40;

impl DeckBody {
    pub fn new(view: DeckView) -> Self {
        DeckBody {
            view,
            presenting: false,
            wheel: 0,
            strip_y: 0,
            strip_h: STRIP_H_GUESS,
            menu: None,
            last_hit: None,
        }
    }

    /// The deck now has `count` slides (a slide was added or taken away): the strip lists them all, the slide on the
    /// stage stays inside the deck and the strip's offset inside the strip.
    pub fn set_count(&mut self, count: u32) {
        let count = count.max(1);
        self.view.count = count;
        self.view.slides = (1..=count).map(|n| SlideThumb { n }).collect();
        self.view.current = self.view.current.clamp(1, count);
        self.menu = None;
        self.reveal_current();
    }

    /// The wheel over the slide (in the window and in Present): no free scroll, the next or the previous slide. A mouse
    /// wheel steps once per notch ([`NOTCH`] points, whether it comes whole or, from a smooth wheel, in parts; toward
    /// you: the next); a touchpad once per [`SWIPE`] points of travel, and its coasting after the fingers lift moves
    /// nothing. Turning back starts afresh. A sideways delta is not the deck's. `true`: taken.
    pub fn wheel_step(&mut self, ev: &rune::WheelEvent) -> bool {
        if ev.dy.abs() < ev.dx.abs() {
            return false;
        }
        if ev.phase == rune::GesturePhase::Began {
            self.wheel = 0;
        }
        if ev.momentum {
            return true;
        }
        let dy = ev.dy.round() as i32;
        if dy != 0 && (self.wheel > 0) != (dy > 0) {
            self.wheel = 0;
        }
        self.wheel = self.wheel.saturating_add(dy);
        let step = if ev.precise { SWIPE } else { NOTCH };
        if self.wheel <= -step {
            self.next();
            self.wheel = 0;
        } else if self.wheel >= step {
            self.previous();
            self.wheel = 0;
        }
        true
    }

    /// Show slide `n` (clamped to the deck).
    pub fn go(&mut self, n: u32) {
        self.view.current = n.clamp(1, self.view.count.max(1));
        self.reveal_current();
    }

    pub fn next(&mut self) {
        self.go(self.view.current.saturating_add(1));
    }

    pub fn previous(&mut self) {
        self.go(self.view.current.saturating_sub(1));
    }

    pub fn zoom_in(&mut self) {
        self.view.zoom = (self.view.zoom + ZOOM_STEP).min(ZOOM_MAX);
    }

    pub fn zoom_out(&mut self) {
        self.view.zoom = self.view.zoom.saturating_sub(ZOOM_STEP).max(ZOOM_MIN);
    }

    /// Present Ctrl+Enter / F5: the slide takes the screen.
    pub fn present(&mut self) {
        self.presenting = true;
    }

    /// Esc: back to the slides, on the slide that was showing.
    pub fn leave(&mut self) {
        self.presenting = false;
    }

    /// The deck's keys. `true`: taken. Present: right arrow, Space, Page Down, next; left arrow, Page Up,
    /// previous; Esc leaves. In the window: F5 and Ctrl+Enter present, the arrows move the stage.
    pub fn on_key(&mut self, key: &Key, m: &Mods) -> bool {
        let plain = !m.alt && !m.cmd && !m.ctrl && !m.shift;
        match key {
            Key::Escape if self.presenting => self.leave(),
            Key::Right | Key::PageDown | Key::Down if plain => self.next(),
            Key::Left | Key::PageUp | Key::Up if plain => self.previous(),
            Key::Text(t) if plain && t == " " && self.presenting => self.next(),
            Key::Function(5) if !self.presenting => self.present(),
            Key::Enter if (m.ctrl || m.cmd) && !self.presenting => self.present(),
            _ => return false,
        }
        true
    }
}

// ---------------------------------------------------------------------------
// The window (DW-1509)
// ---------------------------------------------------------------------------

/// The words the board shows in this frame.
pub const PRESENT_LABEL: &str = "Present";
pub const PRESENT_HINT: &str = "F5";
pub const ENCRYPTED: &str = "Encrypted";
/// Present's way back, said where the key is read.
pub const BACK_TO_SLIDES: &str = "Back to the slides";

const BAR_LABEL: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
const BAR_HINT: TextStyle = TextStyle::new(10.0, 500.0).family(Family::Mono).lh(13.0);
const NUM: TextStyle = TextStyle::new(11.0, 500.0).family(Family::Mono).lh(14.0);
const STATUS_ENC: TextStyle = TextStyle::new(11.5, 600.0).lh(17.0);

static LESS: Icon = Icon {
    name: "slides-less",
    view: 24.0,
    body: concat!(
        r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
        r#"<path d="M5 12h14"></path></g>"#
    ),
};

static MORE: Icon = Icon {
    name: "slides-more",
    view: 24.0,
    body: concat!(
        r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
        r#"<path d="M12 5v14M5 12h14"></path></g>"#
    ),
};

const BAR_MONO: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono).lh(15.0);
const PANE_LABEL: TextStyle = TextStyle::new(10.0, 600.0).lh(13.0).tracking(0.7);
const THUMB_TITLE: TextStyle = TextStyle::new(8.5, 700.0).lh(10.0);
const THUMB_TEXT: TextStyle = TextStyle::new(5.5, 400.0).lh(7.0);
const ZOOM_STEP: u16 = 25;
const ZOOM_MIN: u16 = 50;
const ZOOM_MAX: u16 = 125;

/// The filmstrip (board: 190 wide, thumbnails 140 x 79 beside their number) and the stage's slide at 100 %
/// (the board's `SlideCanvas`: 640 x 360).
const FILMSTRIP_W: f32 = 190.0;
const THUMB_W: f32 = 140.0;
const THUMB_H: f32 = 79.0;
/// The gap between two filmstrip rows, a row's pitch, and the strip's height before its first layout.
const STRIP_GAP: u32 = 10;
const STRIP_PITCH: u32 = THUMB_H as u32 + STRIP_GAP;
const STRIP_H_GUESS: u32 = 560;
const SLIDE_W: f32 = 640.0;
const SLIDE_H: f32 = 360.0;
/// A slide's size in points until the source says: PowerPoint's 16:9 (13.333 by 7.5 inches).
pub const PAGE_PT: (u32, u32) = (960, 540);

/// The board's `.mk-ph5`: a sunset, three stops at 180 degrees.
const SUNSET: [(f32, &str); 3] = [(0.0, "#f3d9a8"), (0.5, "#d58c6a"), (1.0, "#3f3a58")];

fn sunset(block: El) -> El {
    let stop = |hex: &str| rune::color::hex(hex).unwrap_or(Color::BLACK);
    block.linear_gradient(
        180.0_f32.to_radians(),
        [
            (SUNSET[0].0, stop(SUNSET[0].1)),
            (SUNSET[1].0, stop(SUNSET[1].1)),
            (SUNSET[2].0, stop(SUNSET[2].1)),
        ],
    )
}

/// Present's glyph on the bar: the eye (the board's `Btn icon="eye"`), and its border's width.
fn present_icon() -> &'static Icon {
    &EYE
}
const PRESENT_BORDER_W: f32 = 1.0;

fn with_deck(act: impl Fn(&mut DeckBody) + 'static) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        if let Some(deck) = viewers::deck_mut(s) {
            act(deck);
        }
    }
}

fn step_button(key: &'static str, label: &'static str, icon: &'static Icon, act: fn(&mut DeckBody), t: &Theme) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .size(24.0, 24.0)
        .center()
        .radius(6.0)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .on_press(with_deck(act))
        .child(kit::glyph(format!("{key}-icon"), icon, 13.0, t.text_secondary))
}

/// Open in another app...: the file must be kept on this PC; if it is not, the press says so and does nothing else.
fn open_elsewhere(s: &mut State) {
    if let Some(ui) = viewers::ui(s) {
        if ui.file.details.offline {
            ui.intent(Intent::OpenInAnotherApp);
        } else {
            ui.toast = Some(KEEP_FIRST.to_string());
        }
    }
}

/// The bar under the window's toolbar (the board's `OvBar present`): zoom, then Present (bordered, the eye and its
/// key) and the way to another app.
fn bar(deck: &DeckBody, t: &Theme) -> El {
    let zoom = format!("{}%", deck.view.zoom);
    row()
        .key("vbar")
        .role("toolbar")
        .label("Presentation")
        .w_full()
        .h(38.0)
        .px(12.0)
        .gap(12.0)
        .center_y()
        .fill(t.card)
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(
            row()
                .key("vbar-zoom")
                .role("group")
                .label(format!("Zoom, {} per cent", deck.view.zoom))
                .gap(4.0)
                .center_y()
                .child(step_button("zoom-less", "Zoom out", &LESS, DeckBody::zoom_out, t))
                .child(
                    kit::label("zoom", zoom.clone(), BAR_MONO, t.text_secondary)
                        .w(rune::text::advance(&zoom, BAR_MONO)),
                )
                .child(step_button("zoom-more", "Zoom in", &MORE, DeckBody::zoom_in, t)),
        )
        .child(el().key("vbar-gap").grow(1.0))
        .child(
            row()
                .key("present")
                .role("button")
                .label(PRESENT_LABEL)
                .shortcut_hint("F5")
                .h(28.0)
                .px(10.0)
                .gap(6.0)
                .center_y()
                .radius(7.0)
                .border(PRESENT_BORDER_W, t.card_border)
                .border_layout()
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .on_press(with_deck(DeckBody::present))
                .child(kit::glyph("present-icon", present_icon(), 14.0, t.text_secondary))
                .child(kit::label("present-label", PRESENT_LABEL, BAR_LABEL, t.text_secondary))
                .child(kit::label("present-hint", PRESENT_HINT, BAR_HINT, t.text_quaternary).ml(4.0)),
        )
        .child(
            row()
                .key("elsewhere")
                .role("button")
                .label(OPEN_ELSEWHERE)
                .h(28.0)
                .px(10.0)
                .center_y()
                .radius(7.0)
                .fill(t.paper)
                .border(1.0, t.card_border)
                .border_layout()
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .on_press(open_elsewhere)
                .child(kit::label(
                    "elsewhere-label",
                    OPEN_ELSEWHERE,
                    BAR_LABEL,
                    t.text_secondary,
                )),
        )
}

fn pixels_of(page: Option<SlidePage>) -> Option<Pixels> {
    page.and_then(|p| p.pixels)
}

/// The stage's slide: white, with the source's pixels when it has them.
fn picture(key: String, label: String, w: f32, h: f32, pixels: Option<Pixels>) -> El {
    let frame = el()
        .key(key)
        .role("image")
        .label(label)
        .size(w, h)
        .radius(4.0)
        .fill(Color::WHITE);
    match pixels {
        Some(p) => frame.pixels_fit(&p, SurfaceFit::Fill),
        None => frame,
    }
}

/// A filmstrip thumbnail: the slide itself, small (board: its title, sub, points and the picture strip, white,
/// radius 4, padding 8). A slide with no words yet is the source's pixels, or a blank slide while it renders.
fn thumbnail(i: usize, n: u32, page: Option<SlidePage>, current: bool, t: &Theme) -> El {
    let frame = col()
        .key(format!("fs-{i}-slide"))
        .role("image")
        .label(format!("Slide {n}"))
        .size(THUMB_W, THUMB_H)
        .padding(8.0)
        .gap(2.0)
        .radius(4.0)
        .clip_rounded(4.0)
        .fill(Color::WHITE);
    // The current slide wears the board's 2 px dark ring as a real border (a keyed, gated edge), inside the 8 px
    // padding so the words do not move between the current slide and the others.
    let frame = if current {
        frame
            .border(2.0, t.text)
            .border_layout()
            .box_shadow(kit::drop(1.0, 4.0, Color::BLACK.alpha(0.14)))
    } else {
        frame.box_shadow(kit::drop(1.0, 4.0, Color::BLACK.alpha(0.14)))
    };
    let page = page.unwrap_or_default();
    let Some(words) = page.words else {
        return match page.pixels {
            Some(p) => frame.pixels_fit(&p, SurfaceFit::Fill),
            None => frame,
        };
    };
    let mut frame = frame.child(kit::label(format!("fs-{i}-title"), words.title.clone(), THUMB_TITLE, ink()).w_full());
    if !words.sub.is_empty() {
        frame =
            frame.child(kit::label(format!("fs-{i}-sub"), words.sub.clone(), THUMB_TEXT, ink().alpha(0.65)).w_full());
    }
    for (k, point) in words.points.iter().enumerate() {
        frame =
            frame.child(kit::label(format!("fs-{i}-li-{k}"), format!("\u{2022} {point}"), THUMB_TEXT, ink()).w_full());
    }
    if words.picture {
        frame = frame.child(sunset(
            el().key(format!("fs-{i}-image"))
                .w_full()
                .grow(1.0)
                .min_h(14.0)
                .mt(2.0)
                .radius(2.0),
        ));
    }
    frame
}

fn filmstrip(deck: &DeckBody, page: &dyn Fn(u32) -> Option<SlidePage>, editing: bool, t: &Theme) -> El {
    let rows = deck.strip_rows();
    let first = *rows.start();
    let mut list = col()
        .key("fs-rows")
        .role("list")
        .label("Slides")
        .w_full()
        .gap(STRIP_GAP as f32);
    for (i, thumb) in deck.view.slides.iter().enumerate().skip(first).take(rows.count()) {
        let n = thumb.n;
        let current = n == deck.view.current;
        let mut line = row()
            .key(format!("fs-{i}"))
            .role("button")
            .label(format!("Slide {n}"))
            .checked(current)
            .gap(8.0)
            .items_start()
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut State| edit_slide::change_slide(s, n))
            .child(
                el().key(format!("fs-{i}-num-box"))
                    .pt(1.0)
                    .child(kit::label(format!("fs-{i}-num"), n.to_string(), NUM, t.text_tertiary).w(12.0)),
            )
            .child(thumbnail(i, n, page(n), current, t));
        if editing {
            // Right-click (or the Menu key): Duplicate, Delete, Move up, Move down; Ctrl+M and Delete work here too.
            line = line
                .on_secondary_press(move |s: &mut State, p: rune::PointerPress, tx: &mut rune::Tx| {
                    strip::open_menu(s, n, (p.x, p.y), tx)
                })
                .on_key(strip::row_key);
        }
        list = list.child(line);
    }
    // The rows drawn are a window of the deck: they start where slide `first` stands, less the scroll offset.
    list = list.offset(0.0, (first as u32 * STRIP_PITCH) as f32 - deck.strip_y as f32);
    col()
        .key("filmstrip")
        .w(FILMSTRIP_W)
        .h_full()
        .min_h(0.0)
        .py(12.0)
        .px(12.0)
        .gap(10.0)
        .fill(t.sidebar_bg)
        .border_sides([0.0, kit::HAIR, 0.0, 0.0], t.separator)
        .border_layout()
        .child(row().key("fs-head").w_full().px(4.0).center_y().child(kit::label(
            "fs-title",
            "SLIDES",
            PANE_LABEL,
            t.text_tertiary,
        )))
        .child(
            col()
                .key("fs-list")
                .w_full()
                .instant_changes()
                .grow(1.0)
                .min_h(0.0)
                .clip_rounded(0.0)
                .on_wheel(strip_wheel)
                .on_layout_tx(|s: &mut State, r: [f32; 4], tx: &mut rune::Tx| {
                    if !viewers::deck_mut(s).is_some_and(|d| d.set_strip_h(r[3])) {
                        tx.live_only();
                    }
                })
                .child(list),
        )
}

/// The strip's wheel handler: [`DeckBody::strip_wheel`] on the open deck.
fn strip_wheel(s: &mut State, ev: rune::WheelEvent, _: &mut rune::Tx) -> bool {
    viewers::deck_mut(s).is_some_and(|deck| deck.strip_wheel(&ev))
}

/// The stage's slide in layout points at the deck's zoom.
fn stage_size(deck: &DeckBody) -> (f32, f32) {
    let zoom = f32::from(deck.view.zoom) / 100.0;
    (SLIDE_W * zoom, SLIDE_H * zoom)
}

fn stage(deck: &DeckBody, page: &dyn Fn(u32) -> Option<SlidePage>, edit: Option<&EditUi>, t: &Theme) -> El {
    let n = deck.view.current;
    let (w, h) = stage_size(deck);
    let slide = picture("slide".into(), format!("Slide {n}"), w, h, pixels_of(page(n))).box_shadow(kit::drop(
        4.0,
        18.0,
        Color::BLACK.alpha(0.16),
    ));
    // Editing: the slide is a focusable surface (keys and pointer are `edit::slide`'s) with the caret, the selected
    // text and the selected shape drawn over the helper's picture.
    let slide = match edit {
        None => slide,
        Some(edit) => el()
            .key(edit_slide::FOCUS_KEY)
            .role("group")
            .label(format!("Slide {n}, editing"))
            .size(w, h)
            .focusable(true)
            .on_key(|s: &mut State, ev: &rune::KeyEvent, tx: &mut rune::Tx| edit_slide::on_key(s, tx, ev))
            .on_drag(edit_slide::on_drag)
            .child(slide)
            .children(edit_slide::overlay(deck, edit, (w, h))),
    };
    row()
        .key("stage")
        .role("group")
        .label("Slide")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .center()
        .fill(t.field)
        .on_wheel(wheel_steps)
        .child(slide)
}

/// The slide's wheel handler: [`DeckBody::wheel_step`] on the open deck. A shape selected on the slide it left is let
/// go (while editing).
fn wheel_steps(s: &mut State, ev: rune::WheelEvent, _: &mut rune::Tx) -> bool {
    let Some(deck) = viewers::deck_mut(s) else { return false };
    let before = deck.view.current;
    let took = deck.wheel_step(&ev);
    if took && deck.view.current != before && edit_slide::live(s).is_some() {
        edit_slide::send(s, EditOp::ClearShape);
    }
    took
}

/// The status line: "Slide 2 of 3" at the left; Encrypted, a dot and About (a link to the viewer's About) at the right.
/// While editing the line also says how the deck is kept ("saved as version 3", "unsaved changes"), `kept`.
fn status(deck: &DeckBody, kept: Option<&str>, t: &Theme) -> El {
    let words = format!("Slide {} of {}", deck.view.current, deck.view.count);
    let mut bar = row()
        .key("status-bar")
        .role("status")
        .w_full()
        .h(28.0)
        .px(16.0)
        .gap(7.0)
        .center_y()
        .fill(t.status_bar_bg)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .child(
            kit::label("st-0", words.clone(), type_::STATUS, t.text_tertiary)
                .w(rune::text::advance(&words, type_::STATUS)),
        );
    // The board's OvStatus: after the left words, `·` and the one quiet line.
    let mut n = 1;
    let mut said = String::new();
    if deck.view.quiet {
        bar = bar
            .child(kit::label("st-1", "\u{00B7}", type_::STATUS, t.text_tertiary))
            .child(
                kit::label("st-2", states::QUIET_LINE, type_::STATUS, t.text_tertiary)
                    .w(rune::text::advance(states::QUIET_LINE, type_::STATUS)),
            );
        n = 3;
        said = format!(". {}", states::QUIET_LINE);
    }
    let (enc, dot) = (format!("st-{n}"), format!("st-{}", n + 1));
    let mut next = n + 2;
    let kept_els = kept.map(|words| {
        let (kept_key, dot_key) = (format!("st-{next}"), format!("st-{}", next + 1));
        next += 2;
        (
            kit::label(kept_key, words.to_string(), type_::STATUS, t.text_tertiary)
                .w(rune::text::advance(words, type_::STATUS)),
            kit::label(dot_key, "\u{00B7}", type_::STATUS, t.text_tertiary),
        )
    });
    let about = format!("st-{next}");
    let kept_said = kept.map(|words| format!(". {words}")).unwrap_or_default();
    let mut bar = bar
        .label(format!("{words}{said}. {ENCRYPTED}{kept_said}"))
        .child(el().key("st-gap").grow(1.0))
        .child(
            row()
                .key(format!("{enc}-box"))
                .gap(4.0)
                .center_y()
                .child(kit::glyph(format!("{enc}-icon"), &icons::LOCK, 11.0, t.accent_deep))
                .child(
                    kit::label(enc, ENCRYPTED, STATUS_ENC, t.accent_deep).w(rune::text::advance(ENCRYPTED, STATUS_ENC)),
                ),
        )
        .child(kit::label(dot, "\u{00B7}", type_::STATUS, t.text_tertiary));
    if let Some((words, dot)) = kept_els {
        bar = bar.child(words).child(dot);
    }
    bar.child(
        el().key(format!("{about}-box"))
            .role("link")
            .label("About this viewer")
            .focusable(true)
            .on_press(|s: &mut State| {
                if let Some(ui) = viewers::ui(s) {
                    ui.office_about();
                }
            })
            .child(
                kit::label(about, ABOUT, type_::STATUS, t.text_tertiary)
                    .w(rune::text::advance(ABOUT, type_::STATUS))
                    .spans(vec![Span::new(0, ABOUT.len()).underline(t.text_tertiary)]),
            ),
    )
}

/// The window of DW-1509: the caption, the sidebar, and the deck beside it. `page(n)`: the source's answer for
/// slide `n`.
pub fn window(s: &State, cx: &mut Cx<State>, ui: &Ui, deck: &DeckBody, page: &dyn Fn(u32) -> Option<SlidePage>) -> El {
    window_with(s, cx, ui, deck, page, page)
}

/// The picture of a live slide as a [`SlidePage`]: its pixels only, never the fixture's words.
fn live_page(pixels: Option<Pixels>) -> Option<SlidePage> {
    pixels.map(|p| SlidePage {
        pixels: Some(p),
        words: None,
    })
}

/// The window of a deck the Office engine renders: the filmstrip draws and asks the thumbnails in view and one row each side, the
/// stage asks the slide on it at the stage's zoom.
pub fn window_live(s: &State, cx: &mut Cx<State>, ui: &Ui, deck: &DeckBody, live: &Live) -> El {
    let dpi = live.dpi;
    let thumb = move |n: u32| live_page(live.asks.get(OfficePicture::Thumb { slide: n - 1, dpi }));
    let zoom = deck.view.zoom;
    let stage = move |n: u32| {
        live_page(live.asks.get(OfficePicture::Page {
            page: n - 1,
            zoom_pct: zoom,
            dpi,
        }))
    };
    window_with(s, cx, ui, deck, &thumb, &stage)
}

fn window_with(
    s: &State,
    cx: &mut Cx<State>,
    ui: &Ui,
    deck: &DeckBody,
    thumb: &dyn Fn(u32) -> Option<SlidePage>,
    stage_page: &dyn Fn(u32) -> Option<SlidePage>,
) -> El {
    let t = tokens::theme(cx.system_dark());
    let main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(viewers::office::toolbar(s, ui, false, &t));
    let body = row()
        .key("viewer-row")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .child(filmstrip(deck, thumb, ui.office.edit.is_some(), &t))
        .child(stage(deck, stage_page, ui.office.edit.as_ref(), &t));
    let main = match ui.office.edit.as_ref() {
        // The editor's frame, composed here rather than by `edit::chrome` so the status line is the deck's (Slide x of
        // y, how it is kept), not the document's.
        Some(edit) => {
            let kept = kept_words(edit, ui.file.version);
            main.child(
                col()
                    .key("edit-chrome")
                    .grow(1.0)
                    .min_h(0.0)
                    .w_full()
                    .child(slide_ribbon::ribbon(ui, edit.unsaved(), &t))
                    .child(body)
                    .child(status(deck, Some(&kept), &t)),
            )
        }
        None => main.child(bar(deck, &t)).child(body).child(status(deck, None, &t)),
    };
    let menu = ui.office.edit.is_some().then(|| strip::menu_layer(deck, &t)).flatten();
    let window = col()
        .key("window")
        .role("window")
        .label(s.identity.main_window_title())
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption::caption_bar(s.identity.main_window_title(), &t, cx))
        .child(
            el().key("under-caption").grow(1.0).min_h(0.0).w_full().child(
                row()
                    .key("body")
                    .w_full()
                    .h_full()
                    .child(sidebar::sidebar(s, &t))
                    .child(main),
            ),
        );
    match menu {
        Some(menu) => window.child(menu),
        None => window,
    }
}

/// How the deck is kept, as the status line says it after Encrypted.
fn kept_words(edit: &EditUi, version: u32) -> String {
    if edit.stopped {
        "read-only for now".to_string()
    } else if edit.unsaved() {
        "unsaved changes".to_string()
    } else {
        format!("saved as version {version}")
    }
}

// ---------------------------------------------------------------------------
// Present (DW-1561)
// ---------------------------------------------------------------------------

/// The board's frame is 1278 x 798 at the client size and the slide 1100 x 619 on it. Rune has no full-screen scene
/// path that this file knows of: Present is drawn at the window's size (BRANCH-NOTES.md).
const PRES_W: f32 = 1100.0;
const PRES_H: f32 = 619.0;
const PRES_TITLE: TextStyle = TextStyle::new(44.0, 700.0).lh(50.6).tracking(-0.88);
const PRES_POINT: TextStyle = TextStyle::new(22.0, 400.0).lh(33.0);
const PRES_ESC: TextStyle = TextStyle::new(11.0, 500.0).family(Family::Mono).lh(14.5);
const PRES_COUNT: TextStyle = TextStyle::new(11.0, 500.0).family(Family::Mono).lh(14.0);
/// The slide's ink on its white (the board's `#090909`).
fn ink() -> Color {
    rune::color::hex("#090909").unwrap_or(Color::BLACK)
}

fn slide_of_words(words: Option<&SlideWords>) -> El {
    let mut slide = col()
        .key("pres-slide")
        .role("img")
        .size(PRES_W, PRES_H)
        .padding(40.0)
        .fill(Color::WHITE);
    let Some(words) = words else { return slide };
    slide = slide.child(kit::label("pres-0", words.title.clone(), PRES_TITLE, ink()).w_full());
    let mut points = col().key("pres-points").w_full().mt(20.0);
    for (i, point) in words.points.iter().enumerate() {
        points = points.child(
            row()
                .key(format!("pres-li-{i}-row"))
                .w_full()
                .child(
                    el().key(format!("pres-li-{i}-mark"))
                        .size(24.2, 33.0)
                        .child(el().absolute(9.0, 13.0).size(7.0, 7.0).radius(3.5).fill(ink())),
                )
                .child(kit::label(format!("pres-li-{i}"), point.clone(), PRES_POINT, ink()).grow(1.0)),
        );
    }
    slide = slide.child(points);
    if words.picture {
        slide = slide.child(sunset(
            el().key("pres-image")
                .role("img")
                .label("Picture on the slide")
                .w_full()
                .grow(1.0)
                .mt(26.0)
                .radius(4.0),
        ));
    }
    slide
}

/// Present: the slide on black, `esc` at the top right (it says "Back to the slides" to a screen reader), the
/// count at the bottom right. Dark and light are the same: the slide is a page.
pub fn present(cx: &mut Cx<State>, deck: &DeckBody, page: &dyn Fn(u32) -> Option<SlidePage>) -> El {
    let words = page(deck.view.current).and_then(|p| p.words);
    present_of(cx, deck, slide_of_words(words.as_ref()))
}

/// The zoom, in per cent of the slide's own size, at which a slide fits a screen of `screen` points (16:9 slides, as
/// the board's: [`SLIDE_W`] by [`SLIDE_H`] at 100 %).
pub fn present_zoom(screen: (f32, f32)) -> u16 {
    let fit = (screen.0 / SLIDE_W).min(screen.1 / SLIDE_H) * 100.0;
    fit.floor().clamp(1.0, f32::from(u16::MAX)) as u16
}

/// Present for a deck the Office engine renders: the slide's own pixels asked at the screen's size, never the
/// fixture's words. While the picture comes the screen is black.
pub fn present_live(cx: &mut Cx<State>, deck: &DeckBody, live: &Live) -> El {
    let zoom = present_zoom(live.screen);
    let pixels = live.asks.get(OfficePicture::Page {
        page: deck.view.current - 1,
        zoom_pct: zoom,
        dpi: live.dpi,
    });
    let slide = match pixels {
        Some(p) => {
            let (w, h) = (SLIDE_W * f32::from(zoom) / 100.0, SLIDE_H * f32::from(zoom) / 100.0);
            el().key("pres-picture")
                .role("image")
                .label(format!("Slide {}", deck.view.current))
                .size(w, h)
                .pixels_fit(&p, SurfaceFit::Fill)
        }
        None => el().key("pres-picture").size(0.0, 0.0),
    };
    present_of(cx, deck, slide)
}

/// Whether the open file is a deck in Present: the shell keeps the screen out of captures and shares meanwhile.
pub fn presenting(s: &State) -> bool {
    matches!(
        s.viewer.ui.as_ref().map(|ui| &ui.file.body),
        Some(Body::Deck(deck)) if deck.presenting
    )
}

fn present_of(cx: &mut Cx<State>, deck: &DeckBody, centre: El) -> El {
    let t = tokens::theme(cx.system_dark());
    col()
        .key("pres-stage")
        .role("group")
        .label("Presenting")
        .w_full()
        .h_full()
        .fill(Color::BLACK)
        .on_wheel(wheel_steps)
        .child(
            el().key("pres-centre")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .center()
                .child(centre),
        )
        .child(
            row()
                .key("pres-top")
                .absolute(0.0, 12.0)
                .w_full()
                .child(el().key("pres-top-gap").grow(1.0))
                .child(
                    row()
                        .key("pres-esc-box")
                        .role("button")
                        .label(BACK_TO_SLIDES)
                        .shortcut_hint("Esc")
                        .h(20.0)
                        .px(6.0)
                        .center_y()
                        .radius(5.0)
                        .fill(t.card)
                        .focusable(true)
                        .on_press(with_deck(DeckBody::leave))
                        .child(
                            kit::label("pres-esc", "esc", PRES_ESC, t.text_tertiary)
                                .w(rune::text::advance("esc", PRES_ESC)),
                        ),
                )
                .child(el().key("pres-top-inset").w(16.2)),
        )
        .child(
            row()
                .key("pres-bottom")
                .absolute(0.0, 772.0)
                .w_full()
                .child(el().key("pres-bottom-gap").grow(1.0))
                .child({
                    let count = format!("{} / {}", deck.view.current, deck.view.count);
                    kit::label("pres-count", count.clone(), PRES_COUNT, Color::WHITE.alpha(0.55))
                        .w(rune::text::advance(&count, PRES_COUNT))
                })
                .child(el().key("pres-bottom-inset").w(20.0)),
        )
}

// ---------------------------------------------------------------------------
// Tests (they run at the checkpoint)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use rune::{Env, Runtime};

    use super::*;
    use crate::{init, view};

    fn runtime_of(state: State, dark: bool) -> Runtime<State> {
        let env = Env {
            system_dark: dark,
            scale: 2.0,
        };
        let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
        rt.on_key = Some(crate::shell::host::on_key);
        rt.poll = Some(crate::shell::host::poll);
        rt.frame(0.0);
        rt
    }

    fn scene(f: fn(&mut State), dark: bool) -> Runtime<State> {
        let mut state = init();
        f(&mut state);
        runtime_of(state, dark)
    }

    fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }

    fn words(rt: &Runtime<State>, key: &str) -> String {
        find(rt, key)
            .unwrap_or_else(|| panic!("no element keyed {key}"))
            .text
            .unwrap_or_default()
    }

    fn deck_state(rt: &mut Runtime<State>) -> (u32, bool) {
        let deck = viewers::deck_mut(&mut rt.state).expect("a deck is open");
        (deck.view.current, deck.presenting)
    }

    fn press(rt: &mut Runtime<State>, key: Key) {
        rt.key(rune::KeyEvent {
            key,
            mods: Mods::default(),
        });
        rt.frame(0.0);
    }

    /// The scene fns leave the state the frame names: DW-1509 on slide 2 of 3, not presenting; DW-1561 the same
    /// slide, presenting.
    #[test]
    fn office_slides_scenes_set_their_state() {
        let mut rt = scene(scene_deck, false);
        assert_eq!(deck_state(&mut rt), (slides_fixture::BOARD_SLIDE, false));
        assert!(find(&rt, "filmstrip").is_some() && find(&rt, "pres-stage").is_none());
        let mut rt = scene(scene_present, false);
        assert_eq!(deck_state(&mut rt), (slides_fixture::BOARD_SLIDE, true));
        assert!(find(&rt, "pres-stage").is_some() && find(&rt, "filmstrip").is_none());
        // DW-1560 is a 0.9.1 frame: still the placeholder.
        assert!(
            FRAMES
                .iter()
                .any(|(id, f)| *id == "DW-1560" && *f as usize == placeholder::scene as usize)
        );
    }

    /// The viewer has no edit control: nothing keyed or labelled New slide, Duplicate, Delete, Save, Undo, Redo,
    /// Bold, Insert, Design, Transitions, and no ribbon.
    #[test]
    fn office_slides_viewer_has_no_edit_control() {
        for f in [scene_deck as fn(&mut State), scene_present] {
            let rt = scene(f, false);
            for e in rt.elements() {
                let label = e.label.clone().unwrap_or_default().to_lowercase();
                let text = e.text.clone().unwrap_or_default().to_lowercase();
                let key = e.key.clone().unwrap_or_default();
                for banned in [
                    "new slide",
                    "duplicate",
                    "delete",
                    "save",
                    "undo",
                    "redo",
                    "bold",
                    "insert",
                    "design",
                    "transitions",
                ] {
                    assert!(!label.contains(banned) && !text.contains(banned), "{banned} on {key}");
                }
                assert!(!key.starts_with("rib") && !key.starts_with("rt-"), "ribbon key {key}");
            }
        }
        // Keys that would edit reach nothing: Ctrl+M, Ctrl+D, Delete leave the deck as it was.
        let mut rt = scene(scene_deck, false);
        for key in [Key::Command('m'), Key::Command('d'), Key::Delete] {
            press(&mut rt, key);
        }
        assert_eq!(deck_state(&mut rt), (slides_fixture::BOARD_SLIDE, false));
    }

    /// The board's words: Present and its key, "Slide 2 of 3", Encrypted; in Present "esc" and "2 / 3".
    #[test]
    fn office_slides_words_from_the_board() {
        let rt = scene(scene_deck, false);
        assert_eq!(words(&rt, "present-label"), "Present");
        assert_eq!(words(&rt, "present-hint"), "F5");
        assert_eq!(words(&rt, "st-0"), "Slide 2 of 3");
        assert_eq!(words(&rt, "st-1"), "Encrypted");
        let rt = scene(scene_present, false);
        assert_eq!(words(&rt, "pres-esc"), "esc");
        assert_eq!(words(&rt, "pres-count"), "2 / 3");
        assert_eq!(words(&rt, "pres-0"), "What we shot");
        assert_eq!(words(&rt, "pres-li-1"), "60 edited selects for the hotel");
        assert_eq!(
            find(&rt, "pres-esc-box").and_then(|e| e.label),
            Some(BACK_TO_SLIDES.to_string())
        );
    }

    /// Next steps on, previous steps back, Esc returns to the slides on the slide that was showing; the ends hold.
    #[test]
    fn office_slides_present_steps_and_leaves() {
        let mut rt = scene(scene_present, false);
        press(&mut rt, Key::Right);
        assert_eq!(deck_state(&mut rt), (3, true));
        press(&mut rt, Key::Right);
        assert_eq!(deck_state(&mut rt), (3, true), "the last slide holds");
        press(&mut rt, Key::Left);
        press(&mut rt, Key::Left);
        press(&mut rt, Key::Left);
        assert_eq!(deck_state(&mut rt), (1, true), "the first slide holds");
        press(&mut rt, Key::Text(" ".into()));
        assert_eq!(deck_state(&mut rt), (2, true));
        assert_eq!(words(&rt, "pres-count"), "2 / 3");
        press(&mut rt, Key::Escape);
        assert_eq!(deck_state(&mut rt), (2, false));
        assert!(find(&rt, "filmstrip").is_some(), "Esc is back to DW-1509");
        // F5 presents again.
        press(&mut rt, Key::Function(5));
        assert_eq!(deck_state(&mut rt), (2, true));
    }

    /// Amber is the primary action's and the encryption state's: the toolbar's New (`new-btn`, `plus-icon`,
    /// `new-caret`, `new-label`) and the lock and word Encrypted (`st-1*`). Nothing of this file's elements besides.
    #[test]
    fn office_slides_no_amber_outside_primary() {
        for dark in [false, true] {
            for f in [scene_deck as fn(&mut State), scene_present] {
                let rt = scene(f, dark);
                let t = tokens::theme(dark);
                let amber = [t.accent, t.accent_deep];
                for e in rt.elements() {
                    let key = e.key.clone().unwrap_or_default();
                    let allowed = matches!(key.as_str(), "new-btn" | "plus-icon" | "new-caret" | "new-label")
                        || key.starts_with("st-1");
                    if allowed {
                        continue;
                    }
                    assert!(!amber.contains(&e.fill), "amber fill on {key}");
                    assert!(!e.text_color.is_some_and(|c| amber.contains(&c)), "amber text on {key}");
                }
            }
        }
    }

    /// `DeckBody` alone: the clamps (a model test with no window).
    #[test]
    fn office_slides_deck_clamps() {
        let mut d = DeckBody::new(DeckView::of(3));
        d.go(0);
        assert_eq!(d.view.current, 1);
        d.go(99);
        assert_eq!(d.view.current, 3);
        d.previous();
        d.previous();
        d.previous();
        assert_eq!(d.view.current, 1);
    }

    /// The filmstrip draws every slide: 1 and 3 hold their own title (never a blank white box), with the picture strip
    /// where the board has one.
    #[test]
    fn office_slides_thumbnails_draw_their_slides() {
        let rt = scene(scene_deck, false);
        assert_eq!(words(&rt, "fs-0-title"), "Lisbon trip recap");
        assert_eq!(words(&rt, "fs-1-title"), "What we shot");
        assert_eq!(words(&rt, "fs-2-title"), "Next steps");
        assert!(find(&rt, "fs-0-image").is_some() && find(&rt, "fs-1-image").is_some());
        assert!(find(&rt, "fs-2-image").is_none(), "slide 3 has no picture strip");
        assert_eq!(words(&rt, "fs-title"), "SLIDES");
    }

    /// The status line ends "Encrypted", a dot, "About".
    #[test]
    fn office_slides_status_bar_ends_with_about() {
        for dark in [false, true] {
            let rt = scene(scene_deck, dark);
            assert_eq!(words(&rt, "st-1"), "Encrypted");
            assert_eq!(words(&rt, "st-2"), "\u{00B7}");
            assert_eq!(words(&rt, "st-3"), "About");
            let (enc, dot, about) = (
                find(&rt, "st-1").unwrap(),
                find(&rt, "st-2").unwrap(),
                find(&rt, "st-3").unwrap(),
            );
            assert!(
                enc.rect[0] < dot.rect[0] && dot.rect[0] < about.rect[0],
                "left to right"
            );
        }
    }

    /// Present on the bar is a bordered button with the eye (not the play triangle) and its key.
    #[test]
    fn office_slides_present_button_is_bordered_with_the_eye() {
        assert_eq!(present_icon().name, "eye");
        assert!(PRESENT_BORDER_W > 0.0);
        let rt = scene(scene_deck, false);
        let button = find(&rt, "present").expect("the Present button");
        assert_eq!(button.rect[3], 28.0);
        assert!(find(&rt, "present-icon").is_some());
        assert_eq!(words(&rt, "present-hint"), "F5");
        assert!(find(&rt, "elsewhere").is_some() && find(&rt, "zoom").is_some());
    }

    /// DW-1509: "Open in another app…" is the outlined kind (paper fill with its border; the element info carries no
    /// border, as docs' test notes), and the current slide's thumbnail keeps the size of the others under its ring.
    #[test]
    fn office_slides_elsewhere_is_outlined_and_the_ring_keeps_the_size() {
        for dark in [false, true] {
            let rt = scene(scene_deck, dark);
            let t = tokens::theme(dark);
            assert_eq!(
                find(&rt, "elsewhere").expect("elsewhere").fill,
                t.paper,
                "outlined: paper fill"
            );
            let size = |k: &str| {
                let r = find(&rt, k).unwrap().rect;
                (r[2], r[3])
            };
            assert_eq!(
                size("fs-1-slide"),
                size("fs-0-slide"),
                "the ring does not grow the thumbnail"
            );
            assert_eq!(size("fs-1-slide"), (THUMB_W, THUMB_H));
        }
    }

    /// DW-1561's picture is the board's sunset (three stops, 180 degrees): the element carries a gradient, not a fill.
    #[test]
    fn office_slides_present_picture_is_a_gradient() {
        assert_eq!(SUNSET.len(), 3);
        assert_eq!(SUNSET[0].1, "#f3d9a8");
        for dark in [false, true] {
            let rt = scene(scene_present, dark);
            let image = find(&rt, "pres-image").expect("the picture on the slide");
            assert_eq!(image.fill.a, 0.0, "a gradient, not a flat fill");
        }
    }
}

#[cfg(test)]
#[path = "slides_live_tests.rs"]
mod live_tests;
