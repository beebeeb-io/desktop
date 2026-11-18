//! The open file's presses reach Files and the shell, for the open file only (audit V1).

use std::sync::Arc;

use rune::{Env, Runtime};

use super::super::actions::FixtureEngine;
use super::super::host::{Shell, poll};
use super::super::platform::fake::FakePlatform;
use super::super::{registry, state};
use super::*;
use crate::engine_host::signin::OPENED;
use crate::frames::files::dialogs::Dialog;
use crate::frames::viewers::source::{FixtureViewerSource, board};

const ENV: Env = Env {
    system_dark: false,
    scale: 2.0,
};

fn runtime() -> Runtime<State> {
    let state = state::init_with(Shell::fixture_on(
        Arc::new(FixtureEngine::ready()),
        Arc::new(FakePlatform::default()),
    ));
    let mut rt = Runtime::new(state, registry::view, (1280.0, 800.0), ENV);
    rt.poll = Some(poll);
    super::super::intents::poll_drains_for_this_test();
    rt.frame(0.0);
    rt
}

/// Opens the board's audio file and presses `intent` in its viewer; the poll takes it.
fn press(rt: &mut Runtime<State>, intent: Intent) {
    rt.update(|s, _| {
        if viewers::ui(s).is_none() {
            assert!(viewers::open_from(s, Box::new(FixtureViewerSource::new()), board::AUDIO));
        }
        viewers::ui(s).expect("open").intent(intent);
    });
    rt.frame(1.0);
}

fn toast(rt: &mut Runtime<State>) -> Option<String> {
    let mut out = None;
    rt.update(|s, _| out = viewers::ui(s).and_then(|ui| ui.toast.clone()));
    out
}

#[test]
fn share_and_version_history_open_files_dialogs_for_the_open_file() {
    let mut rt = runtime();
    press(&mut rt, Intent::Share);
    rt.update(|s, _| {
        assert!(
            matches!(&s.actions.dialog, Some(Dialog::Share(sh)) if sh.items.len() == 1 && sh.items[0].id == board::AUDIO),
            "Share is for the open file alone"
        );
        s.actions.dialog = None;
    });
    press(&mut rt, Intent::VersionHistory);
    rt.update(|s, _| {
        assert!(matches!(&s.actions.dialog, Some(Dialog::Versions(item)) if item.id == board::AUDIO));
    });
    assert_ne!(toast(&mut rt).as_deref(), Some("Not built yet: Share"));
}

#[test]
fn trash_moves_the_open_file_and_closes_the_viewer_with_files_undo_toast() {
    let mut rt = runtime();
    press(&mut rt, Intent::Trash);
    rt.update(|s, _| {
        assert!(viewers::ui(s).is_none(), "the viewer closed back to Files");
        assert_eq!(s.actions.toast, Some(Toast::Trashed));
        assert_eq!(s.actions.undo.len(), 1);
        assert_eq!(s.actions.undo[0].1.id, board::AUDIO);
    });
}

#[test]
fn edit_says_why_instead_of_not_built_yet() {
    let mut rt = runtime();
    press(&mut rt, Intent::Edit);
    let said = toast(&mut rt).expect("a toast");
    assert!(!said.starts_with("Not built yet"), "{said}");
}

#[test]
fn keep_offline_and_free_up_space_say_what_happened_for_this_file() {
    let mut rt = runtime();
    press(&mut rt, Intent::KeepOffline);
    let said = toast(&mut rt).expect("a toast");
    assert!(!said.starts_with("Not built yet"), "{said}");
    press(&mut rt, Intent::FreeUpSpace);
    let said = toast(&mut rt).expect("a toast");
    assert!(!said.starts_with("Not built yet"), "{said}");
}

#[test]
fn report_a_problem_opens_settings_general_with_the_report_dialog() {
    let mut rt = runtime();
    press(&mut rt, Intent::ReportProblem);
    rt.update(|s, _| {
        assert!(matches!(
            s.shell.app.screen(),
            crate::shell::app::Screen::Settings(crate::shell::app::SettingsPane::General)
        ));
    });
}

#[test]
fn only_web_links_open_in_the_browser() {
    OPENED.with(|o| o.borrow_mut().clear());
    let mut rt = runtime();
    press(&mut rt, Intent::OpenInBrowser("file:///C:/Windows/system32/calc.exe".into()));
    press(&mut rt, Intent::OpenInBrowser("javascript:alert(1)".into()));
    assert!(OPENED.with(|o| o.borrow().is_empty()), "nothing but http(s) reaches the opener");
    assert_eq!(toast(&mut rt).as_deref(), Some("Only web links open in the browser"));
    press(&mut rt, Intent::OpenInBrowser("https://beebeeb.io/security".into()));
    assert_eq!(OPENED.with(|o| o.borrow().clone()), vec!["https://beebeeb.io/security"]);
}

