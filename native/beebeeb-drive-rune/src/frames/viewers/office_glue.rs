//! The hooks that let the Office viewer's scenes (`frames::office`) draw in the viewer: the spreadsheet (DW-1508),
//! the presentation (DW-1509, DW-1561) and the licence panel (DW-1557). Read-only: nothing here edits a file, and the
//! editor frames (DW-1558, DW-1559, DW-1560) stay placeholders that no control reaches.
//!
//! Kept in its own file because `mod.rs` is at its line limit.

use rune::{Cx, El, KeyEvent, Tx};

use super::Ui;
use super::kinds::OfficePicture;
use super::office_over::Over;
use super::source::{Body, FixtureViewerSource, OpenFile};
use crate::State;
use crate::frames::office::docs::{self, Asks, DocBody, Live};
use crate::frames::office::slides::{self, DeckBody};
use crate::frames::office::{dialogs, sheets, states};

/// The open presentation, while the viewer shows one.
pub fn deck_mut(s: &mut State) -> Option<&mut DeckBody> {
    match &mut super::ui(s)?.file.body {
        Body::Deck(deck) => Some(deck),
        _ => None,
    }
}

/// Open a presentation in the viewer, from the fixture source (the live host's answer replaces it at O-CP3).
/// `false`: the file is not a presentation, or it did not open (no session).
pub fn open_deck(s: &mut State, file: OpenFile) -> bool {
    super::open_from(s, Box::new(FixtureViewerSource::new()), &file.id) && deck_mut(s).is_some()
}

/// The open Word document, while the viewer shows one.
pub fn doc_mut(s: &mut State) -> Option<&mut DocBody> {
    match &mut super::ui(s)?.file.body {
        Body::Doc(doc) => Some(doc),
        _ => None,
    }
}

/// Open a Word document in the viewer, from the fixture source (the live host's answer replaces it at O-CP3).
/// `false`: the file is not a Word document, or it did not open (no session).
pub fn open_doc(s: &mut State, file: OpenFile) -> bool {
    super::open_from(s, Box::new(FixtureViewerSource::new()), &file.id) && doc_mut(s).is_some()
}

/// The window's DPI: 96 at a scale of 1 (what Rune's environment says, never assumed).
pub(super) fn dpi_of(cx: &Cx<State>) -> u16 {
    (cx.env().scale * 96.0).round().clamp(48.0, 960.0) as u16
}

/// The window's size in points as the last frame laid it out (a first frame: the board's).
pub(super) fn screen_of(cx: &Cx<State>) -> (f32, f32) {
    cx.layout_size("window").map(|[w, h]| (w, h)).unwrap_or((1278.0, 798.0))
}

/// The window of a spreadsheet or a presentation: what `view` hands over for those bodies. A file in a renderer
/// (`office_live`) asks it for what is drawn; the fixture source draws the fixture's pictures.
pub(super) fn window(s: &State, cx: &mut Cx<State>, ui: &Ui) -> El {
    let live = ui.source.office_live();
    let id = ui.file.id.clone();
    let ask = |which: OfficePicture| ui.source.office_picture(&id, which);
    let asks = Asks::new(&ask);
    let (dpi, screen) = (dpi_of(cx), screen_of(cx));
    let pics = Live {
        asks: &asks,
        dpi,
        screen,
    };
    match &ui.file.body {
        Body::Sheet(view) if live => {
            let cells = |sheet, row, col, rows, cols| ui.source.office_cells(&id, sheet, row, col, rows, cols);
            sheets::window_live(s, cx, ui, view, &pics, &cells)
        }
        Body::Sheet(view) => sheets::window(s, cx, ui, view),
        Body::Deck(deck) if deck.presenting && live => slides::present_live(cx, deck, &pics),
        Body::Deck(deck) if deck.presenting => slides::present(cx, deck, &slides::slides_fixture::page),
        Body::Deck(deck) if live => slides::window_live(s, cx, ui, deck, &pics),
        Body::Deck(deck) => slides::window(s, cx, ui, deck, &slides::slides_fixture::page),
        Body::Doc(doc) if live => docs::window_live(s, cx, ui, doc, &pics),
        Body::Doc(doc) => {
            let older = doc.view.older_format;
            docs::window(s, cx, ui, doc, &move |n| docs::docs_fixture::page(older, n))
        }
        Body::OfficeState(body) => states::window(s, cx, ui, body),
        _ => unreachable!("only a sheet, a deck, a document or a state is handed to the Office glue"),
    }
}

