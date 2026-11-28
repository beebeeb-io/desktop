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
pub const TURN_OFF_FAILED: &str = "Couldn’t turn off Beebeeb in Finder";
pub const TURNING_OFF: &str = "Turning off…";

/// The failed turn-off's line: what macOS said, then the one thing to do.
pub fn turn_off_failed_why(error: &str) -> String {
    let error = error.trim().trim_end_matches('.');
    if error.is_empty() {
        "Beebeeb is still in Finder. Try again.".to_string()
    } else {
        format!("Beebeeb is still in Finder. macOS said: {error}. Try again.")
    }
}

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
    /// The failed turn-off's "Try again": the same removal once more.
    RetryTurnOff,
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
            Self::RetryTurnOff => "Try again",
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
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Work {
    #[default]
    Idle,
    Repairing,
    RepairFailed,
    TurningOff,
    /// The removal failed: the reconciler holds and Beebeeb stays in Finder. The error is shown with "Try again".
    TurnOffFailed(String),
}

/// What the page holds between draws. `page` is a fixed answer for scenes and tests; None reads the live reconciler.
#[derive(Default)]
pub struct FinderUi {
    pub page: Option<Page>,
    /// Every press while `page` is fixed (nothing reaches the reconciler then).
    pub pressed: Vec<Press>,
    pub work: Arc<Mutex<Work>>,
    kept: Option<String>,
    /// A folder a worker's removal kept, there until the next press or open moves it into `kept` (the pane is drawn
    /// from `&SettingsUi`, so a worker cannot set `kept` itself). The row shows it at once.
    arrived: Arc<Mutex<Option<String>>>,
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
        self.work.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn set_work(&self, work: Work) {
        *self.work.lock().unwrap_or_else(PoisonError::into_inner) = work;
    }

    /// The folder the row shows: one a worker just brought, else the one held.
    fn shown_kept(&self) -> Option<String> {
        let arrived = self.arrived.lock().unwrap_or_else(PoisonError::into_inner).clone();
        arrived.or_else(|| self.kept.clone())
    }

    /// Moves a folder a worker brought into `kept` (newer replaces older).
    fn settle_kept(&mut self) {
        if let Some(folder) = self.arrived.lock().unwrap_or_else(PoisonError::into_inner).take() {
            self.kept = Some(folder);
        }
    }

    /// The page as it is now.
    pub fn read(&self, drive_paused: bool) -> Page {
        match &self.page {
            Some(page) => page.clone(),
            None => Page {
                view: crate::engine_host::finder_setup::view(),
                sync_paused: drive_paused,
                kept: self.shown_kept(),
            },
        }
    }
}

/// The page opened: a folder macOS kept since the last open (`take_kept_folder`, saved to the settings file so a
/// restart keeps it), else the one the settings file holds (`kept_unsynced_folder`) until it is dismissed.
pub fn open(ui: &mut SettingsUi) {
    if ui.finder.page.is_some() {
        return;
    }
    ui.finder.settle_kept();
    if let Some(folder) = crate::engine_host::finder_setup::take_kept_folder() {
        saved::remember(&folder);
        ui.finder.kept = Some(folder);
    }
    if ui.finder.kept.is_none() {
        ui.finder.kept = saved::read();
    }
}

/// How a removal on a worker ended, handed to the page: the work's end state, and the folder macOS kept (with a
/// success or with an error) goes to the row and the settings file, so it outlives a closed pane and a restart.
fn finish(
    work: &Mutex<Work>,
    arrived: &Mutex<Option<String>>,
    ended: Work,
    kept: Option<String>,
    remember: impl FnOnce(&str),
) {
    if let Some(folder) = kept {
        remember(&folder);
        *arrived.lock().unwrap_or_else(PoisonError::into_inner) = Some(folder);
    }
    *work.lock().unwrap_or_else(PoisonError::into_inner) = ended;
}

/// Starts a turn-off on a worker: `remove` is the removal (the host's `turn_off`), `take` hands back the folder it
/// kept (`take_kept_folder`). The removal blocks about a second, so never on the main thread.
fn start_turn_off(
    ui: &FinderUi,
    remove: impl FnOnce() -> Result<(), String> + Send + 'static,
    take: impl FnOnce() -> Option<String> + Send + 'static,
    remember: impl FnOnce(&str) + Send + 'static,
) -> std::thread::JoinHandle<()> {
    ui.set_work(Work::TurningOff);
    let (work, arrived) = (ui.work.clone(), ui.arrived.clone());
    std::thread::spawn(move || {
        let ended = match remove() {
            Ok(()) => Work::Idle,
            Err(error) => {
                tracing::warn!("finder setup: Settings turn-off failed");
                Work::TurnOffFailed(error)
            }
        };
        finish(&work, &arrived, ended, take(), remember);
    })
}

