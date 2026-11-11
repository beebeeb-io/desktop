//! The frame registry: which function draws which part of the app.
//!
//! **A frame lane registers its group with ONE line in [`FRAMES`]** (or sets [`FLYOUT`]), and
//! nothing else here changes. A group that is not in the table draws the shell's "not built
//! yet" frame (`placeholder.rs`), which names the group and its board frames and never shows
//! invented data.

use rune::{Cx, El, KeyEvent, Tx};

use super::app::{AppMachine, Dialog, GateState, Screen, Section, SettingsPane};
use super::flyout_window::FlyoutState;
use super::{files_frame, placeholder};
use crate::State;

/// A part of the app one lane builds. Every state of the route machine maps to exactly one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameGroup {
    SignIn,
    Locked,
    RecoveryPhrase,
    NoPlan,
    KeyStoreUnavailable,
    Section(Section),
    /// An open file in the window's content area.
    Viewer,
    Settings(SettingsPane),
    ConflictDialog,
    ShortcutsDialog,
    /// The notification-area flyout (its own window).
    Flyout,
}

impl FrameGroup {
    pub fn name(self) -> String {
        match self {
            FrameGroup::SignIn => "Sign in".into(),
            FrameGroup::Locked => "Locked".into(),
            FrameGroup::RecoveryPhrase => "Recovery phrase".into(),
            FrameGroup::NoPlan => "No plan yet".into(),
            FrameGroup::KeyStoreUnavailable => "Credential store unavailable".into(),
            FrameGroup::Section(section) => section.label().into(),
            FrameGroup::Viewer => "Open file".into(),
            FrameGroup::Settings(pane) => format!("Settings: {}", pane.label()),
            FrameGroup::ConflictDialog => "Two versions".into(),
            FrameGroup::ShortcutsDialog => "Commands and keyboard shortcuts".into(),
            FrameGroup::Flyout => "Flyout".into(),
        }
    }

    /// Where the group is on the board: the section titles of `design/desktop-v2/FRAMES.md`
    /// (Windows tab), with the frame id where the group is a single frame.
    pub fn board_section(self) -> &'static str {
        match self {
            FrameGroup::SignIn => "Sign in (DW-A02), and the sign-in sections after it",
            FrameGroup::Locked => "Before Files: locked (DW-0009)",
            FrameGroup::RecoveryPhrase => "Email and password: the recovery phrase (DW-A23)",
            FrameGroup::NoPlan => "Before Files: no plan yet (DW-0014)",
            FrameGroup::KeyStoreUnavailable => "no board frame yet (the auth gate's KeyStoreUnavailable)",
            FrameGroup::Section(Section::Files) => "Files: the full Drive in the window (DW-0401)",
            FrameGroup::Section(Section::Shared) => "Shared (DW-0501)",
            FrameGroup::Section(Section::Photos) => "Photos (DW-0601)",
            FrameGroup::Section(Section::Backups) => "Backups on Windows (DW-0737)",
            FrameGroup::Section(Section::Trash) => "Trash (DW-0747)",
            FrameGroup::Section(Section::SyncHistory) => "This PC: Sync history and Available offline (DW-1301)",
            FrameGroup::Section(Section::AvailableOffline) => "This PC: Sync history and Available offline",
            FrameGroup::Viewer => "Open files: the viewers (DW-0408, DW-1501)",
            FrameGroup::Settings(_) => "Settings, inside the window (DW-0701)",
            FrameGroup::ConflictDialog => "Folders, Backups and uploads: two versions (DW-0429)",
            FrameGroup::ShortcutsDialog => "The b, its menu, and the app menus (DW-1232)",
            FrameGroup::Flyout => "The notification area: the b and its flyout (DW-1201)",
        }
    }
}

pub type FrameFn = fn(&State, &mut Cx<State>) -> El;
pub type KeyFn = fn(&mut State, &KeyEvent, &mut Tx) -> bool;
pub type FlyoutFn = fn(&FlyoutState, &mut Cx<FlyoutState>) -> El;

/// One built frame group of the main window: its view, and its keys if it takes any (the
/// shell's own shortcuts run first: `host::on_key`).
pub struct Frame {
    pub group: FrameGroup,
    pub view: FrameFn,
    pub on_key: Option<KeyFn>,
}

/// The built frame groups of the main window. ONE line per group.
pub const FRAMES: &[Frame] = &[
    Frame {
        group: FrameGroup::Section(Section::Files),
        view: files_frame::view,
        on_key: Some(files_frame::on_key),
    },
    // Frame { group: FrameGroup::Section(Section::Shared), view: crate::frames::shared::view, on_key: None },
];

/// The flyout's content, once the tray lane builds it: `Some(crate::frames::tray::view)`.
pub const FLYOUT: Option<FlyoutFn> = None;

/// The group that draws the main window in this state: a dialog in front wins, then the
/// gate, then settings, then the viewer, then the section.
pub fn group_for(app: &AppMachine) -> FrameGroup {
    match app.dialog() {
        Some(Dialog::Conflict { .. }) => return FrameGroup::ConflictDialog,
        Some(Dialog::Shortcuts) => return FrameGroup::ShortcutsDialog,
        None => {}
    }
    match app.screen() {
        Screen::SignIn => FrameGroup::SignIn,
        Screen::Gate(GateState::Locked) => FrameGroup::Locked,
        Screen::Gate(GateState::NeedsRecoveryPhrase) => FrameGroup::RecoveryPhrase,
        Screen::Gate(GateState::NoPlan) => FrameGroup::NoPlan,
        Screen::Gate(_) => FrameGroup::KeyStoreUnavailable,
        Screen::Settings(pane) => FrameGroup::Settings(pane),
        Screen::Drive(route) if route.item.is_some() => FrameGroup::Viewer,
        Screen::Drive(route) => FrameGroup::Section(route.section),
    }
}

pub fn frame(group: FrameGroup) -> Option<&'static Frame> {
    FRAMES.iter().find(|frame| frame.group == group)
}

/// The main window's view: the registered frame for the state, or the placeholder.
pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let group = group_for(&s.shell.app);
    match frame(group) {
        Some(frame) => (frame.view)(s, cx),
        None => placeholder::window(s, cx, group),
    }
}
