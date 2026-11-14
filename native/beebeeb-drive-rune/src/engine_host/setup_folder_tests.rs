//! The first run's sync folder (contract S2, ruling Q2): no start before a folder is chosen, the engine's rule
//! decides whether a folder may be it, the folder is written once by the start that worked, and never on a
//! refusal. Against the double of the engine handle, and in the window. Every name is invented.

use std::sync::atomic::Ordering;

use super::*;
use crate::frames::global::sync_off::{Refused, SyncOff};
use crate::frames::signin::setup::source::FolderSaid;
use folder::{PICKER_TITLE, start_now};

const PICKED: &str = "/invented/disk/Cloud";

/// A new PC: the settings name no sync folder.
fn new_pc_on(platform: FakePlatform) -> Set {
    let engine = fake();
    *engine.settings_folder.lock().unwrap() = Ok(None);
    set_on(engine, platform)
}

fn new_pc() -> Set {
    new_pc_on(FakePlatform::default())
}

/// The start worker's call, as `LiveEngine::start_sync` makes it.
fn start(set: &Set) -> Option<String> {
    let name = Mutex::new(None);
    start_now(&*set.engine, &name, &set.first_run);
    name.into_inner().unwrap()
}

fn asks(folder: &str, said: Option<FolderSaid>) -> FolderAsk {
    FolderAsk::Ask {
        folder: folder.into(),
        said,
        busy: false,
    }
}

fn started(set: &Set) -> Vec<PathBuf> {
    set.engine.started.lock().unwrap().clone()
}

fn starts_asked(set: &Set) -> usize {
    set.asked.starts.load(Ordering::SeqCst)
}

fn chosen(set: &Set) -> Option<PathBuf> {
    set.first_run.chosen_for(&set.engine.stamp())
}

#[test]
fn the_first_run_asks_for_the_folder_before_the_first_start() {
    // Keeping the suggestion.
    let mut set = new_pc();
    assert_eq!(set.source.folder_ask(), asks(SUGGESTED, None));
    assert_eq!(start(&set), None, "the shell asks for a start as the gate opens");
    assert!(started(&set).is_empty(), "no start before a folder is chosen");
    set.source.keep_folder();
    assert!(
        matches!(set.source.folder_ask(), FolderAsk::Ask { busy: true, .. }),
        "the choice is on its way"
    );
    settle(&mut set, 1);
    assert_eq!(set.source.folder_ask(), FolderAsk::Settled);
    assert_eq!(set.engine.checked.lock().unwrap().clone(), [PathBuf::from(SUGGESTED)]);
    assert_eq!(starts_asked(&set), 1, "the choice asks for one start");
    assert_eq!(
        start(&set).as_deref(),
        Some("Beebeeb"),
        "the folder's name, for the modal"
    );
    assert_eq!(started(&set), [PathBuf::from(SUGGESTED)]);

    // Picking another folder with the OS's picker.
    let mut set = new_pc();
    set.platform
        .picks
        .lock()
        .unwrap()
        .push_back(Ok(Some(PathBuf::from(PICKED))));
    set.source.choose_folder();
    settle(&mut set, 1);
    assert_eq!(
        set.platform.pick_asks.lock().unwrap().clone(),
        [(PICKER_TITLE.to_string(), PathBuf::from(SUGGESTED))],
        "the picker opens where the suggestion is"
    );
    assert_eq!(set.source.folder_ask(), FolderAsk::Settled);
    assert_eq!(set.engine.checked.lock().unwrap().clone(), [PathBuf::from(PICKED)]);
    assert_eq!(starts_asked(&set), 1);
    start(&set);
    assert_eq!(
        started(&set),
        [PathBuf::from(PICKED)],
        "the picked folder, not the suggestion"
    );
}

