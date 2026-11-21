//! Slice OF6 (Office viewer): the viewer's states.
//!
//! - DW-1556: the viewer cannot show the file. One panel, ONE reason line under the title, what still works.
//! - DW-1563: the document is opening: the page's skeleton, nothing drawn as the document.
//! - DW-1564: part of the file cannot be drawn: one quiet line in the status bar, the rest is shown.
//!
//! Rules live in [`OfficeState`] (the host maps the engine's answer to it at O-CP3); the window only binds.
//! DW-1556's "THE REASON LINE IS ONE OF" card is a design note and is not drawn (board fix DW-1556).

use rune::tok::Family;
use rune::{Color, Cx, El, Icon, TextStyle, col, el, row};

use crate::State;
use crate::frames::office::docs::docs_fixture;
use crate::frames::viewers::source::Body;
use crate::frames::viewers::{self, Intent, Ui};
use crate::theme::icons;
use crate::theme::tokens::{self, Theme, type_};
use crate::ui::button::{self, ButtonState, Size, Variant};
use crate::ui::settings_dialog::{BUTTON_MD, DIALOG_TITLE};
use crate::ui::view_state::{self, ViewState};
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
    /// No preview on this computer yet (macOS until its sandbox exists). Words not on the board: flagged in the notes.
    NotOnThisComputer,
    /// This Windows is older than the viewer's sandbox needs.
    NeedsWindowsUpdate,
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
pub const EMPTY_TITLE: &str = "This document is empty";
pub const EMPTY_BODY: &str = "Nothing to show yet. Open it for editing to add the first words.";

pub const REASON_TYPE: &str = "No preview for this type (.ppt, .odp, password-protected)";
pub const REASON_DAMAGED: &str = "Damaged or over a size limit";
pub const REASON_STOPPED: &str = "The viewer stopped";
pub const REASON_TOO_LARGE: &str = "Too large to preview here";
pub const REASON_NOT_HERE: &str = "No preview on this computer yet";
pub const REASON_NEEDS_WINDOWS: &str = crate::windows_floor::NEEDS_WINDOWS_UPDATE;

impl OfficeState {
    /// The one reason line of DW-1556; `None` where the state is not a failure.
    pub fn reason_line(self) -> Option<&'static str> {
        match self {
            OfficeState::NoPreview(Reason::UnsupportedType) => Some(REASON_TYPE),
            OfficeState::NoPreview(Reason::DamagedOrOverLimit) => Some(REASON_DAMAGED),
            OfficeState::NoPreview(Reason::TooLarge) => Some(REASON_TOO_LARGE),
            OfficeState::NoPreview(Reason::NotOnThisComputer) => Some(REASON_NOT_HERE),
            OfficeState::NoPreview(Reason::NeedsWindowsUpdate) => Some(REASON_NEEDS_WINDOWS),
            OfficeState::Stopped { .. } => Some(REASON_STOPPED),
            OfficeState::Opening | OfficeState::Ready | OfficeState::ContentMissing => None,
        }
    }

    /// Try again is offered every time the viewer stopped; the source decides whether the file opens.
    pub fn try_again_shown(self) -> bool {
        matches!(self, OfficeState::Stopped { .. })
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

    /// Try again: after a stop, every time. The file opens once more (`Opening`); the host answers.
    /// `true`: it was taken.
    pub fn try_again(&mut self) -> bool {
        if !self.state.try_again_shown() {
            return false;
        }
        self.tries += 1;
        self.state = OfficeState::Opening;
        true
    }

    /// The host's answer after a retry: it stopped again. Try again stays.
    pub fn stopped_again(&mut self) {
        self.state = OfficeState::Stopped {
            retried: self.tries > 0,
        };
    }
}

// ---------------------------------------------------------------------------
// Scenes
// ---------------------------------------------------------------------------

#[cfg(test)]
fn state_mut(s: &mut State) -> Option<&mut StateBody> {
    match &mut viewers::ui(s)?.file.body {
        Body::OfficeState(body) => Some(body),
        _ => None,
    }
}

/// Open the viewer's document (facts "Word document \u{00B7} 38 KB": never the editor's), then hold the state `body` says.
fn scene_of(s: &mut State, body: StateBody) {
    if viewers::open_doc(s, docs_fixture::file(false))
        && let Some(ui) = viewers::ui(s)
    {
        ui.file.body = Body::OfficeState(body);
    }
}

