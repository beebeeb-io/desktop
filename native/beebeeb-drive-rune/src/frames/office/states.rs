//! Slice OF6 (Office viewer): the viewer's states.
//!
//! - DW-1556: the viewer cannot show the file. One panel, ONE reason line under the title, what still works.
//! - DW-1563: the document is opening: the page's skeleton, nothing drawn as the document.
//! - DW-1564: part of the file cannot be drawn: one quiet line in the status bar, the rest is shown.
//!
//! Rules live in [`OfficeState`] (the host maps the engine's answer to it at O-CP3); the window only binds.
//! DW-1556's "THE REASON LINE IS ONE OF" card is a design note and is not drawn (board fix DW-1556).

use rune::{Color, Cx, El, TextStyle, col, el, row};

use crate::State;
use crate::frames::viewers::source::{Body, board};
use crate::frames::viewers::{self, Intent, Ui};
use crate::theme::icons;
use crate::theme::tokens::{self, Theme, type_};
use crate::ui::settings_dialog::{BUTTON_MD, DIALOG_TITLE};
use crate::ui::{caption, kit, sidebar};

// ---------------------------------------------------------------------------
// The rules
// ---------------------------------------------------------------------------

/// Why a file has no preview. Exactly one reason line shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// .ppt, .odp, password-protected.
    UnsupportedType,
    /// The file is damaged, or over a size limit of the engine.
    DamagedOrOverLimit,
    /// Too large to preview on this PC.
    TooLarge,
}

/// What the viewer is in. The host maps the engine's answer to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfficeState {
    /// The engine has not answered yet: the skeleton shows.
    Opening,
    /// Drawn.
    Ready,
    /// No preview for this file.
    NoPreview(Reason),
    /// The viewer stopped. `retried`: Try again was already pressed once.
    Stopped { retried: bool },
    /// Part of the file cannot be drawn; the rest is shown with one quiet line.
    ContentMissing,
}

pub const TITLE: &str = "We couldn't show this file here.";
pub const NOT_CHANGED: &str = "The file was not changed.";
pub const TRY_AGAIN: &str = "Try again";
pub const OPEN_ELSEWHERE: &str = "Open in another app\u{2026}";
pub const KEEP_FIRST: &str = "Keep this file on this PC first.";
pub const KEEP_ACTION: &str = "Keep on this PC";
pub const QUIET_LINE: &str = "Some content isn't shown.";
pub const OPENING: &str = "Opening\u{2026}";
pub const ENCRYPTED: &str = "Encrypted";
pub const ABOUT: &str = "About";

pub const REASON_TYPE: &str = "No preview for this type (.ppt, .odp, password-protected)";
pub const REASON_DAMAGED: &str = "Damaged or over a size limit";
pub const REASON_STOPPED: &str = "The viewer stopped";
pub const REASON_TOO_LARGE: &str = "Too large to preview here";

impl OfficeState {
    /// The one reason line of DW-1556; `None` where the state is not a failure.
    pub fn reason_line(self) -> Option<&'static str> {
        match self {
            OfficeState::NoPreview(Reason::UnsupportedType) => Some(REASON_TYPE),
            OfficeState::NoPreview(Reason::DamagedOrOverLimit) => Some(REASON_DAMAGED),
            OfficeState::NoPreview(Reason::TooLarge) => Some(REASON_TOO_LARGE),
            OfficeState::Stopped { .. } => Some(REASON_STOPPED),
            OfficeState::Opening | OfficeState::Ready | OfficeState::ContentMissing => None,
        }
    }

    /// Try again is offered after "The viewer stopped", once: a second failure hides it.
    pub fn try_again_shown(self) -> bool {
        matches!(self, OfficeState::Stopped { retried: false })
    }

    /// "Open in another app…" is offered only for a file kept on this PC: the viewer makes no temporary decrypted copy
    /// (decision 2482, Q8). Otherwise the panel says "Keep this file on this PC first." with that action.
    pub fn open_elsewhere_offered(self, kept: bool) -> bool {
        kept
    }

    /// The panel of DW-1556 shows for these states.
    pub fn is_failure(self) -> bool {
        self.reason_line().is_some()
    }

    /// The skeleton shows until the first page is drawn.
    pub fn is_opening(self) -> bool {
        self == OfficeState::Opening
    }

    /// The quiet line of DW-1564, in the status bar.
    pub fn quiet_line(self) -> Option<&'static str> {
        (self == OfficeState::ContentMissing).then_some(QUIET_LINE)
    }
}

