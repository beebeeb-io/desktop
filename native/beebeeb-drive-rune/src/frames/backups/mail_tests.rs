//! Mail Backup's page and dialogs, by the app's own controls: every frame's words, the flow from
//! Turn on… to the first archive, the refusal, turning off behind a code, and the app password
//! staying out of the tree.

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::mail::{self, Connection, MailState, board};
use super::source::{BackupsSource, FixtureSource};
use super::{BackupsUi, Dialog, FRAMES, Intent, Page};
use crate::State;
use crate::shell::registry::view;
use crate::shell::state::init;

fn runtime(id: &str) -> Runtime<State> {
    let mut state = init();
    let scene = FRAMES
        .iter()
        .find(|(frame, _)| *frame == id)
        .unwrap_or_else(|| panic!("no scene {id}"))
        .1;
    scene(&mut state);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 958.0), env);
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

fn text(rt: &Runtime<State>, key: &str) -> String {
    info(rt, key).text.unwrap_or_default()
}

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

fn mail_of(rt: &Runtime<State>) -> mail::Mail {
    rt.state
        .backups
        .source()
        .expect("the fixture")
        .mail()
        .expect("mail answers")
}

fn connection(rt: &Runtime<State>) -> Connection {
    rt.state.backups.source().expect("the fixture").mail_connection()
}

#[test]
fn every_mail_frame_shows_the_boards_words() {
    for (frame, title, sub) in [
        ("DW-0740", "Mail Backup is on", "Runs on this PC (Anna-Laptop)"),
        (
            "DW-0740b",
            "Mail Backup is off",
            "It runs on a PC: this one, once you turn it on.",
        ),
        (
            "DW-1405",
            "Backing up your mail",
            "The first backup takes longest. You can use your PC as normal.",
        ),
        ("DW-1406", "Mail Backup is on", "Runs on this PC (Anna-Laptop)"),
    ] {
        let rt = runtime(frame);
        assert_eq!(rt.state.backups.page, Page::Mail, "{frame}");
        assert_eq!(text(&rt, "mail-title"), title, "{frame}");
        assert_eq!(text(&rt, "mail-sub"), sub, "{frame}");
    }
    let rt = runtime("DW-0740");
    assert_eq!(text(&rt, "item-0-0-value"), "6.2 GB");
    assert_eq!(text(&rt, "item-0-2-value"), "In 12 min");
    assert_eq!(
        text(&rt, "item-1-0-sub"),
        "IMAP \u{00B7} mail.northwind.studio \u{00B7} 4 folders"
    );
    assert_eq!(text(&rt, "pill-0-label"), "On");
    let rt = runtime("DW-1405");
    // 40 % of the bar, as the board draws 6,912 of 17,480.
    let (bar, fill) = (info(&rt, "card-0-bar").rect, info(&rt, "card-0-bar-fill").rect);
    assert!((fill[2] / bar[2] - 0.40).abs() < 0.001, "{} of {}", fill[2], bar[2]);
    assert_eq!(text(&rt, "mail-left"), "about 40 min left");
    let rt = runtime("DW-1406");
    assert_eq!(text(&rt, "banner-0-title"), "Couldn't back up anna@northwind.studio");
    assert_eq!(info(&rt, "banner-0").role, "alert");
    for (frame, dialog, title) in [
        ("DW-1401", Dialog::MailTurnOn, "Turn on Mail Backup"),
        ("DW-1402", Dialog::MailAccount, "Add a mail account"),
        ("DW-1403", Dialog::MailAccount, "Add a mail account"),
        ("DW-1404", Dialog::MailFolders, "Choose folders"),
        ("DW-0743", Dialog::MailTurnOff, "Turn off Mail Backup?"),
        ("DW-0743b", Dialog::MailKeepFiles, "Keep Mail Backup's files?"),
    ] {
        let rt = runtime(frame);
        assert_eq!(rt.state.backups.dialog, Some(dialog), "{frame}");
        assert_eq!(text(&rt, "dlg-title"), title, "{frame}");
    }
}

