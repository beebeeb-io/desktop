//! Settings › Beebeeb in Finder (macOS; dm-18): where Beebeeb stands in Finder, read from
//! `engine_host::finder_setup::view()`, and the one thing to do about it. Adding, Ready, Turned off in
//! System Settings, Failed (its one sentence and its one action), Missing. Over it: Turn off / Turn on, Repair
//! with its question, and Sync, which shows paused while the extension is turned off (RELEASE_BLOCKERS 2179),
//! with the row for a folder macOS kept (`kept_unsynced_folder`).
//!
//! Must not: decide a state, a retry or a transition (the reconciler does). This page only reads the view and
//! sends the person's press. The copy is `finder_setup` spec §6.2's table and the design's "exact strings".

use crate::theme::icons_settings as ic;
use crate::theme::tokens::Theme;
use crate::ui::settings_controls::{self as controls, Emphasis};
use crate::ui::settings_dialog::{self as dialogs, Weight};
use crate::ui::settings_list::{self as lists, Item, Trailing};
use crate::ui::settings_notice::{self as notices, Tone};
use crate::ui::settings_shell as shell;
use crate::State;
use beebeeb_engine::finder_setup::driver::FinderSetupView;
use beebeeb_engine::surfaces::phase::{FinderFailureReason, FinderSetup};
use rune::El;
use std::sync::{Arc, Mutex, PoisonError};

use super::drive::SyncStatus;
use super::{Dialog, Intent, SettingsUi};

pub const TITLE: &str = "Beebeeb in Finder";
pub const SUB: &str = "Your vault appears under Locations in Finder.";
pub const ADDING: &str = "Adding Beebeeb to Finder…";
pub const READY: &str = "Your vault is under Locations in Finder.";
pub const MISSING: &str = "Beebeeb isn’t in Finder.";
pub const TURNED_OFF: &str = "Beebeeb is turned off in System Settings.";
pub const REPAIR_TITLE: &str = "Repair Beebeeb in Finder?";
pub const REPAIR_BODY: &str = "Beebeeb removes its Finder location and turns off Open Beebeeb at login, then adds itself back to Finder. Files waiting to upload are kept.";
pub const REPAIR_FAILED: &str = "Couldn’t repair Beebeeb in Finder";
pub const REPAIR_FAILED_WHY: &str = "Nothing was changed that you need to undo. Try again.";
pub const SYNC_PAUSED_OFF: &str = "Sync is paused";
pub const SYNC_PAUSED_OFF_WHY: &str = "Nothing syncs while Beebeeb is turned off in System Settings.";
pub const KEPT_TITLE: &str = "Files kept on this Mac";
pub const KEPT_WHY: &str = "Files that hadn’t reached your vault yet were kept on this Mac, in this folder:";

/// The sentence for a failure (spec §6.2), one per reason.
pub fn failure_sentence(reason: Option<FinderFailureReason>) -> &'static str {
    match reason {
        Some(FinderFailureReason::ExtensionLoading) => "macOS hasn’t finished loading Beebeeb’s Finder extension.",
        Some(FinderFailureReason::UserDisabled) => TURNED_OFF,
        Some(FinderFailureReason::NotInApplications) => {
            "Beebeeb is running from the disk image. Move it to Applications, then open it again."
        }
        Some(FinderFailureReason::FolderTaken) => {
            "An older Beebeeb installation still holds Beebeeb’s place in Finder. Contact support and we’ll help you clear it."
        }
        Some(FinderFailureReason::Signing) => {
            "This copy of Beebeeb can’t add itself to Finder. Download it again from beebeeb.io."
        }
        Some(FinderFailureReason::Timeout) => "macOS didn’t finish adding Beebeeb to Finder.",
        Some(FinderFailureReason::Unknown) | None => "Beebeeb couldn’t be added to Finder.",
    }
}

