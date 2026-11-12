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
}

/// A dialog over a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialog {
    /// "Stop backing up <folder>?" for the chosen folder of this id (DW-1410).
    StopFolder(String),
    /// "Delete this backup?" for the device of this id, with the code (DW-0742).
    DeleteDevice(String),
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
    /// Mail Backup (DW-0740: another lane's).
    MailBackup,
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
    intents: Vec<Intent>,
}

impl BackupsUi {
    pub fn new(source: Option<Box<dyn BackupsSource>>) -> BackupsUi {
        BackupsUi {
            source,
            page: Page::Section,
            dialog: None,
            code: TextEdit::new("").secure(),
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
            (_, Some(_)) => true,
            (_, None) => false,
        };
        self.page = if known { page } else { Page::Section };
    }

    pub fn close_dialog(&mut self) {
        self.dialog = None;
        self.code.clear();
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

    pub fn mail_backup(&mut self) {
        self.ask(Intent::MailBackup);
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
    if matches!(ui.dialog, Some(Dialog::DeleteDevice(_))) && plain {
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
                ui.delete_device();
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

/// The group's board frames, by frame id (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] = &[
    ("DW-0737", dw_0737),
    ("DW-1411", dw_1411),
    ("DW-1410", dw_1410),
    ("DW-0739", dw_0739),
    ("DW-0742", dw_0742),
    ("DW-0741", dw_0741),
];
