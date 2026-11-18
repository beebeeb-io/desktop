//! The first run on Windows (DW-0201 to DW-0209): the seven setup steps between sign-in and the
//! main window, "You're set up", and the compare view over step 4. Drawn in the first-run
//! window (960 x 640, `surfaces::WindowKind::FirstRun`) with ONE caption bar and no menu bar.
//!
//! [`Ui`] is the window's state; every control changes it through one of its methods, which
//! call the engine through [`source::SetupSource`]. What leaves the group (the main window, a
//! settings pane) is an [`Intent`] the shell takes with [`Ui::take_intents`].
//!
//! One secret is typed here: step 1's account password ([`Ui::passkey_password`], a secure field), which "Add a
//! passkey" hands to the engine once (owner ruling: a passkey change proves the password). It is wiped from the field
//! right after that call and whenever step 1 is left; it is never shown, logged or kept. No code or phrase is shown or
//! typed in these steps. The first-run window counts as a window that shows a secret (`shell::registry`).

mod folder;
pub mod guide;
pub mod guide_marks;
pub mod icons;
mod passkey;
pub mod source;
mod view;

#[cfg(test)]
mod guide_tests;
#[cfg(test)]
mod tests;

use rune::{Cx, El, Key, KeyEvent, Pixels, Tx};

use crate::State;
use crate::frames::account::source::Password;
use crate::shell::app::{Event, GateState};
use crate::shell::host;
use source::{
    Explorer, FixtureSetupSource, Folder, FolderAsk, LockAfter, Outcome, PasskeyAnswer, PasskeyRefused, PreviewFormat,
    PreviewSize, Sample, Script, SetupSource, Switch,
};

pub use view::view;

/// The screen's title (`ui::au::heading`).
pub const TITLE_KEY: &str = "title";
/// The compare view's Done button: focus goes there when the view opens.
pub const COMPARE_DONE_KEY: &str = "cmp-done";
/// "Compare" on step 4: focus returns there when the view closes.
pub const COMPARE_KEY: &str = "btn-0";
/// Step 1's password field (its text input): focus goes there when the password is missing or wrong.
pub const PASSWORD_KEY: &str = "passkey-password";

/// The compare view's sheet, on the board: 700 x 520 inside the 960 x 640 window.
pub const SHEET: (f32, f32) = (700.0, 520.0);
/// The photo in the compare view.
pub const PHOTO: (f32, f32) = (560.0, 330.0);
/// How far an arrow key moves the wipe line.
pub(super) const WIPE_STEP: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    /// DW-0201.
    Passkey,
    /// DW-0202.
    Lock,
    /// DW-0203.
    Backup,
    /// DW-0204 (and DW-0209 over it).
    Previews,
    /// DW-0206, DW-0206b, DW-0280.
    Explorer,
    /// DW-0207.
    Notifications,
    /// DW-0281.
    Login,
    /// DW-0208.
    Done,
}

impl Step {
    pub const STEPS: [Step; 7] = [
        Step::Passkey,
        Step::Lock,
        Step::Backup,
        Step::Previews,
        Step::Explorer,
        Step::Notifications,
        Step::Login,
    ];

    /// The step's name in the list at the left.
    pub fn label(self) -> &'static str {
        match self {
            Step::Passkey => "Add a passkey",
            Step::Lock => "Lock Beebeeb",
            Step::Backup => "Back up your folders",
            Step::Previews => "Thumbnails and previews",
            Step::Explorer => "Beebeeb in File Explorer",
            Step::Notifications => "Notifications",
            Step::Login => "Open at login",
            Step::Done => "You're set up",
        }
    }
}

/// How the compare view shows the two pictures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareMode {
    Stacked,
    Wipe,
    Flip,
}

impl CompareMode {
    pub const ALL: [CompareMode; 3] = [CompareMode::Stacked, CompareMode::Wipe, CompareMode::Flip];

    pub fn label(self) -> &'static str {
        match self {
            CompareMode::Stacked => "Stacked",
            CompareMode::Wipe => "Wipe",
            CompareMode::Flip => "Hold to flip",
        }
    }
}

