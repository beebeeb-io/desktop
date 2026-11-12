//! The board's frames, one module per group (one lane each). A group exposes `view`, `on_key`
//! and a `FRAMES` table of its board frames (frame id, and the scene that shows it).
//!
//! A new group adds TWO lines here: its `pub mod`, and its `FRAMES` in [`scenes`]. The shell
//! names its own scenes once (`shell::SCENES`) and never changes for a new group.
//!
//! Lane f-shared-search-trash: `shared` (DW-05xx), `trash` (DW-0747…, DW-0117), drawn from a
//! `<Group>Source`; their scenes are named in `shell::SCENES`. The checklist of frames, built and
//! not built, is `shared/FRAMES.md`.

pub mod account;
pub mod files;
pub mod global;
pub mod settings;
pub mod shared;
pub mod signin;
pub mod trash;

use std::sync::OnceLock;

use crate::State;
use crate::shell::app::{Event, Route, Section};
use crate::shell::registry::{FeedbackPolicy, Frame, FrameGroup};
use crate::ui::gate::Scene;
use crate::ui::sidebar::SidebarSection;
use rune::{KeyEvent, Tx};

/// Every scene the app can be rendered in (`--scene`, `scripts/compare.sh`): the shell's own,
/// then every group's board frames. Built once.
pub fn scenes(shell: &'static [Scene<State>]) -> &'static [Scene<State>] {
    static ALL: OnceLock<Vec<Scene<State>>> = OnceLock::new();
    ALL.get_or_init(|| {
        [
            shell,
            files::SCENES,
            global::FRAMES,
            settings::FRAMES,
            account::FRAMES,
            signin::FRAMES,
            signin::setup::FRAMES,
        ]
        .concat()
    })
}

/// Somewhere a control wants to go that this lane does not draw: the shell's route machine
/// (or the lane that owns the target frame) picks it up. A control never does nothing, and an
/// intent about ONE item always carries that item's id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Open a board frame that is about no single item (`DW-0749` Empty Trash, `DW-0504` New
    /// request, `DW-0415` New, `DW-0406` Sort).
    Frame(&'static str),
    /// Open a board frame about the item with this id (`DW-0411` Details, `DW-0511` People
    /// with access, `DW-1180` Show in File Explorer).
    About(&'static str, String),
    /// Open the item with this id as Files would.
    OpenItem(String),
    /// Something the engine or the shell does with the item of this id, named by the board's
    /// words ("copy-link", "preview", "leave", "delete-permanently"): a clipboard write, a
    /// confirmation, a further menu. For "search" the string is what was typed.
    Do(&'static str, String),
}

/// What the status line says about this PC: the engine's words, not a view's.
pub trait PcStatus {
    /// The sync state in words ("Up to date").
    fn sync_words(&self) -> String;
    /// How much is on this PC: the status line of a section with nothing of its own to count.
    fn summary(&self) -> String;
}

/// The board's sample PC (the words Files shows in DW-0401).
pub struct BoardPc;

impl PcStatus for BoardPc {
    fn sync_words(&self) -> String {
        "Up to date".into()
    }
    fn summary(&self) -> String {
        "4 on this PC \u{00B7} 3 available offline \u{00B7} 4 online only".into()
    }
}

/// What the lane's groups keep on `State` (ONE field: `State::frames`).
pub struct Ui {
    pub shared: shared::SharedUi,
    pub trash: trash::TrashUi,
    /// The navigation intents emitted so far, oldest first.
    pub intents: Vec<Intent>,
    /// What is typed in the toolbar's search field of these sections (not Files' field).
    pub search: rune::TextEdit,
    pub pc: Box<dyn PcStatus>,
    /// The window opened on this section, with no place before it: Back has nowhere to go.
    /// The board draws its frames so; only a scene sets it (the route machine always starts
    /// on Files, so in the running app a section has Files behind it).
    pub first_place: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Ui {
            shared: shared::SharedUi::default(),
            trash: trash::TrashUi::default(),
            intents: Vec::new(),
            search: rune::TextEdit::new(""),
            pc: Box::new(BoardPc),
            first_place: false,
        }
    }
}

/// The registry's lines (`shell::registry::FRAMES`): Shared and Trash. Neither shows a secret;
/// comment mode stays off until the lane has the proving test of `feedback/README.md`.
pub const SHARED: Frame = Frame {
    group: FrameGroup::Section(Section::Shared),
    view: shared::view,
    on_key: Some(shared::on_key),
    feedback: FeedbackPolicy::Denied("its lane has not opted in yet"),
};
pub const TRASH: Frame = Frame {
    group: FrameGroup::Section(Section::Trash),
    view: trash::view,
    on_key: Some(trash::on_key),
    feedback: FeedbackPolicy::Denied("its lane has not opted in yet"),
};

/// Put the window on a section, as a click in the sidebar does (the tests).
pub fn go(s: &mut State, section: Section) {
    crate::shell::host::dispatch(s, Event::Navigate(Route::section(section)));
}

/// A board frame: the section as the window's first place (`Ui::first_place`).
pub fn scene(s: &mut State, section: Section) {
    go(s, section);
    s.frames.first_place = true;
}

/// Whether Back and Forward have somewhere to go from this section.
pub fn history(s: &State) -> (bool, bool) {
    if s.frames.first_place {
        return (false, false);
    }
    (s.shell.app.can_go_back(), s.shell.app.can_go_forward())
}

/// The keys of the section the window shows. `None`: not one of this lane's sections.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> Option<bool> {
    match s.sidebar_section {
        SidebarSection::Shared => Some(shared::on_key(s, ev, tx)),
        SidebarSection::Trash => Some(trash::on_key(s, ev, tx)),
        _ => None,
    }
}

/// The root key handler of this lane's windows (`El::on_key`).
pub fn on_key_root(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    on_key(s, ev, tx).unwrap_or(false)
}

/// Keys the lists of both sections share: whether `ev` is a plain key (no modifier).
pub fn plain(ev: &KeyEvent) -> bool {
    let m = ev.mods;
    !m.cmd && !m.ctrl && !m.alt && !m.shift && !m.logo
}

/// The Menu key, or Shift+F10.
pub fn is_menu_key(ev: &KeyEvent) -> bool {
    let m = ev.mods;
    let bare = !m.cmd && !m.ctrl && !m.alt && !m.logo;
    match ev.key {
        rune::Key::ContextMenu => bare && !m.shift,
        rune::Key::Function(10) => bare && m.shift,
        _ => false,
    }
}

/// Alt+Enter: Details, as the menus say.
pub fn is_alt_enter(ev: &KeyEvent) -> bool {
    let m = ev.mods;
    ev.key == rune::Key::Enter && m.alt && !m.cmd && !m.ctrl && !m.shift && !m.logo
}

/// Space, alone.
pub fn is_space(ev: &KeyEvent) -> bool {
    plain(ev) && matches!(&ev.key, rune::Key::Text(t) if t == " ")
}
