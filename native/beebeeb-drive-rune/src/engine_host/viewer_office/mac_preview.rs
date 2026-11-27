//! macOS shows the file itself (dm-70, dm-74): a recording or a video AVFoundation plays, and a document Quick Look
//! previews (Word, Excel, PowerPoint, Pages, Numbers, Keynote, RTF, PSD, HEIC, RAW, …), in a view of the system's
//! INSIDE the window, where the viewer's body is (`frames::viewers::native_preview`). Part of
//! [`EngineViewerSource`]: `viewer_media.rs` asks it for the kinds the app has no viewer for,
//! `viewer_office/glue.rs` for an Office file (the Mac has no Office helper). macOS only.
//!
//! - **The type decides** ([`Host::shower_of`]): a kind the app shows itself (text, Markdown, pictures, PDF) stays
//!   the app's; a kind AVFoundation reads is a [`Shower::Player`] one, a kind Quick Look previews a
//!   [`Shower::QuickLook`] one; any other kind keeps the page with the other apps (dm-63). A player kind is asked
//!   again of the file itself (`AVURLAsset.isPlayable`) once the file is there.
//! - **Finder's copy first** (dm-74, [`Finder`]): the account's own file has a place in Finder, and the system is
//!   handed THAT file by its URL. The app decrypts nothing, copies nothing and reads none of it into memory. A file
//!   that is only in the cloud is first asked of the File Provider (the whole file, the one download Finder's own
//!   open would make); the page shows how far it is, and a download that stops moving ([`Native::stall`]) or is
//!   refused ends on the page with its reason, and in the log. The security scope of Finder's URL lasts as long as
//!   the port's read call does, so the worker's thread stays inside that call for as long as the file is on screen
//!   ([`FinderCopy`]); leaving the file lets the thread go.
//! - **The temp copy** (dm-70) is only for a file Finder holds no place for (Beebeeb is not in Finder, a file shared
//!   with the account): the decrypted bytes are written where Files' Open puts its own (`<temp>/beebeeb-open/`, here
//!   `<id>.preview/<name>`, the folder 0700 and the file 0600). Those bytes pass through memory, so the limit is a
//!   document's ([`PREVIEW_MAX`]); a larger file gets the page that says so, and nothing of it is downloaded.
//! - **In the window:** the seam hosts a view ([`Host::embed`]: `AVPlayerView`, `QLPreviewView`) and the viewer's
//!   body says where it sits ([`Host::place`]) through the [`Stage`] the body carries: the body's laid-out rect, so
//!   the view follows a resize and the details panel, and nothing while one of the viewer's menus, toasts or dialogs
//!   is open over it. When no view could be hosted the Quick Look panel is the fallback ([`Host::show`]); when that
//!   fails too, the page with the other apps.
//! - **When it goes:** when the viewer closes or opens another file, when the session is not the one it was opened
//!   for (a lock, a sign-out, another account) or Beebeeb is locked, and when it is no longer on screen (the person
//!   closed the panel). The view leaves the window first, then the file is let go: Finder's is untouched, a temp
//!   copy is removed ([`TempCopy`]'s drop; one a crash or a quit left behind goes when the next is made).
//!
//! Session rule, as the rest of the viewer: the file is made ready for the session of the press and shown only
//! while that session is the session.
//!
//! Must not: log a name, an id, a path or a byte; leave a copy behind; read a whole file into memory to show it
//! from Finder.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::viewer_api::OpenFileError;

use crate::engine_host::viewer::{EngineViewerSource, Recorded, Shows, ViewerEngine, body_of, log, read, shows};
use crate::frames::viewers::native_preview::{NativeDoc, NativeKind, Stage, StagePort};
use crate::frames::viewers::source::{Body, NoPreview, Stopped};

#[cfg(test)]
#[path = "mac_preview_tests.rs"]
mod tests;

/// The largest file shown from a temp copy: its bytes are held in memory, whole, while the copy is written. Finder's
/// own copy has no limit (the app reads none of it).
pub(crate) const PREVIEW_MAX: u64 = 64 * 1024 * 1024;