/// The open compare view (DW-0209).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Compare {
    pub mode: CompareMode,
    /// Where the wipe line stands, 0 (all preview) to 1 (all original).
    pub wipe: f32,
    /// Flip mode: the original is showing.
    pub original_showing: bool,
}

/// A row of "You're set up": the setting it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    /// DW-A73.
    Passkey,
    /// DW-A52.
    AppLock,
    /// DW-0737.
    Backup,
    /// DW-0744.
    Previews,
    /// DW-0746.
    FileExplorer,
    /// DW-0750.
    Notifications,
    /// DW-0746.
    OpenAtLogin,
}

impl Setting {
    /// The step that sets it.
    pub fn step(self) -> Step {
        match self {
            Setting::Passkey => Step::Passkey,
            Setting::AppLock => Step::Lock,
            Setting::Backup => Step::Backup,
            Setting::Previews => Step::Previews,
            Setting::FileExplorer => Step::Explorer,
            Setting::Notifications => Step::Notifications,
            Setting::OpenAtLogin => Step::Login,
        }
    }
}

/// Where a switch of "You're set up" stands when its change is answered after the press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answered {
    /// Never asked of a source that answers later ("Skip setup", a step that was left with nothing chosen):
    /// nothing was set here, and no value is claimed.
    NotAsked,
    /// Asked, not answered yet: no value is drawn.
    Waiting,
    /// The change did not happen, and what the switch is now is known (`results` holds it).
    NotChanged,
    /// The change did not happen, and what the switch is now is not known.
    NotKnown,
}

/// Step 1 between "Add a passkey" and its answer. The add runs while Windows' passkey dialog is open, off the
/// window's thread: the window is never held for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PasskeyStep {
    /// Nothing asked, or the passkey was added (`Results::passkey`).
    #[default]
    Ready,
    /// Asked, not answered: Windows' passkey dialog is open, or Beebeeb is being asked. "Add a passkey" takes no
    /// second press; "Not now" and "Skip setup" stay.
    Waiting,
    /// Not added, and why: step 1 stays, says it in one line, and can still be skipped.
    Refused(PasskeyRefused),
}

/// Where the person goes when they leave setup. The shell opens it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// "Open my Beebeeb": the main window on Files, then the guide (DW-0380). Setup closes.
    OpenDrive,
    /// A row of "You're set up".
    OpenSetting(Setting),
}

/// What setup turned on, for "You're set up". A step that was skipped keeps the product's own
/// default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Results {
    pub passkey: bool,
    pub app_lock: bool,
    pub backup: Vec<String>,
    pub notifications: bool,
    pub open_at_login: bool,
}

pub struct Ui {
    source: Box<dyn SetupSource>,
    pub step: Step,
    pub compare: Option<Compare>,
    pub explorer: Explorer,
    pub lock_on: bool,
    pub lock_after: LockAfter,
    pub folders: Vec<Folder>,
    pub size: PreviewSize,
    pub format: PreviewFormat,
    pub login_on: bool,
    pub results: Results,
    sample: Sample,
    /// The decoded pictures of `sample`.
    original: Pixels,
    preview: Pixels,
    focus: Option<&'static str>,
    intents: Vec<Intent>,
    /// "Open my Beebeeb" was pressed and the guide has not asked yet ([`Ui::guide_due`]).
    finished: bool,
    /// The sync folder, while it still has to be asked for (the first state of the File Explorer step).
    pub folder: FolderAsk,
    /// A start worked and its folder could not be saved, while sync runs (`SetupSource::folder_unsaved`).
    unsaved: bool,
    /// Switches whose change is answered after the press, and where each stands. One that is not listed here was
    /// made (or never asked): `results` says what it is.
    answered: Vec<(Switch, Answered)>,
    /// The switches that were asked of the source at all.
    asked: Vec<Switch>,
    /// A size or format was chosen on step 4: the stored one no longer moves it.
    previews_chosen: bool,
    /// The account password step 1 hands to "Add a passkey" (owner ruling: a passkey change proves the password).
    /// Secure; drawn by `view.rs`. Wiped right after it is handed over, and when step 1 is left.
    pub passkey_password: rune::TextEdit,
    /// Step 1 between its press and its answer.
    pub passkey_step: PasskeyStep,
}

