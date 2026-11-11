//! Which frame is on screen, and whether comment mode may run on it: read from the shell's
//! route machine (`shell::app::AppMachine`) and its frame registry (`shell::registry`).
//!
//! **Default deny.** Comment mode runs only where the frame group's ONE registry line says
//! `feedback: FeedbackPolicy::Allowed`. A group with no line (the placeholder is on screen) is
//! denied; the groups that show a password, a recovery phrase or a code say so in the reason.
//! With a dialog open, the dialog's line and the line of the frame behind it must both allow it.

use super::{FrameInfo, Policy};
use crate::shell::app::{AppMachine, Dialog, GateState, Screen};
use crate::shell::registry::{self, FrameGroup};

/// Why a group that has no registry line is denied.
pub fn unregistered(group: FrameGroup) -> &'static str {
    match group {
        FrameGroup::SignIn => "sign-in shows your email and takes your password",
        FrameGroup::FirstRun => "first-run setup can show your recovery phrase and codes",
        FrameGroup::RecoveryPhrase => "it takes your recovery phrase",
        FrameGroup::Locked => "the locked screen takes your password",
        _ => "this screen has not opted in",
    }
}

/// What the registry says for a group: its line's word, or the default denial.
pub fn policy(group: FrameGroup, declared: Option<Policy>) -> Policy {
    declared.unwrap_or(Policy::Denied(unregistered(group)))
}

/// The dialog in front and the frame behind it must both allow comment mode; the dialog's
/// refusal is the one shown when both refuse (it is what the person is looking at).
pub fn both(behind: Policy, dialog: Option<Policy>) -> Policy {
    match (dialog, behind) {
        (Some(Policy::Denied(why)), _) | (_, Policy::Denied(why)) => Policy::Denied(why),
        (Some(Policy::Allowed) | None, Policy::Allowed) => Policy::Allowed,
    }
}

/// The frame's id on the board, or the group's name where the board has no single frame.
fn id_of(group: FrameGroup) -> String {
    group
        .frame_id()
        .map_or_else(|| format!("no board frame ({})", group.name()), str::to_string)
}

/// Where the route machine is, in words: the section or pane by its slug, whether a folder or
/// a file is open (never its id or name), and the dialog in front.
pub fn route(app: &AppMachine) -> String {
    let mut route = match app.screen() {
        Screen::SignIn => "sign-in".to_string(),
        Screen::Gate(GateState::Locked) => "gate/locked".into(),
        Screen::Gate(GateState::NeedsRecoveryPhrase) => "gate/recovery-phrase".into(),
        Screen::Gate(GateState::NoPlan) => "gate/no-plan".into(),
        Screen::Gate(GateState::FirstRun) => "gate/first-run".into(),
        Screen::Gate(_) => "gate/credential-store-unavailable".into(),
        Screen::Settings(pane) => format!("settings/{}", pane.slug()),
        Screen::Drive(route) => {
            let mut text = route.section.slug().to_string();
            if route.folder.is_some() {
                text.push_str("/folder");
            }
            if route.item.is_some() {
                text.push_str("/item");
            }
            text
        }
    };
    match app.dialog() {
        Some(Dialog::Conflict { .. }) => route.push_str("#dialog=conflict"),
        Some(Dialog::Shortcuts) => route.push_str("#dialog=shortcuts"),
        None => {}
    }
    route
}

/// The name of the frame group on screen (the dialog in front, when one is open).
pub fn group_name(app: &AppMachine) -> String {
    registry::dialog_group_for(app)
        .unwrap_or_else(|| registry::group_for(app))
        .name()
}

/// The frame on screen for this state of the route machine.
pub fn on_screen(app: &AppMachine) -> FrameInfo {
    let behind = registry::group_for(app);
    let dialog = registry::dialog_group_for(app);
    let behind_policy = policy(behind, registry::frame(behind).map(|frame| frame.feedback));
    let dialog_policy = dialog.map(|group| policy(group, registry::dialog_frame(group).map(|frame| frame.feedback)));
    FrameInfo {
        id: id_of(dialog.unwrap_or(behind)),
        route: route(app),
        feedback: both(behind_policy, dialog_policy),
    }
}