#[test]
fn a_folder_the_engine_refuses_is_said_and_never_started_in() {
    for (refused, said) in [
        (SyncFolderRefused::Network, FolderSaid::Network),
        (SyncFolderRefused::System, FolderSaid::System),
        (SyncFolderRefused::Home, FolderSaid::Home),
        (SyncFolderRefused::OwnData, FolderSaid::OwnData),
        (SyncFolderRefused::NotAFolder, FolderSaid::NotAFolder),
        (SyncFolderRefused::AnotherApps, FolderSaid::AnotherApps),
        (SyncFolderRefused::Unreadable, FolderSaid::Unreadable),
        (SyncFolderRefused::NotAFullPath, FolderSaid::NotAFullPath),
    ] {
        let mut set = new_pc();
        *set.engine.rule.lock().unwrap() = Err(refused);
        set.platform
            .picks
            .lock()
            .unwrap()
            .push_back(Ok(Some(PathBuf::from(PICKED))));
        set.source.choose_folder();
        settle(&mut set, 1);
        // The question comes back with the reason and the suggestion; the refused folder is not kept anywhere.
        assert_eq!(set.source.folder_ask(), asks(SUGGESTED, Some(said)), "{refused:?}");
        assert_eq!(chosen(&set), None, "{refused:?}");
        assert_eq!(starts_asked(&set), 0, "{refused:?}");
        start(&set);
        assert!(started(&set).is_empty(), "{refused:?}");
        assert!(set.engine.kept.lock().unwrap().is_empty(), "{refused:?}");
        // The suggestion itself is asked of the rule too (a home folder on a share).
        set.source.keep_folder();
        settle(&mut set, 2);
        assert_eq!(set.source.folder_ask(), asks(SUGGESTED, Some(said)), "{refused:?}");
        assert_eq!(starts_asked(&set), 0, "{refused:?}");
        // Another folder that the rule lets through is taken.
        *set.engine.rule.lock().unwrap() = Ok(());
        set.source.keep_folder();
        settle(&mut set, 3);
        assert_eq!(set.source.folder_ask(), FolderAsk::Settled, "{refused:?}");
        assert_eq!(starts_asked(&set), 1, "{refused:?}");
    }
}

#[test]
fn a_picker_that_is_closed_or_does_not_open_changes_nothing() {
    // Closed without a folder: the question stands as it was.
    let mut set = new_pc();
    set.source.choose_folder();
    settle(&mut set, 1);
    assert_eq!(set.source.folder_ask(), asks(SUGGESTED, None));
    // It did not open (the OS refused; a platform without one): said, and the suggestion can still be used.
    for platform in [FakePlatform::default(), FakePlatform::unbuilt()] {
        let mut set = new_pc_on(platform);
        set.platform
            .picks
            .lock()
            .unwrap()
            .push_back(Err(crate::shell::platform::PlatformError::Os(
                "an invented failure".into(),
            )));
        set.source.choose_folder();
        settle(&mut set, 1);
        assert_eq!(set.source.folder_ask(), asks(SUGGESTED, Some(FolderSaid::NoPicker)));
        assert!(set.engine.checked.lock().unwrap().is_empty());
        assert_eq!((starts_asked(&set), chosen(&set)), (0, None));
        set.source.keep_folder();
        settle(&mut set, 2);
        assert_eq!(set.source.folder_ask(), FolderAsk::Settled);
    }
}

#[test]
fn one_choice_at_a_time() {
    let mut set = new_pc();
    let held = set.engine.held.lock().unwrap();
    set.source.keep_folder();
    set.source.keep_folder();
    set.source.choose_folder();
    drop(held);
    settle(&mut set, 1);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(set.engine.checked.lock().unwrap().len(), 1, "one check");
    assert!(set.platform.pick_asks.lock().unwrap().is_empty(), "no picker beside it");
    assert_eq!(starts_asked(&set), 1);
}

#[test]
fn the_folder_is_persisted_once() {
    let mut set = new_pc();
    set.source.keep_folder();
    settle(&mut set, 1);
    start(&set);
    assert_eq!(set.engine.kept.lock().unwrap().clone(), [PathBuf::from(SUGGESTED)]);
    assert_eq!(
        set.engine.settings_folder(),
        Ok(Some(PathBuf::from(SUGGESTED))),
        "after Started, the settings name the folder"
    );
    // A second start, with another folder chosen meanwhile: the settings' folder is used and stays.
    set.first_run.choose(set.engine.stamp(), PathBuf::from(PICKED));
    start(&set);
    assert_eq!(started(&set), [PathBuf::from(SUGGESTED), PathBuf::from(SUGGESTED)]);
    assert_eq!(set.engine.kept.lock().unwrap().len(), 1, "written once");

    // Settings that name a folder from the start: nothing is asked, it is started in, nothing is written.
    let set = set_on(fake(), FakePlatform::default());
    assert_eq!(set.source.folder_ask(), FolderAsk::Settled);
    start(&set);
    assert_eq!(started(&set), [PathBuf::from("an-invented-folder")]);
    assert!(set.engine.kept.lock().unwrap().is_empty());

    // A start that was already running, or had no session, writes nothing either.
    for answer in [SyncStart::AlreadyRunning, SyncStart::NoSession] {
        let mut set = new_pc();
        *set.engine.start_answer.lock().unwrap() = Ok(answer);
        set.source.keep_folder();
        settle(&mut set, 1);
        start(&set);
        assert_eq!(started(&set).len(), 1);
        assert!(set.engine.kept.lock().unwrap().is_empty(), "{answer:?}");
    }
}

