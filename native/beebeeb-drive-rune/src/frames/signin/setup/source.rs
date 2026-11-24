//! `SetupSource`: what the first run on Windows (DW-0201 to DW-0209) asks of the engine, and the
//! board's fixture.
//!
//! Reads return what the steps show; the calls are what a press does. Every call returns at once
//! (the engine works off the UI thread); what changes on its own afterwards (File Explorer
//! finishing, a chosen photo) is read again through [`SetupSource::explorer`] and
//! [`SetupSource::sample`] on the next [`SetupSource::poll`] that returns `true`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::frames::account::source::Password;

/// What Windows is doing with Beebeeb's folder in File Explorer (the status at the left).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Explorer {
    /// DW-0206b. Windows gives no progress.
    Adding,
    /// DW-0206.
    Added,
    /// DW-0280. `code`: what `CfRegisterSyncRoot` returned, for support ("0x8007017C"); empty when Windows
    /// gave none.
    Failed { code: String },
    /// Sync did not start on this PC, so Windows was not asked to add the folder at all. Drawn as DW-0280 with
    /// its own sentence and no code: it is not Windows that refused.
    NotSyncing,
}

/// The sync folder, while the first run still has to ask for it (the sync-start contract's ruling Q2: the first
/// run asks for the folder before the first start). The board has no frame for it: it is the first state of the
/// File Explorer step.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum FolderAsk {
    /// Nothing to ask: the settings name the folder, or it was chosen in this first run.
    #[default]
    Settled,
    Ask {
        /// The folder that is suggested, as the person's own PC writes it. Shown, never logged.
        folder: String,
        /// Why the last choice was not taken.
        said: Option<FolderSaid>,
        /// A choice is being checked, or Windows' folder picker is open: no second one is asked.
        busy: bool,
    },
    /// A start already worked in the folder chosen first, and a later pick was not taken: the step says that
    /// moving the folder isn't available in this version, with one "OK".
    AlreadySyncs,
}

/// Never prints the folder: a path is shown to the person who chose it and nowhere else.
impl std::fmt::Debug for FolderAsk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Settled => f.write_str("Settled"),
            Self::Ask { said, busy, .. } => f
                .debug_struct("Ask")
                .field("said", said)
                .field("busy", busy)
                .finish_non_exhaustive(),
            Self::AlreadySyncs => f.write_str("AlreadySyncs"),
        }
    }
}

/// Why a folder was not taken as the sync folder. Nothing was created or written for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderSaid {
    /// A folder on the network.
    Network,
    /// A drive itself, or a folder of Windows or its programs.
    System,
    /// The home folder itself, or the folder apps keep their data in.
    Home,
    /// Beebeeb's own data.
    OwnData,
    /// Not a folder.
    NotAFolder,
    /// Another app's sync folder.
    AnotherApps,
    /// The folder could not be read.
    Unreadable,
    /// Not a full path to a folder on this PC.
    NotAFullPath,
    /// Windows' folder picker could not be opened.
    NoPicker,
}

/// A switch of setup whose change is answered after the press ([`SetupSource::take_outcomes`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    Notifications,
    OpenAtLogin,
    /// Step 3: the folders chosen to back up (done when every one was switched on).
    Backup,
    /// Step 2: app lock.
    AppLock,
    /// Step 4: photo previews turned on at the chosen size and format.
    Previews,
    /// Step 2: "Lock again after", asked with app lock on (no row of "You're set up": said only when not changed).
    LockAfter,
}

/// Why step 1's passkey was not added. Nothing was changed. Each kind is said in Settings › Passkeys' own words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasskeyRefused {
    /// No password was given: nothing was sent, and the field asks for it.
    NoPassword,
    /// The password was not the account's.
    WrongPassword,
    /// Windows' passkey dialog was closed, or it timed out.
    Cancelled,
    /// Beebeeb could not be reached.
    Offline,
    /// Too many tries: Beebeeb asks to wait.
    TooManyTries,
    /// This PC has no passkey dialog.
    NotOnThisPc,
    /// Any other refusal.
    NotAdded,
}

/// How step 1's "Add a passkey" ended ([`SetupSource::take_passkey_answer`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasskeyAnswer {
    /// The passkey was added to the account.
    Added,
    Refused(PasskeyRefused),
}