/// What a press on this page does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    TurnOn,
    TurnOff,
    /// Opens the question; `RepairNow` is its confirm.
    Repair,
    RepairNow,
    TryAgain,
    OpenSystemSettings,
    ShowApp,
    CopyDetails,
    OpenInFinder,
    PauseSync,
    ResumeSync,
    KeptShow,
    KeptDismiss,
}

impl Press {
    pub fn label(self) -> &'static str {
        match self {
            Self::TurnOn => "Turn on",
            Self::TurnOff => "Turn off",
            Self::Repair => "Repair…",
            Self::RepairNow => "Repair",
            Self::TryAgain => "Try again",
            Self::OpenSystemSettings => "Open System Settings",
            Self::ShowApp => "Show in Finder",
            Self::CopyDetails => "Copy details",
            Self::OpenInFinder => "Open in Finder",
            Self::PauseSync => "Pause syncing",
            Self::ResumeSync => "Resume sync",
            Self::KeptShow => "Show in Finder",
            Self::KeptDismiss => "Dismiss",
        }
    }
}

/// What the page reads: the reconciler's view, whether sync is paused, and the folder macOS kept.
#[derive(Clone, Debug)]
pub struct Page {
    pub view: FinderSetupView,
    pub sync_paused: bool,
    pub kept: Option<String>,
}

impl Page {
    /// Sync is paused while the extension is turned off, whatever the person chose.
    pub fn sync_is_paused(&self) -> bool {
        self.sync_paused || self.view.setup == FinderSetup::UserDisabled
    }

    /// The status row's buttons, in the order they are drawn. One action per state, then the always-there ones.
    pub fn buttons(&self) -> Vec<Press> {
        match self.view.setup {
            FinderSetup::Adding => vec![],
            FinderSetup::Ready => vec![Press::OpenInFinder, Press::Repair, Press::TurnOff],
            FinderSetup::UserDisabled => vec![Press::OpenSystemSettings],
            FinderSetup::Missing => match self.view.reason {
                Some(FinderFailureReason::NotInApplications) => vec![Press::ShowApp],
                _ => vec![Press::TurnOn],
            },
            FinderSetup::Failed => vec![match self.view.reason {
                Some(FinderFailureReason::UserDisabled) => Press::OpenSystemSettings,
                Some(FinderFailureReason::NotInApplications) => Press::ShowApp,
                Some(FinderFailureReason::FolderTaken | FinderFailureReason::Signing) => Press::CopyDetails,
                _ => Press::TryAgain,
            }],
        }
    }

    /// The line under the row's title.
    pub fn says(&self) -> &'static str {
        match self.view.setup {
            FinderSetup::Adding => ADDING,
            FinderSetup::Ready => READY,
            FinderSetup::UserDisabled => TURNED_OFF,
            FinderSetup::Missing => match self.view.reason {
                Some(r) => failure_sentence(Some(r)),
                None => MISSING,
            },
            FinderSetup::Failed => failure_sentence(self.view.reason),
        }
    }
}

/// A repair or a turn-off in flight on a worker, and how it ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Work {
    #[default]
    Idle,
    Repairing,
    RepairFailed,
}

/// What the page holds between draws. `page` is a fixed answer for scenes and tests; None reads the live reconciler.
#[derive(Default)]
pub struct FinderUi {
    pub page: Option<Page>,
    /// Every press while `page` is fixed (nothing reaches the reconciler then).
    pub pressed: Vec<Press>,
    pub work: Arc<Mutex<Work>>,
    kept: Option<String>,
}

impl FinderUi {
    /// The folder macOS kept, while its row shows.
    pub fn kept(&self) -> Option<&str> {
        self.kept.as_deref()
    }

    #[cfg(test)]
    pub fn set_kept_for_test(&mut self, folder: Option<&str>) {
        self.kept = folder.map(str::to_owned);
    }

