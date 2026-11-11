//! The frame registry: which function draws which part of the app.
//!
//! **A frame lane registers its group with ONE line in [`FRAMES`]** (a dialog lane: in
//! [`DIALOGS`]; the tray lane sets [`FLYOUT`]), and nothing else here changes.
//!
//! **Dialogs draw OVER the frame behind them.** A dialog is not a screen: the shell draws the
//! state's own frame first ([`group_for`]), then the scrim, then the dialog's card
//! ([`DialogFn`]) centred on it. A dialog lane returns its card and nothing else: no scrim, no
//! window chrome, no copy of what is behind (DW-0429 and DW-1232 are "the dialog in front" of
//! Beebeeb's window). A group that is not in the table draws the shell's "not built
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
    /// The first-run steps after sign-in, in the first-run window (DW-0201 to DW-0208).
    FirstRun,
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
            FrameGroup::FirstRun => "First run".into(),
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
            FrameGroup::SignIn => "First run and Sign in, in the 960 x 640 window (DW-A01, DW-A02)",
            FrameGroup::Locked => "Before Files: locked (DW-0009)",
            FrameGroup::RecoveryPhrase => "Email and password: the recovery phrase (DW-A23)",
            FrameGroup::NoPlan => "Before Files: no plan yet (DW-0014)",
            FrameGroup::KeyStoreUnavailable => "no board frame yet (the auth gate's KeyStoreUnavailable)",
            FrameGroup::FirstRun => "First run on Windows: a window of its own, 960 x 640 (DW-0201 to DW-0208)",
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

impl FrameGroup {
    /// The board frame the group draws (`design/desktop-v2/frames.json`), where the board has
    /// ONE for it; a note of the live feedback mode names it (`feedback/frame.rs`).
    #[cfg_attr(not(feature = "feedback"), allow(dead_code))]
    pub fn frame_id(self) -> Option<&'static str> {
        match self {
            FrameGroup::SignIn => Some("DW-A01"),
            FrameGroup::Locked => Some("DW-0009"),
            FrameGroup::RecoveryPhrase => Some("DW-A23"),
            FrameGroup::NoPlan => Some("DW-0014"),
            FrameGroup::KeyStoreUnavailable => None,
            FrameGroup::FirstRun => Some("DW-0201"),
            FrameGroup::Section(Section::Files) => Some("DW-0401"),
            FrameGroup::Section(Section::Shared) => Some("DW-0501"),
            FrameGroup::Section(Section::Photos) => Some("DW-0601"),
            FrameGroup::Section(Section::Backups) => Some("DW-0737"),
            FrameGroup::Section(Section::Trash) => Some("DW-0747"),
            FrameGroup::Section(Section::SyncHistory) => Some("DW-1301"),
            FrameGroup::Section(Section::AvailableOffline) => None,
            FrameGroup::Viewer => Some("DW-0408"),
            FrameGroup::Settings(_) => Some("DW-0701"),
            FrameGroup::ConflictDialog => Some("DW-0429"),
            FrameGroup::ShortcutsDialog => Some("DW-1232"),
            FrameGroup::Flyout => Some("DW-1201"),
        }
    }
}

