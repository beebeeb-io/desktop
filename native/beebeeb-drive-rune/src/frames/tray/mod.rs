//! The b's flyout (the board's `.mk-pop` in the notification area, 340 x 520): the status block,
//! Recent changes and the three buttons, in every state the board draws (DW-1202 to DW-1222;
//! `FRAMES.md` is the list).
//!
//! The flyout is a window of its own, which the shell opens above the taskbar
//! (`shell/flyout_window.rs`, `shell/flyout.rs`); this module is what it draws
//! (`registry::FLYOUT`) and what its controls do. The b itself (its six icons, DW-1230) and
//! its menu (DW-1231) are the shell's (`shell/tray.rs`, `shell/tray_icon.rs`).
//!
//! The same view draws in two windows: the flyout ([`view`], state `FlyoutState`) and, for the
//! frame gate, the main window of a headless run ([`scene`], state `State`; `--scene DW-1203
//! --size 340x520`). [`Host`] is the one difference between them: where the [`Ui`] lives and
//! who carries out what it asks for.
//!
//! What it draws comes from a [`TraySource`]: in the running app the engine's
//! (`engine_host::EngineTraySource`, put in place by [`on_engine`]; its words are `live.rs`'s),
//! in the frame gate's scenes and in a run without an engine the board's fixture.

mod commands;
mod icons;
pub mod live;
#[cfg(test)]
mod live_tests;
pub mod shortcuts;
#[cfg(test)]
mod shortcuts_tests;
pub mod source;
#[cfg(test)]
mod tests;
mod view;

use rune::{Cx, El, Key, KeyEvent, Live, Tx};

use crate::State;
use crate::shell::app::{Intent as Go, Route, Section, SettingsPane};
use crate::shell::flyout_window::FlyoutState;
use crate::shell::host;
use crate::shell::tray::{EngineState, TrayInputs};
use source::{Action, BoardFixture, Change, Changes, Snapshot, TraySource};

/// Something the flyout asks for that it cannot do itself: the window moves, a file is shown,
/// sync pauses. [`Host::after`] carries them out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// "Open Beebeeb": the window on Files, or to the front (DW-0401).
    OpenBeebeeb,
    /// "Open in File Explorer": a File Explorer window at the Beebeeb location (DW-1180).
    OpenInFileExplorer,
    /// "Settings": the window in settings mode, on General (DW-0701).
    OpenSettings,
    /// "View full history", and "Show" with `problems`: Sync history (DW-1301; its Problems
    /// filter, DW-0428).
    OpenSyncHistory {
        problems: bool,
    },
    /// A trashed file's row: Trash in the window.
    OpenTrash,
    /// "Review…": the first file's choice (DW-0429).
    ReviewConflict,
    /// "Sign in" and "Unlock with Windows Hello": the window, which shows what the session
    /// needs (DW-1109, DW-0009).
    OpenWindow,
    PauseSync,
    ResumeSync,
    /// The flyout's work is done: close it.
    Close,
}

/// The flyout's own state.
pub struct Ui {
    source: Box<dyn TraySource>,
    snapshot: Snapshot,
    /// The row the arrow keys are on.
    selected: Option<usize>,
    /// The list opens at its end (the scene of DW-1220, which the board draws scrolled down).
    opens_at_end: bool,
    intents: Vec<Intent>,
    /// The shell's tray state the source was last told about ([`Ui::follow`]).
    followed: Option<TrayInputs>,
    /// The scroll thumb: how far down its track it is, and whether it shows.
    thumb_at: Live<[f32; 2]>,
    thumb_shown: Live<f32>,
}

impl Ui {
    pub fn new(source: Box<dyn TraySource>) -> Ui {
        let snapshot = source.snapshot();
        Ui {
            source,
            snapshot,
            selected: None,
            opens_at_end: false,
            intents: Vec::new(),
            followed: None,
            thumb_at: Live::new([0.0, 0.0]),
            thumb_shown: Live::new(0.0),
        }
    }

    /// The flyout on the board's fixture, as DW-1202 draws it.
    pub fn fixture() -> Ui {
        Ui::board("DW-1202").unwrap_or_else(|| unreachable!("DW-1202 is a flyout frame"))
    }

    /// The flyout of a run without an engine (tooling, `--db`, tests): the fixture, on the frame
    /// the shell's tray state names. The app on the real engine replaces it ([`on_engine`]).
    pub fn following(shell: &TrayInputs) -> Ui {
        let mut ui = Ui::fixture();
        ui.follow(shell);
        ui
    }

