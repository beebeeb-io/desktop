//! The first run on Windows (DW-0201 to DW-0209): the seven setup steps between sign-in and the
//! main window, "You're set up", and the compare view over step 4. Drawn in the first-run
//! window (960 x 640, `surfaces::WindowKind::FirstRun`) with ONE caption bar and no menu bar.
//!
//! [`Ui`] is the window's state; every control changes it through one of its methods, which
//! call the engine through [`source::SetupSource`]. What leaves the group (the main window, a
//! settings pane) is an [`Intent`] the shell takes with [`Ui::take_intents`].
//!
//! Nothing here is a secret: no password, code or phrase is shown or typed in these steps.

pub mod guide;
pub mod icons;
pub mod source;
mod view;

#[cfg(test)]
mod guide_tests;
#[cfg(test)]
mod tests;

use rune::{Cx, El, Key, KeyEvent, Pixels, Tx};

use crate::State;
use crate::shell::app::{Event, GateState};
use crate::shell::host;
use source::{
    Explorer, FixtureSetupSource, Folder, LockAfter, PreviewFormat, PreviewSize, Sample, Script, SetupSource,
};

pub use view::view;

/// The screen's title (`ui::au::heading`).
pub const TITLE_KEY: &str = "title";
/// The compare view's Done button: focus goes there when the view opens.
pub const COMPARE_DONE_KEY: &str = "cmp-done";
/// "Compare" on step 4: focus returns there when the view closes.
pub const COMPARE_KEY: &str = "btn-0";

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

    fn next(self) -> Step {
        match self {
            Step::Passkey => Step::Lock,
            Step::Lock => Step::Backup,
            Step::Backup => Step::Previews,
            Step::Previews => Step::Explorer,
            Step::Explorer => Step::Notifications,
            Step::Notifications => Step::Login,
            Step::Login | Step::Done => Step::Done,
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
        Ui {
            step: Step::Passkey,
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
            original: decode(sample.original_png),
            preview: decode(sample.preview_png),
            sample,
            source,
            focus: None,
            intents: Vec::new(),
            finished: false,
        }
    }

    /// What the shell should open, oldest first.
    #[cfg_attr(not(test), allow(dead_code))] // waits for its reader in the shell (PR body)
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

    fn advance(&mut self) {
        self.step = self.step.next();
        self.focus = Some(TITLE_KEY);
    }

    /// "Add a passkey": Windows' own dialog, then step 2. A cancelled dialog stays on step 1.
    pub fn add_passkey(&mut self) {
        if self.step != Step::Passkey {
            return;
        }
        if self.source.add_passkey() {
            self.results.passkey = true;
            self.advance();
        }
    }

    /// The amber button of steps 2 to 7.
    pub fn go_on(&mut self) {
        if self.compare.is_some() {
            return;
        }
        match self.step {
            Step::Passkey | Step::Done => return,
            Step::Lock => {
                self.source.set_app_lock(self.lock_on, self.lock_after);
                self.results.app_lock = self.lock_on;
            }
            Step::Backup => {
                let on: Vec<String> = self.folders.iter().filter(|f| f.on).map(|f| f.name.clone()).collect();
                if on.is_empty() {
                    return;
                }
                self.source.start_backup(&on);
                self.results.backup = on;
            }
            Step::Previews => self.source.set_previews(self.size, self.format),
            Step::Explorer => {
                // "Try again" is the amber button while it failed (DW-0280).
                if matches!(self.explorer, Explorer::Failed { .. }) {
                    self.source.retry_explorer();
                    self.pump();
                    self.focus = Some(TITLE_KEY);
                    return;
                }
            }
            Step::Notifications => {
                self.source.set_notifications(true);
                self.results.notifications = true;
            }
            Step::Login => {
                self.source.set_open_at_login(self.login_on);
                self.results.open_at_login = self.login_on;
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
            Step::Notifications => {
                self.source.set_notifications(false);
                self.results.notifications = false;
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
        self.step = Step::Done;
        self.focus = Some(TITLE_KEY);
    }

    pub fn toggle_lock(&mut self) {
        self.lock_on = !self.lock_on;
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
    }

    pub fn set_format(&mut self, format: PreviewFormat) {
        self.format = format;
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
        if self.step == Step::Done {
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

    /// Once after "Open my Beebeeb": whether the first-time guide starts now. It does unless this
    /// PC has shown it (`SetupSource::guide_shown`), and it is marked shown here. For `guide::poll`.
    pub fn guide_due(&mut self) -> bool {
        if !std::mem::take(&mut self.finished) || self.source.guide_shown() {
            return false;
        }
        self.source.mark_guide_shown();
        true
    }

    /// Read again what the engine changed on its own. `true`: something did.
    pub fn pump(&mut self) -> bool {
        if !self.source.poll() {
            return false;
        }
        self.explorer = self.source.explorer();
        let sample = self.source.sample();
        if sample != self.sample {
            self.original = decode(sample.original_png);
            self.preview = decode(sample.preview_png);
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

/// The registry's line for `FrameGroup::FirstRun`. With no setup state (nothing routes here
/// until the engine lane adds the first-run state to the gate) the shell's "not built yet"
/// frame shows, never sample values.
pub fn frame(s: &State, cx: &mut Cx<State>) -> El {
    let name = s.identity.product_name();
    match &s.setup {
        Some(ui) => view(ui, name, cx),
        None => crate::shell::placeholder::window(s, cx, crate::shell::registry::FrameGroup::FirstRun),
    }
}

/// What the engine changed between presses (File Explorer finished). For `shell::host::poll`.
#[allow(dead_code)] // waits for its one line in `shell/host.rs` (PR body)
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
