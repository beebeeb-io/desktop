//! The two states of the first run the board has no frame for: the sync folder's question (the first state of the
//! File Explorer step) and a step nothing answers for in this version. On the fixture, through the same calls a
//! person's presses make.

use super::super::source::{FolderAsk, FolderSaid};
use super::super::{RENDER_SCENES, render_scene};
use super::*;

const SUGGESTED: &str = r"C:\Users\Sam\Beebeeb";

fn asks(said: Option<FolderSaid>) -> FolderAsk {
    FolderAsk::Ask {
        folder: SUGGESTED.into(),
        said,
        busy: false,
    }
}

fn says(rt: &Runtime<State>, words: &str) -> bool {
    rt.elements().iter().any(|e| {
        !e.exiting
            && (e.text.as_deref().is_some_and(|t| t.contains(words))
                || e.label.as_deref().is_some_and(|l| l.contains(words)))
    })
}

fn heading(rt: &Runtime<State>) -> String {
    info(rt, "title").text.unwrap_or_default()
}

fn has_button(rt: &Runtime<State>, label: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| !e.exiting && e.role == "button" && e.label.as_deref() == Some(label))
}

/// A step nothing answers for keeps its place and its name, says that it is not in this version, asks nothing
/// of the engine and claims nothing on the last page.
#[test]
fn a_step_nothing_answers_for_keeps_its_place_says_so_and_claims_nothing() {
    let (ui_, log) = fixture(Script {
        not_available: &[
            Step::Passkey,
            Step::Lock,
            Step::Backup,
            Step::Previews,
            Step::Notifications,
        ],
        explorer: Explorer::Added,
        ..Script::default()
    });
    let mut rt = runtime_at(ui_, 2.0, false);
    for step in [Step::Passkey, Step::Lock, Step::Backup, Step::Previews] {
        assert_eq!(ui_step(&mut rt), step);
        assert_eq!(heading(&rt), step.label());
        assert!(says(&rt, "This isn't in this version of Beebeeb yet"), "{step:?}");
        // The step's own controls are not there: only "Continue".
        for control in ["Not now", "Add a passkey", "Turn on backup", "Compare"] {
            assert!(!has_button(&rt, control), "{step:?}: {control}");
        }
        // Every step is in the list at the left.
        for listed in Step::STEPS {
            assert!(says(&rt, listed.label()), "{step:?} lists {}", listed.label());
        }
        click(&mut rt, "btn-0");
    }
    assert_eq!(ui_step(&mut rt), Step::Explorer);
    click(&mut rt, "btn-0"); // File Explorer is answered for here: Continue
    assert_eq!(ui_step(&mut rt), Step::Notifications);
    assert_eq!(heading(&rt), Step::Notifications.label());
    click(&mut rt, "btn-0");
    assert_eq!(ui_step(&mut rt), Step::Login);
    click(&mut rt, "btn-0"); // open at login is answered for
    assert_eq!(ui_step(&mut rt), Step::Done);
    assert_eq!(
        log.borrow().clone(),
        ["set_open_at_login true"],
        "nothing was asked for a step nothing answers for"
    );
    let results = ui(&mut rt).results.clone();
    assert!(!results.passkey && !results.app_lock && results.backup.is_empty());
    for row in ["Passkey", "App lock", "Backup", "Previews", "Notifications"] {
        assert!(says(&rt, &format!("{row}, Not available yet")), "{row}");
    }
    assert!(says(&rt, "File Explorer, On") && says(&rt, "Open at login, On"));
    assert!(!says(&rt, "4K HEIC"), "no value for a setting nothing set");

    // "Not now" and the passkey button's own call on such a step ask nothing either.
    let (mut ui_, log) = fixture(Script {
        not_available: &[Step::Passkey, Step::Notifications],
        ..Script::default()
    });
    ui_.add_passkey();
    assert_eq!(ui_.step, Step::Lock);
    assert!(!ui_.results.passkey);
    ui_.step = Step::Notifications;
    ui_.not_now();
    assert_eq!(ui_.step, Step::Login);
    assert!(log.borrow().is_empty());
}

/// The File Explorer step asks for the sync folder first: the amber button keeps the suggestion, the other one
/// opens the picker, the question is not skipped from the step, and nothing of File Explorer is claimed.
#[test]
fn the_file_explorer_step_asks_for_the_folder_first() {
    let (mut ui_, log) = fixture(Script {
        folder: asks(None),
        ..Script::default()
    });
    while ui_.step < Step::Explorer {
        if ui_.step == Step::Passkey {
            ui_.add_passkey();
        } else {
            ui_.go_on();
        }
    }
    log.borrow_mut().clear();
    let mut rt = runtime_at(ui_, 2.0, false);
    assert!(ui(&mut rt).asks_folder());
    assert_eq!(heading(&rt), "Choose where Beebeeb's folder goes");
    assert!(says(&rt, SUGGESTED), "the suggested folder");
    assert!(says(&rt, "isn't in File Explorer yet"), "the status at the left");
    assert!(!says(&rt, "Adding"), "nothing is being added");
    assert!(has_button(&rt, "Use this folder") && has_button(&rt, "Choose another folder\u{2026}"));
    assert!(!has_button(&rt, "Continue") && !has_button(&rt, "Not now"));

    // Not skipped from the step, and nothing to show in File Explorer.
    ui(&mut rt).not_now();
    ui(&mut rt).show_in_explorer();
    assert_eq!(ui_step(&mut rt), Step::Explorer);
    assert!(log.borrow().is_empty());

    click(&mut rt, "btn-1");
    assert_eq!(log.borrow().clone(), ["choose_folder"]);
    assert_eq!(ui_step(&mut rt), Step::Explorer);
    click(&mut rt, "btn-0");
    assert_eq!(log.borrow().clone(), ["choose_folder", "keep_folder"]);
    // The folder is settled: the step shows File Explorer's own state, and its button goes on.
    assert_eq!(ui_step(&mut rt), Step::Explorer);
    assert!(!ui(&mut rt).asks_folder());
    assert_eq!(heading(&rt), "Adding Beebeeb to File Explorer");
    ui(&mut rt).choose_folder();
    assert_eq!(log.borrow().len(), 2, "no picker once the folder is settled");
    click(&mut rt, "btn-0");
    assert_eq!(ui_step(&mut rt), Step::Notifications);
}