fn ui_button(key: &str, words: &str, outlined: bool, t: &Theme) -> El {
    button::button(
        button::keys(key),
        words,
        None,
        if outlined { Variant::Secondary } else { Variant::Ghost },
        Size::Md,
        ButtonState::Enabled,
        t,
    )
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
    if viewers::open_doc(s, super::docs::docs_fixture::file(false))
        && let Some(doc) = viewers::doc_mut(s)
    {
        doc.quiet = OfficeState::ContentMissing.quiet_line();
    }
}

/// The group's board frames and the scene that shows each (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] = &[("DW-1563", scene_opening), ("DW-1564", scene_quiet)];

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
const STATUS_STATE: TextStyle = TextStyle::new(11.5, 500.0).lh(17.0);
const STATUS_ENC: TextStyle = TextStyle::new(11.5, 600.0).lh(17.0);
const BAR_LABEL: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
const BAR_MONO: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono).lh(15.0);

static ZOOM_LESS: Icon = Icon {
    name: "states-zoom-less",
    view: 24.0,
    body: concat!(
        r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
        r#"<path d="M5 12h14"></path></g>"#
    ),
};

static ZOOM_MORE: Icon = Icon {
    name: "states-zoom-more",
    view: 24.0,
    body: concat!(
        r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
        r#"<path d="M12 5v14M5 12h14"></path></g>"#
    ),
};

const PAGE_W: f32 = 520.0;
const OUTLINE_W: f32 = 210.0;

/// A bar of the skeleton: the board's `color-mix(in oklch, ink 9%, ground)`, taken from THIS theme's ink and ground (the
/// board's `--kit-ground` is `surface`: paper-2 in light, paper in dark; on `bg` the bars were nearly white in light).
pub fn bone_fill(t: &Theme) -> Color {
    let (ink, ground) = (t.text, t.surface);
    let mix = |a: f32, b: f32| a * 0.09 + b * 0.91;
    Color {
        r: mix(ink.r, ground.r),
        g: mix(ink.g, ground.g),
        b: mix(ink.b, ground.b),
        a: 1.0,
    }
}

/// The ground behind the pages: the board's `color-mix(in oklch, ink 6%, ground)` (light [231,228,224], dark [26,23,19]).
pub fn stage_fill(t: &Theme) -> Color {
    rune::color::hex(if t.dark { "#1A1713" } else { "#E7E4E0" }).unwrap_or(Color::BLACK)
}

fn bone(key: String, w: f32, h: f32, t: &Theme) -> El {
    el().key(key).w(w).h(h).radius(3.0).fill(bone_fill(t))
}

/// The board's `mk-sdot is-warn` (the golden's `status-dot` fill, both themes).
fn warn_dot(t: &Theme) -> Color {
    rune::color::hex(if t.dark { "#F67222" } else { "#C94B00" }).unwrap_or(Color::BLACK)
}

/// The board's `OvBar page="Page 1 of 4"`: zoom, the page words, Open in another app. Drawn here for the skeleton: the
/// document window's own bar is private to its file.
fn viewer_bar(t: &Theme) -> El {
    row()
        .key("vbar")
        .role("toolbar")
        .label("Document")
        .w_full()
        .h(38.0)
        .px(12.0)
        .gap(12.0)
        .center_y()
        .fill(t.bg)
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(
            row()
                .key("vbar-zoom")
                .role("group")
                .label("Zoom, 100 per cent")
                .gap(6.0)
                .center_y()
                .child(kit::glyph("zoom-less-icon", &ZOOM_LESS, 13.0, t.text_secondary))
                .child(kit::label("zoom", "100%", BAR_MONO, t.text_secondary))
                .child(kit::glyph("zoom-more-icon", &ZOOM_MORE, 13.0, t.text_secondary)),
        )
        .child(el().key("vbar-sep").w(kit::HAIR).h(18.0).fill(t.separator))
        .child(kit::label("vbar-page", "Page 1 of 4", BAR_LABEL, t.text_secondary))
        .child(el().key("vbar-gap").grow(1.0))
        .child(
            ui_button("elsewhere", OPEN_ELSEWHERE, true, t).on_press(|s: &mut State| {
                if let Some(ui) = viewers::ui(s) {
                    ui.intent(Intent::OpenInAnotherApp);
                }
            }),
        )
}