/// Decode a PNG into pixels Rune draws. A file that does not decode draws nothing.
fn decode(png_bytes: &[u8]) -> Pixels {
    let pixels = Pixels::new();
    let Ok(mut reader) = png::Decoder::new(std::io::Cursor::new(png_bytes)).read_info() else {
        return pixels;
    };
    let Some(size) = reader.output_buffer_size() else {
        return pixels;
    };
    let mut buf = vec![0; size];
    let Ok(info) = reader.next_frame(&mut buf) else {
        return pixels;
    };
    let bytes = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => bytes.to_vec(),
        (png::ColorType::Rgb, png::BitDepth::Eight) => {
            bytes.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
        }
        _ => return pixels,
    };
    pixels.set(info.width, info.height, rgba);
    pixels
}

impl Ui {
    pub fn new(source: Box<dyn SetupSource>) -> Self {
        let sample = source.sample();
        let mut ui = Ui {
            step: Step::Passkey,
            folder: source.folder_ask(),
            unsaved: source.folder_unsaved(),
            answered: Vec::new(),
            asked: Vec::new(),
            compare: None,
            explorer: source.explorer(),
            // Step 2 draws the switch on and "5 min" chosen; step 7 draws its switch on.
            lock_on: true,
            lock_after: LockAfter::Min5,
            folders: source.folders(),
            size: PreviewSize::K4,
            format: PreviewFormat::Heic,
            login_on: true,
            results: Results {
                passkey: false,
                app_lock: false,
                backup: Vec::new(),
                // Windows shows an app's notifications until they are turned off (DW-0207's rule).
                notifications: true,
                open_at_login: true,
            },
            original: decode(&sample.original_png),
            preview: decode(&sample.preview_png),
            sample,
            source,
            focus: None,
            intents: Vec::new(),
            finished: false,
            previews_chosen: false,
            passkey_password: rune::TextEdit::new("").secure(),
            passkey_step: PasskeyStep::Ready,
        };
        ui.start_on_stored_previews();
        ui
    }

