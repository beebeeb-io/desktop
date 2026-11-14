//! Backups on Windows (the board's DW-0737, DW-1411, DW-1410, DW-0739, DW-0742, DW-0741): a
//! section of the settings view (`frames/this_pc/FRAMES.md` is the checklist). DW-0738 is
//! Windows' own folder picker: the shell opens it on [`Intent::PickFolder`].
//!
//! Whether the window shows Backups is the shell's route (`Screen::Settings(Backups)`).
//! [`BackupsUi`] holds only what is open inside the section: the page (the section, one
//! device, the list of what could not back up), a dialog over it, and the one-time code.
//! Every control changes it through one of its methods and reads the engine through
//! [`source::BackupsSource`].
//!
//! The Backups root is protected exactly as on the web: [`source::BackupsRoot::is_protected`]
//! comes from the engine, and no frame here draws a control that renames, moves, deletes or
//! shares it. "Open in Files" opens it; "Delete this backup" is about ONE device's copies,
//! behind a second-factor code.
//!
//! Secret: the authenticator code (DW-0742). It lives in ONE secure `TextEdit` (wiped on
//! drop and on every close), is drawn as secure text inside `secure-code`, and is in no
//! label. Comment mode stays denied on this section (`FRAME`).

pub mod mail;
#[cfg(test)]
mod mail_tests;
mod mail_view;
pub mod source;
#[cfg(test)]
mod tests;
mod view;

use rune::{Key, KeyEvent, TextEdit, Tx};

use crate::State;
use crate::shell::app::{Screen, SettingsPane};
use crate::shell::registry::{FeedbackPolicy, Frame, FrameGroup};
use crate::ui::backups_kit::CODE_LEN;
use source::{BackupsSource, FixtureSource};

/// The registry's line for the section (`shell/registry.rs`).
pub const FRAME: Frame = Frame {
    group: FrameGroup::Settings(SettingsPane::Backups),
    view: view::view,
    on_key: Some(on_key),
    feedback: FeedbackPolicy::Denied("this section can ask for your authenticator code"),
};

/// What the section shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Page {
    /// DW-0737, DW-1411.
    Section,
    /// Another device's backup, by its id (DW-0739).
    Device(String),
    /// What could not back up (DW-0741).
    Failures,
    /// Mail Backup: on, off, or its first archive running (DW-0740, DW-0740b, DW-1405, DW-1406).
    Mail,
}

/// A dialog over a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialog {
    /// "Stop backing up <folder>?" for the chosen folder of this id (DW-1410).
    StopFolder(String),
    /// "Delete this backup?" for the device of this id, with the code (DW-0742).
    DeleteDevice(String),
    /// "Turn on Mail Backup": what it does, before anything is asked (DW-1401).
    MailTurnOn,
    /// "Add a mail account": address, server, port, app password (DW-1402, DW-1403).
    MailAccount,
    /// "Choose folders" of the mailbox that just signed in (DW-1404).
    MailFolders,
    /// "Turn off Mail Backup?" with the code (DW-0743).
    MailTurnOff,
    /// "Keep Mail Backup's files?" once it is off (DW-0743b).
    MailKeepFiles,
}

/// What Backups asks of the app around it; the shell takes them with [`BackupsUi::take_intents`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Open Windows' folder picker, "Choose a folder to back up" (DW-0738); the answer comes
    /// back through [`BackupsUi::folder_picked`].
    PickFolder,
    /// Photos, filtered to the device of this id (DW-0601).
    OpenInPhotos(String),
    /// The device's folder inside the Backups root, in Files (DW-0422).
    OpenInFiles(String),
    /// Mail Backup, asked of the shell while no source answers for it (DW-0740).
    MailBackup,
    /// Mail Backup's protected folder inside the Backups root, in Files (DW-0424).
    OpenMailFolder,
}

/// The most intents kept while nothing takes them: the oldest fall off.
pub const INTENT_LIMIT: usize = 32;

pub struct BackupsUi {
    /// `None`: nothing answers for Backups yet (a build that ships before the engine lane
    /// wires it). The view then draws the shell's "not built yet" frame.
    source: Option<Box<dyn BackupsSource>>,
    pub page: Page,
    pub dialog: Option<Dialog>,
    /// The authenticator code being typed. Secure: wiped on drop, on close and after use.
    code: TextEdit,
    /// What is typed in Mail Backup's dialogs. The app password is its one secret.
    pub mail: mail::MailUi,
    intents: Vec<Intent>,
}

