//! What macOS shows itself, inside the window (dm-74): a recording or a video in the system's player view (AVKit,
//! with the system's playback controls), a document in a Quick Look view. The view is the system's own, a child of
//! the window over the toolkit's surface (`shell::platform::macos_preview`); this frame draws the room for it, under
//! the open file's own toolbar (`chrome::window`, with its "…" menu), and tells it where that room is.
//!
//! - **Where the view sits** is the body's laid-out rect ([`Stage::at`], from `El::on_layout`): it is told again
//!   whenever the rect changes, so the view follows a resize of the window and the details panel, as the photo's
//!   `aspect_fit` does.
//! - **What is over it:** a child view covers whatever the toolkit draws in the same place. While a dialog is open
//!   the view is hidden ([`Stage::cover`]) and the plain stage shows; a player keeps playing. Dm-83 (RC14: the
//!   document vanished while the "…" menu was open): on a Mac the "…" menu is the system's own menu, over the view
//!   (`more_menu`), so the document stays; only where there is no system menu does the app-drawn one hide the view.
//!   A toast no longer hides it either: the view's room ends above the toast ([`TOAST_ROOM`]). The details panel
//!   stands beside the view, never over it.
//! - **Without a stage** the system could host no view and the file is in the Quick Look panel (a window of its
//!   own): the body says so.
//!
//! The source makes the view and takes it away (on Back, on close, when another file opens, on a lock); this frame
//! holds nothing of the file.

use std::sync::Arc;

use rune::{Align, Color, Cx, El, TextStyle, col, el};

use super::chrome::{self, Dot, Status};
use super::{Ui, details};
use crate::State;
use crate::theme::scale;
use crate::theme::tokens::Theme;
use crate::ui::kit;

/// The place of the system's view, as the source hears it. `Send + Sync`: the body's layout handler holds it.
pub trait StagePort: Send + Sync {
    /// The room for the view: `[x, y, w, h]` in window points from the window's top left.
    fn at(&self, rect: [f32; 4]);
    /// Something the app draws is over that room (`true`): the view is hidden until it is not.
    fn cover(&self, on: bool);
}

/// The handle the body places the view through. Two are equal when they are the same view's.
#[derive(Clone)]
pub struct Stage(Arc<dyn StagePort>);

impl Stage {
    pub fn new(port: Arc<dyn StagePort>) -> Self {
        Stage(port)
    }

    pub fn at(&self, rect: [f32; 4]) {
        self.0.at(rect);
    }

    pub fn cover(&self, on: bool) {
        self.0.cover(on);
    }
}

impl PartialEq for Stage {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Which of the system's views shows the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeKind {
    /// The player view: a recording or a video.
    Player,
    /// The Quick Look view: a document.
    QuickLook,
}

/// A file the system shows.
#[derive(Clone, PartialEq)]
pub struct NativeDoc {
    pub kind: NativeKind,
    /// Where the view inside the window sits. `None`: no view could be hosted, the file is in the Quick Look panel.
    pub stage: Option<Stage>,
}

/// What the body says while the file is in the Quick Look panel.
pub const IN_PANEL: &str = "This file is open in the Quick Look window.";

/// `.t-body`: 13px/400 on 20px (the theme's 13/400 reading style on a taller line).
const BODY: TextStyle = scale::text::READ.lh(20.0);

/// Whether something the app draws is over the body now: the app-drawn "…" menu (only where there is no system
/// menu: `more_menu`), the viewer's own dialog, or one of Files' dialogs for the open file (Share, Version history).
/// A toast is not (dm-83): the room ends above it.
fn covered(s: &State, ui: &Ui, t: &Theme) -> bool {
    ui.more.is_some() || ui.dialog.is_some() || crate::frames::files::window::dialog_over_viewer(s, t).is_some()
}

/// How much of the body's foot is left free while a toast shows, so the view ends above it (dm-83): the toast stands
/// [`crate::ui::toast::INSET`] over the window's foot and is 34 tall; the status line (28) is under the body
/// already; 8 more between them.
pub const TOAST_ROOM: f32 = crate::ui::toast::INSET + 34.0 + 8.0 - 28.0;

