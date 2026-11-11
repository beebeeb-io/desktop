//! The flyout as a Rune window: borderless, always on top, 340 x 520 points, hidden until the
//! b is clicked; it closes on Esc and when it loses focus (DW-1201). What it draws is the tray
//! lane's (`registry::FLYOUT`: `frames/tray`).
//!
//! It takes focus when it opens. A window that never activates cannot hear that the person
//! clicked elsewhere, and "closes when it loses focus" is the board's rule.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use rune::{AppHandle, Cx, El, Key, KeyEvent, Tx};

use super::actions::{FixtureEngine, HostActions, ShellActions};
use super::tray::{TrayInputs, TrayModel, tray_model};
use super::{placeholder, platform, registry};
use crate::State;

pub struct FlyoutState {
    /// What the b shows now; the flyout's status block reads the same state.
    pub model: TrayModel,
    pub inputs: TrayInputs,
    pub actions: Arc<dyn ShellActions>,
    /// The main window's app: "Open Beebeeb", "Settings" and "Review" go through it
    /// (`host::request`).
    pub main: Option<AppHandle<State>>,
    /// Where `inputs` comes from, for the placeholder's one line.
    pub source: &'static str,
    /// When the flyout last hid itself because it lost focus (see `flyout::on_tray_click`).
    pub(super) hidden_at: Arc<Mutex<Option<Instant>>>,
    /// The flyout's content and its own state (the tray lane's).
    pub tray: crate::frames::tray::Ui,
}

impl FlyoutState {
    pub fn new(
        actions: Arc<dyn ShellActions>,
        main: Option<AppHandle<State>>,
        hidden_at: Arc<Mutex<Option<Instant>>>,
    ) -> FlyoutState {
        let inputs = actions.tray_inputs();
        let inputs_for_tray = inputs.clone();
        FlyoutState {
            model: tray_model(&inputs),
            inputs,
            actions,
            main,
            source: "fixture engine",
            hidden_at,
            tray: crate::frames::tray::Ui::following(&inputs_for_tray),
        }
    }

    /// Reads the tray inputs again. Called each time the flyout opens and when they change.
    pub fn refresh(&mut self) {
        self.inputs = self.actions.tray_inputs();
        self.model = tray_model(&self.inputs);
        self.tray.follow(&self.inputs);
    }
}

/// A flyout on the fixture engine: for headless runs and tests.
pub fn fixture_state() -> FlyoutState {
    let actions = Arc::new(HostActions::new(Arc::new(FixtureEngine::ready()), platform::current()));
    FlyoutState::new(actions, None, Arc::default())
}

pub fn view(s: &FlyoutState, cx: &mut Cx<FlyoutState>) -> El {
    match registry::FLYOUT {
        Some(view) => view(s, cx),
        None => placeholder::flyout(s, cx),
    }
}

/// Esc closes the flyout.
pub fn on_key(s: &mut FlyoutState, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if ev.key == Key::Escape {
        tx.close_window();
        return true;
    }
    // The list's keys, also before anything in the flyout has focus (the tray lane's).
    crate::frames::tray::on_window_key(s, ev, tx)
}

/// A click outside it (the flyout lost focus) closes it, and notes when.
pub fn on_focus(s: &mut FlyoutState, focused: bool, tx: &mut Tx) {
    if !focused {
        *s.hidden_at.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now());
        tx.close_window();
    }
}