impl BackupsUi {
    /// Whether anything stands behind the section (`false`: the "not built yet" frame).
    #[cfg(test)]
    pub fn has_source(&self) -> bool {
        self.source.is_some()
    }

    pub fn new(source: Option<Box<dyn BackupsSource>>) -> BackupsUi {
        BackupsUi {
            source,
            page: Page::Section,
            dialog: None,
            code: TextEdit::new("").secure(),
            mail: mail::MailUi::default(),
            intents: Vec::new(),
        }
    }

    /// What the section starts on: the board's sample content where tooling runs, nothing
    /// in a build that ships.
    pub fn first(dev_build: bool) -> BackupsUi {
        BackupsUi::new(source::first(dev_build))
    }

    /// The board's sample PC (scenes and tests).
    pub fn fixture(source: FixtureSource) -> BackupsUi {
        BackupsUi::new(Some(Box::new(source)))
    }

    pub fn source(&self) -> Option<&dyn BackupsSource> {
        self.source.as_deref()
    }

    /// The window left the section: nothing stays open, the code is wiped.
    pub fn reset(&mut self) {
        self.page = Page::Section;
        self.close_dialog();
    }

    /// Show a page with nothing over it.
    pub fn show(&mut self, page: Page) {
        self.close_dialog();
        // A device that is gone, or a list with nothing in it, shows the section.
        let known = match (&page, self.source()) {
            (Page::Device(id), Some(source)) => source.devices().iter().any(|d| &d.id == id),
            // No page while nothing answers for Mail Backup.
            (Page::Mail, Some(source)) => source.mail().is_some(),
            (_, Some(_)) => true,
            (_, None) => false,
        };
        self.page = if known { page } else { Page::Section };
    }

    pub fn close_dialog(&mut self) {
        if matches!(self.dialog, Some(Dialog::MailAccount | Dialog::MailFolders))
            && let Some(source) = &mut self.source
        {
            source.mail_forget_connection();
        }
        self.dialog = None;
        self.code.clear();
        self.mail.clear();
    }