/// How long Finder's download of a file may go without a byte more before the open is given up.
const STALL: Duration = Duration::from_secs(60);
/// How often the download is looked at.
const LOOK_EVERY: Duration = Duration::from_millis(200);
/// How often Finder's place for a file is asked for while another File Provider call holds the bridge's gate.
const PLACE_TRIES: usize = 8;

/// What of macOS shows a kind of file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shower {
    /// AVFoundation plays it (a recording, a video): the file itself is asked once it is there.
    Player,
    /// Quick Look previews it (a document, a picture the app does not decode).
    QuickLook,
}

impl Shower {
    fn kind(self) -> NativeKind {
        match self {
            Shower::Player => NativeKind::Player,
            Shower::QuickLook => NativeKind::QuickLook,
        }
    }
}

/// The hosting seam: what is asked of macOS. The app's is [`SystemHost`]; a test's remembers what it was asked.
pub(crate) trait Host: Send {
    /// What shows this kind of file, by its name. `None`: macOS does not show it.
    fn shower_of(&self, name: &str) -> Option<Shower>;
    /// Whether AVFoundation plays this file (`AVURLAsset.isPlayable`).
    fn playable(&self, file: &Path) -> bool;
    /// Hosts the file in a view inside the window (a player view, a Quick Look view), not yet placed. `false`: no
    /// view could be hosted.
    fn embed(&mut self, shower: Shower, file: &Path) -> bool;
    /// Where the hosted view of `file` sits: the viewer's body, `[x, y, w, h]` in window points. `None`: hidden.
    fn place(&mut self, file: &Path, rect: Option<[f32; 4]>);
    /// Shows the file in the Quick Look panel (the fallback). `false`: it could not be shown.
    fn show(&mut self, file: &Path) -> bool;
    /// Whether the file is still on screen (in the window, or in a panel the person did not close).
    fn showing(&self, file: &Path) -> bool;
    /// Takes the file off the screen.
    fn close(&mut self, file: &Path);
}

/// The seam, shared with the [`Stage`] the viewer's body places the view through.
pub(crate) type SharedHost = Arc<Mutex<Box<dyn Host>>>;

fn held(host: &SharedHost) -> MutexGuard<'_, Box<dyn Host>> {
    host.lock().unwrap_or_else(PoisonError::into_inner)
}

/// AVFoundation, the views inside the window and the Quick Look panel (`shell::platform::macos_preview`).
pub(crate) struct SystemHost;

impl Host for SystemHost {
    fn shower_of(&self, name: &str) -> Option<Shower> {
        use crate::shell::platform::macos_preview as mac;
        if mac::plays_type(name) {
            Some(Shower::Player)
        } else if mac::quicklook_shows_type(name) {
            Some(Shower::QuickLook)
        } else {
            None
        }
    }

    fn playable(&self, file: &Path) -> bool {
        crate::shell::platform::macos_preview::playable(file)
    }

    fn embed(&mut self, shower: Shower, file: &Path) -> bool {
        use crate::shell::platform::macos_preview::{Hosted, embed};
        let kind = match shower {
            Shower::Player => Hosted::Player,
            Shower::QuickLook => Hosted::QuickLook,
        };
        embed(kind, file)
    }

    fn place(&mut self, file: &Path, rect: Option<[f32; 4]>) {
        crate::shell::platform::macos_preview::place(file, rect);
    }

    fn show(&mut self, file: &Path) -> bool {
        crate::shell::platform::macos_preview::show(file)
    }

    fn showing(&self, file: &Path) -> bool {
        crate::shell::platform::macos_preview::showing(file)
    }

    fn close(&mut self, file: &Path) {
        crate::shell::platform::macos_preview::close(file);
    }
}

/// A test's source shows nothing through macOS unless the test puts a host of its own in.
#[cfg(test)]
struct NoHost;