#[test]
fn the_services_row_shows_the_page_only_where_a_source_answers_for_mail() {
    // A source with Mail Backup behind it: the page, and nothing asked of the shell.
    let mut rt = runtime("DW-0740");
    rt.state.backups.show(Page::Section);
    settle(&mut rt);
    assert_eq!(text(&rt, "item-2-0-value"), "On");
    click(&mut rt, "item-2-0");
    assert_eq!(rt.state.backups.page, Page::Mail);
    assert!(rt.state.backups.take_intents().is_empty());
    // Off says Off on the row.
    let mut rt = runtime("DW-0740b");
    rt.state.backups.show(Page::Section);
    settle(&mut rt);
    assert_eq!(text(&rt, "item-2-0-value"), "Off");

    // The trait's default (no mail engine): no page, by the row or by any other way in.
    let mut ui = BackupsUi::fixture(FixtureSource::board());
    assert!(ui.source().expect("the fixture").mail().is_none());
    ui.show(Page::Mail);
    assert_eq!(ui.page, Page::Section);
    ui.mail_backup();
    assert_eq!(ui.page, Page::Section);
    assert_eq!(ui.take_intents(), vec![Intent::MailBackup]);
}

#[test]
fn turning_on_goes_step_by_step_and_never_ahead_of_the_servers_answer() {
    let mut rt = runtime("DW-0740b");
    assert_eq!(text(&rt, "btn-0-label"), "Turn on\u{2026}");
    click(&mut rt, "btn-0");
    assert_eq!(rt.state.backups.dialog, Some(Dialog::MailTurnOn));
    assert_eq!(text(&rt, "item-1-3-title"), "Read only");
    click(&mut rt, "btn-1");
    assert_eq!(rt.state.backups.dialog, Some(Dialog::MailAccount));
    assert!(rt.state.backups.mail.email.focused, "the first field has the keyboard");
    assert_eq!(text(&rt, "btn-1-label"), "Connect");

    // Nothing typed: Connect is off, a press and Enter ask nothing.
    assert!(!info(&rt, "btn-1").focusable);
    click(&mut rt, "btn-1");
    assert!(!rt.state.backups.mail_connect());
    assert_eq!(connection(&rt), Connection::Idle);

    // Typed by the keyboard, field by field.
    type_text(&mut rt, board::ADDRESS);
    click(&mut rt, "field-1-input");
    type_text(&mut rt, board::SERVER);
    click(&mut rt, "field-3-input");
    type_text(&mut rt, "not-the-password");
    assert_eq!(rt.state.backups.mail.email.text, board::ADDRESS);
    assert_eq!(rt.state.backups.mail.server.text, board::SERVER);
    assert_eq!(rt.state.backups.mail.port.text, "993");
    assert!(info(&rt, "btn-1").focusable);

    // The server refuses: the step stays, says so, saves nothing.
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.backups.dialog, Some(Dialog::MailAccount));
    assert!(matches!(connection(&rt), Connection::Refused { .. }));
    assert_eq!(text(&rt, "banner-0-title"), "Couldn't sign in to mail.northwind.studio");
    assert_eq!(text(&rt, "field-3-msg"), "That password wasn't accepted.");
    assert_eq!(info(&rt, "field-3-msg").role, "alert");
    assert_eq!(text(&rt, "btn-1-label"), "Try again");
    assert!(
        find(&rt, "dlg-text-0").is_none(),
        "the notice takes the small print's place"
    );
    assert_eq!(mail_of(&rt).state, MailState::Off);

    // The right password: the folders, the password wiped.
    rt.state.backups.mail.password.replace_text(board::APP_PASSWORD);
    settle(&mut rt);
    click(&mut rt, "btn-1");
    assert_eq!(rt.state.backups.dialog, Some(Dialog::MailFolders));
    assert!(rt.state.backups.mail.password.text.is_empty());
    assert_eq!(text(&rt, "dlg-sub"), board::ADDRESS);
    assert_eq!(rt.state.backups.mail.picked, ["Inbox", "Sent", "Archive", "Clients"]);
    assert_eq!(info(&rt, "item-1-0").role, "checkbox");

    // Tick Newsletters, untick Sent.
    click(&mut rt, "item-1-4");
    click(&mut rt, "item-1-1");
    assert_eq!(
        rt.state.backups.mail.picked,
        ["Inbox", "Archive", "Clients", "Newsletters"]
    );
    assert_eq!(mail_of(&rt).state, MailState::Off, "nothing starts before Start backup");
    click(&mut rt, "btn-1");
    assert_eq!(rt.state.backups.dialog, None);
    assert!(matches!(mail_of(&rt).state, MailState::First { paused: false, .. }));
    assert_eq!(mail_of(&rt).accounts[0].folders, 4);
    assert_eq!(text(&rt, "mail-title"), "Backing up your mail");
    assert_eq!(connection(&rt), Connection::Idle);
}

