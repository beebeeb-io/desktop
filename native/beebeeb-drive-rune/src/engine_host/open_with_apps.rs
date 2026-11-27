//! DW-1562's Open With: the apps Windows reports for a type, and the start of one of them on the copy of a file kept
//! on this PC (decision D-OV-14: "the handlers Windows reports for the type, no registry writes, no activation by
//! path").
//!
//! - The apps: `SHAssocEnumHandlers(<extension>, ASSOC_FILTER_RECOMMENDED)`, each handler's `GetUIName` as the words
//!   and `GetName` as its id, in Windows' order, at most [`MAX_APPS`], none that is this app's own program. Asked once
//!   per type for the run of the app, on a COM thread of its own (never the window's), and held.
//! - The start: on a COM thread of its own the handlers are listed again, the one whose `GetName` is the id picked is
//!   found (a handler gone since the menu: refused), and `IAssocHandler::Invoke` gets the kept file as a data object
//!   (`SHCreateItemFromParsingName` → `BindToHandler(BHID_DataObject)`), under the launch lock (`launch.rs`), and only
//!   while the session of the press is still the one once the lock is taken (`files_launch.rs`).
//! - Never: a registry write, `MakeDefault` / `SetAsDefaultFor`, a temporary copy, a path made from a name.
//!
//! Must not: log a path or a file's name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use crate::frames::files::source::{ActionError, AppChoice};

/// At most this many apps in the submenu.
#[cfg(any(test, windows))]
pub(crate) const MAX_APPS: usize = 8;

/// The extension Windows is asked about, as it takes it: ".docx", lower case. `None`: the name has none.
pub(crate) fn extension_of(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty() && !ext.is_empty()).then(|| format!(".{}", ext.to_lowercase()))
}

/// Is the handler with this name (`GetName`: a program's path, or its file name) this app's own program? By the
/// program's file name, without case, as Windows compares them.
fn is_own(handler: &str, own_exe: &str) -> bool {
    let file = |path: &str| path.rsplit(['\\', '/']).next().unwrap_or(path).to_lowercase();
    !own_exe.is_empty() && file(handler) == file(own_exe)
}

/// The submenu's apps from the handlers Windows reported, `(GetName, GetUIName)` in its order: none whose program is
/// `own_exe` (Beebeeb is the submenu's first line already, and is never made a type's app), none without words, at
/// most [`MAX_APPS`].
#[cfg(any(test, windows))]
pub(crate) fn choices(handlers: impl IntoIterator<Item = (String, String)>, own_exe: &str) -> Vec<AppChoice> {
    handlers
        .into_iter()
        .filter(|(name, label)| !is_own(name, own_exe) && !label.trim().is_empty())
        .take(MAX_APPS)
        .map(|(id, label)| AppChoice { id, label })
        .collect()
}

/// The apps held per type for the run of the app (`None`: asked, not answered yet).
#[derive(Clone, Default)]
pub(crate) struct Apps {
    known: Arc<Mutex<HashMap<String, Option<Vec<AppChoice>>>>>,
}

impl Apps {
    /// The apps for a file named `name`: what is held. The first time a type is asked, Windows is asked on a thread of
    /// its own, and `wake` runs once it answered (the menu is drawn again). Never waits.
    pub(crate) fn for_name(&self, name: &str, wake: impl FnOnce() + Send + 'static) -> Vec<AppChoice> {
        let Some(ext) = extension_of(name) else {
            return Vec::new();
        };
        {
            let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
            match known.get(&ext) {
                Some(Some(apps)) => return apps.clone(),
                Some(None) => return Vec::new(),
                None => {
                    known.insert(ext.clone(), None);
                }
            }
        }
        let known = self.known.clone();
        ask_windows(ext.clone(), move |apps| {
            known
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(ext, Some(apps));
            wake();
        });
        Vec::new()
    }
}

/// The friendly name of the app the system opens `ext` (".pptx") with by default, held per type for the run of the
/// app (`None` inside: asked, no name). The first ask goes to the system on a thread of its own and `wake` runs once
/// it answered; until then, and when there is no default (or it is this app), `None`: the caller says "another app".
/// Never waits.
pub(crate) fn default_app_name(ext: &str, wake: impl FnOnce() + Send + 'static) -> Option<String> {
    static KNOWN: std::sync::OnceLock<Mutex<HashMap<String, Option<Option<String>>>>> = std::sync::OnceLock::new();
    let known = KNOWN.get_or_init(Mutex::default);
    let ext = ext.to_lowercase();
    {
        let mut held = known.lock().unwrap_or_else(PoisonError::into_inner);
        match held.get(&ext) {
            Some(Some(name)) => return name.clone(),
            Some(None) => return None,
            None => {
                held.insert(ext.clone(), None);
            }
        }
    }
    let asked = ext.clone();
    let started = std::thread::Builder::new().name("default-app-name".into()).spawn(move || {
        let name = default_name(ask_default(&asked), &own_exe());
        known.lock().unwrap_or_else(PoisonError::into_inner).insert(asked, Some(name));
        wake();
    });
    if started.is_err() {
        known.lock().unwrap_or_else(PoisonError::into_inner).insert(ext, Some(None));
    }
    None
}