    /// One board frame's flyout. `None` for an id that is not a flyout frame.
    pub fn board(frame: &str) -> Option<Ui> {
        let mut ui = Ui::new(Box::new(BoardFixture::frame(frame)?));
        ui.opens_at_end = frame == "DW-1220";
        Some(ui)
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// What was asked for since the last call.
    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    /// Reads the source again. A selection past the end of a shorter list goes.
    pub fn refresh(&mut self) {
        self.snapshot = self.source.snapshot();
        let rows = self.rows().len();
        self.selected = self.selected.filter(|&row| row < rows);
    }

    /// The shell's tray state changed (the b's menu paused sync, the session locked): the
    /// source hears it, so the flyout never says something else than the b. Called each time
    /// the flyout opens and whenever the shell refreshes.
    pub fn follow(&mut self, shell: &TrayInputs) {
        if self.followed.as_ref() != Some(shell) {
            self.followed = Some(shell.clone());
            self.source.follow(shell);
        }
        self.refresh();
    }

    fn rows(&self) -> &[Change] {
        match &self.snapshot.changes {
            Changes::Rows { rows, .. } => rows,
            _ => &[],
        }
    }

    /// The status block's button, as drawn ([`Snapshot::button`]).
    pub fn press_action(&mut self) {
        let Some((action, _)) = self.snapshot.button else {
            return;
        };
        self.source.act(action);
        self.intents.extend(match action {
            Action::Pause => vec![Intent::PauseSync],
            Action::Resume => vec![Intent::ResumeSync],
            Action::Review => vec![Intent::ReviewConflict, Intent::Close],
            Action::ShowProblems => vec![Intent::OpenSyncHistory { problems: true }, Intent::Close],
            Action::SignIn | Action::Unlock => vec![Intent::OpenWindow, Intent::Close],
            // The plans page opens in the browser, the installer takes over: the flyout is
            // in the way of neither, so it closes.
            Action::AddStorage | Action::ChoosePlan | Action::RestartToUpdate => vec![Intent::Close],
            Action::SyncAnyway | Action::TryAgain | Action::RetryExplorer => Vec::new(),
        });
        self.refresh();
    }

    /// A row was clicked, or Enter was pressed on it.
    pub fn open_row(&mut self, row: usize) {
        let Some(change) = self.rows().get(row).cloned() else {
            return;
        };
        self.selected = Some(row);
        if change.in_trash {
            self.intents.push(Intent::OpenTrash);
        } else {
            self.source.reveal(&change.id);
        }
        self.intents.push(Intent::Close);
    }

    pub fn open_beebeeb(&mut self) {
        self.intents.extend([Intent::OpenBeebeeb, Intent::Close]);
    }

    /// Does nothing while Beebeeb is not in File Explorer (the button is drawn disabled).
    pub fn open_in_file_explorer(&mut self) {
        if self.snapshot.explorer_ready {
            self.intents.extend([Intent::OpenInFileExplorer, Intent::Close]);
        }
    }

    pub fn open_settings(&mut self) {
        self.intents.extend([Intent::OpenSettings, Intent::Close]);
    }

    pub fn open_full_history(&mut self) {
        self.intents
            .extend([Intent::OpenSyncHistory { problems: false }, Intent::Close]);
    }

    /// Moves the selection by `delta` rows, staying inside the list; the first Down selects
    /// the first row, the first Up the last. Returns the selected row.
    pub fn move_selection(&mut self, delta: isize) -> Option<usize> {
        let rows = self.rows().len();
        if rows == 0 {
            return None;
        }
        let last = rows - 1;
        let next = match self.selected {
            None if delta >= 0 => 0,
            None => last,
            Some(row) => row.saturating_add_signed(delta).min(last),
        };
        self.selected = Some(next);
        self.selected
    }

    fn select_edge(&mut self, last: bool) -> Option<usize> {
        let rows = self.rows().len();
        self.selected = (rows > 0).then(|| if last { rows - 1 } else { 0 });
        self.selected
    }
}

/// Where the flyout's [`Ui`] lives in a window's state, and who carries out what it asks for.
pub struct Host<S: 'static> {
    pub ui: fn(&mut S) -> &mut Ui,
    pub after: fn(&mut S, &mut Tx),
}

impl<S: 'static> Clone for Host<S> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<S: 'static> Copy for Host<S> {}

impl<S: 'static> Host<S> {
    /// A press handler: `f` on the flyout's state, then the host's part.
    pub fn act(self, f: impl Fn(&mut Ui) + 'static) -> impl Fn(&mut S, &mut Tx) + 'static {
        move |s, tx| {
            f((self.ui)(s));
            (self.after)(s, tx);
        }
    }
}