    /// What Backups asked of the shell since the last call, oldest first. (The shell lane
    /// takes them; until it does, only the tests call this.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    fn ask(&mut self, intent: Intent) {
        if self.intents.len() >= INTENT_LIMIT {
            self.intents.remove(0);
        }
        self.intents.push(intent);
    }

    pub fn back_up_now(&mut self) {
        if let Some(source) = &mut self.source {
            source.back_up_now();
        }
    }

    /// A switch of one of the six known folders.
    pub fn toggle_folder(&mut self, id: &str) {
        if let Some(source) = &mut self.source {
            let on = source.folders().iter().find(|f| f.id == id).map(|f| f.on);
            if let Some(on) = on {
                source.set_folder(id, !on);
            }
        }
    }

    pub fn add_folder(&mut self) {
        self.ask(Intent::PickFolder);
    }

    /// The folder picker's answer. `false`: Beebeeb's own folder, or one inside it, cannot
    /// be chosen; nothing changed.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn folder_picked(&mut self, path: &str) -> bool {
        self.source.as_mut().is_some_and(|source| source.add_folder(path))
    }

    /// Remove on a chosen folder: ask first (DW-1410).
    pub fn ask_stop_folder(&mut self, id: &str) {
        self.close_dialog();
        self.dialog = Some(Dialog::StopFolder(id.to_string()));
    }

    /// DW-1410's two answers. Nothing on Windows is touched either way.
    pub fn stop_folder(&mut self, move_copy_to_trash: bool) {
        if let (Some(Dialog::StopFolder(id)), Some(source)) = (self.dialog.clone(), &mut self.source) {
            source.remove_folder(&id, move_copy_to_trash);
        }
        self.close_dialog();
    }

    pub fn open_in_photos(&mut self, device: &str) {
        self.ask(Intent::OpenInPhotos(device.to_string()));
    }

    /// Opening is the one thing the protected root allows.
    pub fn open_in_files(&mut self, device: &str) {
        let allowed = self
            .source()
            .is_some_and(|source| source.root().allows(source::RootAction::Open));
        if allowed {
            self.ask(Intent::OpenInFiles(device.to_string()));
        }
    }

    /// The Mail Backup row: its page where a source answers for it, the shell otherwise.
    pub fn mail_backup(&mut self) {
        if self.source().is_some_and(|source| source.mail().is_some()) {
            self.show(Page::Mail);
        } else {
            self.ask(Intent::MailBackup);
        }
    }

    /// Turn on…: what Mail Backup does, first (DW-1401).
    pub fn mail_ask_turn_on(&mut self) {
        self.close_dialog();
        self.dialog = Some(Dialog::MailTurnOn);
    }

    /// "Add a mail account" with nothing typed (DW-1402).
    pub fn mail_add_account(&mut self) {
        self.close_dialog();
        self.dialog = Some(Dialog::MailAccount);
        self.mail.focus(Some(mail::Field::Email));
    }

    /// Update password (DW-1406): the account step with the address filled in, the password asked again.
    pub fn mail_update_password(&mut self) {
        let failing = self.source().and_then(|source| source.mail()).and_then(|m| {
            let failure = m.failure?;
            m.accounts.into_iter().find(|a| a.address == failure.address)
        });
        self.mail_add_account();
        if let Some(account) = failing {
            self.mail.email.replace_text(account.address);
            self.mail.server.replace_text(account.server);
            self.mail.focus(Some(mail::Field::Password));
        }
    }

    /// Connect, or Try again: hands what was typed to the source. Nothing is asked while a
    /// field is missing or an answer is still out. The dialog moves on only when the source
    /// says the server accepted ([`BackupsUi::mail_answered`]).
    pub fn mail_connect(&mut self) -> bool {
        if self.dialog != Some(Dialog::MailAccount) || !self.mail.complete() {
            return false;
        }
        let (Some(port), Some(source)) = (self.mail.port_number(), &mut self.source) else {
            return false;
        };
        if source.mail_connection() == mail::Connection::Checking {
            return false;
        }
        source.mail_connect(&mail::Entry {
            address: self.mail.email.text.trim(),
            server: self.mail.server.text.trim(),
            port,
            password: &self.mail.password.text,
        });
        self.mail_answered();
        true
    }

    /// The source may have an answer to the sign-in: on "accepted" the password is wiped and
    /// Choose folders shows (DW-1404). A refusal stays in the account step (DW-1403). The
    /// window's poll calls this for a source that answers late.
    pub fn mail_answered(&mut self) {
        if self.dialog != Some(Dialog::MailAccount) {
            return;
        }
        let answer = self.source().map(|source| source.mail_connection());
        if let Some(mail::Connection::Folders { folders, .. }) = answer {
            self.mail.password.clear();
            self.mail.focus(None);
            self.mail.picked = folders.iter().map(|f| f.suggested).collect();
            self.dialog = Some(Dialog::MailFolders);
        }
    }

    pub fn mail_toggle_folder(&mut self, index: usize) {
        if self.dialog == Some(Dialog::MailFolders)
            && let Some(picked) = self.mail.picked.get_mut(index)
        {
            *picked = !*picked;
        }
    }

    /// Start backup: the ticked folders, by name. `false`: nothing ticked, or the source did
    /// not start; the dialog stays.
    pub fn mail_start(&mut self) -> bool {
        if self.dialog != Some(Dialog::MailFolders) {
            return false;
        }
        let Some(mail::Connection::Folders { folders, .. }) = self.source().map(|source| source.mail_connection())
        else {
            return false;
        };
        let names: Vec<String> = folders
            .iter()
            .zip(&self.mail.picked)
            .filter(|(_, picked)| **picked)
            .map(|(folder, _)| folder.name.clone())
            .collect();
        if names.is_empty() {
            return false;
        }
        let started = self.source.as_mut().is_some_and(|source| source.mail_start(&names));
        if started {
            self.dialog = None;
            self.mail.clear();
        }
        started
    }

    /// Back, in Choose folders: the account step again. The sign-in is dropped; the password
    /// was wiped when the server accepted it, so it is asked again.
    pub fn mail_back(&mut self) {
        if self.dialog != Some(Dialog::MailFolders) {
            return;
        }
        if let Some(source) = &mut self.source {
            source.mail_forget_connection();
        }
        self.mail.picked.clear();
        self.dialog = Some(Dialog::MailAccount);
        self.mail.focus(Some(mail::Field::Password));
    }

    /// Pause, or carry on with, the first archive (DW-1405).
    pub fn mail_pause(&mut self, paused: bool) {
        if let Some(source) = &mut self.source {
            source.mail_pause(paused);
        }
    }

    /// Opening is the one thing the protected folder allows.
    pub fn mail_open_folder(&mut self) {
        let allowed = self
            .source()
            .is_some_and(|source| source.root().allows(source::RootAction::Open));
        if allowed {
            self.ask(Intent::OpenMailFolder);
        }
    }

    /// Turn off Mail Backup: a code first (DW-0743).
    pub fn mail_ask_turn_off(&mut self) {
        self.close_dialog();
        self.dialog = Some(Dialog::MailTurnOff);
    }

    /// "Turn off": off until six digits. The engine checks the code; one it does not accept
    /// empties the boxes and the dialog stays. Accepted: Mail Backup is off, and where it kept
    /// files the choice about them follows (DW-0743b).
    pub fn mail_turn_off(&mut self) -> bool {
        if self.dialog != Some(Dialog::MailTurnOff) || self.code_len() != CODE_LEN {
            return false;
        }
        let accepted = self
            .source
            .as_mut()
            .is_some_and(|source| source.mail_turn_off(&self.code.text));
        self.code.clear();
        if accepted {
            let kept = self
                .source()
                .and_then(|source| source.mail())
                .is_some_and(|m| m.used.is_some());
            self.dialog = kept.then_some(Dialog::MailKeepFiles);
        }
        accepted
    }

    /// DW-0743b's two answers. Keeping changes nothing. Moving asks the source; while it has
    /// not moved them the dialog stays.
    pub fn mail_keep_files(&mut self, keep: bool) -> bool {
        if self.dialog != Some(Dialog::MailKeepFiles) {
            return false;
        }
        let done = keep || self.source.as_mut().is_some_and(|source| source.mail_files_to_trash());
        if done {
            self.close_dialog();
        }
        done
    }

    /// Delete this backup: the warning with a code first (DW-0742).
    pub fn ask_delete_device(&mut self, id: &str) {
        self.close_dialog();
        self.dialog = Some(Dialog::DeleteDevice(id.to_string()));
    }

    /// How many digits are typed. (The digits themselves leave this type only to be drawn
    /// as secure text and to be checked by the engine.)
    pub fn code_len(&self) -> usize {
        self.code.text.chars().count()
    }

    pub(crate) fn code_digits(&self) -> &str {
        &self.code.text
    }

    /// Type one character into the code: digits only, six at most.
    pub fn type_code(&mut self, typed: &str) {
        for c in typed.chars().filter(char::is_ascii_digit) {
            if self.code_len() < CODE_LEN {
                let mut next = self.code.text.clone();
                next.push(c);
                self.code.replace_text(next);
            }
        }
    }

    pub fn erase_code_digit(&mut self) {
        let mut next = self.code.text.clone();
        if next.pop().is_some() {
            self.code.replace_text(next);
        }
    }

    /// "Delete backup": off until six digits. The engine checks the code; a code it does not
    /// accept empties the boxes and the dialog stays (the board draws no words for it).
    pub fn delete_device(&mut self) -> bool {
        let Some(Dialog::DeleteDevice(id)) = self.dialog.clone() else {
            return false;
        };
        if self.code_len() != CODE_LEN {
            return false;
        }
        let accepted = self
            .source
            .as_mut()
            .is_some_and(|source| source.delete_device_backup(&id, &self.code.text));
        self.code.clear();
        if accepted {
            self.dialog = None;
            self.page = Page::Section;
        }
        accepted
    }

    pub fn retry(&mut self, id: &str) {
        if let Some(source) = &mut self.source {
            source.retry(id);
        }
    }

    pub fn retry_all(&mut self) {
        if let Some(source) = &mut self.source {
            source.retry_all();
        }
    }
}