/// The viewer's state as the open file holds it (`Body::OfficeState`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateBody {
    pub state: OfficeState,
    /// The file's bytes are kept on this PC (the host asks the source).
    pub kept: bool,
    /// How many times Try again was pressed on this open file.
    pub tries: u32,
}

impl StateBody {
    pub fn new(state: OfficeState, kept: bool) -> StateBody {
        StateBody { state, kept, tries: 0 }
    }

    /// Try again: only after a stop that was not retried yet. The file opens once more (`Opening`); the host answers.
    /// `true`: it was taken.
    pub fn try_again(&mut self) -> bool {
        if !self.state.try_again_shown() {
            return false;
        }
        self.tries += 1;
        self.state = OfficeState::Opening;
        true
    }

    /// The host's answer after a retry: it stopped again. Try again is hidden from now on.
    pub fn stopped_again(&mut self) {
        self.state = OfficeState::Stopped {
            retried: self.tries > 0,
        };
    }
}

fn state_mut(s: &mut State) -> Option<&mut StateBody> {
    match &mut viewers::ui(s)?.file.body {
        Body::OfficeState(body) => Some(body),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Scenes
// ---------------------------------------------------------------------------

/// Open the document, then hold the state `body` says.
fn scene_of(s: &mut State, body: StateBody) {
    if viewers::open_from(s, Box::new(viewers::source::FixtureViewerSource::new()), board::DOCX)
        && let Some(ui) = viewers::ui(s)
    {
        ui.file.body = Body::OfficeState(body);
    }
}

/// DW-1556: the viewer stopped, on a file kept on this PC (the board's panel: Try again and Open in another app).
pub fn scene_failed(s: &mut State) {
    scene_of(s, StateBody::new(OfficeState::Stopped { retried: false }, true));
}

/// DW-1563: the document is opening.
pub fn scene_opening(s: &mut State) {
    scene_of(s, StateBody::new(OfficeState::Opening, true));
}

/// DW-1564: the pages are shown, one quiet line in the status bar.
pub fn scene_quiet(s: &mut State) {
    scene_of(s, StateBody::new(OfficeState::ContentMissing, true));
}

/// The group's board frames and the scene that shows each (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] = &[("DW-1563", scene_opening), ("DW-1564", scene_quiet)];

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
const STATUS_STATE: TextStyle = TextStyle::new(11.5, 500.0).lh(17.0);
const STATUS_ENC: TextStyle = TextStyle::new(11.5, 600.0).lh(17.0);
const OUTLINE_LABEL: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);

const PAGE_W: f32 = 520.0;
const OUTLINE_W: f32 = 210.0;

/// A grey bar of the skeleton: `color-mix(ink 9%, ground)`.
fn bone(key: String, w: f32, h: f32, t: &Theme) -> El {
    el().key(key).w(w).h(h).radius(3.0).fill(t.text.alpha(0.09))
}

fn text_button(key: &str, words: &str, primary_ink: Color, outlined: bool, t: &Theme) -> El {
    let mut b = row()
        .key(key.to_string())
        .role("button")
        .label(words)
        .h(28.0)
        .px(12.0)
        .center()
        .radius(7.0)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .cursor(rune::Cursor::Pointer);
    if outlined {
        b = b.box_shadow(kit::ring(kit::HAIR, t.separator_strong)).fill(t.card);
    }
    b.child(kit::label(format!("{key}-label"), words, BUTTON_MD, primary_ink))
}

/// DW-1556's panel. Exactly one reason line; the actions that still work.
pub fn failure_panel(body: &StateBody, t: &Theme) -> El {
    let reason = body.state.reason_line().unwrap_or_default();
    let line = format!("{reason}. {NOT_CHANGED}");
    let mut actions = row().key("empty-actions").gap(8.0).mt(8.0).center_y();
    if body.state.try_again_shown() {
        actions = actions.child(
            text_button("empty-try", TRY_AGAIN, t.text, true, t).on_press(|s: &mut State| {
                if let Some(b) = state_mut(s) {
                    b.try_again();
                }
            }),
        );
    }
    if body.state.open_elsewhere_offered(body.kept) {
        actions = actions.child(
            text_button("empty-else", OPEN_ELSEWHERE, t.text_secondary, false, t).on_press(|s: &mut State| {
                if let Some(ui) = viewers::ui(s) {
                    ui.intent(Intent::OpenInAnotherApp);
                }
            }),
        );
    } else {
        actions = actions
            .child(kit::label("empty-keep-first", KEEP_FIRST, SUB, t.text_secondary))
            .child(
                text_button("empty-keep", KEEP_ACTION, t.text_secondary, false, t).on_press(|s: &mut State| {
                    if let Some(ui) = viewers::ui(s) {
                        ui.intent(Intent::KeepOffline);
                    }
                }),
            );
    }
    col()
        .key("empty")
        .role("alert")
        .py(28.0)
        .px(24.0)
        .gap(8.0)
        .items_center()
        .child(
            el().key("empty-tile")
                .size(52.0, 52.0)
                .radius(15.0)
                .fill(t.tile)
                .center()
                .child(kit::glyph("empty-icon", &viewers::icons::ALERT, 26.0, t.text_secondary)),
        )
        .child(
            kit::label("empty-title", TITLE, DIALOG_TITLE, t.text)
                .role("heading")
                .level(1),
        )
        .child(
            kit::label("empty-reason", line, SUB, t.text_secondary)
                .wrap()
                .w(360.0)
                .align(rune::Align::Center),
        )
        .child(actions)
}

/// The page's skeleton on grey, and the outline's: nothing is drawn as the document before the first page.
pub fn skeleton(t: &Theme) -> El {
    let mut outline = col()
        .key("outline-skeleton")
        .w(OUTLINE_W)
        .h_full()
        .py(12.0)
        .px(12.0)
        .gap(8.0)
        .fill(t.sidebar_bg)
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(kit::label("outline-label", "Outline", OUTLINE_LABEL, t.text_tertiary).px(8.0));
    for (i, pct) in [70.0_f32, 54.0, 62.0, 48.0].iter().enumerate() {
        outline = outline.child(bone(format!("ol-bone-{i}"), OUTLINE_W * 0.8 * pct / 100.0, 12.0, t).ml(8.0));
    }
    let inner = PAGE_W - 96.0;
    let mut page = col()
        .key("page-skeleton")
        .role("img")
        .label("The page is opening")
        .w(PAGE_W)
        .min_h(700.0)
        .px(48.0)
        .py(44.0)
        .gap(12.0)
        .fill(Color::WHITE)
        .child(bone("pg-bone-title".into(), inner * 0.46, 22.0, t))
        .child(bone("pg-bone-sub".into(), inner * 0.70, 10.0, t));
    for (i, pct) in [96.0_f32, 90.0, 94.0, 60.0, 0.0, 92.0, 97.0, 84.0, 40.0]
        .iter()
        .enumerate()
    {
        page = page.child(if *pct == 0.0 {
            el().key(format!("pg-gap-{i}")).h(8.0)
        } else {
            bone(format!("pg-bone-{i}"), inner * pct / 100.0, 10.0, t)
        });
    }
    row()
        .key("viewer-row")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .child(outline)
        .child(
            row()
                .key("stage")
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .pt(24.0)
                .justify_center()
                .fill(t.field)
                .child(page),
        )
}

/// The viewer's status bar: where you are, Encrypted, About. `quiet` is the one quiet line of DW-1564 (no banner, no
/// colour); `opening` shows the state word in place of the position.
pub fn status_bar(state: OfficeState, left: &[&str], t: &Theme) -> El {
    let plain = |key: String, words: &str| kit::label(key, words, type_::STATUS, t.text_tertiary);
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
        .border_layout();
    let mut said = String::new();
    if state.is_opening() {
        bar = bar
            .child(el().key("status-dot").size(7.0, 7.0).radius(3.5).fill(t.accent_deep))
            .child(kit::label("status-text", OPENING, STATUS_STATE, t.text_secondary));
        said.push_str(OPENING);
    } else {
        for (i, w) in left.iter().enumerate() {
            if i > 0 {
                bar = bar.child(plain(format!("st-dot-{i}"), "\u{00B7}"));
            }
            bar = bar.child(plain(format!("st-{i}"), w));
            said.push_str(w);
            said.push_str(", ");
        }
    }
    if let Some(quiet) = state.quiet_line() {
        bar = bar
            .child(plain("st-quiet-dot".into(), "\u{00B7}"))
            .child(plain("st-quiet".into(), quiet));
        said.push_str(quiet);
    }
    bar.label(format!("{said}. {ENCRYPTED}"))
        .child(el().key("st-gap").grow(1.0))
        .child(
            row()
                .key("st-enc-box")
                .gap(4.0)
                .center_y()
                .child(kit::glyph("st-enc-icon", &icons::LOCK, 11.0, t.accent_deep))
                .child(kit::label("st-enc", ENCRYPTED, STATUS_ENC, t.accent_deep)),
        )
        .child(plain("st-about-dot".into(), "\u{00B7}"))
        .child(
            el().key("st-about-box")
                .role("link")
                .label("About this viewer")
                .focusable(true)
                .on_press(|s: &mut State| {
                    if let Some(ui) = viewers::ui(s) {
                        ui.office_about();
                    }
                })
                .child(plain("st-about".into(), ABOUT)),
        )
}

/// The page of DW-1564 until OF5's docs window hosts it: the pages that can be drawn come from the source; this holds
/// their place as a plain white page (BRANCH-NOTES, "Integration gap").
fn page_of_words(t: &Theme) -> El {
    el().key("stage")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .pt(24.0)
        .fill(t.field)
        .child(
            col()
                .key("page")
                .role("img")
                .label("Page 1 of 4")
                .w(PAGE_W)
                .min_h(700.0)
                .fill(Color::WHITE),
        )
}

/// The window of the three states: the caption, the sidebar, the window's toolbar, the body, the status bar.
pub fn window(s: &State, cx: &mut Cx<State>, ui: &Ui, body: &StateBody) -> El {
    let t = tokens::theme(cx.system_dark());
    let title = s.identity.main_window_title();
    let state = body.state;
    let middle = if state.is_failure() {
        col()
            .key("stage")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .center()
            .fill(t.field)
            .child(failure_panel(body, &t))
    } else if state.is_opening() {
        skeleton(&t)
    } else {
        row()
            .key("viewer-row")
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .child(page_of_words(&t))
    };
    let left: &[&str] = if state.is_failure() || state.is_opening() {
        &[]
    } else {
        &["Page 1 of 4", "1,284 words", "English (UK)"]
    };
    let main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(viewers::office::toolbar(s, ui, false, &t))
        .child(middle)
        .child(status_bar(state, left, &t));
    col()
        .key("window")
        .role("window")
        .label(title)
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption::caption_bar(title, &t, cx))
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

#[cfg(test)]
mod tests {
    use rune::{Env, Runtime};

    use super::*;
    use crate::{init, view};

    fn runtime_of(state: State) -> Runtime<State> {
        let env = Env {
            system_dark: false,
            scale: 2.0,
        };
        let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
        rt.on_key = Some(crate::shell::host::on_key);
        rt.poll = Some(crate::shell::host::poll);
        rt.frame(0.0);
        rt
    }

    /// A runtime that shows `body` in the viewer.
    fn showing(body: StateBody) -> Runtime<State> {
        let mut state = init();
        scene_of(&mut state, body);
        runtime_of(state)
    }

    fn keyed(rt: &Runtime<State>, key: &str) -> bool {
        rt.elements()
            .into_iter()
            .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }

    const ALL_REASONS: [OfficeState; 4] = [
        OfficeState::NoPreview(Reason::UnsupportedType),
        OfficeState::NoPreview(Reason::DamagedOrOverLimit),
        OfficeState::NoPreview(Reason::TooLarge),
        OfficeState::Stopped { retried: false },
    ];

    #[test]
    fn office_states_each_reason_has_its_line() {
        let lines: Vec<&str> = ALL_REASONS.iter().map(|s| s.reason_line().expect("a reason")).collect();
        assert_eq!(
            lines,
            [REASON_TYPE, REASON_DAMAGED, REASON_TOO_LARGE, REASON_STOPPED],
            "each failure has its own line"
        );
        for i in 0..lines.len() {
            for j in 0..i {
                assert_ne!(lines[i], lines[j], "lines are distinct");
            }
        }
        for s in [OfficeState::Opening, OfficeState::Ready, OfficeState::ContentMissing] {
            assert_eq!(s.reason_line(), None, "{s:?} is not a failure");
        }
        // The window draws exactly one reason line for each failure, and none for the other states.
        for s in ALL_REASONS {
            let rt = showing(StateBody::new(s, true));
            assert!(keyed(&rt, "empty-reason"), "{s:?} draws its reason line");
            assert!(keyed(&rt, "empty-title"));
        }
        assert!(!keyed(
            &showing(StateBody::new(OfficeState::Opening, true)),
            "empty-reason"
        ));
    }

    #[test]
    fn office_states_try_again_only_once() {
        assert!(OfficeState::Stopped { retried: false }.try_again_shown());
        assert!(!OfficeState::Stopped { retried: true }.try_again_shown());
        for s in [
            OfficeState::Opening,
            OfficeState::Ready,
            OfficeState::ContentMissing,
            OfficeState::NoPreview(Reason::UnsupportedType),
            OfficeState::NoPreview(Reason::DamagedOrOverLimit),
            OfficeState::NoPreview(Reason::TooLarge),
        ] {
            assert!(!s.try_again_shown(), "{s:?} offers no Try again");
        }
        let mut b = StateBody::new(OfficeState::Stopped { retried: false }, true);
        assert!(b.try_again(), "the first press is taken");
        assert_eq!(b.state, OfficeState::Opening);
        b.stopped_again();
        assert_eq!(b.state, OfficeState::Stopped { retried: true });
        assert!(!b.try_again(), "a second failure hides it");
        assert!(keyed(
            &showing(StateBody::new(OfficeState::Stopped { retried: false }, true)),
            "empty-try"
        ));
        assert!(!keyed(&showing(b), "empty-try"));
    }

    #[test]
    fn office_states_open_elsewhere_only_when_kept() {
        let s = OfficeState::Stopped { retried: false };
        assert!(s.open_elsewhere_offered(true));
        assert!(!s.open_elsewhere_offered(false), "not kept: not offered");
        let kept = showing(StateBody::new(s, true));
        assert!(keyed(&kept, "empty-else") && !keyed(&kept, "empty-keep-first"));
        let not_kept = showing(StateBody::new(s, false));
        assert!(
            !keyed(&not_kept, "empty-else"),
            "no Open in another app for a file not kept"
        );
        assert!(keyed(&not_kept, "empty-keep-first") && keyed(&not_kept, "empty-keep"));
    }

    #[test]
    fn office_states_opening_draws_no_document() {
        let rt = showing(StateBody::new(OfficeState::Opening, true));
        assert!(keyed(&rt, "page-skeleton") && keyed(&rt, "ol-bone-0"));
        assert!(keyed(&rt, "status-text"), "the status bar says Opening");
        assert!(!keyed(&rt, "st-0"), "no position until the first page is drawn");
        assert!(!keyed(&rt, "page"), "no page is drawn as the document");
    }

    #[test]
    fn office_states_quiet_line_only_for_missing_content() {
        assert_eq!(OfficeState::ContentMissing.quiet_line(), Some(QUIET_LINE));
        for s in [
            OfficeState::Opening,
            OfficeState::Ready,
            OfficeState::Stopped { retried: false },
        ] {
            assert_eq!(s.quiet_line(), None);
        }
        assert!(keyed(
            &showing(StateBody::new(OfficeState::ContentMissing, true)),
            "st-quiet"
        ));
        assert!(!keyed(&showing(StateBody::new(OfficeState::Ready, true)), "st-quiet"));
    }

    #[test]
    fn office_states_scenes_set_their_state() {
        let ids: Vec<&str> = FRAMES.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, ["DW-1563", "DW-1564"]);
        let mut s = init();
        scene_opening(&mut s);
        assert_eq!(state_mut(&mut s).map(|b| b.state), Some(OfficeState::Opening));
        scene_quiet(&mut s);
        assert_eq!(state_mut(&mut s).map(|b| b.state), Some(OfficeState::ContentMissing));
        scene_failed(&mut s);
        assert_eq!(
            state_mut(&mut s).map(|b| b.state),
            Some(OfficeState::Stopped { retried: false })
        );
    }
}