    /// What the shell should open, oldest first.
    // The shell reads none yet: the first run ends through the gate (`SetupSource::finish`), and the running
    // app's rows of "You're set up" open no setting (`SetupSource::opens_settings`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    pub fn sample(&self) -> &Sample {
        &self.sample
    }

    pub fn pictures(&self) -> (&Pixels, &Pixels) {
        (&self.original, &self.preview)
    }

    /// The weight of a preview at this size and format, where the engine has measured it.
    pub fn weight(&self, size: PreviewSize, format: PreviewFormat) -> Option<String> {
        self.source.weight(size, format)
    }

    pub fn original_weight(&self) -> String {
        self.source.original_weight()
    }

    pub fn fit_line(&self) -> String {
        self.source.fit_line(self.size, self.format)
    }

    pub fn space_limit(&self) -> String {
        self.source.space_limit()
    }

    /// "4K HEIC".
    pub fn preview_name(&self) -> String {
        format!("{} {}", self.size.label(), self.format.label())
    }

    /// Whether something answers for this step. One that nothing answers for says so, and turns nothing on.
    pub fn available(&self, step: Step) -> bool {
        self.source.available(step)
    }

    /// Whether this step's content is still being read (see `SetupSource::loading`).
    pub fn loading(&self, step: Step) -> bool {
        self.source.loading(step)
    }

    /// Whether this step's read failed (see `SetupSource::failed`).
    pub fn failed(&self, step: Step) -> bool {
        self.source.failed(step)
    }

    /// "Try again" on step 3 after a failed read.
    pub fn retry_backup(&mut self) {
        if self.step == Step::Backup && self.failed(Step::Backup) {
            self.source.retry_backups();
        }
    }

    /// "Try again" on step 4 after a failed read of the preview setting.
    pub fn retry_previews(&mut self) {
        if self.step == Step::Previews && self.failed(Step::Previews) {
            self.source.retry_previews();
        }
    }

    /// Whether the File Explorer step is still asking for the sync folder.
    pub fn asks_folder(&self) -> bool {
        self.available(Step::Explorer) && self.folder != FolderAsk::Settled
    }

    /// Whether the folder a start worked in could not be saved, while sync runs.
    pub fn folder_unsaved(&self) -> bool {
        self.unsaved
    }

    /// Whether another folder can be chosen after a start that was refused.
    pub fn can_choose_again(&self) -> bool {
        self.source.can_choose_again()
    }

    /// Whether a row of "You're set up" leads to its setting.
    pub fn opens_settings(&self) -> bool {
        self.source.opens_settings()
    }

    /// Where a switch stands while its change has not been made: `None` once it was (or when it was never asked).
    pub fn answered(&self, switch: Switch) -> Option<Answered> {
        if self.source.answers_later() && !self.asked.contains(&switch) {
            return Some(Answered::NotAsked);
        }
        self.answered.iter().find(|(s, _)| *s == switch).map(|(_, a)| *a)
    }

    /// A switch was asked of the source. A source that answers later: no value is drawn until it does.
    fn asked(&mut self, switch: Switch) {
        if !self.asked.contains(&switch) {
            self.asked.push(switch);
        }
        self.answered.retain(|(s, _)| *s != switch);
        if self.source.answers_later() {
            self.answered.push((switch, Answered::Waiting));
        }
    }

    /// The source answered a change: the value it names is the one drawn, never the one that was asked for.
    fn take_in(&mut self, outcome: Outcome) {
        self.answered.retain(|(s, _)| *s != outcome.switch);
        if let Some(now) = outcome.now {
            match outcome.switch {
                Switch::Notifications => self.results.notifications = now,
                Switch::OpenAtLogin => self.results.open_at_login = now,
                Switch::AppLock => self.results.app_lock = now,
                Switch::Backup | Switch::Previews | Switch::LockAfter => {}
            }
        }
        match (outcome.done, outcome.now) {
            // Made (when what is set could not be read back, the value that was asked for stands).
            (true, _) => {}
            (false, None) => self.answered.push((outcome.switch, Answered::NotKnown)),
            (false, Some(_)) => self.answered.push((outcome.switch, Answered::NotChanged)),
        }
    }

    fn advance(&mut self) {
        self.leave_passkey_step();
        self.step = Step::STEPS
            .into_iter()
            .find(|step| *step > self.step)
            .unwrap_or(Step::Done);
        self.focus = Some(TITLE_KEY);
    }

    /// "Use this folder" (the File Explorer step, while it asks for the sync folder).
    pub fn keep_folder(&mut self) {
        if self.step == Step::Explorer && self.asks_folder() {
            self.source.keep_folder();
            self.focus = Some(TITLE_KEY);
        }
    }

    /// "Choose another folder": while the step asks for the sync folder, and after a start that was refused.
    pub fn choose_folder(&mut self) {
        if self.step == Step::Explorer && (self.asks_folder() || self.can_choose_again()) {
            self.source.choose_folder();
            self.focus = Some(TITLE_KEY);
        }
    }

    /// "Add a passkey": the password goes to the engine once (core's step-up), then Windows' own dialog opens. The
    /// answer comes later ([`Ui::pump`]): step 2 follows only once the passkey was added; a refusal stays on step 1
    /// with its line. The source refuses an empty password itself (nothing is sent), so the board's fixture, which
    /// asks for none, still walks its frames.
    pub fn add_passkey(&mut self) {
        if self.step != Step::Passkey || self.passkey_step == PasskeyStep::Waiting {
            return;
        }
        if !self.available(Step::Passkey) {
            // Nothing answers for a passkey: the step is left, and nothing is claimed.
            return self.advance();
        }
        let started = self
            .source
            .add_passkey(Password::from_field(&self.passkey_password.text));
        // The password has left the field once it is handed over: wiped, never kept.
        self.passkey_password.clear();
        if started {
            self.passkey_step = PasskeyStep::Waiting;
        }
        // An answer that is already there (a refusal said at the press, or the board's fixture) is taken in now.
        self.take_in_passkey();
    }

    /// The answer of "Add a passkey", once it is there. Added: said on "You're set up" whatever step shows by then,
    /// and step 2 follows only from step 1 (a person who went on is not moved). Refused: step 1 says why.
    fn take_in_passkey(&mut self) {
        let Some(answer) = self.source.take_passkey_answer() else {
            return;
        };
        match answer {
            PasskeyAnswer::Added => {
                self.results.passkey = true;
                self.passkey_step = PasskeyStep::Ready;
                if self.step == Step::Passkey {
                    self.advance();
                }
            }
            PasskeyAnswer::Refused(why) => {
                self.passkey_step = PasskeyStep::Refused(why);
                if self.step == Step::Passkey {
                    self.focus = Some(match why {
                        PasskeyRefused::NoPassword | PasskeyRefused::WrongPassword => PASSWORD_KEY,
                        _ => TITLE_KEY,
                    });
                }
            }
        }
    }

    /// Step 1 is left (its answer may still come): what was typed into its password field goes.
    fn leave_passkey_step(&mut self) {
        if self.step == Step::Passkey {
            self.passkey_password.clear();
        }
    }

    /// The amber button of steps 2 to 7.
    pub fn go_on(&mut self) {
        if self.compare.is_some() {
            return;
        }
        if self.step != Step::Done && !self.available(self.step) {
            // "Continue" on a step nothing answers for: the next step, nothing asked, nothing claimed.
            return self.advance();
        }
        match self.step {
            Step::Passkey | Step::Done => return,
            Step::Lock => {
                self.source.set_app_lock(self.lock_on, self.lock_after);
                self.results.app_lock = self.lock_on;
                self.asked(Switch::AppLock);
                // The delay goes with the switch on only (off never locks).
                if self.lock_on && self.sets_lock_after() {
                    self.asked(Switch::LockAfter);
                }
            }
            Step::Backup => {
                let on: Vec<String> = self.folders.iter().filter(|f| f.on).map(|f| f.name.clone()).collect();
                if on.is_empty() {
                    return;
                }
                self.source.start_backup(&on);
                self.results.backup = on;
                self.asked(Switch::Backup);
            }
            // Nothing is stored before the account's own setting was read: Continue waits (loading), and a read
            // that failed offers Try again instead.
            Step::Previews if self.loading(Step::Previews) || self.failed(Step::Previews) => return,
            Step::Previews => {
                self.source.set_previews(self.size, self.format);
                self.asked(Switch::Previews);
            }
            Step::Explorer => {
                // "OK" on the note that the folder cannot be moved yet.
                if self.folder == FolderAsk::AlreadySyncs {
                    self.source.dismiss_folder_note();
                    self.pump();
                    self.focus = Some(TITLE_KEY);
                    return;
                }
                // "Use this folder" is the amber button while the step asks for the sync folder.
                if self.asks_folder() {
                    return self.keep_folder();
                }
                // "Try again" on a folder that was not saved: write it.
                if self.unsaved {
                    self.source.save_folder();
                    self.pump();
                    self.focus = Some(TITLE_KEY);
                    return;
                }
                // "Try again" is the amber button while it failed (DW-0280).
                if matches!(self.explorer, Explorer::Failed { .. } | Explorer::NotSyncing) {
                    self.source.retry_explorer();
                    self.pump();
                    self.focus = Some(TITLE_KEY);
                    return;
                }
            }
            Step::Notifications => {
                self.source.set_notifications(true);
                self.results.notifications = true;
                self.asked(Switch::Notifications);
            }
            Step::Login => {
                self.source.set_open_at_login(self.login_on);
                self.results.open_at_login = self.login_on;
                self.asked(Switch::OpenAtLogin);
            }
        }
        self.advance();
    }

    /// "Not now" (and "Continue without File Explorer"): the next step, nothing turned on.
    pub fn not_now(&mut self) {
        if self.compare.is_some() {
            return;
        }
        match self.step {
            Step::Done => return,
            // The sync folder is not skipped from its own question ("Skip setup" leaves the first run).
            Step::Explorer if self.asks_folder() => return,
            // Works like "Continue" with the switch off: Windows Hello is asked first, and a prompt that did not
            // confirm leaves app lock as it was ("Not changed", never "Off").
            Step::Lock if self.available(Step::Lock) => self.ask_lock_off(),
            Step::Notifications if self.available(Step::Notifications) => {
                self.source.set_notifications(false);
                self.results.notifications = false;
                self.asked(Switch::Notifications);
            }
            _ => {}
        }
        self.advance();
    }

    /// "Skip setup": ends setup with the defaults.
    pub fn skip(&mut self) {
        if self.compare.is_some() || self.step == Step::Done {
            return;
        }
        // "Skip setup" from the Lock step is "Not now" there (the switch off, asked like "Continue").
        if self.step == Step::Lock && self.available(Step::Lock) {
            self.ask_lock_off();
        }
        self.leave_passkey_step();
        self.step = Step::Done;
        self.focus = Some(TITLE_KEY);
    }

    /// Step 2 with the switch off, whatever the switch shows: the source asks Windows Hello first.
    fn ask_lock_off(&mut self) {
        self.source.set_app_lock(false, self.lock_after);
        self.results.app_lock = false;
        self.asked(Switch::AppLock);
    }

    pub fn toggle_lock(&mut self) {
        self.lock_on = !self.lock_on;
    }

    /// Whether step 2 draws "Lock again after" (`SetupSource::sets_lock_after`).
    pub fn sets_lock_after(&self) -> bool {
        self.source.sets_lock_after()
    }

    pub fn set_lock_after(&mut self, after: LockAfter) {
        self.lock_after = after;
    }

    pub fn toggle_folder(&mut self, index: usize) {
        if let Some(folder) = self.folders.get_mut(index) {
            folder.on = !folder.on;
        }
    }

    pub fn toggle_login(&mut self) {
        self.login_on = !self.login_on;
    }

    pub fn set_size(&mut self, size: PreviewSize) {
        self.size = size;
        self.previews_chosen = true;
    }

    pub fn set_format(&mut self, format: PreviewFormat) {
        self.format = format;
        self.previews_chosen = true;
    }

    /// Step 4 starts on the size and format the account has stored, once the source read them, until the person
    /// chose one: "Continue" on untouched choices then changes nothing but turning previews on.
    fn start_on_stored_previews(&mut self) {
        if self.previews_chosen {
            return;
        }
        if let Some((size, format)) = self.source.stored_previews() {
            self.size = size;
            self.format = format;
        }
    }

    /// "Compare" on step 4.
    pub fn open_compare(&mut self) {
        if self.step == Step::Previews && self.compare.is_none() {
            self.compare = Some(Compare {
                mode: CompareMode::Wipe,
                wipe: 0.5,
                original_showing: false,
            });
            self.focus = Some(COMPARE_DONE_KEY);
        }
    }

    /// "Done" and Esc: back to step 4 with the choice set.
    pub fn close_compare(&mut self) {
        if self.compare.take().is_some() {
            self.focus = Some(COMPARE_KEY);
        }
    }

    pub fn set_compare_mode(&mut self, mode: CompareMode) {
        if let Some(compare) = &mut self.compare {
            compare.mode = mode;
            compare.original_showing = false;
        }
    }

    /// Move the wipe line by `by` of the photo's width.
    pub fn move_wipe(&mut self, by: f32) {
        if let Some(compare) = &mut self.compare
            && compare.mode == CompareMode::Wipe
        {
            compare.wipe = (compare.wipe + by).clamp(0.0, 1.0);
        }
    }

    /// Flip mode: show the other picture.
    pub fn flip(&mut self) {
        if let Some(compare) = &mut self.compare
            && compare.mode == CompareMode::Flip
        {
            compare.original_showing = !compare.original_showing;
        }
    }

    /// "Choose photo": another of the person's photos, through the engine's picker.
    pub fn choose_photo(&mut self) {
        if self.compare.is_some() {
            self.source.choose_photo();
            self.pump();
        }
    }

    pub fn show_in_explorer(&mut self) {
        if self.step == Step::Explorer && self.explorer == Explorer::Added {
            self.source.show_in_explorer();
        }
    }

    /// A row of "You're set up".
    pub fn open_setting(&mut self, setting: Setting) {
        if self.step == Step::Done && self.opens_settings() && self.available(setting.step()) {
            self.intents.push(Intent::OpenSetting(setting));
        }
    }

    /// "Open my Beebeeb".
    pub fn finish(&mut self) {
        if self.step == Step::Done {
            self.source.finish();
            self.intents.push(Intent::OpenDrive);
            self.finished = true;
        }
    }

    /// Once after "Open my Beebeeb": the first run has just ended. For `guide::poll`, which then asks whether the
    /// signed-in account has done the first-time guide (`guide_marks`).
    pub fn just_finished(&mut self) -> bool {
        std::mem::take(&mut self.finished)
    }

    /// Read again what the engine changed on its own. `true`: something did.
    pub fn pump(&mut self) -> bool {
        if !self.source.poll() {
            return false;
        }
        for outcome in self.source.take_outcomes() {
            self.take_in(outcome);
        }
        self.take_in_passkey();
        if let Some(folders) = self.source.fresh_folders() {
            self.folders = folders;
        }
        self.explorer = self.source.explorer();
        self.folder = self.source.folder_ask();
        self.unsaved = self.source.folder_unsaved();
        self.start_on_stored_previews();
        let sample = self.source.sample();
        if sample != self.sample {
            self.original = decode(&sample.original_png);
            self.preview = decode(&sample.preview_png);
            self.sample = sample;
        }
        true
    }

    fn settle(&mut self, tx: &mut Tx) {
        if let Some(key) = self.focus.take() {
            tx.focus(key, tx.focus_visible());
        }
    }
}

