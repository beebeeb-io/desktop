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

use rune::tok::Family;
use rune::{Color, Cx, El, Icon, Key, Mods, Pixels, SurfaceFit, TextStyle, col, el, row};

use crate::State;
use crate::frames::placeholder;
use crate::frames::viewers::{self, Ui};
use crate::theme::icons;
use crate::theme::tokens::{self, Theme, type_};
use crate::ui::{caption, kit, sidebar};

#[path = "slides_fixture.rs"]
pub mod slides_fixture;

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
}

impl DeckView {
    /// A deck of `count` slides, on the first.
    pub fn of(count: u32) -> Self {
        DeckView {
            slides: (1..=count).map(|n| SlideThumb { n }).collect(),
            current: 1,
            count,
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
    pub points: Vec<String>,
    pub picture: bool,
}

/// What `viewers` holds for an open deck: the view and whether Present has the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeckBody {
    pub view: DeckView,
    pub presenting: bool,
}

impl DeckBody {
    pub fn new(view: DeckView) -> Self {
        DeckBody {
            view,
            presenting: false,
        }
    }

    /// Show slide `n` (clamped to the deck).
    pub fn go(&mut self, n: u32) {
        self.view.current = n.clamp(1, self.view.count.max(1));
    }

    pub fn next(&mut self) {
        self.go(self.view.current.saturating_add(1));
    }

    pub fn previous(&mut self) {
        self.go(self.view.current.saturating_sub(1));
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

/// A play triangle: Present's glyph.
static PLAY: Icon = Icon {
    name: "present-play",
    view: 24.0,
    body: concat!(
        r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
        r#"<path d="M7 4l13 8-13 8V4z"></path>"#,
        "</g>"
    ),
};

/// The filmstrip's width, a thumbnail's, and the stage's slide at 16:9.
const FILMSTRIP_W: f32 = 168.0;
const THUMB_W: f32 = 120.0;
const THUMB_H: f32 = 67.5;
const SLIDE_W: f32 = 760.0;
const SLIDE_H: f32 = 427.5;

fn with_deck(act: impl Fn(&mut DeckBody) + 'static) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        if let Some(deck) = viewers::deck_mut(s) {
            act(deck);
        }
    }
}

/// The bar under the window's toolbar: Present, the one thing the deck offers beyond reading it.
fn bar(t: &Theme) -> El {
    row()
        .key("vbar")
        .role("toolbar")
        .label("Presentation")
        .w_full()
        .h(38.0)
        .px(12.0)
        .center_y()
        .fill(t.card)
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(
            row()
                .key("present")
                .role("button")
                .label(PRESENT_LABEL)
                .shortcut_hint("F5")
                .h(28.0)
                .px(8.0)
                .gap(6.0)
                .center_y()
                .radius(7.0)
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .on_press(with_deck(DeckBody::present))
                .child(kit::glyph("present-icon", &PLAY, 15.0, t.text_secondary))
                .child(kit::label("present-label", PRESENT_LABEL, BAR_LABEL, t.text_secondary))
                .child(kit::label("present-hint", PRESENT_HINT, BAR_HINT, t.text_quaternary)),
        )
}

fn pixels_of(page: Option<SlidePage>) -> Option<Pixels> {
    page.and_then(|p| p.pixels)
}

/// One slide's picture frame: white, with the source's pixels when it has them.
fn picture(key: String, label: String, w: f32, h: f32, pixels: Option<Pixels>) -> El {
    let frame = el()
        .key(key)
        .role("image")
        .label(label)
        .size(w, h)
        .radius(2.0)
        .fill(Color::WHITE);
    match pixels {
        Some(p) => frame.pixels_fit(&p, SurfaceFit::Fill),
        None => frame,
    }
}

fn filmstrip(deck: &DeckBody, page: &dyn Fn(u32) -> Option<SlidePage>, t: &Theme) -> El {
    let mut list = col()
        .key("filmstrip")
        .role("list")
        .label("Slides")
        .w(FILMSTRIP_W)
        .h_full()
        .min_h(0.0)
        .py(16.0)
        .gap(12.0)
        .items_center()
        .fill(t.sidebar_bg)
        .border_sides([0.0, kit::HAIR, 0.0, 0.0], t.separator)
        .border_layout();
    for (i, thumb) in deck.view.slides.iter().enumerate() {
        let n = thumb.n;
        let current = n == deck.view.current;
        let slide = picture(
            format!("fs-{i}-slide"),
            format!("Slide {n}"),
            THUMB_W,
            THUMB_H,
            pixels_of(page(n)),
        );
        let slide = if current {
            slide.box_shadow(kit::ring(2.0, t.text))
        } else {
            slide.box_shadow(kit::drop(1.0, 4.0, Color::BLACK.alpha(0.14)))
        };
        list = list.child(
            col()
                .key(format!("fs-{i}"))
                .role("button")
                .label(format!("Slide {n}"))
                .checked(current)
                .items_center()
                .gap(4.0)
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .on_press(with_deck(move |d| d.go(n)))
                .child(slide)
                .child(kit::label(format!("fs-{i}-num"), n.to_string(), NUM, t.text_tertiary)),
        );
    }
    list
}

fn stage(deck: &DeckBody, page: &dyn Fn(u32) -> Option<SlidePage>, t: &Theme) -> El {
    let n = deck.view.current;
    let slide = picture(
        "slide".into(),
        format!("Slide {n}"),
        SLIDE_W,
        SLIDE_H,
        pixels_of(page(n)),
    )
    .box_shadow(kit::drop(4.0, 18.0, Color::BLACK.alpha(0.16)));
    row()
        .key("stage")
        .role("group")
        .label("Slide")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .center()
        .fill(t.field)
        .child(slide)
}

/// The status line: "Slide 2 of 3" at the left; Encrypted at the right.
fn status(deck: &DeckBody, t: &Theme) -> El {
    let words = format!("Slide {} of {}", deck.view.current, deck.view.count);
    row()
        .key("status-bar")
        .role("status")
        .label(format!("{words}. {ENCRYPTED}"))
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
        )
        .child(el().key("st-gap").grow(1.0))
        .child(
            row()
                .key("st-1-box")
                .gap(4.0)
                .center_y()
                .child(kit::glyph("st-1-icon", &icons::LOCK, 11.0, t.accent_deep))
                .child(
                    kit::label("st-1", ENCRYPTED, STATUS_ENC, t.accent_deep)
                        .w(rune::text::advance(ENCRYPTED, STATUS_ENC)),
                ),
        )
}