/// The status line: the file is on this Mac, and macOS shows it.
pub fn status(doc: &NativeDoc) -> Status {
    Status {
        dot: Dot::Green,
        text: details::DECRYPTED_HERE.into(),
        details: match (doc.kind, doc.stage.is_some()) {
            (NativeKind::Player, true) => "Played by macOS".into(),
            (NativeKind::QuickLook, true) => "Shown by Quick Look".into(),
            (_, false) => "Open in the Quick Look window".into(),
        },
    }
}

/// The room for the system's view: the whole body. Its rect goes to the stage whenever it changes. Keys:
/// `viewer-body`, `native-stage` (the room), `native-panel` (the line when the file is in the panel).
pub fn body(ui: &Ui, doc: &NativeDoc, covered: bool, t: &Theme) -> El {
    // A film stands on black, as in the app's own player; a document on the window's paper.
    let ground = match doc.kind {
        NativeKind::Player => Color::BLACK,
        NativeKind::QuickLook => t.bg,
    };
    let body = col().key("viewer-body").grow(1.0).min_w(0.0).h_full().fill(ground);
    let Some(stage) = &doc.stage else {
        let ink = match doc.kind {
            NativeKind::Player => Color::WHITE,
            NativeKind::QuickLook => t.text_secondary,
        };
        return body
            .center()
            .child(kit::label("native-panel", IN_PANEL, BODY, ink).align(Align::Center));
    };
    // Said at every draw: the view is hidden the moment a dialog opens, and back when it closes.
    stage.cover(covered);
    let place = stage.clone();
    // Dm-83: while a toast shows, the view's room ends above it, so both are seen.
    let body = if ui.toast.is_some() { body.pb(TOAST_ROOM) } else { body };
    body.child(
        el().key("native-stage")
            .role("group")
            .label(ui.file.name.clone())
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .on_layout(move |_: &mut State, rect: [f32; 4]| place.at(rect)),
    )
}