fn ui(s: &mut State) -> Option<&mut Ui> {
    s.setup.as_mut()
}

/// A control's handler: `f` on the setup state, then what the engine changed, then the focus
/// the change asked for.
pub fn act(f: impl Fn(&mut Ui) + 'static) -> impl Fn(&mut State, &mut Tx) + 'static {
    move |s: &mut State, tx: &mut Tx| {
        if let Some(ui) = ui(s) {
            f(ui);
            ui.pump();
            ui.settle(tx);
        }
    }
}

/// The registry's line for `FrameGroup::FirstRun`. The shell makes the setup state when its gate turns to the
/// first run (`shell::host::start_first_run`); with none (a build whose engine has no first run) the shell's
/// "not built yet" frame shows, never sample values.
pub fn frame(s: &State, cx: &mut Cx<State>) -> El {
    let name = s.identity.product_name();
    match &s.setup {
        Some(ui) => view(ui, name, cx),
        None => crate::shell::placeholder::window(s, cx, crate::shell::registry::FrameGroup::FirstRun),
    }
}

/// What the engine changed between presses (File Explorer finished). For `shell::host::poll`.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let Some(ui) = ui(s) else { return false };
    let changed = ui.pump();
    ui.settle(tx);
    changed
}

/// Esc closes the compare view. In it the photo is a slider: with focus on it the arrow keys
/// move the wipe line (`view::photo`: Rune hands a focused slider its arrows).
/// Return is Rune's: it presses the step's amber button (its default button).
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(ui) = ui(s) else { return false };
    if ui.compare.is_none() {
        return false;
    }
    let handled = ev.key == Key::Escape;
    if handled {
        ui.close_compare();
    }
    ui.settle(tx);
    handled
}

