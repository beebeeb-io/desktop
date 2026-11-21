//! Tests of Backups: every built frame's content, the controls by pointer and by keyboard,
//! names and roles, the protected root, and the one-time code staying out of the tree.

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::source::{BackupsSource, FixtureSource, RootAction, Status};
use super::{BackupsUi, Dialog, FRAMES, Intent, Page};
use crate::State;
use crate::shell::app::{Screen, SettingsPane};
use crate::shell::registry::view;
use crate::shell::state::init;

fn scene(id: &str) -> fn(&mut State) {
    FRAMES
        .iter()
        .find(|(frame, _)| *frame == id)
        .unwrap_or_else(|| panic!("no scene {id}"))
        .1
}

fn runtime(id: &str) -> Runtime<State> {
    let mut state = init();
    scene(id)(&mut state);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 958.0), env);
    // As the app runs: the shell's keys first, then this group's through the registry.
    rt.on_key = Some(crate::shell::host::on_key);
    rt.frame(0.0);
    rt
}

fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    find(rt, key).is_some()
}

fn text(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

fn label(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).label.unwrap_or_default()
}

/// After an input: rebuild the tree, let what moves arrive and what leaves finish leaving (a
/// keyed element that changed place moves there, and a pointer finds it only once it has).
fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}

fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

fn type_text(rt: &mut Runtime<State>, typed: &str) {
    for c in typed.chars() {
        press(rt, Key::Text(c.to_string().into()));
    }
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
}

/// Tab until `key` has focus.
fn tab_to(rt: &mut Runtime<State>, key: &str) {
    for _ in 0..80 {
        if focused(rt).as_deref() == Some(key) {
            return;
        }
        press(rt, Key::Tab);
    }
    panic!("Tab never reached {key}; focus is on {:?}", focused(rt));
}

fn source(rt: &Runtime<State>) -> &dyn BackupsSource {
    rt.state.backups.source().expect("the fixture")
}

#[test]
fn the_six_built_frames_show_the_boards_words() {
    assert_eq!(
        FRAMES.len(),
        16,
        "the Backups frames built: 6 of Backups, 10 of Mail Backup"
    );
    let rt = runtime("DW-0737");
    assert_eq!(rt.state.shell.app.screen(), Screen::Settings(SettingsPane::Backups));
    assert_eq!(text(&rt, "set-title"), "Backups");
    assert_eq!(text(&rt, "card-0-head"), "All backed up");
    assert_eq!(text(&rt, "card-1-text"), "Couldn't back up 3 items");
    assert_eq!(
        text(&rt, "item-0-1-sub"),
        "1,820 items \u{00B7} 5.9 GB \u{00B7} Today, 09:30"
    );
    assert_eq!(text(&rt, "item-0-3-sub"), "1,106 items \u{00B7} 7.4 GB \u{00B7} off");
    assert_eq!(
        text(&rt, "item-0-6-sub"),
        "46 items \u{00B7} 640 MB \u{00B7} chosen by you \u{00B7} Today, 09:28"
    );
    assert_eq!(text(&rt, "item-0-7-title"), "Add a folder\u{2026}");
    assert_eq!(text(&rt, "item-1-2-sub"), "8,204 photos \u{00B7} Not signed in");
    assert_eq!(text(&rt, "item-2-0-value"), "On");
    assert!(text(&rt, "list-0-foot").contains("Backups \u{203A} Anna-Laptop"));

    let rt = runtime("DW-1411");
    assert_eq!(text(&rt, "card-0-head"), "Backing up Documents");
    assert_eq!(
        text(&rt, "card-0-sub"),
        "612 of 1,820 files \u{00B7} about 20 min left \u{00B7} the first backup takes longest"
    );
    // 34 % of the bar, as the board draws it.
    let (bar, fill) = (info(&rt, "card-0-bar").rect, info(&rt, "card-0-bar-fill").rect);
    assert!((fill[2] / bar[2] - 0.34).abs() < 0.001, "{fill:?} of {bar:?}");
    assert!(!has(&rt, "card-1"), "nothing failed in DW-1411");
    assert_eq!(text(&rt, "btn-0-label"), "Remove", "no Back up now while a backup runs");

    let rt = runtime("DW-1410");
    assert_eq!(text(&rt, "dlg-title"), "Stop backing up Projects?");
    assert!(text(&rt, "dlg-sub").contains("The 640 MB already backed up"));
    assert_eq!(text(&rt, "btn-2-label"), "Stop and keep the backup");
    assert_eq!(text(&rt, "btn-3-label"), "Stop and move the backup to Trash");
    assert_eq!(text(&rt, "btn-4-label"), "Cancel");

    let rt = runtime("DW-0739");
    assert_eq!(text(&rt, "set-title"), "Anna's iPad");
    assert_eq!(text(&rt, "item-0-0-value"), "4,120 photos \u{00B7} 38 GB");
    assert_eq!(text(&rt, "item-0-1-value"), "19 Oct");
    assert_eq!(text(&rt, "item-0-2-value"), "Signed in");
    assert_eq!(text(&rt, "item-2-0-title"), "Delete this backup");

    let rt = runtime("DW-0742");
    assert_eq!(text(&rt, "dlg-title"), "Delete this backup?");
    assert!(text(&rt, "dlg-text-0").starts_with("4,120 photos and 38 GB will be moved to Trash."));
    assert_eq!(rt.state.backups.code_len(), 13);

    let rt = runtime("DW-0741");
    assert_eq!(text(&rt, "set-title"), "Couldn't back up");
    assert_eq!(
        text(&rt, "item-0-2-sub"),
        "Documents \u{00B7} it kept changing while it was backing up"
    );
    assert_eq!(text(&rt, "btn-3-label"), "Try all again");
    assert_eq!(
        text(&rt, "item-1-0-sub"),
        "Desktop",
        "an exclusion says where, never an error"
    );
}

