//! Tests of the New file dialog (DW-1520 to DW-1522): it opens from the Files window, picks a type,
//! names the file, and asks the shell for what it cannot do itself. Nothing in it writes a file.

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::Intent;
use super::new_file::{Kind, Step, TEXT_EXTENSIONS, UNTITLED};
use crate::frames::files::source::FilesFixture;
use crate::frames::files::{Actions, Intent as FilesIntent};
use crate::{State, init, view};

/// Ctrl, as Windows reports it (the shortcut modifier and the Control key).
fn ctrl() -> Mods {
    Mods {
        cmd: true,
        ctrl: true,
        ..Mods::default()
    }
}

/// The board's Files window, its file actions recorded, as the app runs it.
fn runtime() -> (
    Runtime<State>,
    std::sync::Arc<std::sync::Mutex<Vec<crate::frames::files::source::Call>>>,
) {
    let mut state = init();
    let fixture = FilesFixture::default();
    let calls = fixture.calls.clone();
    state.actions = Actions::new(Box::new(fixture));
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    (rt, calls)
}

/// A click on the board's selected row (Contract draft.pdf): the keyboard is in the Files list.
fn click_the_list(rt: &mut Runtime<State>) {
    // A second apart, so two clicks are never one double click (that opens the file).
    rt.frame(1.0);
    let [x, y, w, h] = info(rt, "row-6-date").rect;
    rt.pointer_down(x + w / 2.0, y + h / 2.0);
    rt.pointer_up(x + w / 2.0, y + h / 2.0);
    rt.frame(0.0);
}

fn press_with(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(0.0);
}

fn press(rt: &mut Runtime<State>, key: Key) {
    press_with(rt, key, Mods::default());
}