/// Try again on the Office failure panel: the window says it is opening, and a file in a renderer is opened again by
/// its source (once: the host's rule). A refusal leaves "The viewer stopped" without the button. The fixture source's
/// retry would open the board's file, so it is not asked.
pub fn office_try_again(s: &mut State) {
    let Some(ui) = super::ui(s) else { return };
    let Body::OfficeState(body) = &mut ui.file.body else {
        return;
    };
    if !body.try_again() || !ui.source.office_live() {
        return;
    }
    let id = ui.file.id.clone();
    match ui.source.retry(&id) {
        Ok(file) => ui.replace(file),
        Err(_) => {
            if let Body::OfficeState(body) = &mut ui.file.body {
                body.stopped_again();
            }
        }
    }
}

/// The deck's keys (Present's arrows, Space, Esc; F5 and Ctrl+Enter in the window). `true`: taken. A document's and a
/// workbook's keys move their scroll viewport with `tx` too.
pub(super) fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if deck_mut(s).is_some_and(|deck| deck.on_key(&ev.key, &ev.mods)) {
        return true;
    }
    if docs::on_key(s, ev, tx) {
        return true;
    }
    // A workbook in a renderer: the keys scroll what is in view.
    match super::ui(s) {
        Some(ui) if ui.source.office_live() => match &mut ui.file.body {
            Body::Sheet(view) => sheets::scroll_key_tx(view, &ev.key, &ev.mods, tx),
            _ => false,
        },
        _ => false,
    }
}

/// Esc while Present is on leaves Present, not the viewer: the shell asks before it closes the file. `true`: taken.
pub fn leave_present(s: &mut State) -> bool {
    match deck_mut(s) {
        Some(deck) if deck.presenting => {
            deck.leave();
            true
        }
        _ => false,
    }
}

/// DW-1557: the viewer's document, with the "About this viewer" card open over it (the status bar's About).
pub fn scene_licence(s: &mut State) {
    if open_doc(s, docs::docs_fixture::file(false))
        && let Some(ui) = super::ui(s)
    {
        ui.office_about();
    }
}

/// The panel's words come from the Office dialogs' module; this is the one place it is laid over the viewer.
pub(super) fn licence(t: &crate::theme::tokens::Theme) -> El {
    dialogs::licence_panel(t)
}

#[cfg(test)]
mod tests {
    use rune::{Env, Key, Mods, Runtime};

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

    fn keyed(rt: &Runtime<State>, key: &str) -> bool {
        rt.elements()
            .into_iter()
            .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }

    fn press(rt: &mut Runtime<State>, key: Key) {
        rt.key(KeyEvent {
            key,
            mods: Mods::default(),
        });
        rt.frame(0.0);
    }

    /// DW-1508 draws the spreadsheet window from the sheet body, not the placeholder.
    #[test]
    fn office_glue_sheet_scene_draws_the_real_window() {
        let mut state = init();
        (sheets::FRAMES[0].1)(&mut state);
        let ui = super::super::ui(&mut state).expect("the scene opened a file");
        assert!(matches!(ui.file.body, Body::Sheet(_)), "the file body is a sheet");
        let rt = runtime_of(state);
        assert!(keyed(&rt, "sh-cell-1-1-t"), "a cell of the grid is drawn");
    }