#[cfg(test)]
impl Host for NoHost {
    fn shower_of(&self, _name: &str) -> Option<Shower> {
        None
    }
    fn playable(&self, _file: &Path) -> bool {
        false
    }
    fn embed(&mut self, _shower: Shower, _file: &Path) -> bool {
        false
    }
    fn place(&mut self, _file: &Path, _rect: Option<[f32; 4]>) {}
    fn show(&mut self, _file: &Path) -> bool {
        false
    }
    fn showing(&self, _file: &Path) -> bool {
        false
    }
    fn close(&mut self, _file: &Path) {}
}

/// What there is of a file at Finder's place for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    /// A plain file with its bytes on this Mac.
    Here,
    /// A file without its bytes (online only): the system downloads it when asked.
    Cloud,
    /// No plain file there.
    Gone,
}

/// `SF_DATALESS` (`sys/stat.h`): the file's bytes are not on this Mac.
const SF_DATALESS: u32 = 0x4000_0000;

/// [`Presence`] from the file's attributes alone: nothing is opened, so nothing is downloaded by the look.
fn presence_on_disk(file: &Path) -> Presence {
    use std::os::macos::fs::MetadataExt;
    match std::fs::symlink_metadata(file) {
        Ok(meta) if !meta.is_file() => Presence::Gone,
        Ok(meta) if meta.st_flags() & SF_DATALESS != 0 => Presence::Cloud,
        Ok(_) => Presence::Here,
        Err(_) => Presence::Gone,
    }
}

/// Finder's copies, as the File Provider hands them out. Asked on the worker (the calls wait for the system). The
/// app's is [`SystemFinder`]; a test's is a folder of its own.
pub(crate) trait Finder: Send + Sync {
    /// `FileProviderPort::with_finder_copy`: `read` is called once with Finder's place for the file, and only for
    /// the length of that call may the app (and the views it hosts) read there. `false`: no place, `read` not called.
    fn with_copy(&self, file_id: &str, read: &mut dyn FnMut(&Path)) -> bool;
    /// What is at that place now.
    fn presence(&self, file: &Path) -> Presence {
        presence_on_disk(file)
    }
    /// Asks the system to download the whole file (`requestDownloadForItemWithIdentifier:`): the File Provider's own
    /// fetch is the one download. `Err`: the system's domain and code, for the log.
    fn request(&self, file_id: &str) -> Result<(), String>;
}

/// The File Provider bridge. A test never reaches it.
#[cfg(not(test))]
pub(crate) struct SystemFinder;

#[cfg(not(test))]
impl Finder for SystemFinder {
    fn with_copy(&self, file_id: &str, read: &mut dyn FnMut(&Path)) -> bool {
        crate::shell::platform::macos::macos_fp::bridge::with_finder_copy(file_id, read)
    }

    fn request(&self, file_id: &str) -> Result<(), String> {
        use beebeeb_engine::finder_setup::error::{BRIDGE_DOMAIN, bridge_code};

        use crate::shell::platform::macos::macos_fp::bridge::BeebeebFpErrorC;
        unsafe extern "C" {
            /// `requestDownloadForItemWithIdentifier:` (the whole file): 0 = the system took the request.
            fn beebeeb_fp_request_download(identifier: *const std::ffi::c_char, out_error: *mut BeebeebFpErrorC)
            -> i32;
        }
        let id = std::ffi::CString::new(file_id).map_err(|_| "bad item id".to_string())?;
        let mut error = BeebeebFpErrorC::zeroed();
        // SAFETY: `id` is a NUL-terminated string that outlives the call; `error` is the bridge's own struct.
        if unsafe { beebeeb_fp_request_download(id.as_ptr(), &mut error) } == 0 {
            return Ok(());
        }
        let error = error.into_fp_error();
        // The system did not answer within the bridge's limit: it may still have taken the request, and the wait
        // for the file tells.
        if error.domain == BRIDGE_DOMAIN && error.code == bridge_code::SIGNAL_TIMEOUT {
            return Ok(());
        }
        Err(error.redacted())
    }
}

/// A test's source finds nothing in Finder unless the test puts a finder of its own in.
#[cfg(test)]
struct NoFinder;

