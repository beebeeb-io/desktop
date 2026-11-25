//! Beebeeb in Finder (dm-18): the copy is the design's "exact strings", every state offers its own buttons, and a
//! press reaches the right place. The page reads a fixed answer here: nothing reaches the reconciler.

use super::*;
use crate::{init, view, State};
use rune::{Env, Runtime};

const DIALOGS_HTML: &str = include_str!("../../../../../design/hifi/macos-settings-dialogs.html");
const SURFACE_HTML: &str = include_str!("../../../../../design/hifi/macos-two-surface.html");

fn page(setup: FinderSetup, reason: Option<FinderFailureReason>) -> Page {
    use beebeeb_engine::finder_setup::launch_location::LaunchLocation;
    let mut view = FinderSetupView::initial(LaunchLocation::Applications);
    view.setup = setup;
    view.reason = reason;
    Page {
        view,
        sync_paused: false,
        kept: None,
    }
}

fn runtime(scene: &str) -> Runtime<State> {
    let mut state = init();
    assert!(super::super::scene(scene, &mut state), "{scene} is built");
    let mut rt = Runtime::new(
        state,
        view,
        (958.0, 998.0),
        Env {
            system_dark: false,
            scale: 2.0,
        },
    );
    rt.frame(0.0);
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
    rt
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

#[test]
fn the_copy_is_the_designs_exact_strings() {
    for words in [
        KEPT_WHY,
        REPAIR_TITLE,
        REPAIR_BODY,
        REPAIR_FAILED,
        REPAIR_FAILED_WHY,
        "Repair…",
        "Repairing…",
        "Cancel",
    ] {
        assert!(
            DIALOGS_HTML.contains(words),
            "not in macos-settings-dialogs.html: {words}"
        );
    }
    for words in [
        TITLE,
        SUB,
        ADDING,
        TURNED_OFF,
        "Open System Settings",
        "Sync is paused",
        KEPT_TITLE,
    ] {
        assert!(SURFACE_HTML.contains(words), "not in macos-two-surface.html: {words}");
    }
}

#[test]
fn every_state_offers_its_own_buttons() {
    use FinderFailureReason as R;
    use FinderSetup as S;
    let cases = [
        (S::Adding, None, vec![]),
        (S::Ready, None, vec![Press::OpenInFinder, Press::Repair, Press::TurnOff]),
        (S::UserDisabled, Some(R::UserDisabled), vec![Press::OpenSystemSettings]),
        (S::Missing, None, vec![Press::TurnOn]),
        (S::Missing, Some(R::NotInApplications), vec![Press::ShowApp]),
        (S::Failed, Some(R::Timeout), vec![Press::TryAgain]),
        (S::Failed, Some(R::ExtensionLoading), vec![Press::TryAgain]),
        (S::Failed, Some(R::Unknown), vec![Press::TryAgain]),
        (S::Failed, None, vec![Press::TryAgain]),
        (S::Failed, Some(R::FolderTaken), vec![Press::CopyDetails]),
        (S::Failed, Some(R::Signing), vec![Press::CopyDetails]),
        (S::Failed, Some(R::NotInApplications), vec![Press::ShowApp]),
        (S::Failed, Some(R::UserDisabled), vec![Press::OpenSystemSettings]),
    ];
    for (setup, reason, want) in cases {
        assert_eq!(page(setup, reason).buttons(), want, "{setup:?} {reason:?}");
    }
}

#[test]
fn a_failure_says_one_sentence_per_reason() {
    for reason in FinderFailureReason::ALL {
        let words = failure_sentence(Some(reason));
        assert!(words.ends_with('.'), "{reason:?}");
        assert!(!words.contains('\''), "typographic apostrophes only: {words}");
    }
    assert_eq!(
        page(FinderSetup::Failed, Some(FinderFailureReason::Timeout)).says(),
        "macOS didn’t finish adding Beebeeb to Finder."
    );
}

#[test]
fn sync_is_paused_while_the_extension_is_turned_off() {
    assert!(page(FinderSetup::UserDisabled, Some(FinderFailureReason::UserDisabled)).sync_is_paused());
    assert!(!page(FinderSetup::Ready, None).sync_is_paused());
    let mut paused = page(FinderSetup::Ready, None);
    paused.sync_paused = true;
    assert!(paused.sync_is_paused());
}

#[test]
fn presses_are_recorded_and_repair_asks_first() {
    let mut s = init();
    assert!(super::super::scene("DM-18-ready", &mut s));
    press(&mut s, Press::Repair);
    assert_eq!(s.settings.dialog, Some(Dialog::Repair));
    assert!(
        s.settings.finder.pressed.is_empty(),
        "the question comes before the repair"
    );
    press(&mut s, Press::RepairNow);
    assert_eq!(s.settings.dialog, None);
    press(&mut s, Press::TurnOff);
    press(&mut s, Press::OpenSystemSettings);
    assert_eq!(
        s.settings.finder.pressed,
        vec![Press::RepairNow, Press::TurnOff, Press::OpenSystemSettings]
    );
}

#[test]
fn the_live_presses_reach_the_shell_or_the_reconciler() {
    let mut s = init();
    s.settings.finder.page = None;
    press(&mut s, Press::OpenSystemSettings);
    press(&mut s, Press::ShowApp);
    let intents = s.settings.take_intents();
    assert_eq!(
        intents,
        vec![
            Intent::Open("login-items-and-extensions"),
            Intent::Open("finder-show-app")
        ]
    );
}

/// Only macOS wires the Finder pane (`SettingsUi::wired`); elsewhere every scene is the placeholder.
#[cfg(target_os = "macos")]
#[test]
fn each_scene_draws_its_buttons() {
    for (scene, first) in [
        ("DM-18-adding", false),
        ("DM-18-ready", true),
        ("DM-18-off", true),
        ("DM-18-failed", true),
        ("DM-18-missing", true),
    ] {
        let rt = runtime(scene);
        assert_eq!(has(&rt, "btn-1"), first, "{scene}");
        assert!(has(&rt, "item-0-0"), "{scene} draws the Finder row");
        assert!(has(&rt, "item-1-0"), "{scene} draws the Sync row");
    }
    assert!(has(&runtime("DM-18-repair"), "dlg"), "the Repair question is a dialog");
}