/// The window of DW-1509: the caption, the sidebar, and the deck beside it. `page(n)`: the source's answer for
/// slide `n`.
pub fn window(s: &State, cx: &mut Cx<State>, ui: &Ui, deck: &DeckBody, page: &dyn Fn(u32) -> Option<SlidePage>) -> El {
    let t = tokens::theme(cx.system_dark());
    let main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(viewers::office::toolbar(s, ui, false, &t))
        .child(bar(&t))
        .child(
            row()
                .key("viewer-row")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .child(filmstrip(deck, page, &t))
                .child(stage(deck, page, &t)),
        )
        .child(status(deck, &t));
    col()
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
        )
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
                        .child(el().absolute(6.0, 14.0).size(5.0, 5.0).radius(2.5).fill(ink())),
                )
                .child(kit::label(format!("pres-li-{i}"), point.clone(), PRES_POINT, ink()).grow(1.0)),
        );
    }
    slide = slide.child(points);
    if words.picture {
        slide = slide.child(
            el().key("pres-image")
                .role("img")
                .label("Picture on the slide")
                .w_full()
                .grow(1.0)
                .mt(26.0)
                .radius(6.0)
                .fill(Color::BLACK.alpha(0.06)),
        );
    }
    slide
}

/// Present: the slide on black, `esc` at the top right (it says "Back to the slides" to a screen reader), the
/// count at the bottom right. Dark and light are the same: the slide is a page.
pub fn present(cx: &mut Cx<State>, deck: &DeckBody, page: &dyn Fn(u32) -> Option<SlidePage>) -> El {
    let t = tokens::theme(cx.system_dark());
    let words = page(deck.view.current).and_then(|p| p.words);
    col()
        .key("pres-stage")
        .role("group")
        .label("Presenting")
        .w_full()
        .h_full()
        .fill(Color::BLACK)
        .child(
            el().key("pres-centre")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .center()
                .child(slide_of_words(words.as_ref())),
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
}