// ---------------------------------------------------------------------------
// Scenes: each board frame, reached through the same calls a person's presses make
// ---------------------------------------------------------------------------

fn at(step: Step, explorer: Explorer) -> Ui {
    let mut ui = Ui::new(Box::new(FixtureSetupSource::new(Script {
        explorer,
        ..Script::default()
    })));
    // The board's steps before this one were all done with their amber button.
    while ui.step < step {
        if ui.step == Step::Passkey {
            ui.add_passkey();
        } else {
            ui.go_on();
        }
    }
    ui.focus = None;
    ui
}

/// The state of one board frame, by its id. `None`: not a frame of this group.
pub fn scene(frame: &str) -> Option<Ui> {
    let failed = || Explorer::Failed {
        code: "0x8007017C".into(),
    };
    Some(match frame {
        "DW-0201" => at(Step::Passkey, Explorer::Adding),
        "DW-0202" => at(Step::Lock, Explorer::Adding),
        "DW-0203" => at(Step::Backup, Explorer::Adding),
        "DW-0204" => at(Step::Previews, Explorer::Adding),
        "DW-0209" => {
            let mut ui = at(Step::Previews, Explorer::Adding);
            ui.open_compare();
            ui.focus = None;
            ui
        }
        "DW-0206" => at(Step::Explorer, Explorer::Added),
        "DW-0206b" => at(Step::Explorer, Explorer::Adding),
        "DW-0280" => at(Step::Explorer, failed()),
        "DW-0207" => at(Step::Notifications, Explorer::Added),
        "DW-0281" => at(Step::Login, Explorer::Added),
        "DW-0208" => at(Step::Done, Explorer::Added),
        _ => return None,
    })
}