    fn work(&self) -> Work {
        *self.work.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The page as it is now.
    pub fn read(&self, drive_paused: bool) -> Page {
        match &self.page {
            Some(page) => page.clone(),
            None => Page {
                view: crate::engine_host::finder_setup::view(),
                sync_paused: drive_paused,
                kept: self.kept.clone(),
            },
        }
    }
}

/// The page opened: the folder macOS kept is read once (`take_kept_folder`).
pub fn open(ui: &mut SettingsUi) {
    if ui.finder.page.is_none() {
        if let Some(folder) = crate::engine_host::finder_setup::take_kept_folder() {
            ui.finder.kept = Some(folder);
        }
    }
}

/// Does `press`. With a fixed page it is only recorded.
pub fn press(s: &mut State, press: Press) {
    let ui = &mut s.settings;
    match press {
        Press::Repair => {
            ui.dialog = Some(Dialog::Repair);
            return;
        }
        Press::RepairNow => ui.dialog = None,
        _ => {}
    }
    if ui.finder.page.is_some() {
        ui.finder.pressed.push(press);
        return;
    }
    match press {
        Press::TurnOn => crate::engine_host::finder_setup::turn_on(),
        Press::TurnOff => {
            std::thread::spawn(|| {
                let _ = crate::engine_host::finder_setup::turn_off();
            });
        }
        Press::RepairNow => {
            let work = ui.finder.work.clone();
            *work.lock().unwrap_or_else(PoisonError::into_inner) = Work::Repairing;
            std::thread::spawn(move || {
                let ended = match crate::engine_host::finder_setup::repair() {
                    Ok(()) => Work::Idle,
                    Err(_) => Work::RepairFailed,
                };
                *work.lock().unwrap_or_else(PoisonError::into_inner) = ended;
            });
        }
        Press::TryAgain => {
            crate::engine_host::finder_setup::try_again();
        }
        Press::OpenSystemSettings => ui.ask(Intent::Open("login-items-and-extensions")),
        Press::ShowApp => ui.ask(Intent::Open("finder-show-app")),
        Press::OpenInFinder => ui.ask(Intent::Open("finder-open")),
        Press::CopyDetails => {
            let view = crate::engine_host::finder_setup::view();
            let words = crate::engine_host::finder_setup::copy_details(&view, env!("CARGO_PKG_VERSION"), "macOS");
            ui.ask(Intent::Copy(words));
        }
        Press::PauseSync => ui.drive.set_paused(true),
        Press::ResumeSync => ui.drive.set_paused(false),
        Press::KeptShow => ui.ask(Intent::Open("kept-folder-show")),
        Press::KeptDismiss => {
            ui.finder.kept = None;
            ui.ask(Intent::Open("kept-folder-dismiss"));
        }
        Press::Repair => {}
    }
}

pub fn pane(ui: &SettingsUi, t: &Theme) -> (El, Option<El>) {
    let drive_paused = matches!(ui.drive.sync().status, SyncStatus::Paused);
    let page = ui.finder.read(drive_paused);
    let work = ui.finder.work();
    let mut n = 0;
    let repairing = work == Work::Repairing;
    let mut button = |p: Press, emphasis: Emphasis| {
        n += 1;
        // While a repair runs its button reads Repairing… and does nothing.
        if p == Press::Repair && repairing {
            return controls::button(n - 1, "Repairing…", None, Emphasis::Plain, t);
        }
        controls::button(n - 1, p.label(), None, emphasis, t).on_press(move |s: &mut State| press(s, p))
    };
    let mut children = vec![
        shell::crumb("Sync", t, |s: &mut State| super::go(s, "DW-0746")),
        shell::heading(TITLE, Some(SUB), t),
    ];

    if work == Work::RepairFailed {
        let again = button(Press::Repair, Emphasis::Plain);
        children.push(notices::note_with(
            0,
            Tone::Red,
            &ic::WARN_CIRCLE,
            REPAIR_FAILED,
            REPAIR_FAILED_WHY,
            Some(again),
            t,
        ));
    }

    // The status row.
    let tone_icon = match page.view.setup {
        FinderSetup::Ready => &ic::FOLDER_TAB,
        FinderSetup::Adding => &ic::SYNC,
        _ => &ic::WARN_CIRCLE,
    };
    let mut buttons = Vec::new();
    for p in page.buttons() {
        let emphasis = if buttons.is_empty() && !matches!(p, Press::TurnOff | Press::Repair | Press::OpenInFinder) {
            Emphasis::Primary
        } else {
            Emphasis::Plain
        };
        buttons.push(button(p, emphasis));
    }
    let mut status = Item::new(Some(tone_icon), "Finder").sub(page.says());
    if page.view.setup == FinderSetup::Failed && page.view.attempt > 0 && page.view.max_attempts > 0 {
        status = status.trailing(Trailing::Mono(format!(
            "{} of {}",
            page.view.attempt, page.view.max_attempts
        )));
    }
    children.push(lists::list(
        0,
        Some("Finder"),
        vec![lists::item_with(0, 0, &status, buttons, t)],
        None,
        t,
    ));

    // Sync: paused while the extension is turned off.
    let off = page.view.setup == FinderSetup::UserDisabled;
    let (says, act) = match (off, page.sync_is_paused()) {
        (true, _) => (SYNC_PAUSED_OFF_WHY, Press::ResumeSync),
        (false, true) => ("Paused", Press::ResumeSync),
        (false, false) => ("Up to date", Press::PauseSync),
    };
    let mut sync_item = Item::new(Some(&ic::SYNC), "Sync").sub(says);
    if off {
        sync_item = sync_item.trailing(Trailing::Value(SYNC_PAUSED_OFF.into()));
    }
    let mut sync_rows = vec![lists::item_with(
        1,
        0,
        &sync_item,
        vec![button(act, Emphasis::Plain)],
        t,
    )];
    if let Some(folder) = &page.kept {
        sync_rows.push(lists::item_with(
            1,
            1,
            &Item::new(Some(&ic::FOLDER), KEPT_TITLE)
                .sub(format!("{KEPT_WHY} {folder}"))
                .ruled(),
            vec![
                button(Press::KeptShow, Emphasis::Plain),
                button(Press::KeptDismiss, Emphasis::Plain),
            ],
            t,
        ));
    }
    children.push(lists::list(1, Some("Sync"), sync_rows, None, t));

    let overlay = matches!(ui.dialog, Some(Dialog::Repair)).then(|| repair_question(n, t));
    (shell::pane(children, t), overlay)
}

/// Repair Beebeeb in Finder: what the repair does, before it does it. The confirm stays amber (design).
fn repair_question(first: usize, t: &Theme) -> El {
    let foot = vec![
        dialogs::button(first, "Repair", Weight::Primary, t)
            .default_button()
            .on_press(|s: &mut State| press(s, Press::RepairNow)),
        dialogs::button(first + 1, "Cancel", Weight::Quiet, t).on_press(|s: &mut State| s.settings.dialog = None),
    ];
    dialogs::alert(
        "Repair Beebeeb in Finder",
        380.0,
        notices::tile(0, Tone::Neutral, &ic::FOLDER_TAB, t),
        REPAIR_TITLE,
        REPAIR_BODY,
        foot,
        t,
    )
    .modal(false, |s: &mut State| s.settings.dialog = None)
}

// ---------------------------------------------------------------------------
// Scenes (`--snap`): one per state, plus the Repair question.
// ---------------------------------------------------------------------------

/// A scene's page: the state, its reason, and the Sync line.
pub fn show(s: &mut State, setup: FinderSetup, reason: Option<FinderFailureReason>, repair: bool) {
    use beebeeb_engine::finder_setup::launch_location::LaunchLocation;
    let mut view = FinderSetupView::initial(LaunchLocation::Applications);
    view.setup = setup;
    view.reason = reason;
    if setup == FinderSetup::Failed {
        view.attempt = 3;
        view.max_attempts = 3;
    }
    s.settings.finder.page = Some(Page {
        view,
        sync_paused: false,
        kept: None,
    });
    super::open_on(s, "DM-18");
    if repair {
        s.settings.dialog = Some(Dialog::Repair);
    }
}

#[cfg(test)]
#[path = "finder_tests.rs"]
mod tests;