#[test]
fn the_backups_root_is_protected_and_no_frame_draws_a_control_that_changes_it() {
    for (id, _) in FRAMES {
        let rt = runtime(id);
        let root = source(&rt).root();
        assert!(root.is_protected, "{id}");
        assert!(root.allows(RootAction::Open) && !root.allows(RootAction::Delete));
        for e in rt.elements() {
            if !e.focusable || e.exiting {
                continue;
            }
            let name = e.label.clone().unwrap_or_default();
            for word in ["Rename", "Move Backups", "Share", "Delete Backups"] {
                assert!(!name.contains(word), "{id}: a control named {name:?}");
            }
        }
    }
    // The only deleting control is about ONE device's copies, behind the code.
    let mut rt = runtime("DW-0739");
    click(&mut rt, "item-2-0");
    assert_eq!(rt.state.backups.dialog, Some(Dialog::DeleteDevice("ipad".into())));
    assert_eq!(source(&rt).devices().len(), 3, "asking deletes nothing");
}

#[test]
fn a_switch_starts_and_stops_a_known_folder_by_pointer_and_by_keyboard() {
    let mut rt = runtime("DW-0737");
    let on = |rt: &Runtime<State>, id: &str| source(rt).folders().iter().any(|f| f.id == id && f.on);
    assert!(!on(&rt, "music"));
    assert_eq!(info(&rt, "item-0-3-switch").role, "switch");
    assert_eq!(label(&rt, "item-0-3-switch"), "Music");
    click(&mut rt, "item-0-3-switch");
    assert!(on(&rt, "music"));
    tab_to(&mut rt, "item-0-0-switch");
    press(&mut rt, Key::Text(" ".into()));
    assert!(!on(&rt, "desktop"), "Space on the focused switch");
    press(&mut rt, Key::Enter);
    assert!(on(&rt, "desktop"), "Enter on the focused switch");
    // A chosen folder has Remove, never a switch.
    assert!(!has(&rt, "item-0-6-switch"));
    assert_eq!(label(&rt, "btn-1"), "Remove Projects");
}