#[cfg(test)]
impl Finder for NoFinder {
    fn with_copy(&self, _file_id: &str, _read: &mut dyn FnMut(&Path)) -> bool {
        false
    }
    fn request(&self, _file_id: &str) -> Result<(), String> {
        Err("no finder".into())
    }
}

/// Finder's own copy of the file, readable for as long as this lives: the worker's thread stays inside the port's
/// read call (which holds the URL's security scope) until this is dropped. Nothing is removed by the drop.
pub(crate) struct FinderCopy {
    path: PathBuf,
    /// The thread waits on the other end; the drop lets it go.
    _held: mpsc::Sender<()>,
}

/// The file the system is handed.
pub(crate) enum Ready {
    Finder(FinderCopy),
    Copy(TempCopy),
}

impl Ready {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Ready::Finder(copy) => &copy.path,
            Ready::Copy(copy) => copy.path(),
        }
    }
}

/// The decrypted copy the system reads. Its drop removes it and its folder.
pub(crate) struct TempCopy {
    path: PathBuf,
}

impl TempCopy {
    /// The folder of `file_id`'s copy below `root`. The id is one path component, whatever it holds.
    fn folder(root: &Path, file_id: &str) -> PathBuf {
        let id: String = file_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        root.join(format!("{id}.preview"))
    }

    /// Removes every preview copy below `root`: what a run that ended without closing its viewer (a crash, a quit)
    /// left there. Only folders this module names (`*.preview`), never a copy Files' Open made.
    fn sweep(root: &Path) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "preview") && path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            }
        }
    }

    /// Writes `bytes` as `<root>/<id>.preview/<name>`. One copy at a time (the system has one panel): any copy an
    /// earlier run left below `root` is removed first.
    fn write(root: &Path, file_id: &str, name: &str, bytes: &[u8]) -> std::io::Result<TempCopy> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let folder = Self::folder(root, file_id);
        Self::sweep(root);
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&folder)?;
        // The name's last part only: the system reads the kind from its extension.
        let name = Path::new(name).file_name().unwrap_or_else(|| "file".as_ref());
        // From here on the drop removes what was made, also when the write fails.
        let copy = TempCopy {
            path: folder.join(name),
        };
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&copy.path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(copy)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        if let Some(folder) = self.path.parent() {
            let _ = std::fs::remove_dir(folder);
        }
    }
}

/// Why the file did not get ready.
pub(crate) enum Stop {
    /// The read for the temp copy stopped (the engine's own reasons).
    Open(OpenFileError),
    /// The system refused to download Finder's copy: its domain and code.
    Refused(String),
    /// Finder's download stopped moving: how far it was.
    Stalled { done: u64, total: u64 },
}

/// The worker's one answer: the file and the session it was made ready for.
type Answer = Result<(SessionGeneration, Ready), Stop>;

/// The source's macOS part: the seams, where copies go, and the file shown now.
pub(crate) struct Native {
    pub(crate) host: SharedHost,
    pub(crate) finder: Arc<dyn Finder>,
    /// Where temp copies are made (`<temp>/beebeeb-open`; a test's own folder).
    pub(crate) root: PathBuf,
    /// The largest temp copy ([`PREVIEW_MAX`]).
    pub(crate) max: u64,
    /// How long Finder's download may stand still ([`STALL`]; three times as long while nothing says how far it is).
    pub(crate) stall: Duration,
    on: Option<On>,
}

impl Default for Native {
    fn default() -> Self {
        #[cfg(not(test))]
        let (host, finder): (Box<dyn Host>, Arc<dyn Finder>) = (Box::new(SystemHost), Arc::new(SystemFinder));
        #[cfg(test)]
        let (host, finder): (Box<dyn Host>, Arc<dyn Finder>) = (Box::new(NoHost), Arc::new(NoFinder));
        Native {
            host: Arc::new(Mutex::new(host)),
            finder,
            root: std::env::temp_dir().join("beebeeb-open"),
            max: PREVIEW_MAX,
            stall: STALL,
            on: None,
        }
    }
}