/// A folder that was not taken: the reason is said in the step, as an alert, and the question stays.
#[test]
fn a_folder_that_was_not_taken_is_said_with_its_reason() {
    let every = [
        FolderSaid::Network,
        FolderSaid::System,
        FolderSaid::Home,
        FolderSaid::OwnData,
        FolderSaid::NotAFolder,
        FolderSaid::AnotherApps,
        FolderSaid::Unreadable,
        FolderSaid::NotAFullPath,
        FolderSaid::NoPicker,
    ];
    let mut titles = Vec::new();
    for said in every {
        let (title, text) = super::super::folder::said(said);
        assert!(!title.is_empty() && !text.is_empty(), "{said:?}");
        assert!(
            !title.contains(['\\', '/']) && !text.contains(['\\', '/']),
            "names no path: {said:?}"
        );
        titles.push(title);
        let (mut ui_, _log) = fixture(Script {
            folder: asks(Some(said)),
            ..Script::default()
        });
        ui_.step = Step::Explorer;
        let rt = runtime_at(ui_, 2.0, false);
        assert!(says(&rt, title) && says(&rt, text), "{said:?}");
        assert!(
            rt.elements().iter().any(|e| !e.exiting && e.role == "alert"),
            "{said:?}: an alert"
        );
        assert!(has_button(&rt, "Use this folder"), "{said:?}: the question stays");
    }
    titles.sort_unstable();
    titles.dedup();
    assert_eq!(titles.len(), every.len(), "each reason has its own words");
}

/// While a choice is on its way (the check, or Windows' picker), neither button takes a press.
#[test]
fn while_a_choice_is_on_its_way_no_second_one_is_asked() {
    let (mut ui_, log) = fixture(Script {
        folder: FolderAsk::Ask {
            folder: SUGGESTED.into(),
            said: None,
            busy: true,
        },
        ..Script::default()
    });
    ui_.step = Step::Explorer;
    let mut rt = runtime_at(ui_, 2.0, false);
    click(&mut rt, "btn-0");
    click(&mut rt, "btn-1");
    assert!(log.borrow().is_empty());
}

/// After a start that was refused, another folder can be chosen only where the source says so.
#[test]
fn another_folder_can_be_chosen_after_a_refused_start_only_when_none_was_kept() {
    for (explorer, words) in [
        (Explorer::NotSyncing, "Sync didn't start on this PC"),
        (
            Explorer::Failed {
                code: "0x80070005".into(),
            },
            "Windows didn't finish",
        ),
    ] {
        let (mut ui_, log) = fixture(Script {
            explorer: explorer.clone(),
            choose_again: true,
            ..Script::default()
        });
        ui_.step = Step::Explorer;
        let mut rt = runtime_at(ui_, 2.0, false);
        assert!(says(&rt, words));
        // Only a File Explorer entry that failed leaves a window that works: sync that did not start does not.
        assert_eq!(
            says(&rt, "the Beebeeb window works without File Explorer"),
            explorer != Explorer::NotSyncing
        );
        assert_eq!(
            says(&rt, "Nothing syncs with this PC"),
            explorer == Explorer::NotSyncing
        );
        assert!(has_button(&rt, "Try again") && has_button(&rt, "Continue without File Explorer"));
        click(&mut rt, "btn-1");
        assert_eq!(log.borrow().clone(), ["choose_folder"], "{explorer:?}");

        let (mut ui_, log) = fixture(Script {
            explorer,
            ..Script::default()
        });
        ui_.step = Step::Explorer;
        let mut rt = runtime_at(ui_, 2.0, false);
        assert!(!has_button(&rt, "Choose another folder\u{2026}"));
        ui(&mut rt).choose_folder();
        assert!(log.borrow().is_empty());
    }
}

/// The named renders of these states (`--scene first-run-folder`) draw what their names say, in both themes, inside
/// the window.
#[test]
fn the_render_scenes_draw_their_states() {
    let expected = [
        ("first-run-not-yet", "Add a passkey"),
        ("first-run-folder", "Choose where Beebeeb's folder goes"),
        ("first-run-folder-refused", "Choose where Beebeeb's folder goes"),
        ("first-run-sync-off", "Couldn't add Beebeeb to File Explorer"),
        ("first-run-summary", "You're set up"),
    ];
    assert_eq!(RENDER_SCENES.len(), expected.len());
    for (name, title) in expected {
        assert!(RENDER_SCENES.iter().any(|(listed, _)| *listed == name), "{name}");
        for dark in [false, true] {
            let rt = runtime_at(
                render_scene(name).unwrap_or_else(|| panic!("no scene {name}")),
                1.0,
                dark,
            );
            assert_eq!(heading(&rt), title, "{name}");
            for e in rt.elements().iter().filter(|e| !e.exiting && e.role == "button") {
                let r = e.rect;
                assert!(
                    r[0] >= 0.0 && r[1] >= 0.0 && r[0] + r[2] <= WINDOW.0 && r[1] + r[3] <= WINDOW.1,
                    "{name}: a button is outside the window"
                );
            }
        }
    }
    assert!(render_scene("first-run").is_none() && render_scene("DW-0206").is_none());
}