#[test]
fn back_up_now_starts_a_backup_and_the_card_shows_it_moving() {
    let mut rt = runtime("DW-0737");
    tab_to(&mut rt, "btn-0");
    press(&mut rt, Key::Enter);
    assert!(matches!(source(&rt).status(), Status::Running { .. }));
    assert_eq!(text(&rt, "card-0-head"), "Backing up Documents");
    assert_eq!(info(&rt, "card-0-bar").role, "progressbar");
    assert_eq!(label(&rt, "card-0-bar"), "Backing up Documents, 34 per cent");
}

#[test]
fn remove_asks_first_and_each_answer_does_what_it_says() {
    let mut rt = runtime("DW-0737");
    click(&mut rt, "btn-1");
    assert_eq!(rt.state.backups.dialog, Some(Dialog::StopFolder("projects".into())));
    assert_eq!(info(&rt, "dlg").role, "alertdialog");
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.backups.dialog, None, "Esc closes the alert");
    assert_eq!(source(&rt).folders().len(), 7, "and nothing changed");
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Settings(SettingsPane::Backups),
        "Esc closed the alert, not settings"
    );

    click(&mut rt, "btn-1");
    click(&mut rt, "btn-4");
    assert_eq!(source(&rt).folders().len(), 7, "Cancel");

    click(&mut rt, "btn-1");
    click(&mut rt, "btn-2");
    assert_eq!(rt.state.backups.dialog, None);
    assert_eq!(source(&rt).folders().len(), 6, "Stop and keep the backup");
    assert!(!has(&rt, "btn-1"), "Projects left the list with its Remove");

    let mut rt = runtime("DW-1410");
    tab_to(&mut rt, "btn-3");
    press(&mut rt, Key::Enter);
    assert_eq!(
        source(&rt).folders().len(),
        6,
        "Stop and move the backup to Trash, by keyboard"
    );
}

#[test]
fn what_could_not_back_up_retries_one_or_all_and_never_an_exclusion() {
    let mut rt = runtime("DW-0737");
    click(&mut rt, "card-1");
    assert_eq!(rt.state.backups.page, Page::Failures);
    assert_eq!(label(&rt, "btn-0"), "Try again: Render final.mov");
    click(&mut rt, "btn-0");
    assert_eq!(source(&rt).not_backed_up().len(), 3);
    assert_eq!(text(&rt, "item-0-0-title"), "Invoice scan.pdf");
    assert_eq!(text(&rt, "btn-2-label"), "Try all again");
    tab_to(&mut rt, "btn-2");
    press(&mut rt, Key::Enter);
    let left = source(&rt).not_backed_up();
    assert_eq!(left.len(), 1);
    assert!(left[0].excluded);
    // Only the exclusion is left: one list, no button at all.
    assert_eq!(text(&rt, "item-0-0-title"), "Site walkthrough.mkv");
    assert!(!has(&rt, "btn-0"), "an exclusion has no Try again");
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.backups.page, Page::Section, "Esc goes back to the section");
    assert!(!has(&rt, "card-1"), "no failures, no card");
    press(&mut rt, Key::Escape);
    assert!(
        !matches!(rt.state.shell.app.screen(), Screen::Settings(_)),
        "Esc on the section leaves settings"
    );
}

#[test]
fn deleting_a_devices_backup_needs_the_account_password_the_engine_accepts() {
    let mut rt = runtime("DW-0737");
    click(&mut rt, "item-1-1");
    assert_eq!(rt.state.backups.page, Page::Device("ipad".into()));
    click(&mut rt, "item-2-0");
    assert_eq!(info(&rt, "dlg").role, "dialog");
    assert!(
        !info(&rt, "btn-0").focusable,
        "Delete backup is off until a password is typed"
    );
    assert!(
        !has(&rt, "secure-code"),
        "no authenticator code boxes: the engine proves the password"
    );
    press(&mut rt, Key::Enter);
    assert_eq!(source(&rt).devices().len(), 3, "Enter does nothing before a password");

    click(&mut rt, super::view::PASSWORD_KEY);
    type_text(&mut rt, "pw");
    assert_eq!(rt.state.backups.code_len(), 2);
    assert!(info(&rt, "btn-0").focusable);

    // A password the engine does not accept: the field empties, the dialog stays, nothing is deleted.
    let mut refused = runtime("DW-0739");
    click(&mut refused, "item-2-0");
    click(&mut refused, super::view::PASSWORD_KEY);
    type_text(&mut refused, "wrong password");
    click(&mut refused, "btn-0");
    assert_eq!(source(&refused).devices().len(), 3);
    assert_eq!(refused.state.backups.code_len(), 0);
    assert!(refused.state.backups.dialog.is_some());

    click(&mut rt, "btn-0");
    assert_eq!(source(&rt).devices().len(), 2);
    assert_eq!(rt.state.backups.page, Page::Section);
    assert_eq!(rt.state.backups.dialog, None);
    assert_eq!(rt.state.backups.code_len(), 0, "the password is wiped after use");
    assert_eq!(text(&rt, "item-1-1-title"), "Old iPhone");
}