/// How a change asked through the source ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub switch: Switch,
    /// The change was made.
    pub done: bool,
    /// What the switch is now, when that is known (after a change that failed it may not be).
    pub now: Option<bool>,
}

/// One folder the Windows app can back up (DW-0203), in the app's own order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub name: String,
    /// "214 items · 3.1 GB".
    pub summary: String,
    /// On by default (Desktop, Documents, Pictures).
    pub on: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewSize {
    P720,
    P1080,
    K2,
    K4,
}

impl PreviewSize {
    pub const ALL: [PreviewSize; 4] = [PreviewSize::P720, PreviewSize::P1080, PreviewSize::K2, PreviewSize::K4];

    pub fn label(self) -> &'static str {
        match self {
            PreviewSize::P720 => "720p",
            PreviewSize::P1080 => "1080p",
            PreviewSize::K2 => "2K",
            PreviewSize::K4 => "4K",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewFormat {
    Heic,
    Jpeg,
}

impl PreviewFormat {
    pub const ALL: [PreviewFormat; 2] = [PreviewFormat::Heic, PreviewFormat::Jpeg];

    pub fn label(self) -> &'static str {
        match self {
            PreviewFormat::Heic => "HEIC",
            PreviewFormat::Jpeg => "JPEG",
        }
    }
}

/// "Lock again after" (DW-0202), the same choices as Settings › Security.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockAfter {
    Immediately,
    Min5,
    Min30,
    Min60,
}

impl LockAfter {
    pub const ALL: [LockAfter; 4] = [
        LockAfter::Immediately,
        LockAfter::Min5,
        LockAfter::Min30,
        LockAfter::Min60,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LockAfter::Immediately => "Immediately",
            LockAfter::Min5 => "5 min",
            LockAfter::Min30 => "30 min",
            LockAfter::Min60 => "60 min",
        }
    }
}

/// The photo the compare view shows: an encoded image (PNG) and where it comes from. Shared, never copied: a
/// window compares it on every poll (the same `Arc` compares without reading the bytes).
#[derive(Clone, PartialEq, Eq)]
pub struct Sample {
    /// PNG bytes of the original as the compare view draws it.
    pub original_png: std::sync::Arc<[u8]>,
    /// PNG bytes of the preview at the chosen size and format.
    pub preview_png: std::sync::Arc<[u8]>,
    /// `true`: the photo that ships inside the app, because the account has no photo yet.
    pub bundled: bool,
}

/// A sample can be the person's own photo: its `Debug` prints sizes only.
impl std::fmt::Debug for Sample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sample")
            .field("original_png", &self.original_png.len())
            .field("preview_png", &self.preview_png.len())
            .field("bundled", &self.bundled)
            .finish()
    }
}

/// The person's photo is wiped by whoever lets go of it last; the picture that ships inside the app is never the
/// last (its holder lives as long as the app).
impl Drop for Sample {
    fn drop(&mut self) {
        if std::sync::Arc::ptr_eq(&self.original_png, &self.preview_png) {
            // One picture held twice (with HEIC the original is compared with itself): let go of one hold first.
            self.preview_png = std::sync::Arc::from(&[][..]);
        }
        wipe_if_last(&mut self.original_png);
        wipe_if_last(&mut self.preview_png);
    }
}

/// Wipe `bytes` when nothing else holds them (a picture of the person's photo, let go of).
pub fn wipe_if_last(bytes: &mut std::sync::Arc<[u8]>) {
    if let Some(bytes) = std::sync::Arc::get_mut(bytes) {
        zeroize::Zeroize::zeroize(bytes);
    }
}

impl Sample {
    /// The picture that ships inside the app, said as that.
    pub fn bundled() -> Self {
        static BUNDLED: std::sync::OnceLock<std::sync::Arc<[u8]>> = std::sync::OnceLock::new();
        let png = BUNDLED.get_or_init(|| std::sync::Arc::from(BUNDLED_SAMPLE_PNG)).clone();
        Sample {
            original_png: png.clone(),
            preview_png: png,
            bundled: true,
        }
    }
}

/// The sample photo that ships inside the app: the picture the board itself draws in the compare
/// view (DW-0209, DW-0745), exported from the board. Ours; nothing is fetched.
pub const BUNDLED_SAMPLE_PNG: &[u8] = include_bytes!("../../../../assets/sample/compare-sample.png");