/// The system's answer as a name to show: none when empty or when it is this app's own program.
fn default_name(answer: Option<(String, String)>, own_exe: &str) -> Option<String> {
    let (exe, name) = answer?;
    let name = name.trim();
    (!name.is_empty() && !is_own(&exe, own_exe)).then(|| name.to_owned())
}

/// Windows: `AssocQueryStringW` for the default `open` verb of `ext`: `(executable, friendly app name)`.
#[cfg(windows)]
fn ask_default(ext: &str) -> Option<(String, String)> {
    let exe = win::assoc(ext, windows::Win32::UI::Shell::ASSOCSTR_EXECUTABLE).unwrap_or_default();
    let name = win::assoc(ext, windows::Win32::UI::Shell::ASSOCSTR_FRIENDLYAPPNAME)?;
    Some((exe, name))
}

/// macOS: the app Launch Services opens `ext` with (`NSWorkspace`), as `(its path, its name)`.
#[cfg(target_os = "macos")]
fn ask_default(ext: &str) -> Option<(String, String)> {
    mac::default_app(ext)
}

/// Every other system: not asked here.
#[cfg(not(any(windows, target_os = "macos")))]
fn ask_default(_ext: &str) -> Option<(String, String)> {
    None
}

/// macOS (dm-73): Open with, on Finder's copy of the file. The copy is made present first (a cloud-only file is
/// read, so the system fetches it as it does for Finder's own open, and waited for while the page says so), then the
/// URL the File Provider gives for the item is opened through `NSWorkspace openURLs:withApplicationAtURL:` with the
/// default app of its kind, or with the app picked in the chooser (`NSOpenPanel` on `/Applications`). Every failure
/// is words on the page ([`Failure::words`]) and a line in the log ([`Failure::log`]: the system's domain and code).
///
/// Dm-77: `NSWorkspace` is called only after the wait saw the file no longer dataless, and the no-byte limit holds
/// even when the wait itself stands still (RC12: 80 s, nothing opened, nothing said): the page then says the
/// download stopped, the log says why, and that press opens nothing any more ([`start_on`]).
///
/// Never: a shell, a child process, a temporary copy, a path made from a name. Must not log a path or a name.
#[cfg(target_os = "macos")]
pub(crate) mod mac {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use beebeeb_engine::finder_setup::error::BRIDGE_DOMAIN;

    use crate::frames::viewers::unsupported::{self, Note};
    use crate::shell::platform::macos::macos_fp::materialise::{self, Finder};
    pub(crate) use crate::shell::platform::macos::macos_fp::materialise::{Presence, STALL};