#[test]
fn the_password_is_in_no_name_and_no_text_of_the_element_tree() {
    let mut rt = runtime("DW-0739");
    click(&mut rt, "item-2-0");
    click(&mut rt, super::view::PASSWORD_KEY);
    type_text(&mut rt, "hunter22x");
    assert_eq!(rt.state.backups.code_len(), 9);
    for e in rt.elements() {
        for words in [e.label.as_deref(), e.text.as_deref(), e.help.as_deref()]
            .into_iter()
            .flatten()
        {
            assert!(
                !words.contains("hunter22x"),
                "{:?} holds the password: {words:?}",
                e.key
            );
        }
    }
    // Closing the dialog wipes what was typed.
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.backups.code_len(), 0);
}

#[test]
fn rows_that_lead_elsewhere_ask_the_shell_and_carry_the_devices_id() {
    let mut rt = runtime("DW-0737");
    click(&mut rt, "item-0-7");
    click(&mut rt, "item-2-0");
    click(&mut rt, "item-1-1");
    tab_to(&mut rt, "item-1-0");
    press(&mut rt, Key::Enter);
    click(&mut rt, "item-1-1");
    assert_eq!(
        rt.state.backups.take_intents(),
        vec![
            Intent::PickFolder,
            Intent::MailBackup,
            Intent::OpenInPhotos("ipad".into()),
            Intent::OpenInFiles("ipad".into()),
        ]
    );
    // The picker's answer: Beebeeb's own folder cannot be chosen.
    assert!(!rt.state.backups.folder_picked(r"C:\Users\Anna\Beebeeb\Work"));
    assert!(rt.state.backups.folder_picked(r"C:\Users\Anna\Projects 2"));
    // The crumb goes back to the section.
    click(&mut rt, "crumb-row");
    assert_eq!(rt.state.backups.page, Page::Section);
}

#[test]
fn leaving_the_section_forgets_what_was_open_and_wipes_the_code() {
    let mut rt = runtime("DW-0742");
    assert_eq!(rt.state.backups.code_len(), 13);
    rt.update(|s, _| crate::frames::settings::go(s, "DW-0701"));
    rt.frame(0.0);
    assert_eq!(rt.state.backups.dialog, None);
    assert_eq!(rt.state.backups.page, Page::Section);
    assert_eq!(rt.state.backups.code_len(), 0);
    // The sidebar's Backups item comes back to the section's own page.
    rt.update(|s, _| crate::frames::settings::go(s, "DW-0737"));
    rt.frame(0.0);
    assert_eq!(text(&rt, "set-title"), "Backups");
    assert!(!has(&rt, "dlg"));
}

#[test]
fn a_build_with_no_source_draws_the_not_built_frame_never_sample_values() {
    let mut state = init();
    crate::frames::settings::go(&mut state, "DW-0737");
    state.backups = BackupsUi::new(None);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 958.0), env);
    rt.frame(0.0);
    assert!(has(&rt, "not-built"));
    assert!(!has(&rt, "card-0"));
    assert!(
        super::source::first(false).is_none(),
        "a build that ships starts unwired"
    );
    assert!(super::source::first(true).is_some());
    // The fixture the scenes use is the board's.
    assert_eq!(FixtureSource::board().folders().len(), 7);
}