/// The file macOS shows, or is about to, and the session it was opened for.
struct On {
    file_id: String,
    session: SessionGeneration,
    account: Option<String>,
    shower: Shower,
    cancel: Arc<AtomicBool>,
    /// Bytes done and in all, as the read or Finder's download reports them.
    progress: Arc<(AtomicU64, AtomicU64)>,
    /// The worker's answer, while it is on its way.
    inbox: Option<mpsc::Receiver<Answer>>,
    drawn: (u64, u64),
    /// The file on screen: in the view inside the window (the body's [`Stage`] places it), or in the panel.
    shown: Option<Ready>,
}

/// The [`Stage`] of one hosted view: the body's rect and whether something is drawn over it, told to the seam when
/// either changes.
struct Port {
    host: SharedHost,
    file: PathBuf,
    state: Mutex<Placed>,
}

#[derive(Default)]
struct Placed {
    rect: Option<[f32; 4]>,
    covered: bool,
    /// What the seam was last told.
    told: Option<Option<[f32; 4]>>,
}

impl Port {
    fn tell(&self, change: impl FnOnce(&mut Placed)) {
        let want = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            change(&mut state);
            let want = if state.covered { None } else { state.rect };
            if state.told == Some(want) {
                return;
            }
            state.told = Some(want);
            want
        };
        held(&self.host).place(&self.file, want);
    }
}

impl StagePort for Port {
    fn at(&self, rect: [f32; 4]) {
        self.tell(|state| state.rect = Some(rect));
    }

    fn cover(&self, on: bool) {
        self.tell(|state| state.covered = on);
    }
}

impl Native {
    /// Nothing of the file stays: the worker is told to stop, the view (or the panel) lets go of the file, Finder's
    /// copy is let go and a temp copy is removed.
    fn end(&mut self) {
        if let Some(mut on) = self.on.take() {
            on.cancel.store(true, Ordering::SeqCst);
            on.inbox = None;
            if let Some(shown) = on.shown.take() {
                held(&self.host).close(shown.path());
            }
        }
    }

    /// The file on screen now (a test reads it).
    #[cfg(test)]
    pub(crate) fn shown_copy(&self) -> Option<&Path> {
        self.on.as_ref().and_then(|on| on.shown.as_ref()).map(Ready::path)
    }
}

/// What the worker found at Finder's place for the file.
enum Looked {
    /// The file was handed over, shown, and let go again: nothing more to say.
    Shown,
    Stopped(Stop),
    /// Finder holds no file there: the temp copy is the way.
    NotThere,
}

/// One open's work, on its own thread.
struct Job<E> {
    engine: Arc<E>,
    finder: Arc<dyn Finder>,
    file_id: String,
    name: String,
    session: SessionGeneration,
    recorded: Recorded,
    size: u64,
    max: u64,
    stall: Duration,
    root: PathBuf,
    cancel: Arc<AtomicBool>,
    progress: Arc<(AtomicU64, AtomicU64)>,
    answers: mpsc::Sender<Answer>,
}