#[test]
fn start_backup_is_off_with_no_folder_ticked_and_back_asks_the_password_again() {
    let mut rt = runtime("DW-1404");
    for i in 0..4 {
        click(&mut rt, &format!("item-1-{i}"));
    }
    assert!(rt.state.backups.mail.picked.is_empty());
    assert!(!info(&rt, "btn-1").focusable);
    click(&mut rt, "btn-1");
    assert!(!rt.state.backups.mail_start());
    assert_eq!(rt.state.backups.dialog, Some(Dialog::MailFolders));
    assert_eq!(mail_of(&rt).state, MailState::Off);

    click(&mut rt, "btn-2");
    assert_eq!(rt.state.backups.dialog, Some(Dialog::MailAccount));
    assert_eq!(connection(&rt), Connection::Idle, "the sign-in is dropped");
    assert!(rt.state.backups.mail.password.text.is_empty());
    assert!(rt.state.backups.mail.password.focused);
    assert_eq!(rt.state.backups.mail.email.text, board::ADDRESS);
    assert!(!info(&rt, "btn-1").focusable, "no password, no Connect");
}

#[test]
fn closing_a_dialog_wipes_what_was_typed_and_drops_the_sign_in() {
    let mut rt = runtime("DW-1403");
    assert!(!rt.state.backups.mail.password.text.is_empty());
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.backups.dialog, None);
    assert_eq!(rt.state.backups.page, Page::Mail, "Esc closed the dialog, not the page");
    let typed = &rt.state.backups.mail;
    assert!(typed.password.text.is_empty() && typed.email.text.is_empty() && typed.server.text.is_empty());
    assert_eq!(connection(&rt), Connection::Idle);

    let mut rt = runtime("DW-1404");
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.backups.dialog, None);
    assert!(rt.state.backups.mail.picked.is_empty());
    assert_eq!(connection(&rt), Connection::Idle);
}

#[test]
fn the_app_password_is_in_no_text_label_or_debug_output() {
    let rt = runtime("DW-1402");
    assert_eq!(rt.state.backups.mail.password.text, board::APP_PASSWORD);
    for e in rt.elements() {
        for words in [&e.text, &e.label, &e.help].into_iter().flatten() {
            assert!(
                !words.contains(board::APP_PASSWORD),
                "{:?} shows the app password",
                e.key
            );
        }
    }
    // The gate is told bullets, one per character.
    assert_eq!(
        crate::ui::au::gate_text(&rt.state.backups.mail.password).as_deref(),
        Some("\u{2022}".repeat(board::APP_PASSWORD.len()).as_str())
    );
    // A person's address and server are in no Debug output.
    let shown = format!(
        "{:?} {:?} {:?} {:?}",
        mail_of(&rt),
        board::failed().failure,
        board::on().accounts,
        Connection::Refused {
            server: board::SERVER.into()
        }
    );
    assert!(!shown.contains("northwind") && !shown.contains("anna"), "{shown}");
    let entry = mail::Entry {
        address: board::ADDRESS,
        server: board::SERVER,
        port: 993,
        password: board::APP_PASSWORD,
    };
    let shown = format!("{entry:?}");
    assert!(
        !shown.contains("northwind") && !shown.contains(board::APP_PASSWORD),
        "{shown}"
    );
}