/// A source that answers late, as the engine's does: a delete is sent, then answered; another device's backup that
/// this PC cannot read; no device links (the shell does not take them).
struct Late {
    devices: Vec<super::source::Device>,
    deleting: std::sync::Arc<std::sync::atomic::AtomicBool>,
    asked: std::sync::Arc<std::sync::Mutex<Vec<(String, usize)>>>,
}

impl BackupsSource for Late {
    fn wired(&self, part: super::source::Part) -> bool {
        use super::source::Part;
        matches!(part, Part::Root | Part::Folders | Part::Devices | Part::DeleteDevice)
    }
    fn deleting(&self) -> bool {
        self.deleting.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn root(&self) -> super::source::BackupsRoot {
        super::source::BackupsRoot {
            id: String::new(),
            is_protected: true,
            device_folder: String::new(),
        }
    }
    fn status(&self) -> Status {
        Status::Plain {
            title: String::new(),
            sub: String::new(),
            can_run: false,
        }
    }
    fn folders(&self) -> Vec<super::source::Folder> {
        Vec::new()
    }
    fn devices(&self) -> Vec<super::source::Device> {
        self.devices.clone()
    }
    fn not_backed_up(&self) -> Vec<super::source::NotBackedUp> {
        Vec::new()
    }
    fn mail_backup_on(&self) -> bool {
        false
    }
    fn back_up_now(&mut self) {}
    fn set_folder(&mut self, _id: &str, _on: bool) {}
    fn add_folder(&mut self, _path: &str) -> bool {
        false
    }
    fn remove_folder(&mut self, _id: &str, _move_copy_to_trash: bool) {}
    fn retry(&mut self, _id: &str) {}
    fn retry_all(&mut self) {}
    fn delete_device_backup(&mut self, id: &str, code: &str) -> bool {
        // The password's length only: the test never keeps it.
        self.asked.lock().unwrap().push((id.to_string(), code.len()));
        self.deleting.store(true, std::sync::atomic::Ordering::SeqCst);
        true
    }
}

fn texts(rt: &Runtime<State>) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.text.or(e.label))
        .collect()
}

/// Review of desktop#425: a device that may be this PC's own backup says it cannot read it and why it cannot be
/// deleted here; no "Delete this backup", and pressing the row opens nothing.
#[test]
fn a_device_that_may_be_this_pcs_has_no_delete() {
    use super::source::{Backed, Device, DeviceKind};
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let late = Late {
        devices: vec![Device {
            id: "maybe-me".into(),
            name: "Invented PC".into(),
            model: "device".into(),
            kind: DeviceKind::Unknown,
            backed: Backed::Unreadable,
            size: None,
            last_backup: None,
            signed_in: None,
            deletable: false,
        }],
        deleting: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        asked: asked.clone(),
    };
    let mut state = init();
    crate::frames::settings::go(&mut state, "DW-0737");
    state.backups = BackupsUi::new(Some(Box::new(late)));
    state.backups.show(Page::Device("maybe-me".into()));
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 958.0), env);
    rt.frame(0.0);
    let page = texts(&rt);
    for present in [
        super::view::CANT_READ,
        super::view::MAY_BE_THIS_PC,
        "Can't delete this backup here",
    ] {
        assert!(page.iter().any(|t| t == present), "{present} is drawn");
    }
    assert!(!page.iter().any(|t| t == "Delete this backup"), "never offered");
    click(&mut rt, "item-2-0");
    assert_eq!(rt.state.backups.dialog, None, "pressing the row opens nothing");
    assert!(asked.lock().unwrap().is_empty());
}