/// The settings file's `kept_unsynced_folder` (macOS; merge-only writes under the config lock). Never in tests:
/// those must not touch the real settings file.
mod saved {
    #[cfg(all(target_os = "macos", not(test)))]
    pub fn remember(folder: &str) {
        use crate::shell::platform::macos::macos_fp::removal;
        if let Err(error) = removal::remember_kept_folder(crate::shell::identity::engine_identity(), folder) {
            tracing::warn!("finder setup: the kept folder could not be saved: {error}");
        }
    }

    #[cfg(all(target_os = "macos", not(test)))]
    pub fn read() -> Option<String> {
        beebeeb_engine::config::DesktopConfig::load_for(crate::shell::identity::engine_identity())
            .ok()
            .and_then(|cfg| cfg.kept_unsynced_folder)
    }

    #[cfg(all(target_os = "macos", not(test)))]
    pub fn dismiss(shown: &str) {
        use crate::shell::platform::macos::macos_fp::removal;
        if let Err(error) = removal::dismiss_saved_kept_folder(crate::shell::identity::engine_identity(), shown) {
            tracing::warn!("finder setup: the kept folder could not be dismissed: {error}");
        }
    }

    #[cfg(any(not(target_os = "macos"), test))]
    pub fn remember(_folder: &str) {}

    #[cfg(any(not(target_os = "macos"), test))]
    pub fn read() -> Option<String> {
        None
    }