/// DW-1556's panel, drawn by `ui::view_state` Failed: the title, the reason with "The file was not changed.", and
/// Try again every time the viewer stopped (a second stop shows it again). Under it, the actions that still work.
pub fn failure_panel(body: &StateBody, t: &Theme) -> El {
    let reason = body.state.reason_line().unwrap_or_default();
    let why = format!("{reason}. {NOT_CHANGED}");
    let retry = body.state.try_again_shown().then(|| {
        view_state::try_again("empty", t).on_press(|s: &mut State| {
            // A file in a renderer is opened again by its source (`viewers::office_try_again`).
            viewers::office_try_again(s);
        })
    });
    let mut actions = row().key("empty-actions").gap(8.0).center_y();
    if body.state.open_elsewhere_offered(body.kept) {
        actions = actions.child(
            ui_button("empty-btn-1", OPEN_ELSEWHERE, false, t).on_press(|s: &mut State| {
                if let Some(ui) = viewers::ui(s) {
                    ui.intent(Intent::OpenInAnotherApp);
                }
            }),
        );
    } else {
        actions = actions
            .child(kit::label("empty-keep-first", KEEP_FIRST, SUB, t.text_secondary))
            .child(
                ui_button("empty-keep", KEEP_ACTION, false, t).on_press(|s: &mut State| {
                    if let Some(ui) = viewers::ui(s) {
                        ui.intent(Intent::KeepOffline);
                    }
                }),
            );
    }
    col()
        .key("failure")
        .items_center()
        .child(view_state::view_state(
            ViewState::Failed {
                key: "empty",
                title: TITLE,
                why: &why,
                retry,
            },
            t,
        ))
        .child(actions)
}

/// An empty document: nothing in the file to show, and what to do about it.
pub fn empty_document(t: &Theme) -> El {
    view_state::view_state(
        ViewState::Empty {
            key: "doc-empty",
            icon: &icons::DOC_GENERIC,
            title: EMPTY_TITLE,
            body: EMPTY_BODY,
            action: None,
        },
        t,
    )
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
        .child(kit::label("outline-label", "OUTLINE", type_::SECTION_LABEL, t.text_tertiary).px(8.0));
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
        .box_shadow(kit::drop(2.0, 12.0, t.shadow_page))
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
                .fill(stage_fill(t))
                .child(page),
        )
}