#[test]
fn the_folder_is_not_persisted_on_a_refusal() {
    for refused in [
        SyncStartRefused::FolderOfAnotherProvider,
        // Another account's data on this PC: the start's own check (the folder rule cannot know it).
        SyncStartRefused::OtherAccountsData,
        SyncStartRefused::FolderUnreadable,
        SyncStartRefused::RegistrationFailed { code: Some(5) },
        SyncStartRefused::Other,
    ] {
        let mut set = new_pc();
        *set.engine.start_answer.lock().unwrap() = Err(refused);
        set.source.keep_folder();
        settle(&mut set, 1);
        start(&set);
        assert_eq!(started(&set), [PathBuf::from(SUGGESTED)], "{refused:?}");
        assert!(set.engine.kept.lock().unwrap().is_empty(), "{refused:?}");
        assert_eq!(set.engine.settings_folder(), Ok(None), "{refused:?}");

        // The step shows the refused start, and another folder can be chosen while none was kept.
        assert!(!set.source.can_choose_again(), "the view does not say refused yet");
        set.engine.set(Start::Refused(refused), EngineExplorer::Unknown);
        assert!(set.source.can_choose_again(), "{refused:?}");
        set.platform
            .picks
            .lock()
            .unwrap()
            .push_back(Ok(Some(PathBuf::from(PICKED))));
        set.source.choose_folder();
        settle(&mut set, 2);
        assert_eq!(chosen(&set), Some(PathBuf::from(PICKED)), "{refused:?}");
        assert_eq!(starts_asked(&set), 2, "the new choice asks for one start");
        *set.engine.start_answer.lock().unwrap() = Ok(SyncStart::Started);
        start(&set);
        assert_eq!(
            set.engine.kept.lock().unwrap().clone(),
            [PathBuf::from(PICKED)],
            "{refused:?}"
        );
        // Sync ran: the folder is settled, whatever a later start says.
        set.engine.set(Start::Running, EngineExplorer::Added);
        set.source.poll();
        set.engine.set(Start::Refused(refused), EngineExplorer::Unknown);
        assert!(!set.source.can_choose_again(), "{refused:?}");
    }
    // A PC whose settings named the folder never offers another one from the first run.
    let set = set_on(fake(), FakePlatform::default());
    set.engine.set(
        Start::Refused(SyncStartRefused::FolderOfAnotherProvider),
        EngineExplorer::Unknown,
    );
    assert!(!set.source.can_choose_again());
}

#[test]
fn a_folder_chosen_under_another_session_or_account_is_never_started_in() {
    for moved_to in [
        (session(2), Some(FIRST.to_string())),
        (session(1), Some(SECOND.to_string())),
    ] {
        let mut set = new_pc();
        set.source.keep_folder();
        settle(&mut set, 1);
        assert!(chosen(&set).is_some());
        *set.engine.stamp.lock().unwrap() = moved_to.clone();
        assert_eq!(chosen(&set), None, "{moved_to:?}");
        start(&set);
        assert!(
            started(&set).is_empty(),
            "the first run is still due and this session chose nothing: {moved_to:?}"
        );

        // And a press in a first run that was made for the session before sends nothing and says nothing.
        let mut set = new_pc();
        *set.engine.stamp.lock().unwrap() = moved_to.clone();
        set.source.keep_folder();
        set.source.choose_folder();
        std::thread::sleep(Duration::from_millis(50));
        assert!(set.engine.checked.lock().unwrap().is_empty(), "{moved_to:?}");
        assert!(set.platform.pick_asks.lock().unwrap().is_empty(), "{moved_to:?}");
        assert_eq!(set.engine.wakes.load(Ordering::SeqCst), 0, "no worker: {moved_to:?}");
        assert_eq!(starts_asked(&set), 0, "{moved_to:?}");
    }
}