fn type_text(rt: &mut Runtime<State>, text: &str) {
    press(rt, Key::Text(text.into()));
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
}

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn text_of(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

/// Tab until `key` has focus (at most 30 stops), then press it with Space.
fn operate(rt: &mut Runtime<State>, key: &str) {
    for _ in 0..30 {
        if focused(rt).as_deref() == Some(key) {
            press(rt, Key::Text(" ".into()));
            // Where the press sends the keyboard is applied on the poll of the next frame.
            rt.frame(0.0);
            return;
        }
        press(rt, Key::Tab);
    }
    panic!("Tab never reached {key}; focus is on {:?}", focused(rt));
}

fn dialog_step(rt: &Runtime<State>) -> Option<Step> {
    rt.state.viewer.new_file.as_ref().map(|nf| nf.step)
}

fn picked(rt: &Runtime<State>) -> Option<Kind> {
    rt.state.viewer.new_file.as_ref().map(|nf| nf.kind)
}

/// Ctrl+N in the Files list: the dialog, on the type.
fn open_dialog(rt: &mut Runtime<State>) {
    click_the_list(rt);
    press_with(rt, Key::Command('n'), ctrl());
    // The key queues Files' intent; the dialog opens on the poll of the frame after it.
    rt.frame(0.0);
    assert_eq!(dialog_step(rt), Some(Step::Pick), "Ctrl+N opens New file on the type");
}

#[test]
fn new_file_from_the_menu_and_ctrl_n_opens_the_dialog_on_markdown_and_cancel_makes_nothing() {
    let (mut rt, calls) = runtime();
    // New's menu item and Ctrl+N both name this group's frame; the dialog opens on the next poll.
    rt.state.actions.intents.push(FilesIntent::Frame("DW-1521"));
    rt.frame(0.0);
    assert_eq!(dialog_step(&rt), Some(Step::Pick), "New file… opens the dialog");
    assert_eq!(
        picked(&rt),
        Some(Kind::Markdown),
        "Markdown is chosen first, as the board draws it"
    );
    assert_eq!(text_of(&rt, "dlg-title"), "New file");
    assert_eq!(
        focused(&rt).as_deref(),
        Some("type-0"),
        "the keyboard starts on the chosen type"
    );
    operate(&mut rt, "btn-1");
    assert_eq!(dialog_step(&rt), None, "Cancel closes the dialog");
    assert!(super::take_asked(&mut rt.state).is_empty(), "Cancel asks for nothing");
    assert!(
        calls.lock().unwrap().is_empty(),
        "nothing was written: {:?}",
        calls.lock().unwrap()
    );

    open_dialog(&mut rt);
    press(&mut rt, Key::Escape);
    assert_eq!(dialog_step(&rt), None, "Esc closes it too");
}

#[test]
fn the_types_are_one_radio_group_and_the_arrows_choose_one() {
    let (mut rt, calls) = runtime();
    open_dialog(&mut rt);
    let group = info(&rt, "types");
    assert_eq!(group.role, "radiogroup");
    assert_eq!(group.label.as_deref(), Some("Pick a type"));
    for (i, kind) in Kind::ALL.into_iter().enumerate() {
        let tile = info(&rt, &format!("type-{i}"));
        assert_eq!(tile.role, "radio", "{kind:?}");
        assert_eq!(tile.label, Some(format!("{}, {}", kind.word(), kind.ext())), "{kind:?}");
        assert_eq!(text_of(&rt, &format!("type-{i}-ext")), kind.ext());
        assert_eq!(text_of(&rt, &format!("type-{i}-label")), kind.word());
    }
    press(&mut rt, Key::Right);
    assert_eq!(picked(&rt), Some(Kind::Text), "Right chooses the next type");
    assert_eq!(focused(&rt).as_deref(), Some("type-1"), "and the keyboard goes with it");
    press(&mut rt, Key::Left);
    press(&mut rt, Key::Left);
    assert_eq!(
        picked(&rt),
        Some(Kind::PowerPoint),
        "Left from the first wraps to the last"
    );
    // A key the dialog does not use stops at it. The Files window answers Ctrl+F wherever focus is
    // (the search field), and Delete moves the selected file to Trash: neither reaches it.
    press_with(&mut rt, Key::Command('f'), ctrl());
    rt.frame(0.0);
    assert_eq!(
        focused(&rt).as_deref(),
        Some("type-4"),
        "Ctrl+F reached the window behind the dialog"
    );
    press(&mut rt, Key::Delete);
    assert_eq!(dialog_step(&rt), Some(Step::Pick));
    assert!(
        calls.lock().unwrap().is_empty(),
        "Delete reached the list: {:?}",
        calls.lock().unwrap()
    );
}

#[test]
fn continue_on_text_names_it_and_create_file_asks_for_the_file_and_writes_nothing() {
    let (mut rt, calls) = runtime();
    open_dialog(&mut rt);
    press(&mut rt, Key::Right);
    operate(&mut rt, "btn-0");
    assert_eq!(dialog_step(&rt), Some(Step::Name), "Continue on Text asks for the name");
    assert_eq!(text_of(&rt, "dlg-title"), "New text file");
    let nf = rt.state.viewer.new_file.as_ref().expect("the dialog is open");
    assert_eq!(nf.name.text, UNTITLED);
    assert_eq!(
        nf.name.range(),
        (0, UNTITLED.len()),
        "the name starts selected, so typing replaces it"
    );
    assert_eq!(nf.ext.text, ".txt", "the extension is its own field, the type's");
    assert_eq!(focused(&rt).as_deref(), Some(super::new_file::NAME_INPUT));
    for (i, ext) in TEXT_EXTENSIONS.into_iter().enumerate() {
        let chip = info(&rt, &format!("chip-{i}"));
        assert_eq!(chip.role, "radio", "{ext}");
        assert_eq!(chip.label.as_deref(), Some(ext));
    }
    type_text(&mut rt, "export_invoices");
    operate(&mut rt, "chip-1");
    assert_eq!(
        rt.state
            .viewer
            .new_file
            .as_ref()
            .map(|nf| nf.ext.text.clone())
            .as_deref(),
        Some(".py"),
        "a chip puts its extension in the field"
    );
    operate(&mut rt, "btn-0");
    assert_eq!(dialog_step(&rt), None, "Create file closes the dialog");
    assert_eq!(
        super::take_asked(&mut rt.state),
        [Intent::CreateFile {
            folder: rt.state.listing.folder.id.clone(),
            name: "export_invoices.py".into()
        }],
        "the file is asked for in the folder shown, named with its extension"
    );
    assert!(
        calls.lock().unwrap().is_empty(),
        "this group wrote nothing: {:?}",
        calls.lock().unwrap()
    );
}

#[test]
fn a_name_the_folder_already_has_says_why_and_typing_takes_the_message_away() {
    let (mut rt, _) = runtime();
    let taken = rt
        .state
        .listing
        .items
        .iter()
        .find(|item| !item.is_folder && item.name.contains('.'))
        .map(|item| item.name.clone())
        .expect("the board's folder holds a file");
    let (stem, ext) = taken.rsplit_once('.').expect("a name with an extension");
    open_dialog(&mut rt);
    press(&mut rt, Key::Right);
    operate(&mut rt, "btn-0");
    type_text(&mut rt, stem);
    if let Some(nf) = rt.state.viewer.new_file.as_mut() {
        nf.ext = rune::TextEdit::new(format!(".{ext}"));
    }
    super::new_file::create(&mut rt.state);
    rt.frame(0.0);
    assert_eq!(
        dialog_step(&rt),
        Some(Step::Name),
        "a taken name is not made; the dialog stays"
    );
    assert!(super::take_asked(&mut rt.state).is_empty(), "nothing was asked for");
    assert_eq!(info(&rt, "field-0-msg").role, "alert", "the reason is an alert");
    assert!(!text_of(&rt, "field-0-msg").is_empty());
    type_text(&mut rt, "x");
    assert!(!has(&rt, "field-0-msg"), "typing takes the message away");
}

#[test]
fn an_office_type_goes_on_to_new_document_without_a_name_here() {
    let (mut rt, calls) = runtime();
    for (steps, kind) in [(2, Kind::Word), (3, Kind::Excel), (4, Kind::PowerPoint)] {
        open_dialog(&mut rt);
        for _ in 0..steps {
            press(&mut rt, Key::Right);
        }
        assert_eq!(picked(&rt), Some(kind));
        operate(&mut rt, "btn-0");
        assert_eq!(dialog_step(&rt), None, "{kind:?}: New document takes over");
        assert_eq!(super::take_asked(&mut rt.state), [Intent::NewDocument(kind)]);
    }
    assert!(
        calls.lock().unwrap().is_empty(),
        "nothing was written: {:?}",
        calls.lock().unwrap()
    );
}

#[test]
fn leaving_the_files_window_closes_the_dialog() {
    let (mut rt, _) = runtime();
    open_dialog(&mut rt);
    crate::shell::host::dispatch(
        &mut rt.state,
        crate::shell::app::Event::Navigate(crate::shell::app::Route::section(crate::shell::app::Section::Photos)),
    );
    rt.frame(0.0);
    assert_eq!(dialog_step(&rt), None, "the dialog belongs to the Files window");
    assert!(!has(&rt, "dialog-new-file"));
}