#[test]
fn turning_off_takes_a_code_the_engine_accepts_then_asks_about_the_files() {
    let mut rt = runtime("DW-0740");
    click(&mut rt, "item-2-0");
    assert_eq!(rt.state.backups.dialog, Some(Dialog::MailTurnOff));
    assert!(!info(&rt, "btn-0").focusable, "Turn off waits for six digits");

    // A code the engine refuses: the boxes empty, the dialog stays, Mail Backup stays on.
    type_text(&mut rt, "000000");
    assert_eq!(rt.state.backups.code_len(), 6);
    press(&mut rt, Key::Enter);
    assert_eq!(rt.state.backups.dialog, Some(Dialog::MailTurnOff));
    assert_eq!(rt.state.backups.code_len(), 0);
    assert_eq!(mail_of(&rt).state, MailState::On);

    // Five digits: nothing is asked.
    type_text(&mut rt, "51720");
    assert!(!rt.state.backups.mail_turn_off());
    assert_eq!(
        rt.state.backups.code_len(),
        5,
        "nothing was asked, so nothing was wiped"
    );
    assert_eq!(mail_of(&rt).state, MailState::On);
    type_text(&mut rt, "4");
    click(&mut rt, "btn-0");
    assert_eq!(mail_of(&rt).state, MailState::Off);
    assert_eq!(rt.state.backups.code_len(), 0, "the code is wiped after use");
    assert_eq!(rt.state.backups.dialog, Some(Dialog::MailKeepFiles));
    assert_eq!(info(&rt, "dlg").role, "alertdialog");
    assert_eq!(text(&rt, "mail-title"), "Mail Backup is off");

    // Esc keeps the files.
    press(&mut rt, Key::Escape);
    assert_eq!(rt.state.backups.dialog, None);
    assert_eq!(mail_of(&rt).used.as_deref(), Some("6.2 GB"));
    assert_eq!(text(&rt, "list-0-title"), "Kept folder");
    assert_eq!(text(&rt, "btn-0-label"), "Turn on\u{2026}");
}

#[test]
fn move_to_trash_moves_the_kept_files_and_keep_them_does_not() {
    let mut rt = runtime("DW-0743b");
    assert_eq!(text(&rt, "btn-1-label"), "Keep them");
    click(&mut rt, "btn-1");
    assert_eq!(rt.state.backups.dialog, None);
    assert!(mail_of(&rt).used.is_some());

    let mut rt = runtime("DW-0743b");
    assert_eq!(text(&rt, "btn-2-label"), "Move to Trash");
    click(&mut rt, "btn-2");
    assert_eq!(rt.state.backups.dialog, None);
    assert!(mail_of(&rt).used.is_none());
    assert!(
        find(&rt, "list-0-body").is_none(),
        "no kept folder is drawn once it moved"
    );
    assert!(find(&rt, "mail-note").is_some());
}

#[test]
fn pause_update_password_and_open_its_folder_do_what_they_say() {
    let mut rt = runtime("DW-1405");
    assert_eq!(text(&rt, "btn-0-label"), "Pause");
    click(&mut rt, "btn-0");
    assert!(matches!(mail_of(&rt).state, MailState::First { paused: true, .. }));
    assert_eq!(text(&rt, "btn-0-label"), "Resume");
    click(&mut rt, "btn-0");
    assert!(matches!(mail_of(&rt).state, MailState::First { paused: false, .. }));

    let mut rt = runtime("DW-1406");
    let mut other_port = board::failed();
    other_port.accounts[0].port = 1993;
    rt.state.backups = BackupsUi::fixture(FixtureSource::with_mail(other_port));
    rt.state.backups.show(Page::Mail);
    settle(&mut rt);
    click(&mut rt, "btn-0");
    assert_eq!(rt.state.backups.dialog, Some(Dialog::MailAccount));
    let typed = &rt.state.backups.mail;
    assert_eq!(typed.email.text, board::ADDRESS);
    assert_eq!(typed.server.text, board::SERVER);
    assert_eq!(typed.port.text, "1993", "the account's own port, not the default");
    assert!(typed.password.text.is_empty() && typed.password.focused);
    // A refusal here: two notices on screen, each with its own key.
    rt.state.backups.mail.password.replace_text("not-the-password");
    assert!(rt.state.backups.mail_connect());
    settle(&mut rt);
    assert_eq!(text(&rt, "banner-0-title"), "Couldn't back up anna@northwind.studio");
    assert_eq!(text(&rt, "banner-1-title"), "Couldn't sign in to mail.northwind.studio");

    let mut rt = runtime("DW-0740");
    click(&mut rt, "item-0-3");
    assert_eq!(rt.state.backups.take_intents(), vec![Intent::OpenMailFolder]);
    click(&mut rt, "crumb-row");
    assert_eq!(rt.state.backups.page, Page::Section);
}