#[test]
fn a_late_delete_keeps_the_dialog_until_the_answer_and_a_refusal_shows_under_the_field() {
    use super::source::{Backed, Device, DeviceKind};
    let deleting = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let late = Late {
        devices: vec![Device {
            id: "pc".into(),
            name: "Invented old laptop".into(),
            model: "device".into(),
            kind: DeviceKind::Unknown,
            backed: Backed::Unreadable,
            size: None,
            last_backup: None,
            signed_in: None,
            deletable: true,
        }],
        deleting: deleting.clone(),
        asked: asked.clone(),
    };
    let mut state = init();
    crate::frames::settings::go(&mut state, "DW-0737");
    state.backups = BackupsUi::new(Some(Box::new(late)));
    state.backups.show(Page::Device("pc".into()));
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 958.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.frame(0.0);
    let page = texts(&rt);
    assert!(
        page.iter().any(|t| t == super::view::CANT_READ),
        "the page says it cannot read it"
    );
    for absent in [
        "Open in Photos",
        "Open in Files",
        "Status",
        "Signed in",
        "Not signed in",
    ] {
        assert!(!page.iter().any(|t| t == absent), "{absent} is not drawn");
    }

    click(&mut rt, "item-2-0");
    assert_eq!(rt.state.backups.dialog, Some(Dialog::DeleteDevice("pc".into())));
    click(&mut rt, super::view::PASSWORD_KEY);
    type_text(&mut rt, "an invented password");
    click(&mut rt, "btn-0");
    assert_eq!(*asked.lock().unwrap(), [("pc".to_string(), 20)]);
    assert_eq!(
        rt.state.backups.dialog,
        Some(Dialog::DeleteDevice("pc".into())),
        "sent: the dialog stays until Beebeeb answers"
    );
    assert_eq!(rt.state.backups.code_len(), 0, "the password is wiped once handed over");
    assert!(texts(&rt).iter().any(|t| t == "Deleting\u{2026}"));
    // A second press while it is on its way sends nothing: the button is off, and the call refuses too.
    click(&mut rt, super::view::PASSWORD_KEY);
    type_text(&mut rt, "again");
    assert_eq!(rt.state.backups.code_len(), 5, "typed into the field");
    assert!(
        !info(&rt, "btn-0").focusable,
        "Delete backup is off while one is on its way"
    );
    click(&mut rt, "btn-0");
    assert!(!rt.state.backups.delete_device());
    assert_eq!(asked.lock().unwrap().len(), 1);

    // Refused: the words under the field, the dialog open, the device still listed.
    deleting.store(false, std::sync::atomic::Ordering::SeqCst);
    rt.state
        .backups
        .delete_answered(Err("That password isn't right. Nothing was deleted."));
    settle(&mut rt);
    assert_eq!(rt.state.backups.dialog, Some(Dialog::DeleteDevice("pc".into())));
    assert!(
        texts(&rt)
            .iter()
            .any(|t| t == "That password isn't right. Nothing was deleted.")
    );
    assert_eq!(source(&rt).devices().len(), 1);

    // Done: the dialog closes and the section shows (the source reads its list again).
    rt.state.backups.code.clear();
    click(&mut rt, super::view::PASSWORD_KEY);
    type_text(&mut rt, "an invented password");
    assert_eq!(rt.state.backups.code_len(), 20, "typed into the field");
    click(&mut rt, "btn-0");
    assert_eq!(asked.lock().unwrap().len(), 2);
    deleting.store(false, std::sync::atomic::Ordering::SeqCst);
    rt.state.backups.delete_answered(Ok(()));
    assert_eq!(rt.state.backups.dialog, None);
    assert_eq!(rt.state.backups.page, Page::Section);

    // An answer after the dialog was closed is said in the section, with what it means for the list.
    rt.state
        .backups
        .delete_answered(Err("This backup could not be found. Nothing was deleted."));
    assert_eq!(
        rt.state.backups.notice(),
        Some((
            "This backup could not be found. Nothing was deleted.",
            super::DELETE_NOTICE_SUB
        ))
    );
}