    #[cfg(any(not(target_os = "macos"), test))]
    pub fn dismiss(_shown: &str) {}
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
    ui.finder.settle_kept();
    match press {
        Press::TurnOn => {
            if matches!(ui.finder.work(), Work::TurnOffFailed(_)) {
                ui.finder.set_work(Work::Idle);
            }
            crate::engine_host::finder_setup::turn_on();
        }
        Press::TurnOff | Press::RetryTurnOff => {
            if ui.finder.work() != Work::TurningOff {
                start_turn_off(
                    &ui.finder,
                    crate::engine_host::finder_setup::turn_off,
                    crate::engine_host::finder_setup::take_kept_folder,
                    saved::remember,
                );
            }
        }
        Press::RepairNow => {
            let (work, arrived) = (ui.finder.work.clone(), ui.finder.arrived.clone());
            ui.finder.set_work(Work::Repairing);
            std::thread::spawn(move || {
                let ended = match crate::engine_host::finder_setup::repair() {
                    Ok(()) => Work::Idle,
                    Err(_) => Work::RepairFailed,
                };
                let kept = crate::engine_host::finder_setup::take_kept_folder();
                finish(&work, &arrived, ended, kept, saved::remember);
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
            if let Some(shown) = ui.finder.kept.take() {
                saved::dismiss(&shown);
            }
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
    let turning_off = work == Work::TurningOff;
    let mut button = |p: Press, emphasis: Emphasis| {
        n += 1;
        // While a repair or a turn-off runs its button reads Repairing… / Turning off… and does nothing.
        if p == Press::Repair && repairing {
            return controls::button(n - 1, "Repairing…", None, Emphasis::Plain, t);
        }
        if matches!(p, Press::TurnOff | Press::RetryTurnOff) && turning_off {
            return controls::button(n - 1, TURNING_OFF, None, Emphasis::Plain, t);
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
    if let Work::TurnOffFailed(error) = &work {
        let again = button(Press::RetryTurnOff, Emphasis::Plain);
        children.push(notices::note_with(
            0,
            Tone::Red,
            &ic::WARN_CIRCLE,
            TURN_OFF_FAILED,
            &turn_off_failed_why(error),
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

/// dm-98: Settings "Turn off" keeps its result. A failed removal shows its error with Try again, and the folder macOS
/// kept reaches the row and the settings file without the pane being opened again.
#[cfg(test)]
mod turn_off_tests {
    use super::*;
    use std::sync::Arc;

    fn remembered() -> (Arc<Mutex<Vec<String>>>, impl FnOnce(&str) + Send + 'static) {
        let saved = Arc::new(Mutex::new(Vec::new()));
        let into = saved.clone();
        (saved, move |folder: &str| into.lock().unwrap().push(folder.to_owned()))
    }

    #[test]
    fn settings_a_failed_turn_off_shows_its_error_with_try_again() {
        let ui = FinderUi::default();
        let (_, remember) = remembered();
        start_turn_off(
            &ui,
            || Err("The operation couldn’t be completed.".into()),
            || None,
            remember,
        )
        .join()
        .unwrap();
        let Work::TurnOffFailed(error) = ui.work() else {
            panic!("a failed removal is kept, not thrown away: {:?}", ui.work());
        };
        assert_eq!(
            turn_off_failed_why(&error),
            "Beebeeb is still in Finder. macOS said: The operation couldn’t be completed. Try again."
        );
        assert_eq!(Press::RetryTurnOff.label(), "Try again");
        assert_eq!(turn_off_failed_why("  "), "Beebeeb is still in Finder. Try again.");
    }

    #[test]
    fn settings_a_turn_off_shows_turning_off_then_ends_idle() {
        let ui = FinderUi::default();
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let (_, remember) = remembered();
        let worker = start_turn_off(
            &ui,
            move || {
                gate_rx.recv().unwrap();
                Ok(())
            },
            || None,
            remember,
        );
        assert_eq!(ui.work(), Work::TurningOff, "the press shows at once");
        gate_tx.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(ui.work(), Work::Idle);
    }

    #[test]
    fn settings_the_kept_path_survives_without_the_pane() {
        let mut ui = FinderUi::default();
        let (saved, remember) = remembered();
        start_turn_off(&ui, || Ok(()), || Some("/Users/x/Beebeeb (kept)".into()), remember)
            .join()
            .unwrap();
        assert_eq!(
            saved.lock().unwrap().as_slice(),
            ["/Users/x/Beebeeb (kept)"],
            "saved to the settings file, so a closed pane or a restart keeps it"
        );
        assert_eq!(
            ui.shown_kept().as_deref(),
            Some("/Users/x/Beebeeb (kept)"),
            "the open row shows it at once"
        );
        ui.settle_kept();
        assert_eq!(ui.kept(), Some("/Users/x/Beebeeb (kept)"), "Show in Finder reads it");
    }

    #[test]
    fn settings_a_folder_kept_with_a_failed_turn_off_is_not_lost() {
        let mut ui = FinderUi::default();
        ui.set_kept_for_test(Some("/older"));
        let (saved, remember) = remembered();
        start_turn_off(&ui, || Err("busy".into()), || Some("/newer".into()), remember)
            .join()
            .unwrap();
        assert!(matches!(ui.work(), Work::TurnOffFailed(_)));
        assert_eq!(saved.lock().unwrap().as_slice(), ["/newer"]);
        assert_eq!(
            ui.shown_kept().as_deref(),
            Some("/newer"),
            "a newer folder replaces an older one"
        );
    }

    #[test]
    fn settings_a_live_dismiss_clears_the_folder_a_worker_brought() {
        let mut s = crate::init();
        s.settings.finder.page = None;
        *s.settings.finder.arrived.lock().unwrap() = Some("/k".into());
        press(&mut s, Press::KeptDismiss);
        assert_eq!(s.settings.finder.kept(), None);
        assert_eq!(s.settings.finder.shown_kept(), None);
    }

    /// Only macOS wires the Finder pane; the failed turn-off draws its red note with its button. The scene's fixed
    /// Ready page keeps the status row's buttons known; the work is read whatever the page.
    #[cfg(target_os = "macos")]
    #[test]
    fn settings_a_failed_turn_off_draws_a_note_with_try_again() {
        let drawn = |work: Work| {
            let mut state = crate::init();
            assert!(super::super::scene("DM-18-ready", &mut state));
            state.settings.finder.set_work(work);
            let mut rt = rune::Runtime::new(
                state,
                crate::view,
                (958.0, 998.0),
                rune::Env {
                    system_dark: false,
                    scale: 2.0,
                },
            );
            rt.frame(0.0);
            rt.frame(0.016);
            rt.settle_all();
            rt.frame(0.016);
            let has = |key: &str| {
                rt.elements()
                    .iter()
                    .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
            };
            (has("note-0"), has("btn-4"))
        };
        // Ready draws four buttons (Open in Finder, Repair…, Turn off, the Sync row's); Try again makes a fifth.
        assert_eq!(
            drawn(Work::Idle),
            (false, false),
            "no note and four buttons without a failure"
        );
        assert_eq!(
            drawn(Work::TurnOffFailed("busy".into())),
            (true, true),
            "the error shows, with its Try again"
        );
    }
}