/// Whether the shell's route shows this section.
fn routed(s: &State) -> bool {
    s.shell.app.screen() == Screen::Settings(SettingsPane::Backups)
}

/// The shell calls this after every change of route (`shell::host::dispatch`): what was open
/// inside Backups is forgotten when the window leaves it, and the code is wiped.
pub fn sync(s: &mut State) {
    if !routed(s) {
        s.backups.reset();
    }
}

/// Backups' keyboard. Esc closes what is open, innermost first: the dialog, the page, then
/// settings. The code dialog takes digits, Backspace and Enter. Everything else is Rune's.
pub fn on_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    let ui = &mut s.backups;
    let m = ev.mods;
    let plain = !m.cmd && !m.ctrl && !m.alt && !m.logo;
    let code_dialog = matches!(ui.dialog, Some(Dialog::DeleteDevice(_) | Dialog::MailTurnOff));
    if code_dialog && plain {
        match &ev.key {
            Key::Text(typed) if typed.chars().all(|c| c.is_ascii_digit()) && !typed.is_empty() => {
                ui.type_code(typed);
                return true;
            }
            Key::Backspace => {
                ui.erase_code_digit();
                return true;
            }
            Key::Enter if ui.code_len() == CODE_LEN => {
                if ui.dialog == Some(Dialog::MailTurnOff) {
                    ui.mail_turn_off();
                } else {
                    ui.delete_device();
                }
                return true;
            }
            _ => {}
        }
    }
    match ev.key {
        Key::Escape if ui.dialog.is_some() => {
            ui.close_dialog();
            true
        }
        Key::Escape if ui.page != Page::Section => {
            ui.show(Page::Section);
            true
        }
        Key::Escape => {
            crate::frames::settings::go(s, "DW-0401");
            true
        }
        _ => false,
    }
}