/// Whether the live feedback mode (test builds: the `feedback` feature, `src/feedback/`) may run
/// on a frame group: its comment mode writes the window's picture and element tree to a local
/// folder. **Default deny:** a group says `Allowed` only once its lane has met the three duties
/// of `src/feedback/README.md` ("For frame lanes": mark every secret, prove it, keep secrets
/// still); until then its line says `Denied` with the reason the person reads after "Comment
/// mode is off on this screen: ". A group with no line is denied (`feedback::frame`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "feedback"), allow(dead_code))]
pub enum FeedbackPolicy {
    Allowed,
    Denied(&'static str),
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
    /// Read only by a build with the `feedback` feature.
    #[cfg_attr(not(feature = "feedback"), allow(dead_code))]
    pub feedback: FeedbackPolicy,
}

/// The built frame groups of the main window. ONE line per group.
pub const FRAMES: &[Frame] = &[
    Frame {
        group: FrameGroup::Section(Section::Files),
        view: files_frame::view,
        on_key: Some(files_frame::on_key),
        feedback: FeedbackPolicy::Allowed,
    },
    Frame {
        group: FrameGroup::Settings(SettingsPane::General),
        view: crate::frames::settings::view,
        on_key: Some(crate::frames::settings::on_key),
        feedback: FeedbackPolicy::Denied("its lane has not opted in yet"),
    },
    Frame {
        group: FrameGroup::Settings(SettingsPane::Notifications),
        view: crate::frames::settings::view,
        on_key: Some(crate::frames::settings::on_key),
        feedback: FeedbackPolicy::Denied("its lane has not opted in yet"),
    },
    // Frame { group: FrameGroup::Section(Section::Shared), view: crate::frames::shared::view, on_key: None, feedback: FeedbackPolicy::Denied("its lane has not opted in yet") },
    Frame {
        group: FrameGroup::SignIn,
        view: crate::frames::signin::frame,
        on_key: Some(crate::frames::signin::on_key),
        feedback: FeedbackPolicy::Denied("sign-in shows your email and takes your password"),
    },
];

/// The flyout's content, once the tray lane builds it: `Some(crate::frames::tray::view)`.
pub const FLYOUT: Option<FlyoutFn> = Some(crate::frames::tray::view);

/// A dialog's card: its own content, sized by itself. The shell layers it over the frame
/// behind (which stays drawn), on the scrim, centred. `dialog` is the route machine's state
/// for it (which file a conflict is about).
pub type DialogFn = fn(&Dialog, &State, &mut Cx<State>) -> El;

/// One built dialog: its card, and its keys if it takes any. While a dialog is open the frame
/// behind it gets no keys.
pub struct DialogFrame {
    pub group: FrameGroup,
    pub view: DialogFn,
    pub on_key: Option<KeyFn>,
    /// As [`Frame::feedback`]. With a dialog open, comment mode needs the dialog AND the frame
    /// behind it to allow it.
    #[cfg_attr(not(feature = "feedback"), allow(dead_code))]
    pub feedback: FeedbackPolicy,
}

/// The built dialogs. ONE line per dialog.
pub const DIALOGS: &[DialogFrame] = &[
    // DialogFrame { group: FrameGroup::ConflictDialog, view: crate::frames::conflict::card, on_key: None, feedback: FeedbackPolicy::Denied("its lane has not opted in yet") },
];

/// The scrim between a dialog and the window: the board's `--kit-scrim`, "color-mix(in oklch,
/// var(--color-black) 28%, transparent)" in light and 55% in dark. It belongs in
/// `theme/tokens.rs` once that file's lane takes it.
pub fn scrim(dark: bool) -> rune::Color {
    rune::Color {
        a: if dark { 0.55 } else { 0.28 },
        ..rune::Color::BLACK
    }
}

/// The group of the frame the window shows in this state, behind any dialog: the gate, then
/// settings, then the viewer, then the section.
pub fn group_for(app: &AppMachine) -> FrameGroup {
    match app.screen() {
        Screen::SignIn => FrameGroup::SignIn,
        Screen::Gate(GateState::Locked) => FrameGroup::Locked,
        Screen::Gate(GateState::NeedsRecoveryPhrase) => FrameGroup::RecoveryPhrase,
        Screen::Gate(GateState::NoPlan) => FrameGroup::NoPlan,
        Screen::Gate(GateState::FirstRun) => FrameGroup::FirstRun,
        Screen::Gate(_) => FrameGroup::KeyStoreUnavailable,
        Screen::Settings(pane) => FrameGroup::Settings(pane),
        Screen::Drive(route) if route.item.is_some() => FrameGroup::Viewer,
        Screen::Drive(route) => FrameGroup::Section(route.section),
    }
}

/// The group of the dialog in front, if one is open.
pub fn dialog_group_for(app: &AppMachine) -> Option<FrameGroup> {
    app.dialog().map(|dialog| match dialog {
        Dialog::Conflict { .. } => FrameGroup::ConflictDialog,
        Dialog::Shortcuts => FrameGroup::ShortcutsDialog,
    })
}

pub fn frame(group: FrameGroup) -> Option<&'static Frame> {
    FRAMES.iter().find(|frame| frame.group == group)
}

pub fn dialog_frame(group: FrameGroup) -> Option<&'static DialogFrame> {
    DIALOGS.iter().find(|dialog| dialog.group == group)
}

/// The main window's view: the registered frame for the state (or the placeholder), and over
/// it the open dialog on its scrim.
pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    // The frame gate's scenes of the flyout (`--scene DW-1203 --size 340x520`): never set in the running app.
    if let Some(flyout) = crate::frames::tray::scene(s, cx) {
        return flyout;
    }
    let group = group_for(&s.shell.app);
    let behind = match frame(group) {
        Some(frame) => (frame.view)(s, cx),
        None => placeholder::window(s, cx, group),
    };
    let (Some(dialog), Some(dialog_group)) = (s.shell.app.dialog(), dialog_group_for(&s.shell.app)) else {
        return behind;
    };
    let card = match dialog_frame(dialog_group) {
        Some(frame) => (frame.view)(dialog, s, cx),
        None => placeholder::dialog(cx, dialog_group),
    };
    rune::el()
        .key("window-with-dialog")
        .w_full()
        .h_full()
        .child(behind)
        .child(
            rune::el()
                .key("dialog-scrim")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .fill(scrim(cx.system_dark())),
        )
        .child(
            rune::col()
                .key("dialog-layer")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .center()
                // The window behind is inert while the dialog is open; a second way to close
                // it must not also close what is behind (Settings).
                .modal(false, |s: &mut State| {
                    if s.shell.app.dialog().is_some() {
                        super::host::dispatch(s, super::app::Event::Escape);
                    }
                })
                .child(card),
        )
}