/// The open file's window with the system's view in its body: the shared toolbar (with "…") above, the status line
/// below, the details panel beside it when it is open.
pub fn window(s: &State, cx: &mut Cx<State>, ui: &Ui, doc: &NativeDoc, t: &Theme) -> El {
    let body = body(ui, doc, covered(s, ui, t), t);
    chrome::window(
        s,
        cx,
        ui,
        chrome::Head::default(),
        Vec::new(),
        None,
        body,
        &status(doc),
        t,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, PoisonError};

    use rune::{Env, Runtime};

    use super::super::source::{Body, Details, OpenError, OpenFile, ViewerSource};
    use super::*;
    use crate::init;
    use crate::theme::tokens::theme;

    /// What the stage was told, in order.
    #[derive(Default)]
    struct Told {
        rects: Mutex<Vec<[f32; 4]>>,
        covers: Mutex<Vec<bool>>,
    }

    impl Told {
        fn rect(&self) -> [f32; 4] {
            *self
                .rects
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .last()
                .expect("the stage was told where the body is")
        }

        fn covered(&self) -> bool {
            *self
                .covers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .last()
                .expect("the stage was told whether it is covered")
        }
    }

    impl StagePort for Told {
        fn at(&self, rect: [f32; 4]) {
            self.rects.lock().unwrap_or_else(PoisonError::into_inner).push(rect);
        }

        fn cover(&self, on: bool) {
            self.covers.lock().unwrap_or_else(PoisonError::into_inner).push(on);
        }
    }

    /// A source with one file the system shows.
    struct One(NativeDoc);

    impl ViewerSource for One {
        fn open(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
            Ok(OpenFile {
                id: file_id.to_string(),
                path: Vec::new(),
                name: "Invented clip.mov".into(),
                facts: "Video".into(),
                version: 1,
                body: Body::Native(self.0.clone()),
                details: Details {
                    kind: "Video".into(),
                    size: String::new(),
                    facts: Vec::new(),
                    offline: false,
                },
                newer: None,
            })
        }
        fn poll(&mut self, _file_id: &str) -> Option<Body> {
            None
        }
        fn cancel(&mut self, _file_id: &str) {}
        fn retry(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
            self.open(file_id)
        }
    }

    fn window_view(s: &State, cx: &mut Cx<State>) -> El {
        let ui = s.viewer.ui.as_ref().expect("a file is open");
        let Body::Native(doc) = &ui.file.body else {
            panic!("the file is one the system shows");
        };
        window(s, cx, ui, doc, &theme(false))
    }

    const WINDOW: (f32, f32) = (1278.0, 798.0);

    fn opened(doc: NativeDoc) -> Runtime<State> {
        let mut state = init();
        state.viewer.ui = Some(Ui::open(Box::new(One(doc)), "v1").expect("it opens"));
        let env = Env {
            system_dark: false,
            scale: 2.0,
        };
        let mut rt = Runtime::new(state, window_view, WINDOW, env);
        rt.frame(0.0);
        rt.frame(0.016);
        rt
    }

    fn staged(kind: NativeKind) -> (Runtime<State>, Arc<Told>) {
        let told = Arc::new(Told::default());
        let port: Arc<dyn StagePort> = told.clone();
        let rt = opened(NativeDoc {
            kind,
            stage: Some(Stage::new(port)),
        });
        (rt, told)
    }

    fn rect_of(rt: &Runtime<State>, key: &str) -> [f32; 4] {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
            .unwrap_or_else(|| panic!("{key} is drawn"))
            .rect
    }

    fn settle(rt: &mut Runtime<State>) {
        for _ in 0..4 {
            rt.frame(0.016);
        }
    }

    #[test]
    fn the_stage_is_told_the_bodys_rect_under_the_toolbar() {
        let (rt, told) = staged(NativeKind::Player);
        let [x, y, w, h] = told.rect();
        assert_eq!([x, y, w, h], rect_of(&rt, "native-stage"), "the rect is the body's own");
        // Inside the window, right of the sidebar, under the caption and the toolbar, over the status line.
        assert!(x > 0.0 && y > 0.0 && w > 300.0 && h > 300.0, "{x} {y} {w} {h}");
        assert!(x + w <= WINDOW.0 + 0.5 && y + h < WINDOW.1, "{x} {y} {w} {h}");
        let more = rect_of(&rt, "more-btn");
        assert!(
            more[1] + more[3] <= y + 0.5,
            "the toolbar with \u{2026} stays above the view"
        );
        assert!(!told.covered());
    }

    #[test]
    fn the_details_panel_and_a_resize_move_the_stage() {
        let (mut rt, told) = staged(NativeKind::QuickLook);
        let wide = told.rect();
        // The details panel opens beside the body: the same top and left, a narrower room.
        rt.update(|s, _| s.viewer.ui.as_mut().expect("open").toggle_details());
        settle(&mut rt);
        let beside = told.rect();
        assert_eq!((beside[0], beside[1], beside[3]), (wide[0], wide[1], wide[3]));
        assert!(
            beside[2] < wide[2] - 100.0,
            "{beside:?} beside the panel, {wide:?} without it"
        );
        rt.update(|s, _| s.viewer.ui.as_mut().expect("open").toggle_details());
        settle(&mut rt);
        assert_eq!(told.rect(), wide);
        // The window is made smaller: so is the room.
        rt.set_viewport((1000.0, 640.0), 2.0);
        settle(&mut rt);
        let small = told.rect();
        assert!(
            small[2] < wide[2] && small[3] < wide[3],
            "{small:?} in the smaller window, {wide:?} before"
        );
        assert_eq!(small, rect_of(&rt, "native-stage"));
    }

    /// Where there is no system menu (off a Mac), the app-drawn "…" menu hides the view while it is open.
    #[test]
    fn the_view_is_covered_while_the_app_drawn_menu_is_open() {
        let (mut rt, told) = staged(NativeKind::Player);
        assert!(!told.covered());
        rt.update(|s, _| s.viewer.ui.as_mut().expect("open").toggle_more());
        settle(&mut rt);
        assert!(told.covered(), "the \u{2026} menu is drawn where the view is");
        rt.update(|s, _| s.viewer.ui.as_mut().expect("open").toggle_more());
        settle(&mut rt);
        assert!(!told.covered());
    }

    /// Dm-83, RC14: on a .docx, the "…" menu made the document vanish until it closed. With the system's menu (a
    /// Mac's) the view is never hidden: the menu is the system's window over it, and the line picked runs as the
    /// app-drawn one's does.
    #[test]
    fn the_systems_menu_keeps_the_document_on_screen_and_its_pick_runs() {
        use super::super::more_menu::{TEST_POPUP, entries};
        fn rename(entries: &[crate::ui::menu::Entry]) -> Option<usize> {
            entries.iter().position(|e| e.id() == Some("rename"))
        }
        fn closed(_: &[crate::ui::menu::Entry]) -> Option<usize> {
            None
        }
        let (mut rt, told) = staged(NativeKind::QuickLook);
        TEST_POPUP.with(|popup| popup.set(Some(closed)));
        rt.update(|s, _| s.viewer.ui.as_mut().expect("open").toggle_more());
        settle(&mut rt);
        assert!(!told.covered(), "the document stays while the menu is open");
        assert!(told.covers.lock().unwrap().iter().all(|on| !on), "never hidden");
        rt.update(|s, _| assert!(s.viewer.ui.as_ref().expect("open").more.is_none(), "no app-drawn menu"));

        // A line picked in the system's menu is the file's action, as in the app's own menu.
        TEST_POPUP.with(|popup| popup.set(Some(rename)));
        let mut asked = Vec::new();
        rt.update(|s, _| {
            let ui = s.viewer.ui.as_mut().expect("open");
            assert!(entries(ui).iter().any(|e| e.id() == Some("rename")));
            ui.take_intents();
            ui.toggle_more();
            asked = ui.take_intents();
        });
        TEST_POPUP.with(|popup| popup.set(None));
        assert_eq!(asked, [super::super::Intent::Rename]);
        assert!(!told.covered());
    }

    /// Dm-83: a toast leaves the document on screen; the view's room ends above the toast.
    #[test]
    fn a_toast_leaves_the_view_on_screen_above_it() {
        let (mut rt, told) = staged(NativeKind::QuickLook);
        let before = told.rect();
        rt.update(|s, _| s.viewer.ui.as_mut().expect("open").toast = Some("Invented words".into()));
        settle(&mut rt);
        assert!(!told.covered(), "a toast hides nothing");
        let room = told.rect();
        let toast = rect_of(&rt, "toast");
        assert_eq!((room[0], room[1], room[2]), (before[0], before[1], before[2]));
        assert!(
            room[1] + room[3] <= toast[1] + 0.5,
            "the view {room:?} ends above the toast {toast:?}"
        );
        rt.update(|s, _| s.viewer.ui.as_mut().expect("open").toast = None);
        settle(&mut rt);
        assert_eq!(told.rect(), before, "the whole room again");
    }

    #[test]
    fn without_a_stage_the_body_says_the_file_is_in_the_panel() {
        let rt = opened(NativeDoc {
            kind: NativeKind::QuickLook,
            stage: None,
        });
        let line = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("native-panel") && !e.exiting)
            .expect("the line is drawn");
        assert_eq!(line.text.as_deref(), Some(IN_PANEL));
        assert!(
            !rt.elements()
                .into_iter()
                .any(|e| e.key.as_deref() == Some("native-stage"))
        );
    }

    #[test]
    fn the_status_line_no_longer_says_decrypting() {
        let told: Arc<dyn StagePort> = Arc::new(Told::default());
        let doc = NativeDoc {
            kind: NativeKind::QuickLook,
            stage: Some(Stage::new(told)),
        };
        let line = status(&doc);
        assert_eq!(line.text, details::DECRYPTED_HERE);
        assert!(!line.text.contains("Decrypting \u{00B7}") && line.details == "Shown by Quick Look");
    }
}