/// The steps the renders below draw as "not in this version", to show that state. (The running app answers step 1 on
/// Windows now, `engine_host::setup_passkey`; it stays listed so the renders keep showing the state, and the walk
/// below passes it with Continue.)
const NOT_IN_THIS_VERSION: &[Step] = &[Step::Passkey, Step::Backup, Step::Previews];

/// The states of the first run the board has no frame for, by a name of their own, for a headless render
/// (`--scene first-run-folder`): no frame gate, the lane looks at the render. `None`: not one of them.
pub fn render_scene(name: &str) -> Option<Ui> {
    let asks = |said| source::FolderAsk::Ask {
        folder: r"C:\Users\Sam\Beebeeb".into(),
        said,
        busy: false,
    };
    let script = match name {
        "first-run-not-yet" => Script::default(),
        "first-run-folder" => Script {
            folder: asks(None),
            ..Script::default()
        },
        "first-run-folder-refused" => Script {
            folder: asks(Some(source::FolderSaid::Network)),
            ..Script::default()
        },
        "first-run-sync-off" => Script {
            explorer: Explorer::NotSyncing,
            choose_again: true,
            ..Script::default()
        },
        "first-run-summary" => Script {
            explorer: Explorer::Added,
            ..Script::default()
        },
        _ => return None,
    };
    let mut ui = Ui::new(Box::new(FixtureSetupSource::new(Script {
        not_available: NOT_IN_THIS_VERSION,
        ..script
    })));
    let to = match name {
        "first-run-not-yet" => Step::Passkey,
        "first-run-summary" => Step::Done,
        _ => Step::Explorer,
    };
    // At most one press per step: a step that does not move on is left as it is.
    for _ in Step::STEPS {
        if ui.step < to {
            ui.go_on();
        }
    }
    ui.focus = None;
    Some(ui)
}