// ---------------------------------------------------------------------------
// The flyout window
// ---------------------------------------------------------------------------

/// One line for the developer's console. Never a file name, a path or a secret.
fn log(message: impl AsRef<str>) {
    eprintln!("beebeeb flyout: {}", message.as_ref());
}

const FLYOUT: Host<FlyoutState> = Host {
    ui: |s| &mut s.tray,
    after: carry_out,
};

/// The flyout of the app on the real engine: the engine's source in place of the board's
/// fixture (`shell::run` is the one caller; without an engine the flyout keeps following the
/// fixture, [`Ui::following`]). From here the flyout reads the engine itself, also while the
/// main window is closed: every snapshot first takes what the engine reported
/// (`LiveEngine::drain`).
pub fn on_engine<C: beebeeb_engine::app::CredentialStores>(
    mut flyout: FlyoutState,
    live: Option<std::sync::Arc<crate::engine_host::LiveEngine<C>>>,
) -> FlyoutState {
    if let Some(live) = live {
        flyout.tray = Ui::new(Box::new(crate::engine_host::EngineTraySource::new(live)));
        flyout.tray.follow(&flyout.inputs);
    }
    flyout
}

/// The flyout's content (`registry::FLYOUT`).
pub fn view(s: &FlyoutState, cx: &mut Cx<FlyoutState>) -> El {
    let name = crate::shell::identity::AppIdentity::CURRENT.product_name();
    view::flyout(&s.tray, name, FLYOUT, cx.system_dark())
}

/// Carries out what the flyout asked for, through the shell: the window moves by
/// `host::request`, sync pauses by `ShellActions`.
fn carry_out(s: &mut FlyoutState, tx: &mut Tx) {
    for intent in s.tray.take_intents() {
        let go = |s: &FlyoutState, intent: Go| {
            if let Some(main) = &s.main {
                main.update(move |state: &mut State, _| {
                    host::request(state, intent);
                });
            }
        };
        match intent {
            Intent::OpenBeebeeb | Intent::OpenWindow => go(s, Go::Go(Route::section(Section::Files))),
            Intent::OpenSettings => go(s, Go::Settings(SettingsPane::General)),
            Intent::OpenSyncHistory { .. } => go(s, Go::Go(Route::section(Section::SyncHistory))),
            Intent::OpenTrash => go(s, Go::Go(Route::section(Section::Trash))),
            Intent::ReviewConflict => go(s, Go::ReviewConflict { file: None }),
            Intent::OpenInFileExplorer => {
                if let Err(error) = s.actions.open_in_file_explorer() {
                    log(format!("the flyout could not open File Explorer ({error:?})"));
                }
            }
            Intent::PauseSync | Intent::ResumeSync => {
                let done = if intent == Intent::PauseSync {
                    s.actions.pause_sync()
                } else {
                    s.actions.resume_sync()
                };
                if let Err(error) = done {
                    log(format!("the flyout could not pause or resume sync ({error:?})"));
                }
                // The b follows: the main window's shell reads the engine again.
                if let Some(main) = &s.main {
                    main.update(|state: &mut State, _| {
                        state.shell.refresh();
                    });
                }
                s.refresh();
            }
            Intent::Close => tx.close_window(),
        }
    }
}

// ---------------------------------------------------------------------------
// The frame gate's scenes: the flyout in a headless main window
// ---------------------------------------------------------------------------

const SCENE: Host<State> = Host {
    ui: |s| {
        s.tray
            .as_mut()
            .unwrap_or_else(|| unreachable!("a tray scene draws only while State::tray is set"))
    },
    // Nothing carries the intents out in a scene: a test reads them (`Ui::take_intents`).
    after: |_, _| {},
};