#[test]
fn without_a_first_run_the_start_is_as_before() {
    // No first run due (it ended, or this OS has none) and settings that name no folder: the suggestion, kept.
    let engine = fake();
    *engine.settings_folder.lock().unwrap() = Ok(None);
    *engine.due.lock().unwrap() = Ok(false);
    let set = set_on(engine, FakePlatform::default());
    start(&set);
    assert_eq!(started(&set), [PathBuf::from(SUGGESTED)]);
    assert_eq!(set.engine.kept.lock().unwrap().clone(), [PathBuf::from(SUGGESTED)]);

    // Settings that do not read: nothing is started, and nothing is asked in the step.
    let engine = fake();
    *engine.settings_folder.lock().unwrap() = Err(SetupRefused::Settings);
    let set = set_on(engine, FakePlatform::default());
    assert_eq!(set.source.folder_ask(), FolderAsk::Settled);
    assert_eq!(start(&set), None);
    assert!(started(&set).is_empty());

    // A folder that could not be written after a start that worked: sync runs, and the next start writes it.
    let mut set = new_pc();
    *set.engine.writes.lock().unwrap() = Err(SetupRefused::Settings);
    set.source.keep_folder();
    settle(&mut set, 1);
    start(&set);
    assert_eq!(started(&set).len(), 1);
    assert!(set.engine.kept.lock().unwrap().is_empty());
    *set.engine.writes.lock().unwrap() = Ok(());
    start(&set);
    assert_eq!(set.engine.kept.lock().unwrap().clone(), [PathBuf::from(SUGGESTED)]);
}

// ---------------------------------------------------------------------------
// The window
// ---------------------------------------------------------------------------

fn starts_of(fixture: &FixtureEngine) -> usize {
    fixture
        .calls()
        .iter()
        .filter(|call| **call == crate::shell::actions::FixtureCall::StartSync)
        .count()
}

/// The File Explorer step asks for the folder first: the suggestion is shown, nothing reads as "adding", a folder
/// the engine refuses is said with its reason, and the choice that is taken leads to the step's own states.
#[test]
fn the_window_asks_for_the_folder_and_says_why_one_was_not_taken() {
    let set = new_pc();
    *set.engine.rule.lock().unwrap() = Err(SyncFolderRefused::Network);
    let (mut rt, engine, fixture, first_run, platform) = window(set);
    fixture.set_gate_for_scene(ShellGate::FirstRun);
    wait_for(&mut rt, "the first run", |rt| rt.state.setup.is_some());
    wait_for(&mut rt, "the folder was read", |rt| {
        says(rt, "isn't in File Explorer yet")
    });
    assert!(!says(&rt, "Adding"), "nothing is being added while no folder is chosen");
    to_file_explorer(&mut rt);
    assert!(says(&rt, "Choose where Beebeeb's folder goes"));
    assert!(says(&rt, SUGGESTED), "the suggested folder is shown");
    assert!(!says(&rt, "Not now"), "the folder is not skipped from its own question");

    // The picked folder is on the network: said, nothing chosen, the question still up.
    platform
        .picks
        .lock()
        .unwrap()
        .push_back(Ok(Some(PathBuf::from(PICKED))));
    press(&mut rt, "Choose another folder\u{2026}");
    wait_for(&mut rt, "the reason", |rt| says(rt, "That folder is on the network"));
    assert!(says(&rt, "Choose where Beebeeb's folder goes") && says(&rt, SUGGESTED));
    assert!(!says(&rt, PICKED), "the refused folder is not shown as the choice");
    assert_eq!(first_run.chosen_for(&engine.stamp()), None);

    // The suggestion, once the rule lets it through: the step goes on to File Explorer's own states.
    *engine.rule.lock().unwrap() = Ok(());
    press(&mut rt, "Use this folder");
    wait_for(&mut rt, "the choice was taken", |rt| {
        says(rt, "Adding Beebeeb to File Explorer")
    });
    assert_eq!(first_run.chosen_for(&engine.stamp()), Some(PathBuf::from(SUGGESTED)));
    engine.set(Start::Running, EngineExplorer::Added);
    wait_for(&mut rt, "added", |rt| says(rt, "is in File Explorer"));
}