pub trait SetupSource {
    /// Whether something answers for a step. `false`: the step keeps its place in the list (the first run has
    /// its seven steps), says that it is not available in this version yet, and turns nothing on; its row of
    /// "You're set up" says so too. Never a step that looks done while nothing was set. The fixture answers for
    /// all of them (the board).
    fn available(&self, _step: super::Step) -> bool {
        true
    }
    /// Whether this step's content is still being read: the step is drawn as a loading state (never as absent, never
    /// with a sample), then as what the read found. The fixture has everything from the start.
    fn loading(&self, _step: super::Step) -> bool {
        false
    }
    /// Whether this step's read FAILED (not "nothing found"): it says it could not read, with Try again.
    fn failed(&self, _step: super::Step) -> bool {
        false
    }
    /// Try again after a failed read of step 3's folders.
    fn retry_backups(&mut self) {}
    /// Try again after a failed read of step 4's preview setting.
    fn retry_previews(&mut self) {}
    /// Whether the first run still has to ask for the sync folder, and what it suggests.
    fn folder_ask(&self) -> FolderAsk {
        FolderAsk::Settled
    }
    /// "Use this folder": the suggested folder is the sync folder, if it may be; then sync starts.
    fn keep_folder(&mut self) {}
    /// "Choose another folder": Windows' folder picker; the folder picked there is the sync folder, if it may
    /// be; then sync starts.
    fn choose_folder(&mut self) {}
    /// Whether another folder can still be chosen after a start that was refused: only while no folder was
    /// kept in the settings (no start has worked yet).
    fn can_choose_again(&self) -> bool {
        false
    }
    /// A start worked and its folder could not be written to the settings, while sync runs.
    fn folder_unsaved(&self) -> bool {
        false
    }
    /// "Try again" on a folder that was not saved: write it.
    fn save_folder(&mut self) {}
    /// "OK" on the note that the folder cannot be moved yet.
    fn dismiss_folder_note(&mut self) {}
    /// Whether a row of "You're set up" leads to its setting. `false`: the rows are values, not links.
    fn opens_settings(&self) -> bool {
        true
    }
    /// `true`: a switch this source changes is answered after the press ([`SetupSource::take_outcomes`]), and
    /// "You're set up" draws its value only then. The fixture's changes are made at once.
    fn answers_later(&self) -> bool {
        false
    }
    /// `false`: "Lock again after" is not set by this source, and step 2 draws no such row (never a choice
    /// nothing acts on). The fixture draws the board's; the engine's sets it. Drawn only while the switch is on.
    fn sets_lock_after(&self) -> bool {
        true
    }
    /// The changes that were answered since the last call, oldest first.
    fn take_outcomes(&mut self) -> Vec<Outcome> {
        Vec::new()
    }
    fn explorer(&self) -> Explorer;
    /// The six known folders with their sizes. Engine: `known_folder::list` + a size scan.
    fn folders(&self) -> Vec<Folder>;
    /// The folders, when a source reads them late: `Some` once, when a read arrived since the last call (the first
    /// run then draws the new list). The fixture has them from the start.
    fn fresh_folders(&mut self) -> Option<Vec<Folder>> {
        None
    }
    /// The original's weight: "30.9 MB". Engine: the chosen photo's size.
    fn original_weight(&self) -> String;
    /// What a preview at this size and format weighs for the chosen photo ("1.0 MB"), if it has
    /// been measured. Engine: `previews::measure(photo, size, format)`.
    fn weight(&self, size: PreviewSize, format: PreviewFormat) -> Option<String>;
    /// The line under step 4's choices: "All 2,431 photos fit, about 2.4 GB. Also about 2.4 GB in
    /// your Beebeeb storage." Engine: `previews::estimate(size, format)` on 50 of the account's photos.
    fn fit_line(&self, size: PreviewSize, format: PreviewFormat) -> String;
    /// The limit for previews on this PC: "20 GB". Engine: `settings::preview_cache_limit`.
    fn space_limit(&self) -> String;
    /// The photo to compare: one of the account's, or the bundled sample when it has none.
    fn sample(&self) -> Sample;
    /// The size and format the account has stored, once read: step 4 starts on it, so "Continue" on untouched
    /// choices changes nothing but turning previews on. `None` (the fixture): the board's own choice.
    fn stored_previews(&self) -> Option<(PreviewSize, PreviewFormat)> {
        None
    }