impl<E: ViewerEngine> Job<E> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Finder's copy is on this Mac when this answers `Ok(true)`: it was, or the system was asked for it and it
    /// came. `Ok(false)`: no plain file at Finder's place.
    fn arrive(&self, place: &Path, report: &dyn Fn(u64, u64)) -> Result<bool, Stop> {
        match self.finder.presence(place) {
            Presence::Here => return Ok(true),
            Presence::Gone => return Ok(false),
            Presence::Cloud => {}
        }
        self.finder.request(&self.file_id).map_err(Stop::Refused)?;
        // When the download last moved, how far it was then, and whether anything said how far it is at all.
        let (mut moved, mut done, mut seen) = (Instant::now(), 0_u64, false);
        loop {
            if self.cancelled() {
                return Err(Stop::Open(OpenFileError::Cancelled));
            }
            match self.finder.presence(place) {
                Presence::Here => {
                    report(self.size, self.size);
                    return Ok(true);
                }
                Presence::Gone => return Ok(false),
                Presence::Cloud => {}
            }
            if let Some((now, total)) = self.engine.download_progress(self.session, &self.file_id) {
                seen = true;
                report(now, if total == 0 { self.size } else { total });
                if now != done {
                    (moved, done) = (Instant::now(), now);
                }
            }
            let limit = if seen { self.stall } else { self.stall * 3 };
            if moved.elapsed() >= limit {
                return Err(Stop::Stalled { done, total: self.size });
            }
            std::thread::sleep(LOOK_EVERY.min(self.stall));
        }
    }

    /// Inside the port's read call, at Finder's place for the file: makes the file present, hands it over, and stays
    /// here (the URL's scope with it) until the viewer lets the file go.
    fn hold(&self, place: &Path, report: &dyn Fn(u64, u64)) -> Looked {
        match self.arrive(place, report) {
            Ok(true) => {}
            Ok(false) => return Looked::NotThere,
            Err(stop) => return Looked::Stopped(stop),
        }
        if self.cancelled() || self.engine.session_turn() != self.session {
            return Looked::Stopped(Stop::Open(OpenFileError::Cancelled));
        }
        let (held, let_go) = mpsc::channel::<()>();
        let copy = FinderCopy {
            path: place.to_path_buf(),
            _held: held,
        };
        if self.answers.send(Ok((self.session, Ready::Finder(copy)))).is_ok() {
            self.engine.wake();
            // Nothing is ever sent: this returns when the viewer drops its end.
            let _ = let_go.recv();
        }
        Looked::Shown
    }

    /// The whole of the work. `None`: Finder's copy was shown and let go (the answer was sent from inside).
    fn run(&self) -> Option<Answer> {
        let report = {
            let (engine, progress) = (self.engine.clone(), self.progress.clone());
            // The window is woken once per whole percent, as the viewer's own download wakes it.
            let woken = AtomicU64::new(0);
            move |done: u64, total: u64| {
                progress.0.store(done, Ordering::SeqCst);
                progress.1.store(total, Ordering::SeqCst);
                let percent = if total == 0 {
                    0
                } else {
                    done.saturating_mul(100) / total
                };
                if woken.swap(percent, Ordering::SeqCst) != percent {
                    engine.wake();
                }
            }
        };
        // Finder's copy first: the account's own file, while Beebeeb is in Finder.
        let mut looked = Looked::NotThere;
        if self.engine.in_finder(&self.file_id) {
            for attempt in 0..PLACE_TRIES {
                if self.cancelled() {
                    break;
                }
                if attempt > 0 {
                    std::thread::sleep(LOOK_EVERY);
                }
                if self
                    .finder
                    .with_copy(&self.file_id, &mut |place| looked = self.hold(place, &report))
                {
                    break;
                }
            }
        }
        match looked {
            Looked::Shown => return None,
            Looked::Stopped(stop) => return Some(Err(stop)),
            Looked::NotThere => {}
        }
        if self.cancelled() {
            return Some(Err(Stop::Open(OpenFileError::Cancelled)));
        }
        // No place in Finder: a temp copy, for a file small enough to pass through memory. Nothing is downloaded
        // for a larger one.
        if self.size > self.max {
            return Some(Err(Stop::Open(OpenFileError::TooLarge { size: self.size })));
        }
        let read = read(
            &*self.engine,
            &self.file_id,
            self.recorded,
            self.max,
            &report,
            &self.cancel,
        );
        let (session, bytes, _) = match read {
            Ok(read) => read,
            Err(error) => return Some(Err(Stop::Open(error))),
        };
        if self.cancelled() {
            return Some(Err(Stop::Open(OpenFileError::Cancelled)));
        }
        Some(
            TempCopy::write(&self.root, &self.file_id, &self.name, &bytes)
                .map(|copy| (session, Ready::Copy(copy)))
                .map_err(|_| Stop::Open(OpenFileError::Failed)),
        )
    }
}