/// The group's board frames, each as a scene of the window (`frames::scenes`).
pub const FRAMES: &[(&str, fn(&mut State))] = &[
    ("DW-1202", |s| set_scene(s, "DW-1202")),
    ("DW-1203", |s| set_scene(s, "DW-1203")),
    ("DW-1204", |s| set_scene(s, "DW-1204")),
    ("DW-1205", |s| set_scene(s, "DW-1205")),
    ("DW-1206", |s| set_scene(s, "DW-1206")),
    ("DW-1207", |s| set_scene(s, "DW-1207")),
    ("DW-1208", |s| set_scene(s, "DW-1208")),
    ("DW-1209", |s| set_scene(s, "DW-1209")),
    ("DW-1210", |s| set_scene(s, "DW-1210")),
    ("DW-1211", |s| set_scene(s, "DW-1211")),
    ("DW-1212", |s| set_scene(s, "DW-1212")),
    ("DW-1213", |s| set_scene(s, "DW-1213")),
    ("DW-1214", |s| set_scene(s, "DW-1214")),
    ("DW-1215", |s| set_scene(s, "DW-1215")),
    ("DW-1216", |s| set_scene(s, "DW-1216")),
    ("DW-1219", |s| set_scene(s, "DW-1219")),
    ("DW-1220", |s| set_scene(s, "DW-1220")),
    ("DW-1221", |s| set_scene(s, "DW-1221")),
    ("DW-1222", |s| set_scene(s, "DW-1222")),
    // Commands and keyboard shortcuts, over the window (1278 x 798).
    ("DW-1232", shortcuts::open),
    ("DW-1233", |s| {
        shortcuts::open(s);
        s.shortcuts.page = shortcuts::Page::Window;
    }),
    // The same dialog while a document is open: the editor's four lists. The board draws the
    // document editor behind it, which is not built; the scene opens the note in the text
    // editor (DW-1523). No golden, so no gate: `FRAMES.md` says what is left.
    ("DW-1234", |s| {
        crate::frames::viewers::set_scene(s, "DW-1523");
        shortcuts::open(s);
    }),
];

/// The window's status line shows the b's flyout (DW-0401: "a click shows the b's flyout";
/// DW-1201), as a left click on the b does: the flyout reads the engine first, then opens where
/// it last opened. A second click, or the click that took the focus from an open flyout, closes
/// it (`shell::flyout::on_tray_click`).
pub fn show_from_window(s: &mut State) {
    s.shell.toggle_flyout(None);
}

/// Puts one board frame's flyout in the window (`--scene DW-1203 --size 340x520`).
pub fn set_scene(s: &mut State, frame: &str) {
    s.tray = Ui::board(frame);
}

/// The flyout as the window's whole content, while a scene asked for it.
pub fn scene(s: &State, cx: &mut Cx<State>) -> Option<El> {
    let ui = s.tray.as_ref()?;
    Some(view::flyout(ui, s.identity.product_name(), SCENE, cx.system_dark()))
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// The list's keys (DW-1202: "Arrow keys move through the rows; Enter shows the selected row
/// in File Explorer"). Esc is the shell's (`flyout_window::on_key`); Tab and Space are Rune's.
pub fn on_key<S: 'static>(host: Host<S>) -> impl Fn(&mut S, &KeyEvent, &mut Tx) -> bool + 'static {
    move |s, ev, tx| {
        let ui = (host.ui)(s);
        let moved = match ev.key {
            Key::Down => ui.move_selection(1),
            Key::Up => ui.move_selection(-1),
            Key::Home => ui.select_edge(false),
            Key::End => ui.select_edge(true),
            Key::Enter => {
                let Some(row) = ui.selected else { return false };
                ui.open_row(row);
                (host.after)(s, tx);
                return true;
            }
            _ => return false,
        };
        match moved {
            Some(row) => {
                tx.scroll_into_view(view::row_key(row));
                true
            }
            None => false,
        }
    }
}

/// The flyout window's keys beyond Esc (`flyout_window::on_key`): the arrow keys reach the list
/// straight after the flyout opens, before anything in it has focus.
pub fn on_window_key(s: &mut FlyoutState, ev: &KeyEvent, tx: &mut Tx) -> bool {
    on_key(FLYOUT)(s, ev, tx)
}

/// Which of the board's frames says what the shell's tray state says, in DW-1201's order (the
/// order of `shell::tray::tray_model_for`). The fixture follows it, so the flyout and the b
/// agree before the engine is linked.
pub(crate) fn frame_for(shell: &TrayInputs) -> &'static str {
    if shell.session_expired {
        "DW-1219"
    } else if shell.locked {
        "DW-1211"
    } else if !shell.explorer_ready {
        "DW-1215"
    } else if shell.paused {
        "DW-1204"
    } else if shell.engine == EngineState::Checking || shell.engine == EngineState::NotRunning {
        // The board draws no "sync isn't running"; its checking frame says nothing (DW-1212).
        "DW-1212"
    } else if shell.storage_full {
        "DW-1209"
    } else if shell.engine == EngineState::Offline {
        "DW-1206"
    } else if shell.engine == EngineState::Error {
        "DW-1207"
    } else if shell.conflicts > 0 {
        "DW-1205"
    } else if shell.update_available {
        "DW-1210"
    } else if shell.engine == EngineState::Syncing || shell.files_remaining.unwrap_or(0) > 0 {
        "DW-1203"
    } else {
        "DW-1202"
    }
}