    /// "Add a passkey". Engine: core's step-up from the account `password` (owner ruling: a passkey change proves
    /// the password), then Windows' passkey dialog (WebAuthn create), then `auth_v2::passkey_register`, on a worker:
    /// the window is never held while the dialog is open. The password is borrowed for the call, never logged and
    /// never kept. `true`: the add STARTED; how it ended is read with [`SetupSource::take_passkey_answer`] (after a
    /// `poll` that returned `true`, or right after this call for a refusal said at once). `false`: nothing was
    /// started (one is on its way, or the press was another session's).
    fn add_passkey(&mut self, password: Password<'_>) -> bool;
    /// How the last "Add a passkey" ended, once: `None` while it has not (Windows' dialog is still open, or nothing
    /// was asked). Never "Added" before the engine answered.
    fn take_passkey_answer(&mut self) -> Option<PasskeyAnswer> {
        None
    }
    /// "Continue" on step 2. Engine: `app_lock::set(on, after)`.
    fn set_app_lock(&mut self, on: bool, after: LockAfter);
    /// "Turn on backup". Engine: `backup::start(folders)`: the first backup runs in the background.
    fn start_backup(&mut self, folders: &[String]);
    /// "Continue" on step 4, and "Done" in the compare view. Engine: `previews::set(size, format)`.
    fn set_previews(&mut self, size: PreviewSize, format: PreviewFormat);
    /// "Try again" on DW-0280. Engine: `finder_setup::register_sync_root` again.
    fn retry_explorer(&mut self);
    /// "Show in File Explorer". Engine: `shell::open_in_file_explorer` on the sync root.
    fn show_in_explorer(&mut self);
    /// Step 6. Engine: `notifications::set_enabled(on)`.
    fn set_notifications(&mut self, on: bool);
    /// Step 7. Engine: `autostart::set(on)` (the Run value the shell already writes).
    fn set_open_at_login(&mut self, on: bool);
    /// "Choose photo". Engine: the photo picker over the account's photos; the new sample
    /// arrives with a later `poll`.
    fn choose_photo(&mut self);
    /// "Open my Beebeeb" and "Skip setup" reaching the end. Engine: `first_run::mark_done`.
    fn finish(&mut self);
    /// `true`: something a read returns has changed since the last call.
    fn poll(&mut self) -> bool;
}

// ---------------------------------------------------------------------------
// The board's fixture
// ---------------------------------------------------------------------------

/// What the fixture answers. A scene or a test sets what it wants.
#[derive(Debug, Clone)]
pub struct Script {
    pub explorer: Explorer,
    /// What `retry_explorer` leads to.
    pub explorer_after_retry: Explorer,
    /// `false`: the passkey was not added (the board: Windows' passkey dialog was cancelled).
    pub passkey_added: bool,
    /// Why it was not, when `passkey_added` is `false`.
    pub passkey_refused: PasskeyRefused,
    /// The sync folder's question (the board has none: `Settled`).
    pub folder: FolderAsk,
    /// The steps nothing answers for (the board has none).
    pub not_available: &'static [super::Step],
    /// Another folder can be chosen after a start that was refused (the board has no such state).
    pub choose_again: bool,
}

impl Default for Script {
    fn default() -> Self {
        Script {
            explorer: Explorer::Adding,
            explorer_after_retry: Explorer::Added,
            passkey_added: true,
            passkey_refused: PasskeyRefused::Cancelled,
            folder: FolderAsk::Settled,
            not_available: &[],
            choose_again: false,
        }
    }
}

/// The calls a fixture received, in order, as short lines ("set_previews 4K HEIC"): for tests.
pub type CallLog = Rc<RefCell<Vec<String>>>;

pub struct FixtureSetupSource {
    script: Script,
    changed: bool,
    log: CallLog,
    /// How the last "Add a passkey" ended, until the window takes it.
    passkey: Option<PasskeyAnswer>,
}

impl FixtureSetupSource {
    pub fn new(script: Script) -> Self {
        FixtureSetupSource {
            script,
            changed: false,
            log: CallLog::default(),
            passkey: None,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn log(&self) -> CallLog {
        self.log.clone()
    }

    fn note(&self, line: String) {
        self.log.borrow_mut().push(line);
    }
}

impl SetupSource for FixtureSetupSource {
    fn available(&self, step: super::Step) -> bool {
        !self.script.not_available.contains(&step)
    }

    fn folder_ask(&self) -> FolderAsk {
        self.script.folder.clone()
    }

    fn keep_folder(&mut self) {
        self.note("keep_folder".into());
        self.script.folder = FolderAsk::Settled;
        self.changed = true;
    }

    fn choose_folder(&mut self) {
        self.note("choose_folder".into());
    }

    fn can_choose_again(&self) -> bool {
        self.script.choose_again
    }

    fn explorer(&self) -> Explorer {
        self.script.explorer.clone()
    }

    fn folders(&self) -> Vec<Folder> {
        [
            ("Desktop", "214 items · 3.1 GB", true),
            ("Documents", "1,820 items · 5.9 GB", true),
            ("Pictures", "3,412 items · 18.2 GB", true),
            ("Music", "1,106 items · 7.4 GB", false),
            ("Videos", "38 items · 21.6 GB", false),
            ("Downloads", "642 items · 4.8 GB", false),
        ]
        .into_iter()
        .map(|(name, summary, on)| Folder {
            name: name.into(),
            summary: summary.into(),
            on,
        })
        .collect()
    }

    fn original_weight(&self) -> String {
        "30.9 MB".into()
    }

    /// The board measures the four sizes as HEIC and the two formats at 4K (DW-0209); it states
    /// no weight for the other three JPEG sizes, so the fixture has none.
    fn weight(&self, size: PreviewSize, format: PreviewFormat) -> Option<String> {
        let weight = match (size, format) {
            (PreviewSize::P720, PreviewFormat::Heic) => "213 KB",
            (PreviewSize::P1080, PreviewFormat::Heic) => "420 KB",
            (PreviewSize::K2, PreviewFormat::Heic) => "640 KB",
            (PreviewSize::K4, PreviewFormat::Heic) => "1.0 MB",
            (PreviewSize::K4, PreviewFormat::Jpeg) => "2.1 MB",
            _ => return None,
        };
        Some(weight.into())
    }

    fn fit_line(&self, _size: PreviewSize, _format: PreviewFormat) -> String {
        "All 2,431 photos fit, about 2.4 GB. Also about 2.4 GB in your Beebeeb storage.".into()
    }

    fn space_limit(&self) -> String {
        "20 GB".into()
    }

    fn sample(&self) -> Sample {
        Sample::bundled()
    }

    /// The board's dialog answers at once: the window takes the answer right after the press.
    fn add_passkey(&mut self, _password: Password<'_>) -> bool {
        self.note("add_passkey".into());
        self.passkey = Some(if self.script.passkey_added {
            PasskeyAnswer::Added
        } else {
            PasskeyAnswer::Refused(self.script.passkey_refused)
        });
        self.changed = true;
        true
    }

    fn take_passkey_answer(&mut self) -> Option<PasskeyAnswer> {
        self.passkey.take()
    }

    fn set_app_lock(&mut self, on: bool, after: LockAfter) {
        self.note(format!("set_app_lock {on} {}", after.label()));
    }

    fn start_backup(&mut self, folders: &[String]) {
        self.note(format!("start_backup {}", folders.join(", ")));
    }

    fn set_previews(&mut self, size: PreviewSize, format: PreviewFormat) {
        self.note(format!("set_previews {} {}", size.label(), format.label()));
    }

    fn retry_explorer(&mut self) {
        self.note("retry_explorer".into());
        self.script.explorer = self.script.explorer_after_retry.clone();
        self.changed = true;
    }

    fn show_in_explorer(&mut self) {
        self.note("show_in_explorer".into());
    }

    fn set_notifications(&mut self, on: bool) {
        self.note(format!("set_notifications {on}"));
    }

    fn set_open_at_login(&mut self, on: bool) {
        self.note(format!("set_open_at_login {on}"));
    }

    fn choose_photo(&mut self) {
        self.note("choose_photo".into());
    }

    fn finish(&mut self) {
        self.note("finish".into());
    }

    fn poll(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }
}