    /// DW-1509 draws the deck window and DW-1561 Present, each from the deck body.
    #[test]
    fn office_glue_deck_scenes_draw_the_real_windows() {
        let mut state = init();
        (slides::FRAMES[0].1)(&mut state);
        assert!(deck_mut(&mut state).is_some_and(|d| !d.presenting));
        let rt = runtime_of(state);
        assert!(keyed(&rt, "filmstrip") && !keyed(&rt, "pres-stage"));
        let mut state = init();
        (slides::FRAMES[2].1)(&mut state);
        assert!(deck_mut(&mut state).is_some_and(|d| d.presenting));
        let rt = runtime_of(state);
        assert!(keyed(&rt, "pres-stage") && !keyed(&rt, "filmstrip"));
    }

    /// The deck's keys reach the deck, and Esc leaves Present without closing the viewer.
    #[test]
    fn office_glue_deck_keys_and_esc_stay_in_the_deck() {
        let mut state = init();
        (slides::FRAMES[2].1)(&mut state);
        let mut rt = runtime_of(state);
        let before = deck_mut(&mut rt.state).map(|d| d.view.current).expect("a deck");
        press(&mut rt, Key::Right);
        let after = deck_mut(&mut rt.state).map(|d| d.view.current).expect("a deck");
        assert_eq!(after, before + 1, "Right steps the stage");
        press(&mut rt, Key::Escape);
        let deck = deck_mut(&mut rt.state).expect("the viewer is still open on the deck");
        assert!(!deck.presenting, "Esc left Present");
    }

    /// DW-1557 draws the licence panel over the document, and Esc closes it.
    #[test]
    fn office_glue_licence_scene_draws_the_panel() {
        let mut state = init();
        (dialogs::FRAMES[0].1)(&mut state);
        let mut rt = runtime_of(state);
        assert!(keyed(&rt, "lic-panel"), "the licence panel is drawn");
        press(&mut rt, Key::Escape);
        assert!(!keyed(&rt, "lic-panel"), "Esc closes it");
        assert!(
            super::super::ui(&mut rt.state).is_some(),
            "Esc closes the panel, not the file"
        );
    }

    /// DW-1557 is the viewer's document under a card at the bottom right, not the editor.
    #[test]
    fn office_glue_licence_scene_draws_the_viewer_not_the_editor() {
        let mut state = init();
        (dialogs::FRAMES[0].1)(&mut state);
        assert!(doc_mut(&mut state).is_some(), "the body is a viewer document");
        let rt = runtime_of(state);
        assert!(
            !rt.elements()
                .into_iter()
                .any(|e| e.key.as_deref().is_some_and(|k| k.starts_with("rib"))),
            "no ribbon is drawn"
        );
        let panel = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("lic-panel"))
            .expect("the card is drawn");
        assert!(
            (1278.0 - (panel.rect[0] + panel.rect[2]) - 16.0).abs() < 0.6,
            "16 px from the right edge"
        );
        assert!(
            (798.0 - (panel.rect[1] + panel.rect[3]) - 36.0).abs() < 0.6,
            "36 px from the bottom edge, not the top: the card is at {:?}",
            panel.rect
        );
    }

    /// The editor stays unreachable: the viewer has no route to DW-1558, DW-1559 or DW-1560, which are placeholders.
    #[test]
    fn office_glue_editor_frames_stay_placeholders() {
        let is_placeholder = |frames: &[crate::ui::gate::Scene<State>], id: &str| {
            frames
                .iter()
                .any(|(f, scene)| *f == id && *scene as usize == crate::frames::placeholder::scene as usize)
        };
        assert!(is_placeholder(dialogs::FRAMES, "DW-1558"));
        assert!(is_placeholder(sheets::FRAMES, "DW-1559"));
        assert!(is_placeholder(slides::FRAMES, "DW-1560"));
        assert!(
            super::super::kinds::OFFICE_VIEWER_ON,
            "the viewer is on since O-CP3; the editor frames stay placeholders"
        );
    }
}