#[test]
fn links_that_are_not_in_drive_are_refused() {
    let here = vec!["Files".to_string(), "Trips".to_string()];
    for bad in ["../secret.md", "C:\\Users\\a\\b.md", "C:/x.md", "https://x/y.md", "", "other/place/x.md"] {
        assert_eq!(resolve_in_folder(bad, &here), None, "{bad}");
    }
    assert_eq!(resolve_in_folder("notes.md", &here).as_deref(), Some("notes.md"));
    assert_eq!(resolve_in_folder("./notes.md", &here).as_deref(), Some("notes.md"));
    assert_eq!(resolve_in_folder("Files/Trips/notes.md", &here).as_deref(), Some("notes.md"));
    assert_eq!(resolve_in_folder("/Trips/notes.md", &here).as_deref(), Some("notes.md"));
}

#[test]
fn office_commands_keep_the_routers_toast() {
    let mut rt = runtime();
    press(&mut rt, Intent::EditInSheets);
    assert_eq!(toast(&mut rt).as_deref(), Some("Not built yet: Edit in Sheets"));
}

/// A command of the office editor that its source did not take: with an editor running on the open document it is a
/// refusal, said as one; with none (the board's sample document) the router's toast stays.
#[test]
fn a_command_the_running_editor_did_not_take_says_so() {
    use crate::frames::office::edit::EditUi;
    use beebeeb_office_protocol::office::EditKind;
    let mut rt = runtime();
    press(&mut rt, Intent::Office("Bold"));
    assert_eq!(toast(&mut rt).as_deref(), Some("Not built yet: Bold"));
    rt.update(|s, _| viewers::ui(s).expect("open").office.edit = Some(EditUi::new(EditKind::Docx)));
    press(&mut rt, Intent::Office("Bold"));
    assert_eq!(toast(&mut rt).as_deref(), Some(NOT_TAKEN));
    rt.update(|s, _| viewers::ui(s).expect("open").office.edit.as_mut().expect("edit").stopped = true);
    press(&mut rt, Intent::Office("Bold"));
    assert_eq!(
        toast(&mut rt).as_deref(),
        Some(crate::frames::office::edit::recovery::read_only_words())
    );
}

/// A file picked in the editor's command search comes as its name from "Files": changes no version holds are asked
/// about first, as Back asks, and the document stays; a name is taken as it is, whatever characters it has.
#[test]
fn a_file_named_by_the_command_search_asks_about_unsaved_changes_first_and_opens_by_its_name() {
    use crate::frames::office::edit::EditUi;
    use crate::frames::viewers::office_over::Over;
    use beebeeb_office_protocol::office::EditKind;
    let mut rt = runtime();
    let name = "Invented: notes of 14.md";
    rt.update(|s, _| {
        let row = s.listing.items.iter_mut().find(|row| !row.is_folder).expect("a file in the folder shown");
        row.name = name.to_string();
    });
    // Unsaved changes: the question, and the document stays.
    rt.update(|s, _| {
        if viewers::ui(s).is_none() {
            assert!(viewers::open_from(s, Box::new(FixtureViewerSource::new()), board::AUDIO));
        }
        let mut edit = EditUi::new(EditKind::Docx);
        edit.revision = 2;
        viewers::ui(s).expect("open").office.edit = Some(edit);
        viewers::ui(s).expect("open").intent(Intent::OpenInDrive(name.to_string()));
    });
    rt.frame(1.0);
    rt.update(|s, _| {
        let ui = viewers::ui(s).expect("the document stays open");
        assert_eq!(ui.file.id, board::AUDIO);
        assert!(matches!(ui.office.over, Some(Over::Leave)), "the question about unsaved changes is up");
        // Answered (nothing unsaved any more): the same press goes on.
        ui.office.over = None;
        ui.office.edit = None;
        ui.toast = None;
        ui.intent(Intent::OpenInDrive(name.to_string()));
    });
    rt.frame(1.0);
    rt.update(|s, _| {
        let said = viewers::ui(s).and_then(|ui| ui.toast.clone());
        assert_ne!(said.as_deref(), Some("That link isn't in your Beebeeb Drive"), "a name is not a path");
        assert_ne!(said.as_deref(), Some("Couldn't find that in this folder"));
    });
}