/// A source whose mail server answers late: "no answer yet" to every sign-in, until the test
/// sets the answer.
struct LateSource {
    inner: FixtureSource,
    late: std::rc::Rc<std::cell::RefCell<Connection>>,
    asked: std::rc::Rc<std::cell::Cell<usize>>,
}

impl BackupsSource for LateSource {
    fn root(&self) -> super::source::BackupsRoot {
        self.inner.root()
    }
    fn status(&self) -> super::source::Status {
        self.inner.status()
    }
    fn folders(&self) -> Vec<super::source::Folder> {
        self.inner.folders()
    }
    fn devices(&self) -> Vec<super::source::Device> {
        self.inner.devices()
    }
    fn not_backed_up(&self) -> Vec<super::source::NotBackedUp> {
        self.inner.not_backed_up()
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
    fn mail(&self) -> Option<mail::Mail> {
        Some(board::off())
    }
    fn mail_connect(&mut self, _entry: &mail::Entry<'_>) {
        self.asked.set(self.asked.get() + 1);
        *self.late.borrow_mut() = Connection::Checking;
    }
    fn mail_connection(&self) -> Connection {
        self.late.borrow().clone()
    }
}

#[test]
fn a_late_answer_is_asked_once_and_taken_on_the_next_window_key() {
    let late = std::rc::Rc::new(std::cell::RefCell::new(Connection::Idle));
    let asked = std::rc::Rc::new(std::cell::Cell::new(0));
    let mut rt = runtime("DW-0740b");
    rt.state.backups = BackupsUi::new(Some(Box::new(LateSource {
        inner: FixtureSource::board(),
        late: late.clone(),
        asked: asked.clone(),
    })));
    rt.state.backups.show(Page::Mail);
    rt.state.backups.mail_add_account();
    let typed = &mut rt.state.backups.mail;
    typed.email.replace_text(board::ADDRESS);
    typed.server.replace_text(board::SERVER);
    typed.password.replace_text(board::APP_PASSWORD);
    settle(&mut rt);
    click(&mut rt, "btn-1");
    assert_eq!(asked.get(), 1);
    assert_eq!(
        rt.state.backups.dialog,
        Some(Dialog::MailAccount),
        "no answer, no next step"
    );
    assert_eq!(text(&rt, "btn-1-label"), "Connecting\u{2026}");
    assert!(!info(&rt, "btn-1").focusable);
    // A second press and a second call while the answer is out ask nothing.
    click(&mut rt, "btn-1");
    assert!(!rt.state.backups.mail_connect());
    assert_eq!(asked.get(), 1);
    assert!(
        !rt.state.backups.mail.password.text.is_empty(),
        "kept until the server answers"
    );
    // The answer is in but not taken yet: Connect stays off.
    *late.borrow_mut() = Connection::Folders {
        address: board::ADDRESS.into(),
        folders: board::folders(),
        size: board::SIZE.into(),
    };
    settle(&mut rt);
    assert_eq!(rt.state.backups.dialog, Some(Dialog::MailAccount));
    assert!(!info(&rt, "btn-1").focusable);
    // The next key that reaches the window takes it: Choose folders, the password wiped.
    press(&mut rt, Key::ArrowDown);
    assert_eq!(rt.state.backups.dialog, Some(Dialog::MailFolders));
    assert!(rt.state.backups.mail.password.text.is_empty());
    assert_eq!(asked.get(), 1);
}