/// The names [`render_scene`] knows, for the shell's list of scenes.
pub const RENDER_SCENES: &[(&str, fn(&mut State))] = &[
    ("first-run-not-yet", |s| set_render_scene(s, "first-run-not-yet")),
    ("first-run-folder", |s| set_render_scene(s, "first-run-folder")),
    ("first-run-folder-refused", |s| {
        set_render_scene(s, "first-run-folder-refused")
    }),
    ("first-run-sync-off", |s| set_render_scene(s, "first-run-sync-off")),
    ("first-run-summary", |s| set_render_scene(s, "first-run-summary")),
];

fn set_render_scene(s: &mut State, name: &str) {
    host::dispatch(s, Event::Gate(GateState::FirstRun));
    s.setup = render_scene(name);
}

/// Every board frame of the first run, for `frames::scenes`.
pub const FRAMES: &[(&str, fn(&mut State))] = &[
    ("DW-0201", |s| set_scene(s, "DW-0201")),
    ("DW-0202", |s| set_scene(s, "DW-0202")),
    ("DW-0203", |s| set_scene(s, "DW-0203")),
    ("DW-0204", |s| set_scene(s, "DW-0204")),
    ("DW-0206", |s| set_scene(s, "DW-0206")),
    ("DW-0206b", |s| set_scene(s, "DW-0206b")),
    ("DW-0280", |s| set_scene(s, "DW-0280")),
    ("DW-0207", |s| set_scene(s, "DW-0207")),
    ("DW-0281", |s| set_scene(s, "DW-0281")),
    ("DW-0208", |s| set_scene(s, "DW-0208")),
    ("DW-0209", |s| set_scene(s, "DW-0209")),
];

/// Put the window in a board frame's state: the first-run gate state, and the frame's own.
pub fn set_scene(s: &mut State, frame: &str) {
    host::dispatch(s, Event::Gate(GateState::FirstRun));
    s.setup = scene(frame);
}