fn open(s: &mut State, source: FixtureSource) {
    crate::frames::settings::go(s, "DW-0737");
    s.backups = BackupsUi::fixture(source);
}

fn dw_0737(s: &mut State) {
    open(s, FixtureSource::board());
}

fn dw_1411(s: &mut State) {
    open(s, FixtureSource::first_backup());
}

fn dw_1410(s: &mut State) {
    dw_0737(s);
    s.backups.ask_stop_folder("projects");
}

fn dw_0739(s: &mut State) {
    dw_0737(s);
    s.backups.show(Page::Device("ipad".into()));
}

/// The warning with the first two digits typed, as the board draws it.
fn dw_0742(s: &mut State) {
    dw_0739(s);
    s.backups.ask_delete_device("ipad");
    s.backups.type_code("51");
}

fn dw_0741(s: &mut State) {
    dw_0737(s);
    s.backups.show(Page::Failures);
}

fn mail_scene(s: &mut State, mail: mail::Mail) {
    open(s, FixtureSource::with_mail(mail));
    s.backups.show(Page::Mail);
}

fn dw_0740(s: &mut State) {
    mail_scene(s, mail::board::on());
}

fn dw_0740b(s: &mut State) {
    mail_scene(s, mail::board::off());
}

fn dw_1401(s: &mut State) {
    dw_0740b(s);
    s.backups.mail_ask_turn_on();
}

/// The account step filled in as the board draws it, the caret after the password.
fn dw_1402(s: &mut State) {
    dw_0740b(s);
    s.backups.mail_add_account();
    let typed = &mut s.backups.mail;
    typed.email.replace_text(mail::board::ADDRESS);
    typed.server.replace_text(mail::board::SERVER);
    typed.password.replace_text(mail::board::APP_PASSWORD);
    typed.focus(Some(mail::Field::Password));
}

/// The same step after the server refused a password of the same length.
fn dw_1403(s: &mut State) {
    dw_1402(s);
    let wrong = "0".repeat(mail::board::APP_PASSWORD.len());
    s.backups.mail.password.replace_text(wrong);
    s.backups.mail_connect();
}

fn dw_1404(s: &mut State) {
    dw_1402(s);
    s.backups.mail_connect();
}

fn dw_1405(s: &mut State) {
    mail_scene(s, mail::board::first());
}

fn dw_1406(s: &mut State) {
    mail_scene(s, mail::board::failed());
}

/// The code step with all six digits typed, as the board draws it.
fn dw_0743(s: &mut State) {
    dw_0740(s);
    s.backups.mail_ask_turn_off();
    s.backups.type_code(mail::board::CODE);
}

fn dw_0743b(s: &mut State) {
    dw_0743(s);
    s.backups.mail_turn_off();
}

/// The group's board frames, by frame id (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] = &[
    ("DW-0740", dw_0740),
    ("DW-0740b", dw_0740b),
    ("DW-1401", dw_1401),
    ("DW-1402", dw_1402),
    ("DW-1403", dw_1403),
    ("DW-1404", dw_1404),
    ("DW-1405", dw_1405),
    ("DW-1406", dw_1406),
    ("DW-0743", dw_0743),
    ("DW-0743b", dw_0743b),
    ("DW-0737", dw_0737),
    ("DW-1411", dw_1411),
    ("DW-1410", dw_1410),
    ("DW-0739", dw_0739),
    ("DW-0742", dw_0742),
    ("DW-0741", dw_0741),
];