/// The viewer's status bar: where you are, Encrypted, About. `quiet` is the one quiet line of DW-1564 (no banner, no
/// colour); `opening` shows the state word in place of the position.
pub fn status_bar(state: OfficeState, left: &[&str], t: &Theme) -> El {
    let plain = |key: String, words: &str| kit::label(key, words, type_::STATUS, t.text_tertiary);
    let mut bar: Vec<El> = Vec::new();
    let mut said = String::new();
    if state.is_opening() {
        bar.push(el().key("status-dot").size(7.0, 7.0).radius(3.5).fill(warn_dot(t)));
        bar.push(kit::label("status-text", OPENING, STATUS_STATE, t.text_secondary));
        said.push_str(OPENING);
    } else {
        for (i, w) in left.iter().enumerate() {
            if i > 0 {
                bar.push(plain(format!("st-dot-{i}"), "\u{00B7}"));
            }
            bar.push(plain(format!("st-{i}"), w));
            said.push_str(w);
            said.push_str(", ");
        }
    }
    if let Some(quiet) = state.quiet_line() {
        bar.push(plain("st-quiet-dot".into(), "\u{00B7}"));
        bar.push(plain("st-quiet".into(), quiet));
        said.push_str(quiet);
    }
    // The failure frame's bar has nothing on the left: the board's keys count the right side from 0.
    let numbered = left.is_empty() && !state.is_opening() && state.quiet_line().is_none();
    let (k_icon, k_enc, k_dot, k_about) = if numbered {
        ("st-0-icon", "st-0", "st-1", "st-2")
    } else {
        ("st-enc-icon", "st-enc", "st-about-dot", "st-about")
    };
    bar.push(el().key("st-gap").grow(1.0));
    bar.push(
        row()
            .key("st-enc-box")
            .gap(4.0)
            .center_y()
            .child(kit::glyph(k_icon, &icons::LOCK, 11.0, t.accent_deep))
            .child(kit::label(k_enc, ENCRYPTED, STATUS_ENC, t.accent_deep)),
    );
    bar.push(plain(k_dot.into(), "\u{00B7}"));
    bar.push(
        el().key("st-about-box")
            .role("link")
            .label("About this viewer")
            .focusable(true)
            .on_press(|s: &mut State| {
                if let Some(ui) = viewers::ui(s) {
                    ui.office_about();
                }
            })
            .child(plain(k_about.into(), ABOUT)),
    );
    crate::ui::status_bar::frame(bar, t).label(format!("{said}. {ENCRYPTED}"))
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
        .children(state.is_opening().then(|| viewer_bar(&t)))
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

    const ALL_REASONS: [OfficeState; 6] = [
        OfficeState::NoPreview(Reason::UnsupportedType),
        OfficeState::NoPreview(Reason::DamagedOrOverLimit),
        OfficeState::NoPreview(Reason::TooLarge),
        OfficeState::NoPreview(Reason::NotOnThisComputer),
        OfficeState::NoPreview(Reason::NeedsWindowsUpdate),
        OfficeState::Stopped { retried: false },
    ];

    #[test]
    fn office_states_each_reason_has_its_line() {
        let lines: Vec<&str> = ALL_REASONS.iter().map(|s| s.reason_line().expect("a reason")).collect();
        assert_eq!(
            lines,
            [
                REASON_TYPE,
                REASON_DAMAGED,
                REASON_TOO_LARGE,
                REASON_NOT_HERE,
                REASON_NEEDS_WINDOWS,
                REASON_STOPPED
            ],
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
            assert!(keyed(&rt, "empty-text"), "{s:?} draws its reason line");
            assert!(keyed(&rt, "empty-title"));
        }
        assert!(!keyed(
            &showing(StateBody::new(OfficeState::Opening, true)),
            "empty-text"
        ));
    }

    #[test]
    fn office_states_try_again_only_once() {
        assert!(OfficeState::Stopped { retried: false }.try_again_shown());
        assert!(
            OfficeState::Stopped { retried: true }.try_again_shown(),
            "a second stop still offers it"
        );
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
        assert!(b.try_again(), "a second failure keeps it");
        assert!(keyed(
            &showing(StateBody::new(OfficeState::Stopped { retried: false }, true)),
            "empty-retry"
        ));
        b.stopped_again();
        assert!(keyed(&showing(b), "empty-retry"));
    }

    #[test]
    fn office_states_open_elsewhere_only_when_kept() {
        let s = OfficeState::Stopped { retried: false };
        assert!(s.open_elsewhere_offered(true));
        assert!(!s.open_elsewhere_offered(false), "not kept: not offered");
        let kept = showing(StateBody::new(s, true));
        assert!(keyed(&kept, "empty-btn-1") && !keyed(&kept, "empty-keep-first"));
        let not_kept = showing(StateBody::new(s, false));
        assert!(
            !keyed(&not_kept, "empty-btn-1"),
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

    fn runtime_dark(state: State, dark: bool) -> Runtime<State> {
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

    /// Every text the window draws (labels and texts), lower-cased.
    fn said(rt: &Runtime<State>) -> Vec<String> {
        rt.elements()
            .into_iter()
            .filter(|e| !e.exiting)
            .flat_map(|e| [e.label.clone(), e.text.clone()])
            .flatten()
            .map(|w| w.to_lowercase())
            .collect()
    }

    #[test]
    fn office_states_failure_says_reason_and_not_changed() {
        let rt = showing(StateBody::new(OfficeState::Stopped { retried: false }, true));
        assert!(
            keyed(&rt, "empty-title") && keyed(&rt, "empty-text"),
            "title and reason are drawn"
        );
        let words = said(&rt);
        assert!(
            words
                .iter()
                .any(|w| w.contains("the viewer stopped. the file was not changed."))
        );
    }

    #[test]
    fn office_states_empty_document_has_a_state() {
        fn page(_: &(), _: &mut Cx<()>) -> El {
            row().key("page").w(400.0).child(empty_document(&tokens::theme(false)))
        }
        let env = Env {
            system_dark: false,
            scale: 2.0,
        };
        let mut rt = Runtime::new((), page, (400.0, 400.0), env);
        rt.frame(0.0);
        let has = |key: &str| rt.elements().into_iter().any(|e| e.key.as_deref() == Some(key));
        assert!(has("doc-empty-title") && has("doc-empty-text"));
    }

    #[test]
    fn office_states_breadcrumb_never_says_editor() {
        for scene in [scene_failed as fn(&mut State), scene_opening] {
            let mut state = init();
            scene(&mut state);
            let rt = runtime_of(state);
            for w in said(&rt) {
                assert!(!w.contains("editor"), "drawn text {w:?} names the editor");
            }
            assert!(
                said(&rt).iter().any(|w| w.contains("word document")),
                "the viewer's facts show"
            );
        }
    }

    #[test]
    fn office_states_opening_has_the_viewer_bar_and_capital_outline() {
        let rt = showing(StateBody::new(OfficeState::Opening, true));
        let words = said(&rt);
        assert!(
            words.iter().any(|w| w == "open in another app\u{2026}"),
            "the bar's button"
        );
        assert!(words.iter().any(|w| w == "page 1 of 4"), "the bar's page words");
        assert!(words.iter().any(|w| w == "outline"), "the outline header");
        let header = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("outline-label"))
            .and_then(|e| e.text.clone());
        assert_eq!(header.as_deref(), Some("OUTLINE"), "in capitals");
    }

    #[test]
    fn office_states_skeleton_is_visible_in_dark() {
        let away = |c: Color| (1.0 - c.r).abs() + (1.0 - c.g).abs() + (1.0 - c.b).abs();
        let light = away(bone_fill(&tokens::theme(false)));
        let dark = away(bone_fill(&tokens::theme(true)));
        assert!(
            dark >= light,
            "dark bar ({dark}) is at least as far from the white page as light ({light})"
        );
        let mut state = init();
        scene_opening(&mut state);
        let rt = runtime_dark(state, true);
        let bar = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("pg-bone-0") && !e.exiting)
            .expect("a page bone is drawn");
        assert_eq!(
            bar.fill.hex(),
            bone_fill(&tokens::theme(true)).hex(),
            "the drawn bone uses the dark theme's mix"
        );
    }

    #[test]
    fn office_states_scenes_set_their_state() {
        let ids: Vec<&str> = FRAMES.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, ["DW-1563", "DW-1564"]);
        let mut s = init();
        scene_opening(&mut s);
        assert_eq!(state_mut(&mut s).map(|b| b.state), Some(OfficeState::Opening));
        // DW-1564 opens the DOCUMENT with the one quiet line (no state panel): the board draws the pages and the status
        // bar's line, so the scene carries no `StateBody` and the line sits on the document.
        scene_quiet(&mut s);
        assert!(
            state_mut(&mut s).is_none(),
            "the quiet scene is the document, not a state panel"
        );
        assert_eq!(
            viewers::doc_mut(&mut s).and_then(|d| d.quiet),
            Some(QUIET_LINE),
            "the document carries the one quiet line"
        );
        scene_failed(&mut s);
        assert_eq!(
            state_mut(&mut s).map(|b| b.state),
            Some(OfficeState::Stopped { retried: false })
        );
    }

    /// 11.2 / 11.9: the stage is the board's grey and the skeleton's bars are told apart from the page in BOTH themes.
    #[test]
    fn office_states_stage_and_bones_are_the_boards_greys() {
        assert_eq!(
            stage_fill(&tokens::theme(false)).hex(),
            rune::color::hex("#E7E4E0").unwrap().hex()
        );
        assert_eq!(
            stage_fill(&tokens::theme(true)).hex(),
            rune::color::hex("#1A1713").unwrap().hex()
        );
        let bar = bone_fill(&tokens::theme(false));
        assert!(
            bar.r < 0.95 && bar.g < 0.95,
            "the light bars are grey on the white page, not near white: {bar:?}"
        );
    }
}