    /// `(app path, app name)` of the default app for files ending in `ext` (no dot).
    pub(crate) fn default_app(ext: &str) -> Option<(String, String)> {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::{NSString, NSURL};
        // A name of the kind is enough for Launch Services: it answers by the type, not by a file that exists.
        let probe = NSURL::fileURLWithPath(&NSString::from_str(&format!("/Beebeeb-type-probe.{ext}")));
        let app = NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&probe)?;
        let path = app.path()?.to_string();
        let name = Path::new(&path).file_stem()?.to_string_lossy().into_owned();
        Some((path, name))
    }

    /// Shows `file` in Finder through `NSWorkspace`.
    pub(crate) fn reveal(file: &Path) -> Result<(), crate::frames::files::source::ActionError> {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::{NSArray, NSString, NSURL};
        let Some(text) = file.to_str() else {
            return Err(crate::frames::files::source::ActionError::Failed);
        };
        let url = NSURL::fileURLWithPath(&NSString::from_str(text));
        #[cfg(not(test))]
        NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
        #[cfg(test)]
        let _ = (url, NSWorkspace::sharedWorkspace, NSArray::<NSURL>::new);
        Ok(())
    }

    /// Which app opens the file.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Pick {
        /// The app the Mac opens the kind with.
        Default,
        /// The app the person picks in the chooser.
        Choose,
    }

    /// The system's own answer to a call that failed: its error domain and code, never its text (that can name a
    /// file).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct SystemError {
        pub domain: String,
        pub code: i64,
    }

    /// The domain of a failure this app found itself (`real`'s codes: an id or a path that cannot be handed over, no
    /// answer in time).
    pub(crate) fn own_domain() -> &'static str {
        crate::shell::identity::engine_identity().bundle_identifier()
    }

    impl SystemError {
        pub(crate) fn new(domain: impl Into<String>, code: i64) -> Self {
            SystemError {
                domain: domain.into(),
                code,
            }
        }

        /// One line in the log: the domain, when it is one this app knows the name of, and the code (as `status`:
        /// the log masks a field named `code`, which elsewhere is a sign-in code).
        fn log(&self) {
            tracing::warn!(
                status = self.code,
                "open with: the system answered ({})",
                match self.domain.as_str() {
                    "NSFileProviderErrorDomain" => "NSFileProviderErrorDomain",
                    "NSCocoaErrorDomain" => "NSCocoaErrorDomain",
                    "NSOSStatusErrorDomain" => "NSOSStatusErrorDomain",
                    "NSPOSIXErrorDomain" => "NSPOSIXErrorDomain",
                    "NSURLErrorDomain" => "NSURLErrorDomain",
                    domain if domain == BRIDGE_DOMAIN => "the bridge",
                    domain if domain == own_domain() => "this app",
                    _ => "another domain",
                }
            );
        }
    }

    /// Why the file did not open in another app.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) enum Failure {
        /// The person closed the chooser. Nothing is said.
        Cancelled,
        /// The session of the press ended, or a newer press took over. Nothing is said.
        Moved,
        /// The File Provider refused the download.
        Download(SystemError),
        /// The download brought no byte for [`STALL`] (dm-75).
        DownloadStalled,
        /// macOS has no place for the item in Finder.
        NotInFinder,
        /// No app on this Mac opens the kind.
        NoApp,
        /// `NSWorkspace` refused the open.
        Open(SystemError),
    }

    /// What the page says when the download was refused.
    pub(crate) const NOT_DOWNLOADED: &str = concat!(
        "Couldn\u{2019}t download this file to ",
        beebeeb_engine::device_word!(),
        ", so it didn\u{2019}t open. Check your connection and try again."
    );
    /// What the page says when the download stopped moving.
    pub(crate) const NOT_MOVING: &str = concat!(
        "The download to ",
        beebeeb_engine::device_word!(),
        " stopped, so this file didn\u{2019}t open. Check your connection and try again."
    );
    /// What the page says when the item has no place in Finder.
    pub(crate) const NOT_IN_FINDER: &str =
        "Beebeeb couldn\u{2019}t find this file in Finder, so it didn\u{2019}t open. Check that Beebeeb is turned on in Finder.";
    /// What the page says when no app opens the kind.
    pub(crate) const NO_APP: &str = concat!(
        "No app on ",
        beebeeb_engine::device_word!(),
        " opens this kind of file. Choose one with Open with another app\u{2026}"
    );
    /// What the page says when the app did not take the file.
    pub(crate) const APP_REFUSED: &str = "The app didn\u{2019}t open this file. Try another app, or show it in Finder.";
    /// What the page says when the open could not even begin.
    pub(crate) const NOT_STARTED: &str = "Couldn\u{2019}t open this file in another app. Try again.";

    impl Failure {
        /// The sentence for the page. `None`: nothing is said (the person cancelled, or the press is over).
        pub(crate) fn words(&self) -> Option<&'static str> {
            Some(match self {
                Failure::Cancelled | Failure::Moved => return None,
                Failure::Download(_) => NOT_DOWNLOADED,
                Failure::DownloadStalled => NOT_MOVING,
                Failure::NotInFinder => NOT_IN_FINDER,
                Failure::NoApp => NO_APP,
                Failure::Open(_) => APP_REFUSED,
            })
        }

        /// What failed, in the log, and the system's domain and code where it gave one. Never a path, a name or the
        /// system's own text. A closed chooser and a press that is over are not failures: nothing is written.
        pub(crate) fn log(&self) {
            match self {
                Failure::Cancelled | Failure::Moved => {}
                Failure::Download(why) => {
                    tracing::warn!("open with: the file did not open, the download was refused");
                    why.log();
                }
                Failure::DownloadStalled => {
                    tracing::warn!("open with: the file did not open, the download stopped moving")
                }
                Failure::NotInFinder => tracing::warn!("open with: the file did not open, it has no place in Finder"),
                Failure::NoApp => tracing::warn!("open with: the file did not open, no app for its kind"),
                Failure::Open(why) => {
                    tracing::warn!("open with: the file did not open, NSWorkspace refused");
                    why.log();
                }
            }
        }
    }

    /// What is asked of the Mac. The app's is the File Provider bridge and `NSWorkspace` ([`system`]); a test's
    /// remembers what it was asked.
    pub(crate) trait Platform: Send + Sync {
        /// What is at Finder's place for the item now: its bytes, or a file without them (`SF_DATALESS`). Finder's
        /// real state, never the engine's own copy.
        fn presence(&self, file: &Path) -> Presence;
        /// Asks the File Provider to bring the item's bytes down, as Finder's own download does
        /// (`materialise::request_download`). `Ok`: asked; the download runs on.
        fn materialise(&self, item_id: &str) -> Result<(), SystemError>;
        /// Starts a read of Finder's copy, so the system fetches it now (`materialise::Finder::pull`).
        fn pull(&self, file: &Path);
        /// How long that read is given alone before the system is also asked ([`materialise::READ_FIRST`]).
        fn read_first(&self) -> Duration {
            materialise::READ_FIRST
        }
        /// The item is on this Mac now: the marks and the counts read Finder's state again
        /// (`materialise::Finder::landed`). Default: nothing.
        fn landed(&self, _item_id: &str) {}
        /// How far the item's download is (done, total), from the transfer board.
        fn progress(&self, item_id: &str) -> Option<(u64, u64)>;
        /// The app the person picks (`NSOpenPanel` on `/Applications`). `None`: closed without one.
        fn choose_app(&self) -> Option<PathBuf>;
        /// Finder's own place of the item, handed to `with` once while the system's access to it is held. `false`:
        /// the item has no place in Finder, and `with` was not called.
        fn finder_copy(&self, item_id: &str, with: &mut dyn FnMut(&Path)) -> bool;
        /// The app the Mac opens `file` with. `None`: none, or only Beebeeb itself.
        fn default_app(&self, file: &Path) -> Option<PathBuf>;
        /// `NSWorkspace openURLs:withApplicationAtURL:` and its answer.
        fn open_with(&self, app: &Path, file: &Path) -> Result<(), SystemError>;
        /// How long the download may go without a byte more ([`STALL`]).
        fn stall(&self) -> Duration {
            STALL
        }
    }

    /// The platform as dm-75's one call sees it; the system's refusal is kept as it was given.
    struct AsFinder<'a> {
        platform: &'a dyn Platform,
        refused: std::sync::Mutex<Option<SystemError>>,
    }

    impl Finder for AsFinder<'_> {
        fn with_copy(&self, file_id: &str, read: &mut dyn FnMut(&Path)) -> bool {
            self.platform.finder_copy(file_id, read)
        }
        fn presence(&self, file: &Path) -> Presence {
            self.platform.presence(file)
        }
        fn request(&self, file_id: &str) -> Result<(), String> {
            self.platform.materialise(file_id).map_err(|why| {
                let said = format!("{} {}", why.domain, why.code);
                *self.refused.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(why);
                said
            })
        }
        fn pull(&self, file: &Path) {
            self.platform.pull(file);
        }
        fn read_first(&self) -> Duration {
            self.platform.read_first()
        }
        fn landed(&self, file_id: &str) {
            self.platform.landed(file_id);
        }
    }

    /// [`run_watched`] with nobody to tell when the wait stands still.
    #[cfg(test)]
    pub(crate) fn run(
        platform: &dyn Platform,
        item_id: &str,
        pick: Pick,
        still: &dyn Fn() -> bool,
        say: &dyn Fn(Note),
    ) -> Result<(), Failure> {
        run_watched(platform, item_id, pick, still, say, None)
    }

    /// One Open with press, start to end, on the thread of its own that [`start`] makes: the app (asked first, so the
    /// chooser is not shown minutes after the press), then, at Finder's place for the item, its copy made present
    /// (dm-75's one call: a file that is online only is read and asked of the system, and it is waited for until it
    /// is no longer dataless) and the open. `still` is asked before every step that acts: `false` (the session of the
    /// press ended, a newer press took over) ends it with nothing opened. `say` gets what the page shows meanwhile.
    /// `stuck` is told when the wait for the file stood still for the limit inside one of its own asks (dm-77): this
    /// call cannot return then.
    pub(crate) fn run_watched(
        platform: &dyn Platform,
        item_id: &str,
        pick: Pick,
        still: &dyn Fn() -> bool,
        say: &dyn Fn(Note),
        stuck: Option<materialise::Stuck>,
    ) -> Result<(), Failure> {
        let mut stuck = stuck;
        let picked = match pick {
            Pick::Default => None,
            Pick::Choose => Some(platform.choose_app().ok_or(Failure::Cancelled)?),
        };
        if !still() {
            return Err(Failure::Moved);
        }
        let finder = AsFinder {
            platform,
            refused: std::sync::Mutex::new(None),
        };
        let mut opened = Err(Failure::NotInFinder);
        let found = platform.finder_copy(item_id, &mut |file| {
            opened = (|| {
                if platform.presence(file) == Presence::Cloud {
                    say(Note::Downloading);
                }
                let wait = materialise::Wait {
                    size: 0,
                    stall: platform.stall(),
                    still,
                    progress: &|| platform.progress(item_id),
                    report: &|_, _| {},
                };
                match materialise::arrive_watched(&finder, item_id, file, &wait, stuck.take()) {
                    Ok(true) => {}
                    Ok(false) => return Err(Failure::NotInFinder),
                    Err(materialise::Stop::Left) => return Err(Failure::Moved),
                    Err(materialise::Stop::Stalled { .. }) => return Err(Failure::DownloadStalled),
                    Err(materialise::Stop::Refused(_)) => {
                        let why = finder
                            .refused
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        return Err(Failure::Download(
                            why.unwrap_or_else(|| SystemError::new(own_domain(), 0)),
                        ));
                    }
                }
                if !still() {
                    return Err(Failure::Moved);
                }
                say(Note::Opening);
                match picked.clone().or_else(|| platform.default_app(file)) {
                    // The last look at the session, right before the app gets the file.
                    Some(_) if !still() => Err(Failure::Moved),
                    Some(app) => platform.open_with(&app, file).map_err(Failure::Open),
                    None => Err(Failure::NoApp),
                }
            })();
        });
        if found {
            opened
        } else {
            Err(Failure::NotInFinder)
        }
    }

    /// The number of the newest press: an older press's thread stops when it sees a newer one, and says nothing more.
    static NEWEST: AtomicU64 = AtomicU64::new(0);

    /// Wakes the window through the waker a press's thread and the watch over its wait share.
    fn woken<W: Fn()>(wake: &std::sync::Mutex<W>) {
        (*wake.lock().unwrap_or_else(std::sync::PoisonError::into_inner))()
    }

    /// Open with for the item `item_id`, on a thread of its own with the Mac's own platform. `still`: the session of
    /// the press is still the one. `wake`: the window draws again (the page reads its note). `Err`: nothing started;
    /// the page says so (its note is set here too). `progress`: how far the item's download is, from the transfer board.
    pub(crate) fn start(
        item_id: &str,
        pick: Pick,
        still: impl Fn() -> bool + Send + 'static,
        wake: impl Fn() + Send + 'static,
        progress: impl Fn(&str) -> Option<(u64, u64)> + Send + Sync + 'static,
    ) -> Result<(), ()> {
        start_on(system(Box::new(progress)), item_id, pick, still, wake)
    }

    /// [`start`] on `platform` (`None`: this build has none, a test's).
    pub(crate) fn start_on(
        platform: Option<std::sync::Arc<dyn Platform>>,
        item_id: &str,
        pick: Pick,
        still: impl Fn() -> bool + Send + 'static,
        wake: impl Fn() + Send + 'static,
    ) -> Result<(), ()> {
        let refuse = |item_id: &str| {
            tracing::warn!("open with: the open did not start");
            unsupported::set_note(item_id, Some(Note::Failed(NOT_STARTED.to_string())));
            Err(())
        };
        let Some(platform) = platform else {
            return refuse(item_id);
        };
        let press = NEWEST.fetch_add(1, Ordering::SeqCst) + 1;
        let id = item_id.to_string();
        // The watch over the wait wakes the window from a thread of its own.
        let wake = std::sync::Arc::new(std::sync::Mutex::new(wake));
        let started = std::thread::Builder::new()
            .name("viewer-open-with".into())
            .spawn(move || {
                let newest = || NEWEST.load(Ordering::SeqCst) == press;
                let still = || newest() && still();
                let say = |note: Note| {
                    if newest() {
                        unsupported::set_note(&id, Some(note));
                        woken(&wake);
                    }
                };
                // Dm-77: the wait for the file stood still for the limit inside one of its own asks, so this press
                // cannot say so itself. The page says the download stopped and the log says why; the press is over
                // (a newer number is the newest), so its thread, should that ask ever return, opens nothing and says
                // nothing more.
                let stuck: materialise::Stuck = {
                    let (id, wake) = (id.clone(), wake.clone());
                    Box::new(move || {
                        if NEWEST
                            .compare_exchange(press, press + 1, Ordering::SeqCst, Ordering::SeqCst)
                            .is_err()
                        {
                            return;
                        }
                        Failure::DownloadStalled.log();
                        unsupported::set_note(&id, Some(Note::Failed(NOT_MOVING.to_string())));
                        woken(&wake);
                    })
                };
                let done = run_watched(platform.as_ref(), &id, pick, &still, &say, Some(stuck));
                if let Err(why) = &done {
                    why.log();
                }
                // A newer press owns the page's words now.
                if newest() {
                    let note = done
                        .err()
                        .and_then(|why| why.words())
                        .map(|words| Note::Failed(words.to_string()));
                    unsupported::set_note(&id, note);
                    woken(&wake);
                }
            });
        match started {
            Ok(_) => Ok(()),
            Err(_) => refuse(item_id),
        }
    }

    /// How far an item's download is, asked of the engine's transfer board.
    pub(crate) type Progress = Box<dyn Fn(&str) -> Option<(u64, u64)> + Send + Sync>;

    /// The Mac's own platform. A test never reaches the File Provider, the chooser or `NSWorkspace`: none.
    fn system(progress: Progress) -> Option<std::sync::Arc<dyn Platform>> {
        #[cfg(not(test))]
        {
            Some(std::sync::Arc::new(real::System { progress }))
        }
        #[cfg(test)]
        {
            let _ = progress;
            None
        }
    }

    #[cfg(not(test))]
    mod real {
        use std::path::{Path, PathBuf};
        use std::time::Duration;

        use super::{own_domain, Presence, SystemError};
        use crate::shell::platform::macos::macos_fp::bridge;
        use crate::shell::platform::macos::macos_fp::materialise::{self, Finder};

        /// A path that is not text.
        const OWN_NO_PATH: i64 = 2;
        /// `NSWorkspace` did not answer in time.
        const OWN_NO_ANSWER: i64 = 3;

        fn own(code: i64) -> SystemError {
            SystemError::new(own_domain(), code)
        }

        /// How long `NSWorkspace` is waited for.
        const OPEN_WAIT: Duration = Duration::from_secs(20);
        /// How often the item's place is asked for while another File Provider call is in flight.
        const PLACE_TRIES: usize = 12;

        pub(super) struct System {
            pub(super) progress: super::Progress,
        }

        impl super::Platform for System {
            fn presence(&self, file: &Path) -> Presence {
                materialise::presence_on_disk(file)
            }

            fn materialise(&self, item_id: &str) -> Result<(), SystemError> {
                materialise::request_download(item_id).map_err(|error| SystemError::new(error.domain, error.code))
            }

            fn pull(&self, file: &Path) {
                materialise::SystemFinder.pull(file);
            }

            fn landed(&self, item_id: &str) {
                materialise::SystemFinder.landed(item_id);
            }

            fn progress(&self, item_id: &str) -> Option<(u64, u64)> {
                (self.progress)(item_id)
            }

            fn choose_app(&self) -> Option<PathBuf> {
                crate::shell::platform::macos_panels::pick_app()
            }

            fn finder_copy(&self, item_id: &str, with: &mut dyn FnMut(&Path)) -> bool {
                // The bridge's gate is one per process and is waited for briefly: asked again while it is busy.
                for attempt in 0..PLACE_TRIES {
                    if attempt > 0 {
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    if bridge::with_finder_copy(item_id, with) {
                        return true;
                    }
                }
                false
            }

            fn default_app(&self, file: &Path) -> Option<PathBuf> {
                use objc2_app_kit::NSWorkspace;
                use objc2_foundation::{NSString, NSURL};
                let url = NSURL::fileURLWithPath(&NSString::from_str(file.to_str()?));
                let app = NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&url)?;
                let app = PathBuf::from(app.path()?.to_string());
                // Beebeeb is never the other app.
                let own = std::env::current_exe().ok();
                let is_own = own.as_deref().is_some_and(|exe| exe.starts_with(&app));
                (!is_own).then_some(app)
            }

            fn open_with(&self, app: &Path, file: &Path) -> Result<(), SystemError> {
                use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};
                use objc2_foundation::{NSArray, NSError, NSString, NSURL};
                let (Some(app), Some(file)) = (app.to_str(), file.to_str()) else {
                    return Err(own(OWN_NO_PATH));
                };
                let app = NSURL::fileURLWithPath(&NSString::from_str(app));
                let file = NSURL::fileURLWithPath(&NSString::from_str(file));
                let (answer, answered) = std::sync::mpsc::channel::<Option<SystemError>>();
                let handler = block2::RcBlock::new(move |_app: *mut NSRunningApplication, error: *mut NSError| {
                    // SAFETY: AppKit hands a valid NSError or nil for the length of this call.
                    let why = unsafe { error.as_ref() }
                        .map(|error| SystemError::new(error.domain().to_string(), error.code() as i64));
                    let _ = answer.send(why);
                });
                // AppKit copies the block and calls it once, on a queue of its own.
                let config = NSWorkspaceOpenConfiguration::configuration();
                NSWorkspace::sharedWorkspace().openURLs_withApplicationAtURL_configuration_completionHandler(
                    &NSArray::from_retained_slice(&[file]),
                    &app,
                    &config,
                    Some(&handler),
                );
                // Waited for inside `finder_copy`: the system's access to the file is held until the app has it.
                match answered.recv_timeout(OPEN_WAIT) {
                    Ok(None) => Ok(()),
                    Ok(Some(why)) => Err(why),
                    Err(_) => Err(own(OWN_NO_ANSWER)),
                }
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "open_with_apps_mac_tests.rs"]
mod mac_tests;

/// The path of this app's own program, as Windows names a handler's.
fn own_exe() -> String {
    std::env::current_exe()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Ask Windows for the apps of `ext` on a COM thread of its own; `answer` gets them. A thread that does not start
/// answers nothing: the submenu keeps Beebeeb and Other….
#[cfg(windows)]
fn ask_windows(ext: String, answer: impl FnOnce(Vec<AppChoice>) + Send + 'static) {
    let own = own_exe();
    let _ = std::thread::Builder::new()
        .name("files-open-with-apps".into())
        .spawn(move || {
            let apps = match win::Com::enter() {
                Some(_com) => choices(win::handlers(&ext).iter().filter_map(win::names), &own),
                None => Vec::new(),
            };
            answer(apps);
        });
}

/// Every other system: Windows' handlers are not there, so there are none.
#[cfg(not(windows))]
fn ask_windows(_ext: String, answer: impl FnOnce(Vec<AppChoice>) + Send + 'static) {
    let _ = own_exe;
    answer(Vec::new());
}

/// What starts the app picked on the kept copy, on the thread [`open_in`] makes: the path, the app's id, and `still`,
/// which it asks inside the launch lock right before the start (`false`: the press's session moved on; nothing
/// starts). The system's own is [`SYSTEM`]; a test hands a stand-in.
pub(crate) type Invoke = fn(&Path, &str, &dyn Fn() -> bool) -> Result<(), ActionError>;

/// Windows' handlers, with COM set up on the thread that starts one.
#[cfg(windows)]
pub(crate) const SYSTEM: Option<Invoke> = Some(invoke_with_com);
/// Every other system has no handlers to start.
#[cfg(not(windows))]
pub(crate) const SYSTEM: Option<Invoke> = None;

#[cfg(windows)]
fn invoke_with_com(path: &Path, app_id: &str, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    match win::Com::enter() {
        Some(_com) => win::invoke(path, app_id, still),
        None => Err(ActionError::Failed),
    }
}

/// Open `path`, the copy kept on this PC, in the handler whose `GetName` is `app_id`, with `invoke` on a thread of its
/// own. `Ok`: the thread started, and `done` gets what happened (a handler gone since the menu, a start that failed,
/// or a session that moved on before the lock was taken: an `Err`). `None` (every other system): not there.
pub(crate) fn open_in(
    invoke: Option<Invoke>,
    path: PathBuf,
    app_id: String,
    still: impl Fn() -> bool + Send + 'static,
    done: impl FnOnce(Result<(), ActionError>) + Send + 'static,
) -> Result<(), ActionError> {
    let Some(invoke) = invoke else {
        return Err(ActionError::NotOnThisSystem);
    };
    std::thread::Builder::new()
        .name("files-open-in-app".into())
        .spawn(move || done(invoke(&path, &app_id, &still)))
        .map(|_| ())
        .map_err(|_| ActionError::Failed)
}

#[cfg(windows)]
mod win {
    use std::path::Path;

    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoTaskMemFree, CoUninitialize, IDataObject,
    };
    use windows::Win32::UI::Shell::{
        ASSOC_FILTER_RECOMMENDED, BHID_DataObject, IAssocHandler, IShellItem, SHAssocEnumHandlers,
        SHCreateItemFromParsingName,
    };
    use windows::core::{HSTRING, PWSTR};

    use crate::engine_host::launch;
    use crate::frames::files::source::ActionError;

    /// COM on this thread while it lives: taken back only when this call was the one that set it up.
    pub(super) struct Com(bool);

    impl Com {
        pub(super) fn enter() -> Option<Com> {
            // SAFETY: a plain COM set-up call for the calling thread, paired with `CoUninitialize` in `Drop` exactly
            // when it succeeded.
            let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
            if result.is_ok() {
                Some(Com(true))
            } else if result == RPC_E_CHANGED_MODE {
                Some(Com(false))
            } else {
                None
            }
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: pairs the successful `CoInitializeEx` of `enter`, on the same thread.
                unsafe { CoUninitialize() };
            }
        }
    }

    /// The handlers Windows recommends for `ext` (".docx"), in its order. None when it reports none.
    pub(super) fn handlers(ext: &str) -> Vec<IAssocHandler> {
        // SAFETY: COM is set up on this thread by the caller (`Com`). Each interface is owned by its wrapper and
        // released when it drops. `Next` writes at most one handler into the one slot it is given.
        unsafe {
            let Ok(list) = SHAssocEnumHandlers(&HSTRING::from(ext), ASSOC_FILTER_RECOMMENDED) else {
                return Vec::new();
            };
            let mut out = Vec::new();
            // A bound, in case a list never ends: far more than the submenu shows.
            while out.len() < 64 {
                let mut one: [Option<IAssocHandler>; 1] = [None];
                let mut fetched = 0u32;
                if list.Next(&mut one, Some(&raw mut fetched)).is_err() || fetched == 0 {
                    break;
                }
                if let Some(handler) = one[0].take() {
                    out.push(handler);
                }
            }
            out
        }
    }

    /// One string of the default association of `ext` (its `open` verb), as `AssocQueryStringW` answers it. `None`:
    /// no association, or an empty answer.
    pub(super) fn assoc(ext: &str, what: windows::Win32::UI::Shell::ASSOCSTR) -> Option<String> {
        use windows::Win32::UI::Shell::{ASSOCF_INIT_IGNOREUNKNOWN, ASSOCF_NOTRUNCATE, AssocQueryStringW};
        use windows::core::w;
        let ext = HSTRING::from(ext);
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        // SAFETY: `buf` is `len` u16s long and outlives the call; the strings are nul-terminated HSTRINGs / literals.
        // The call writes at most `len` u16s and sets `len` to the count written, the nul included.
        let ok = unsafe {
            AssocQueryStringW(
                ASSOCF_INIT_IGNOREUNKNOWN | ASSOCF_NOTRUNCATE,
                what,
                &ext,
                w!("open"),
                windows::core::PWSTR(buf.as_mut_ptr()),
                &raw mut len,
            )
        };
        if ok.is_err() {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let text = String::from_utf16_lossy(&buf[..end]);
        (!text.trim().is_empty()).then_some(text)
    }

    /// A handler's `(GetName, GetUIName)`. `None`: it gives one of them not.
    pub(super) fn names(handler: &IAssocHandler) -> Option<(String, String)> {
        // SAFETY: COM is set up on this thread by the caller. Each string Windows allocates is copied, then freed
        // with `CoTaskMemFree`, as the two calls' contract says.
        unsafe {
            let name = take(handler.GetName().ok()?);
            let label = take(handler.GetUIName().ok()?);
            Some((name, label))
        }
    }

    /// The text of a string Windows allocated, which is freed here.
    unsafe fn take(text: PWSTR) -> String {
        if text.is_null() {
            return String::new();
        }
        // SAFETY: a non-null, NUL-ended string from a COM call that hands its ownership to the caller.
        unsafe {
            let copy = String::from_utf16_lossy(text.as_wide());
            CoTaskMemFree(Some(text.0 as *const _));
            copy
        }
    }

    /// Start the handler named `app_id` on the file at `path` (the copy kept on this PC). The handlers are listed again
    /// here: one gone since the menu was drawn is refused. It starts only while `still()` holds inside the launch lock.
    pub(super) fn invoke(path: &Path, app_id: &str, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ext = super::extension_of(&name).ok_or(ActionError::Failed)?;
        let handler = handlers(&ext)
            .into_iter()
            .find(|handler| names(handler).is_some_and(|(name, _)| name == app_id))
            .ok_or(ActionError::Failed)?;
        // SAFETY: COM is set up on this thread by the caller. The path is one wide string (never a command line);
        // every interface is owned by its wrapper and released when it drops.
        unsafe {
            let item: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).map_err(|_| ActionError::Failed)?;
            let data: IDataObject = item
                .BindToHandler(None, &BHID_DataObject)
                .map_err(|_| ActionError::Failed)?;
            launch::locked(|| still().then(|| handler.Invoke(&data)))
                .ok_or(ActionError::Unavailable)?
                .map_err(|_| ActionError::Failed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handler(name: &str, label: &str) -> (String, String) {
        (name.to_string(), label.to_string())
    }

    #[test]
    fn apps_for_leaves_out_beebeeb_and_caps_the_list() {
        let own = r"C:\Program Files\Beebeeb\Beebeeb.exe";
        let mut reported: Vec<(String, String)> = (1..=9)
            .map(|n| handler(&format!(r"C:\Apps\app{n}.exe"), &format!("App {n}")))
            .collect();
        // The app's own program, reported third (another path, another case: still ours).
        reported.insert(2, handler(r"c:\program files\beebeeb\BEEBEEB.EXE", "Beebeeb"));
        assert_eq!(reported.len(), 10);
        let apps = choices(reported, own);
        assert_eq!(apps.len(), MAX_APPS);
        assert!(
            apps.iter().all(|app| !app.id.to_lowercase().ends_with("beebeeb.exe")),
            "{apps:?}"
        );
        // Windows' order is kept: app1, app2, then app3 (ours was between them).
        let labels: Vec<&str> = apps.iter().map(|app| app.label.as_str()).collect();
        assert_eq!(
            labels,
            ["App 1", "App 2", "App 3", "App 4", "App 5", "App 6", "App 7", "App 8"]
        );
        assert_eq!(apps[0].id, r"C:\Apps\app1.exe");
        // A handler with no words is not a line of the menu.
        assert_eq!(choices([handler("a.exe", " ")], own), []);
    }

    #[test]
    fn default_app_name_leaves_out_this_app_and_empty_names() {
        let own = r"C:\Program Files\Beebeeb\beebeeb-drive.exe";
        let ppt = r"C:\Program Files\Microsoft Office\root\Office16\POWERPNT.EXE";
        assert_eq!(default_name(Some((ppt.into(), "PowerPoint".into())), own), Some("PowerPoint".into()));
        assert_eq!(default_name(Some((own.into(), "Beebeeb".into())), own), None);
        assert_eq!(default_name(Some((ppt.into(), "  ".into())), own), None);
        assert_eq!(default_name(None, own), None);
    }

    #[test]
    fn open_with_extension_is_windows_own_form() {
        assert_eq!(extension_of("Service agreement.docx").as_deref(), Some(".docx"));
        assert_eq!(extension_of("REPORT.XLSX").as_deref(), Some(".xlsx"));
        assert_eq!(extension_of("notes"), None);
        assert_eq!(extension_of(".hidden"), None);
    }

    #[test]
    fn open_with_apps_are_asked_once_per_type_and_held() {
        let apps = Apps::default();
        let (woke, heard) = std::sync::mpsc::channel();
        let first = apps.for_name("a.docx", move || woke.send(()).unwrap());
        assert_eq!(first, [], "nothing is held before Windows answered");
        heard
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the answer wakes the window");
        let (woke, heard) = std::sync::mpsc::channel::<()>();
        apps.for_name("b.DOCX", move || woke.send(()).unwrap());
        assert!(
            heard.recv_timeout(std::time::Duration::from_millis(200)).is_err(),
            "a type is asked once"
        );
    }

    /// The instrument first, on a type every Windows has: `.txt` has handlers, Notepad among them.
    #[cfg(windows)]
    #[test]
    #[ignore = "O-CP3: the Windows test machine"]
    fn windows_reports_handlers_for_txt() {
        let _com = win::Com::enter().expect("COM");
        let reported: Vec<(String, String)> = win::handlers(".txt").iter().filter_map(win::names).collect();
        assert!(!reported.is_empty(), "Windows reports no handler for .txt");
        assert!(
            reported.iter().any(|(name, _)| name.to_lowercase().contains("notepad")),
            "{reported:?}"
        );
    }

    /// A scratch `.txt` in a temp root opened in Notepad through its handler: a Notepad process appears, and is ended
    /// by its PID. Mutant: invoke with an empty data object → no process.
    #[cfg(windows)]
    #[test]
    #[ignore = "O-CP3: the Windows test machine"]
    fn invoke_opens_the_kept_path() {
        let notepads = || -> Vec<u32> {
            let out = std::process::Command::new("tasklist")
                .args(["/FI", "IMAGENAME eq notepad.exe", "/FO", "CSV", "/NH"])
                .output()
                .expect("tasklist");
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| line.split("\",\"").nth(1)?.parse().ok())
                .collect()
        };
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("open-with scratch.txt");
        std::fs::write(&path, "open with").unwrap();
        let before = notepads();
        let _com = win::Com::enter().expect("COM");
        let notepad = win::handlers(".txt")
            .iter()
            .filter_map(win::names)
            .find(|(name, _)| name.to_lowercase().contains("notepad"))
            .expect("Notepad is a handler of .txt")
            .0;
        win::invoke(&path, &notepad, &|| true).expect("invoked");
        let started = (0..50)
            .find_map(|_| {
                std::thread::sleep(std::time::Duration::from_millis(100));
                notepads().into_iter().find(|pid| !before.contains(pid))
            })
            .expect("a Notepad process appeared");
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &started.to_string(), "/F"])
            .output();
        assert!(
            win::invoke(&path, "no such handler", &|| true).is_err(),
            "a handler that is not there is refused"
        );
    }
}