/// "Skip setup" before a folder was chosen: the last page says File Explorer is off.
#[test]
fn skipping_before_a_folder_was_chosen_claims_no_file_explorer() {
    let set = new_pc();
    let (mut rt, _engine, fixture, ..) = window(set);
    fixture.set_gate_for_scene(ShellGate::FirstRun);
    wait_for(&mut rt, "the first run", |rt| rt.state.setup.is_some());
    wait_for(&mut rt, "the folder was read", |rt| {
        says(rt, "isn't in File Explorer yet")
    });
    rt.update(|s, _| {
        if let Some(ui) = s.setup.as_mut() {
            ui.skip();
        }
    });
    wait_for(&mut rt, "the last page", |rt| says(rt, "You're set up"));
    assert!(says(&rt, "File Explorer, Off"));
}

/// Contract S2 (ruling Q3): a start that is refused in the first run opens the same refusal modal, once, over the
/// step; the step says sync did not start and offers another folder while none was kept; "Try again" asks for
/// one start.
#[test]
fn a_refused_start_shows_the_modal_in_the_first_run_and_another_folder_can_be_chosen() {
    let set = new_pc();
    let (mut rt, engine, fixture, first_run, platform) = window(set);
    fixture.set_gate_for_scene(ShellGate::FirstRun);
    wait_for(&mut rt, "the first run", |rt| rt.state.setup.is_some());
    wait_for(&mut rt, "the folder was read", |rt| {
        says(rt, "isn't in File Explorer yet")
    });
    to_file_explorer(&mut rt);
    press(&mut rt, "Use this folder");
    wait_for(&mut rt, "the choice was taken", |rt| {
        says(rt, "Adding Beebeeb to File Explorer")
    });

    // The start is refused: the folder is another app's sync folder.
    engine.set(
        Start::Refused(SyncStartRefused::FolderOfAnotherProvider),
        EngineExplorer::Unknown,
    );
    fixture.set_sync_refusal(Some(SyncOff {
        attempt: 1,
        kind: Refused::FolderOfAnotherProvider,
        folder: Some("Beebeeb".into()),
    }));
    wait_for(&mut rt, "the modal", |rt| rt.state.shell.sync_off().is_some());
    assert_eq!(rt.state.shell.sync_off_opened(), 1);
    assert_eq!(rt.state.shell.app.gate(), ShellGate::FirstRun, "in the first run");
    assert!(
        rt.elements()
            .iter()
            .any(|e| !e.exiting && e.key.as_deref() == Some("dialog-layer")),
        "the modal is drawn over the first-run window"
    );
    for _ in 0..5 {
        rt.frame(0.1);
    }
    assert_eq!(rt.state.shell.sync_off_opened(), 1, "once per refused attempt");

    // "Try again" in the modal (the step behind it has one too: the modal's is the one in front): it closes,
    // and one start is asked for.
    let before = starts_of(&fixture);
    let rect = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting && e.role == "button" && e.label.as_deref() == Some("Try again"))
        .next_back()
        .expect("the modal's Try again")
        .rect;
    let (x, y) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.016);
    rt.frame(0.1);
    assert_eq!(starts_of(&fixture), before + 1);
    assert!(rt.state.shell.app.dialog().is_none());

    // The step behind it says sync did not start, and offers another folder: none was kept.
    wait_for(&mut rt, "sync did not start", |rt| {
        says(rt, "Sync didn't start on this PC")
    });
    platform
        .picks
        .lock()
        .unwrap()
        .push_back(Ok(Some(PathBuf::from(PICKED))));
    press(&mut rt, "Choose another folder\u{2026}");
    wait_for(&mut rt, "the new choice", |_| {
        first_run.chosen_for(&engine.stamp()) == Some(PathBuf::from(PICKED))
    });
    assert!(
        engine.kept.lock().unwrap().is_empty(),
        "a refused folder was never written"
    );
}