impl<E: ViewerEngine> EngineViewerSource<E> {
    /// What of macOS shows `name`. `None`: the app shows it itself, or macOS does not show it.
    pub(in crate::engine_host::viewer) fn mac_shower(&self, name: &str) -> Option<Shower> {
        if shows(name) != Shows::Nothing {
            return None;
        }
        held(&self.media.mac.host).shower_of(name)
    }

    /// Starts showing `file_id` through macOS: the body shown now. `None`: not a kind macOS shows (nothing started).
    pub(in crate::engine_host::viewer) fn mac_begin(
        &mut self,
        session: SessionGeneration,
        account: Option<String>,
        file_id: &str,
        name: &str,
    ) -> Option<Body> {
        self.mac_leave();
        let shower = self.mac_shower(name)?;
        let facts = self.engine.facts(file_id).ok()?;
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new((AtomicU64::new(0), AtomicU64::new(facts.size)));
        let (answers, inbox) = mpsc::channel();
        let job = Job {
            engine: self.engine.clone(),
            finder: self.media.mac.finder.clone(),
            file_id: file_id.to_string(),
            name: name.to_string(),
            session,
            recorded: Recorded {
                here: facts.on_this_pc.is_some(),
                unsettled: facts.unsettled_here,
            },
            size: facts.size,
            max: self.media.mac.max,
            stall: self.media.mac.stall,
            root: self.media.mac.root.clone(),
            cancel: cancel.clone(),
            progress: progress.clone(),
            answers,
        };
        let started = std::thread::Builder::new()
            .name("viewer-mac-preview".into())
            .spawn(move || {
                let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.run()))
                    .unwrap_or(Some(Err(Stop::Open(OpenFileError::Failed))));
                let Some(answer) = answer else {
                    return;
                };
                // Nobody waits any more: a copy is removed here (its drop), never queued.
                let answer = if job.cancelled() {
                    drop(answer);
                    Err(Stop::Open(OpenFileError::Cancelled))
                } else {
                    answer
                };
                let _ = job.answers.send(answer);
                job.engine.wake();
            })
            .is_ok();
        if !started {
            log("viewer: could not start a thread for an open file");
            return Some(Body::Failed);
        }
        self.media.mac.on = Some(On {
            file_id: file_id.to_string(),
            session,
            account,
            shower,
            cancel,
            progress,
            inbox: Some(inbox),
            drawn: (0, facts.size),
            shown: None,
        });
        Some(Body::Decrypting {
            done: 0,
            total: facts.size,
        })
    }

    /// The poll's macOS arm. `None`: macOS shows no file of this id here (the rest of the poll runs).
    pub(in crate::engine_host::viewer) fn mac_poll(&mut self, file_id: &str) -> Option<Option<Body>> {
        let on = self.media.mac.on.as_ref().filter(|on| on.file_id == file_id)?;
        let engine = &self.engine;
        let current = engine.has_session() && engine.session_turn() == on.session && engine.account() == on.account;
        // A lock, a sign-out, another account: the view leaves the window and the file is let go NOW. A locked
        // Beebeeb that keeps its session shows nothing of a file either.
        if !current || (on.shown.is_some() && !engine.screen_open()) {
            self.media.mac.end();
            return Some(Some(Body::Stopped(Stopped::SessionChanged)));
        }
        // Cancel and Esc reach the opening (`ViewerSource::cancel`): the work stops, and a file on screen goes.
        let cancelled = self
            .open
            .as_ref()
            .is_some_and(|opening| opening.file_id == file_id && opening.cancel.load(Ordering::SeqCst));
        if cancelled {
            let shown = on.shown.is_some();
            self.media.mac.end();
            return Some(shown.then_some(Body::NotPreviewable(NoPreview::NoViewer)));
        }
        let Some(inbox) = &on.inbox else {
            // Shown. It is no longer on screen (the person closed the panel, the window went): the file is let go,
            // and the page offers the other apps.
            let closed = on
                .shown
                .as_ref()
                .is_some_and(|shown| !held(&self.media.mac.host).showing(shown.path()));
            if closed {
                self.media.mac.end();
                return Some(Some(Body::NotPreviewable(NoPreview::NoViewer)));
            }
            return Some(None);
        };
        let answer = match inbox.try_recv() {
            Ok(answer) => answer,
            Err(mpsc::TryRecvError::Empty) => {
                let now = (
                    on.progress.0.load(Ordering::SeqCst),
                    on.progress.1.load(Ordering::SeqCst),
                );
                let on = self.media.mac.on.as_mut()?;
                return Some((now != on.drawn).then(|| {
                    on.drawn = now;
                    Body::Decrypting {
                        done: now.0,
                        total: now.1,
                    }
                }));
            }
            // The worker ended without an answer.
            Err(mpsc::TryRecvError::Disconnected) => {
                log("viewer: the work for a file macOS shows ended without an answer");
                self.media.mac.end();
                return Some(Some(Body::Failed));
            }
        };
        let (session, shower, max) = (on.session, on.shower, self.media.mac.max);
        let body = match answer {
            Ok((made_for, ready)) if made_for == session => {
                let mac = &mut self.media.mac;
                let shared = mac.host.clone();
                let mut host = held(&shared);
                let file = ready.path().to_path_buf();
                if shower == Shower::Player && !host.playable(&file) {
                    // Its kind plays here, this file does not: the page with the other apps. A copy is removed.
                    log("viewer: AVFoundation does not play the file");
                    drop(ready);
                    mac.on = None;
                    Body::NotPreviewable(NoPreview::PlayerRefused)
                } else if host.embed(shower, &file) {
                    let port: Arc<dyn StagePort> = Arc::new(Port {
                        host: shared.clone(),
                        file,
                        state: Mutex::new(Placed::default()),
                    });
                    if let Some(on) = mac.on.as_mut() {
                        on.inbox = None;
                        on.shown = Some(ready);
                    }
                    Body::Native(NativeDoc {
                        kind: shower.kind(),
                        stage: Some(Stage::new(port)),
                    })
                } else if host.show(&file) {
                    log("viewer: no view could be hosted in the window; the file is in the Quick Look panel");
                    if let Some(on) = mac.on.as_mut() {
                        on.inbox = None;
                        on.shown = Some(ready);
                    }
                    Body::Native(NativeDoc {
                        kind: shower.kind(),
                        stage: None,
                    })
                } else {
                    log("viewer: macOS could not show the file (no view in the window, no Quick Look panel)");
                    drop(ready);
                    mac.on = None;
                    Body::NotPreviewable(NoPreview::NoViewer)
                }
            }
            // Made ready for another session than the one this file was opened for: never shown.
            Ok(_) => {
                self.media.mac.end();
                Body::Stopped(Stopped::SessionChanged)
            }
            Err(stop) => {
                self.media.mac.end();
                return Some(match stop {
                    Stop::Refused(why) => {
                        log(format!(
                            "viewer: the system refused to download a file to Finder ({why})"
                        ));
                        Some(Body::Stopped(Stopped::NotNow))
                    }
                    Stop::Stalled { done, total } => {
                        log(format!(
                            "viewer: Finder\u{2019}s download of a file stopped moving ({done} of {total} bytes)"
                        ));
                        Some(Body::Stopped(Stopped::Offline))
                    }
                    Stop::Open(OpenFileError::TooLarge { size }) => {
                        Some(Body::NotPreviewable(NoPreview::TooBig { size, limit: max }))
                    }
                    Stop::Open(OpenFileError::OverLimit) => {
                        Some(Body::NotPreviewable(NoPreview::OverLimit { limit: max }))
                    }
                    Stop::Open(error) => {
                        if error != OpenFileError::Cancelled {
                            log(format!("viewer: a file did not open ({})", error.kind()));
                        }
                        body_of(error)
                    }
                });
            }
        };
        Some(Some(body))
    }

    /// The file is left (closed, another file opened): the view leaves the window, Finder's copy is let go and a
    /// temp copy is removed.
    pub(in crate::engine_host::viewer) fn mac_leave(&mut self) {
        self.media.mac.end();
    }
}
