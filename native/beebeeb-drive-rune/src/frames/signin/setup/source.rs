//! `SetupSource`: what the first run on Windows (DW-0201 to DW-0209) asks of the engine, and the
//! board's fixture.
//!
//! Reads return what the steps show; the calls are what a press does. Every call returns at once
//! (the engine works off the UI thread); what changes on its own afterwards (File Explorer
//! finishing, a chosen photo) is read again through [`SetupSource::explorer`] and
//! [`SetupSource::sample`] on the next [`SetupSource::poll`] that returns `true`.

use std::cell::RefCell;
use std::rc::Rc;

/// What Windows is doing with Beebeeb's folder in File Explorer (the status at the left).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Explorer {
    /// DW-0206b. Windows gives no progress.
    Adding,
    /// DW-0206.
    Added,
    /// DW-0280. `code`: what `CfRegisterSyncRoot` returned, for support ("0x8007017C").
    Failed { code: String },
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

/// The photo the compare view shows: an encoded image (PNG) and where it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    /// PNG bytes of the original as the compare view draws it.
    pub original_png: &'static [u8],
    /// PNG bytes of the preview at the chosen size and format.
    pub preview_png: &'static [u8],
    /// `true`: the photo that ships inside the app, because the account has no photo yet.
    pub bundled: bool,
}

/// The sample photo that ships inside the app: the picture the board itself draws in the compare
/// view (DW-0209, DW-0745), exported from the board. Ours; nothing is fetched.
pub const BUNDLED_SAMPLE_PNG: &[u8] = include_bytes!("../../../../assets/sample/compare-sample.png");

pub trait SetupSource {
    fn explorer(&self) -> Explorer;
    /// The six known folders with their sizes. Engine: `known_folder::list` + a size scan.
    fn folders(&self) -> Vec<Folder>;
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

    /// "Add a passkey". Engine: Windows' passkey dialog (WebAuthn create), then
    /// `auth_v2::passkey_register`. `true`: a passkey was added.
    fn add_passkey(&mut self) -> bool;
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
    /// Whether this PC has shown the first-time guide (DW-0380: "Shown once per PC"). Not wired:
    /// `false`, so the guide follows each first run. The engine binds it to a flag kept per PC.
    fn guide_shown(&self) -> bool {
        false
    }
    /// The guide starts now: remember it for this PC. Not wired: nothing is kept.
    fn mark_guide_shown(&mut self) {}
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
    /// `false`: Windows' passkey dialog was cancelled.
    pub passkey_added: bool,
    /// This PC has shown the first-time guide.
    pub guide_shown: bool,
}

impl Default for Script {
    fn default() -> Self {
        Script {
            explorer: Explorer::Adding,
            explorer_after_retry: Explorer::Added,
            passkey_added: true,
            guide_shown: false,
        }
    }
}

/// The calls a fixture received, in order, as short lines ("set_previews 4K HEIC"): for tests.
pub type CallLog = Rc<RefCell<Vec<String>>>;

pub struct FixtureSetupSource {
    script: Script,
    changed: bool,
    log: CallLog,
}

impl FixtureSetupSource {
    pub fn new(script: Script) -> Self {
        FixtureSetupSource {
            script,
            changed: false,
            log: CallLog::default(),
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
        Sample {
            original_png: BUNDLED_SAMPLE_PNG,
            preview_png: BUNDLED_SAMPLE_PNG,
            bundled: true,
        }
    }

    fn add_passkey(&mut self) -> bool {
        self.note("add_passkey".into());
        self.script.passkey_added
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

    fn guide_shown(&self) -> bool {
        self.script.guide_shown
    }

    fn mark_guide_shown(&mut self) {
        self.script.guide_shown = true;
        self.note("mark_guide_shown".into());
    }
}