#[test]
fn a_delete_answer_never_lands_in_another_devices_dialog() {
    use super::source::{Backed, Device, DeviceKind};
    let device = |id: &str| Device {
        id: id.into(),
        name: format!("Invented {id}"),
        model: "device".into(),
        kind: DeviceKind::Computer,
        backed: Backed::Folders(1),
        size: None,
        last_backup: None,
        signed_in: None,
        deletable: true,
    };
    let deleting = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut ui = BackupsUi::new(Some(Box::new(Late {
        devices: vec![device("a"), device("b")],
        deleting: deleting.clone(),
        asked: asked.clone(),
    })));
    ui.ask_delete_device("a");
    ui.code.replace_text("an invented password");
    assert!(ui.delete_device());
    // Closed while A's is on its way; B's dialog opens.
    ui.close_dialog();
    ui.ask_delete_device("b");
    assert!(
        ui.deleting_for("a") && !ui.deleting_for("b"),
        "B's dialog does not say Deleting"
    );
    ui.code.replace_text("an invented password");
    assert!(!ui.delete_device(), "one at a time");
    assert_eq!(asked.lock().unwrap().len(), 1);
    // A's refusal: said in the section, never under B's field.
    deleting.store(false, std::sync::atomic::Ordering::SeqCst);
    ui.delete_answered(Err("That password isn't right. Nothing was deleted."));
    assert_eq!(ui.delete_refusal(), None);
    assert_eq!(
        ui.dialog,
        Some(Dialog::DeleteDevice("b".into())),
        "B's dialog stays open"
    );
    assert_eq!(
        ui.notice().map(|(words, _)| words),
        Some("That password isn't right. Nothing was deleted.")
    );
    // An answer with nothing sent closes nothing.
    ui.delete_answered(Ok(()));
    assert_eq!(ui.dialog, Some(Dialog::DeleteDevice("b".into())));
}

/// A source with nothing to show: still reading, refused, or an empty PC.
struct Quiet {
    loading: bool,
    refusal: Option<&'static str>,
    read_again: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl BackupsSource for Quiet {
    fn loading(&self) -> bool {
        self.loading
    }
    fn refusal(&self) -> Option<&'static str> {
        self.refusal
    }
    fn read_again(&mut self) {
        self.read_again.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    fn root(&self) -> super::source::BackupsRoot {
        super::source::BackupsRoot {
            id: String::new(),
            is_protected: true,
            device_folder: String::new(),
        }
    }
    fn status(&self) -> Status {
        Status::Plain {
            title: String::new(),
            sub: String::new(),
            can_run: false,
        }
    }
    fn folders(&self) -> Vec<super::source::Folder> {
        Vec::new()
    }
    fn devices(&self) -> Vec<super::source::Device> {
        Vec::new()
    }
    fn not_backed_up(&self) -> Vec<super::source::NotBackedUp> {
        Vec::new()
    }
    fn mail_backup_on(&self) -> bool {
        false
    }
    fn back_up_now(&mut self) {}
    fn set_folder(&mut self, _id: &str, _on: bool) {}
    fn add_folder(&mut self, _path: &str) -> bool {
        false
    }
    fn remove_folder(&mut self, _id: &str, _move_copy_to_trash: bool) {}
    fn retry(&mut self, _id: &str) {}
    fn retry_all(&mut self) {}
    fn delete_device_backup(&mut self, _id: &str, _code: &str) -> bool {
        false
    }
}

fn quiet_runtime(loading: bool, refusal: Option<&'static str>) -> (Runtime<State>, std::sync::Arc<std::sync::atomic::AtomicBool>) {
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut state = init();
    crate::frames::settings::go(&mut state, "DW-0737");
    state.backups = BackupsUi::new(Some(Box::new(Quiet {
        loading,
        refusal,
        read_again: flag.clone(),
    })));
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 958.0), env);
    rt.frame(0.0);
    (rt, flag)
}

#[test]
fn the_section_says_loading_empty_and_failed_and_try_again_reads_again() {
    let (rt, _) = quiet_runtime(true, None);
    assert!(has(&rt, "loading"), "loading while the first answer is out");
    assert!(!has(&rt, "backups-empty"));

    let (rt, _) = quiet_runtime(false, None);
    assert!(has(&rt, "backups-empty"), "no folders: an empty state, not a blank list");

    let (mut rt, flag) = quiet_runtime(false, Some("Could not read."));
    assert!(has(&rt, "backups-failed"));
    assert!(!has(&rt, "backups-empty"));
    click(&mut rt, "backups-failed-retry");
    assert!(flag.load(std::sync::atomic::Ordering::SeqCst), "Try again reads again");
}
