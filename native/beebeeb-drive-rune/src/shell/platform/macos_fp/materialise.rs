//! Dm-75: the ONE "make Finder's copy present" call. The viewer (dm-74), Open with (dm-73), the Files list's Open
//! and keep all come here; none of them downloads a copy of its own on a Mac.
//!
//! What it does, at Finder's own place for the item (the scoped URL `bridge::with_finder_copy` hands out):
//! 1. looks at the file's attributes ([`presence_on_disk`]: `SF_DATALESS`), which downloads nothing;
//! 2. a file that is online only is READ at its first byte, on a thread of its own ([`Finder::pull`]). A read is what
//!    Finder's own open makes, and the system fetches for a reader at once (a shell's read of a dataless file brought
//!    it in 0.4 s);
//! 3. dm-77: only when the read alone brought no byte for [`Finder::read_first`] is the system also asked:
//!    `NSFileProviderManager requestDownloadForItemWithIdentifier:requestedRange:completionHandler:` with the whole
//!    file as the range ([`Finder::request`]). That request is a scheduling hint ("at the earliest convenient time",
//!    `NSFileProviderManager.h`): RC11 asked and the file stayed online only for 80 s, and RC12 asked first and read
//!    second and a small file stayed online only just the same. Why is not known; the one read known to bring a
//!    file at once had no request before it, so the read goes first, alone. Both end in the extension's
//!    `fetchContents`, the one download;
//! 4. waits until the file is no longer dataless. How far it is comes from the transfer board (the caller's
//!    `progress`). No byte more for [`Wait::stall`] ends it ([`Stop::Stalled`]). Dm-77: the limit holds even when
//!    the wait itself stands still inside one of its own asks (RC12: 80 s, no limit, no line): a watch on a thread of
//!    its own says so in the log and tells the caller ([`arrive_watched`]'s `stuck`);
//! 5. dm-77: a file that came is told to the finder ([`Finder::landed`]): the rows' marks and the counts read
//!    Finder's state again (`engine_host::macos_downloaded::finder_state`).
//!
//! Every step is logged, by its kind: never a name, an id, a path or the system's own text.

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long a download may go without a byte more before it is given up.
pub(crate) const STALL: Duration = Duration::from_secs(60);
/// How often the download is looked at.
pub(crate) const LOOK_EVERY: Duration = Duration::from_millis(200);
/// How long the read of Finder's copy is given alone before the system is also asked for the download.
pub(crate) const READ_FIRST: Duration = Duration::from_millis(1500);

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
pub(crate) fn presence_on_disk(file: &Path) -> Presence {
    use std::os::macos::fs::MetadataExt;
    match std::fs::symlink_metadata(file) {
        Ok(meta) if !meta.is_file() => Presence::Gone,
        Ok(meta) if meta.st_flags() & SF_DATALESS != 0 => Presence::Cloud,
        Ok(_) => Presence::Here,
        Err(_) => Presence::Gone,
    }
}

/// Finder's copies, as the File Provider hands them out. Asked on a worker (the calls wait for the system). The
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
    /// Starts a read of the file at Finder's place, on a thread of its own: the system fetches a file that is read
    /// now, whatever becomes of a request. Never waited for. Default: nothing (a test's finder has no system).
    fn pull(&self, _file: &Path) {}
    /// How long the read is given alone before the system is also asked ([`READ_FIRST`] in the app). Default: no
    /// time, so a test's finder is asked at the first look.
    fn read_first(&self) -> Duration {
        Duration::ZERO
    }
    /// The file `file_id` is on this Mac, seen by its own attributes: the marks and the counts drawn from Finder's
    /// state are read again. Default: nothing.
    fn landed(&self, _file_id: &str) {}
}

/// The one place the app asks the system to download an item (`beebeeb_fp_request_download`:
/// `requestDownloadForItemWithIdentifier:requestedRange:completionHandler:`, the whole file). The viewer, Open with,
/// the list's Open and keep all ask here. `Ok`: the system took the request, or did not answer within the bridge's
/// limit (it may still have taken it; the wait for the file tells). `Err`: the system's own answer. Waits up to ten
/// seconds: a worker's. Logged by its kind.
#[cfg(not(test))]
pub(crate) fn request_download(file_id: &str) -> Result<(), beebeeb_engine::finder_setup::error::FpError> {
    use beebeeb_engine::finder_setup::error::{bridge_code, FpError, BRIDGE_DOMAIN};

    use super::bridge::BeebeebFpErrorC;
    unsafe extern "C" {
        /// 0 = the system took the request.
        fn beebeeb_fp_request_download(identifier: *const std::ffi::c_char, out_error: *mut BeebeebFpErrorC) -> i32;
    }
    let Ok(id) = std::ffi::CString::new(file_id) else {
        return Err(FpError::new(
            BRIDGE_DOMAIN,
            bridge_code::NO_IDENTIFIER,
            "the item identifier holds a NUL byte",
        ));
    };
    let mut error = BeebeebFpErrorC::zeroed();
    // SAFETY: `id` is a NUL-terminated string that outlives the call; `error` is the bridge's own struct.
    if unsafe { beebeeb_fp_request_download(id.as_ptr(), &mut error) } == 0 {
        tracing::info!("materialise: the system took the download request");
        return Ok(());
    }
    let error = error.into_fp_error();
    if error.domain == BRIDGE_DOMAIN && error.code == bridge_code::SIGNAL_TIMEOUT {
        tracing::warn!("materialise: the system did not answer the download request in time; waiting for the file");
        return Ok(());
    }
    tracing::warn!(
        "materialise: the system refused the download request ({})",
        error.redacted()
    );
    Err(error)
}

/// The File Provider bridge. A test never reaches it.
#[cfg(not(test))]
pub(crate) struct SystemFinder;

#[cfg(not(test))]
impl Finder for SystemFinder {
    fn with_copy(&self, file_id: &str, read: &mut dyn FnMut(&Path)) -> bool {
        super::bridge::with_finder_copy(file_id, read)
    }

    fn request(&self, file_id: &str) -> Result<(), String> {
        request_download(file_id).map_err(|error| error.redacted())
    }

    fn pull(&self, file: &Path) {
        let file = file.to_path_buf();
        let started = std::thread::Builder::new()
            .name("finder-copy-read".into())
            .spawn(move || {
                use std::io::Read;
                let asked = Instant::now();
                // The read waits in the system until the file's bytes are here, or the system gives the download up.
                let read = std::fs::File::open(&file).and_then(|mut file| file.read(&mut [0_u8; 1]));
                let ms = asked.elapsed().as_millis() as u64;
                match read {
                    Ok(_) => tracing::info!(ms, "materialise: the read of Finder\u{2019}s copy answered"),
                    Err(error) => tracing::warn!(
                        ms,
                        status = error.raw_os_error().unwrap_or(0),
                        "materialise: the read of Finder\u{2019}s copy was refused"
                    ),
                }
            });
        if started.is_err() {
            tracing::warn!("materialise: no thread for the read of Finder\u{2019}s copy");
        }
    }

    fn read_first(&self) -> Duration {
        READ_FIRST
    }

    fn landed(&self, file_id: &str) {
        crate::engine_host::macos_downloaded::finder_state::landed(file_id);
    }
}

/// Why Finder's copy did not get here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    /// The caller left (`still` answered `false`).
    Left,
    /// The system refused the download: its domain and code.
    Refused(String),
    /// No byte more for [`Wait::stall`]: how far it was.
    Stalled { done: u64, total: u64 },
}

/// How one wait for Finder's copy goes.
pub(crate) struct Wait<'a> {
    /// The file's size, for the last report and for a board that does not know the total yet.
    pub size: u64,
    /// How long the download may stand still ([`STALL`]).
    pub stall: Duration,
    /// The caller still wants the file. `false` ends the wait ([`Stop::Left`]).
    pub still: &'a dyn Fn() -> bool,
    /// How far the download is (done, total; a total of 0 is not known yet), from the transfer board.
    pub progress: &'a dyn Fn() -> Option<(u64, u64)>,
    /// Told how far it is (done, total), each time it is looked at.
    pub report: &'a dyn Fn(u64, u64),
}

/// What the wait is asking when it beats: the watch names it when no beat follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// The caller's `still`.
    Still,
    /// The file's attributes.
    File,
    /// The caller's `progress`.
    Bytes,
    /// The system, for the download.
    System,
}

/// What the caller is told when the wait itself stood still for the limit: called once, on the watch's thread. The
/// wait cannot end itself then (it is inside a call that does not return); the caller says what the person sees.
pub(crate) type Stuck = Box<dyn FnOnce() + Send>;

/// The watch over one wait: a thread of its own that hears a beat before every ask the wait makes. No beat for the
/// limit means the wait stands still inside that ask; the watch says so in the log and tells the caller. It ends
/// with the wait.
struct Watch {
    beats: Option<mpsc::Sender<Step>>,
}

impl Watch {
    fn start(stall: Duration, stuck: Option<Stuck>) -> Watch {
        let (beats, heard) = mpsc::channel::<Step>();
        // The wait sleeps between its looks: the watch is longer than the limit by one such sleep.
        let limit = stall + LOOK_EVERY.min(stall);
        let started = std::thread::Builder::new()
            .name("finder-copy-watch".into())
            .spawn(move || {
                let mut at = Step::Still;
                loop {
                    match heard.recv_timeout(limit) {
                        Ok(step) => at = step,
                        // The wait is over.
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                    }
                }
                tracing::warn!(
                    secs = stall.as_secs(),
                    "materialise: the wait itself stopped while it asked ({}); the download is given up",
                    match at {
                        Step::Still => "the wait is still on",
                        Step::File => "the file",
                        Step::Bytes => "the bytes done",
                        Step::System => "the system for the download",
                    }
                );
                if let Some(stuck) = stuck {
                    stuck();
                }
            });
        Watch {
            beats: started.is_ok().then_some(beats),
        }
    }

    fn beat(&self, step: Step) {
        if let Some(beats) = &self.beats {
            // A watch that already spoke hears no more.
            let _ = beats.send(step);
        }
    }
}

/// Finder's copy at `place` is on this Mac when this answers `Ok(true)`: it was, or it was read (and, when the read
/// alone brought nothing, asked of the system) and is no longer dataless. `Ok(false)`: no plain file at Finder's
/// place. Call it inside [`Finder::with_copy`]'s read, where `place` may be looked at. Blocks: a worker's.
pub(crate) fn arrive(finder: &dyn Finder, file_id: &str, place: &Path, wait: &Wait<'_>) -> Result<bool, Stop> {
    arrive_watched(finder, file_id, place, wait, None)
}

/// [`arrive`], and `stuck` is told when the wait itself stood still for the limit inside one of its own asks (it
/// cannot return then).
pub(crate) fn arrive_watched(
    finder: &dyn Finder,
    file_id: &str,
    place: &Path,
    wait: &Wait<'_>,
    stuck: Option<Stuck>,
) -> Result<bool, Stop> {
    match finder.presence(place) {
        Presence::Here => {
            // A row may still draw it as online only: Finder's state is read again.
            finder.landed(file_id);
            return Ok(true);
        }
        Presence::Gone => {
            tracing::warn!("materialise: no plain file at Finder\u{2019}s place");
            return Ok(false);
        }
        Presence::Cloud => {}
    }
    tracing::info!(
        bytes = wait.size,
        "materialise: Finder\u{2019}s copy is online only; asking the system for it"
    );
    let watch = Watch::start(wait.stall, stuck);
    // The read first: the system fetches for a reader now. The request is a hint it may leave for later.
    finder.pull(place);
    let read_first = finder.read_first();
    // When the file was read, when the download last moved, how far it was then, whether a byte was seen, and
    // whether the system was also asked.
    let asked = Instant::now();
    let (mut moved, mut done, mut seen, mut requested) = (asked, 0_u64, false, false);
    loop {
        watch.beat(Step::Still);
        if !(wait.still)() {
            tracing::info!("materialise: the wait was left before Finder\u{2019}s copy was here");
            return Err(Stop::Left);
        }
        watch.beat(Step::File);
        match finder.presence(place) {
            Presence::Here => {
                (wait.report)(wait.size, wait.size);
                if !requested {
                    tracing::info!("materialise: the read alone brought the file; the system was not asked");
                }
                tracing::info!(
                    ms = asked.elapsed().as_millis() as u64,
                    "materialise: Finder\u{2019}s copy is on this Mac"
                );
                finder.landed(file_id);
                return Ok(true);
            }
            Presence::Gone => {
                tracing::warn!("materialise: the file left Finder\u{2019}s place while it was downloading");
                return Ok(false);
            }
            Presence::Cloud => {}
        }
        watch.beat(Step::Bytes);
        if let Some((now, total)) = (wait.progress)() {
            (wait.report)(now, if total == 0 { wait.size } else { total });
            if now != done {
                if !seen {
                    tracing::info!(
                        ms = asked.elapsed().as_millis() as u64,
                        "materialise: the download is moving"
                    );
                }
                (moved, done, seen) = (Instant::now(), now, true);
            }
        }
        // The read alone brought no byte for its time: the system is asked too, once.
        if !requested && moved.elapsed() >= read_first {
            requested = true;
            if !seen {
                tracing::info!("materialise: the read alone brought no byte yet; asking the system for the download");
            }
            watch.beat(Step::System);
            // A refusal is the system's answer for the file (dm-75): it ends the wait, and the page says so.
            if let Err(why) = finder.request(file_id) {
                return Err(Stop::Refused(why));
            }
        }
        if moved.elapsed() >= wait.stall {
            tracing::warn!(
                done,
                total = wait.size,
                secs = wait.stall.as_secs(),
                "materialise: no byte more in time; the download is given up"
            );
            return Err(Stop::Stalled { done, total: wait.size });
        }
        std::thread::sleep(LOOK_EVERY.min(wait.stall));
    }
}

/// How often Finder's place for a file is asked for while another File Provider call holds the bridge's gate.
const PLACE_TRIES: usize = 8;

/// The whole call for a caller that holds no place yet: Finder's place for `file_id` is asked for, the copy is made
/// present there ([`arrive`]) and `then` is called with it while the system's access to the place is held.
/// `Ok(false)`: the item has no place in Finder (or no plain file there) and `then` was not called.
pub(crate) fn make_present(
    finder: &dyn Finder,
    file_id: &str,
    wait: &Wait<'_>,
    then: &mut dyn FnMut(&Path),
) -> Result<bool, Stop> {
    let mut answer = Ok(false);
    for attempt in 0..PLACE_TRIES {
        if !(wait.still)() {
            return Err(Stop::Left);
        }
        if attempt > 0 {
            std::thread::sleep(LOOK_EVERY);
        }
        let found = finder.with_copy(file_id, &mut |place| {
            answer = arrive(finder, file_id, place, wait);
            if answer == Ok(true) && (wait.still)() {
                then(place);
            }
        });
        if found {
            return answer;
        }
    }
    tracing::warn!("materialise: the item has no place in Finder");
    Ok(false)
}

#[cfg(test)]
pub(crate) mod fake {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;

    /// A finder over one place: what is there is a value the test sets, and every ask is remembered.
    pub(crate) struct FakeFinder {
        pub place: Option<PathBuf>,
        pub presence: Mutex<Presence>,
        /// The looks after the ask that still answer online only, before the file is here (`None`: it never is).
        pub arrives_after: Option<usize>,
        /// The ask the system serves: the read (`true`), as a Mac does for a reader, or only the request.
        pub read_serves: bool,
        /// How long the read is given alone.
        pub read_first: Duration,
        pub refuse: Option<String>,
        pub requested: Mutex<Vec<String>>,
        pub pulled: AtomicUsize,
        pub landed: Mutex<Vec<String>>,
        looks: AtomicUsize,
    }

    impl FakeFinder {
        pub fn new(place: Option<&Path>, presence: Presence, arrives_after: Option<usize>) -> Self {
            FakeFinder {
                place: place.map(Path::to_path_buf),
                presence: Mutex::new(presence),
                arrives_after,
                read_serves: false,
                read_first: Duration::ZERO,
                refuse: None,
                requested: Mutex::new(Vec::new()),
                pulled: AtomicUsize::new(0),
                landed: Mutex::new(Vec::new()),
                looks: AtomicUsize::new(0),
            }
        }

        pub fn requested(&self) -> Vec<String> {
            self.requested.lock().unwrap().clone()
        }
    }

    impl Finder for FakeFinder {
        fn with_copy(&self, _file_id: &str, read: &mut dyn FnMut(&Path)) -> bool {
            match &self.place {
                Some(place) => {
                    read(place);
                    true
                }
                None => false,
            }
        }

        fn presence(&self, _file: &Path) -> Presence {
            let mut presence = self.presence.lock().unwrap();
            // The system brings the file down only once it was asked, by the ask it serves.
            let asked = if self.read_serves {
                self.pulled.load(Ordering::SeqCst) > 0
            } else {
                !self.requested.lock().unwrap().is_empty()
            };
            if *presence == Presence::Cloud && asked {
                let looks = self.looks.fetch_add(1, Ordering::SeqCst);
                if self.arrives_after.is_some_and(|after| looks >= after) {
                    *presence = Presence::Here;
                }
            }
            *presence
        }

        fn request(&self, file_id: &str) -> Result<(), String> {
            self.requested.lock().unwrap().push(file_id.to_string());
            match &self.refuse {
                Some(why) => Err(why.clone()),
                None => Ok(()),
            }
        }

        fn pull(&self, _file: &Path) {
            self.pulled.fetch_add(1, Ordering::SeqCst);
        }

        fn read_first(&self) -> Duration {
            self.read_first
        }

        fn landed(&self, file_id: &str) {
            self.landed.lock().unwrap().push(file_id.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use super::fake::FakeFinder;
    use super::*;

    fn wait<'a>(
        stall: Duration,
        still: &'a dyn Fn() -> bool,
        progress: &'a dyn Fn() -> Option<(u64, u64)>,
        report: &'a dyn Fn(u64, u64),
    ) -> Wait<'a> {
        Wait {
            size: 384,
            stall,
            still,
            progress,
            report,
        }
    }

    #[test]
    fn a_dataless_item_is_asked_of_the_system_by_its_identifier_and_handed_over_only_once_it_is_here() {
        let place = Path::new("/finder/IMG.MOV");
        let finder = FakeFinder::new(Some(place), Presence::Cloud, Some(3));
        let opened_while = std::sync::Mutex::new(Vec::new());
        let reported = AtomicU64::new(0);
        let got = make_present(
            &finder,
            "item-7",
            &wait(Duration::from_secs(5), &|| true, &|| None, &|done, _| {
                reported.store(done, Ordering::SeqCst)
            }),
            &mut |at| {
                opened_while
                    .lock()
                    .unwrap()
                    .push((at.to_path_buf(), *finder.presence.lock().unwrap()))
            },
        );
        assert_eq!(got, Ok(true));
        assert_eq!(
            finder.requested(),
            ["item-7"],
            "requestDownload, once, with the item's identifier"
        );
        assert_eq!(
            finder.pulled.load(Ordering::SeqCst),
            1,
            "and the file is read, so the system fetches now"
        );
        assert_eq!(
            *opened_while.lock().unwrap(),
            [(place.to_path_buf(), Presence::Here)],
            "handed over once, and only when the file was no longer dataless"
        );
        assert_eq!(
            reported.load(Ordering::SeqCst),
            384,
            "the last report is the whole file"
        );
        assert_eq!(
            *finder.landed.lock().unwrap(),
            ["item-7"],
            "and the marks drawn from Finder\u{2019}s state are told the file landed"
        );
    }

    /// Dm-77, RC12: the system "took" the request and never served it. The read is what brings the file, and the
    /// file is handed over only once it is no longer dataless.
    #[test]
    fn a_request_the_system_took_but_never_served_still_forces_a_read_and_hands_over_only_a_file_that_is_here() {
        let place = Path::new("/finder/logo.eps");
        let mut finder = FakeFinder::new(Some(place), Presence::Cloud, Some(2));
        // Only a read brings the file down: the request is taken (`Ok`) and nothing follows from it.
        finder.read_serves = true;
        let handed = std::sync::Mutex::new(Vec::new());
        let got = make_present(
            &finder,
            "item-eps",
            &wait(Duration::from_secs(5), &|| true, &|| None, &|_, _| {}),
            &mut |_| handed.lock().unwrap().push(*finder.presence.lock().unwrap()),
        );
        assert_eq!(got, Ok(true));
        assert_eq!(finder.requested(), ["item-eps"], "the request was made and taken");
        assert_eq!(
            finder.pulled.load(Ordering::SeqCst),
            1,
            "and the file was read all the same"
        );
        assert_eq!(
            *handed.lock().unwrap(),
            [Presence::Here],
            "handed over once, no longer dataless"
        );
        assert_eq!(*finder.landed.lock().unwrap(), ["item-eps"]);

        // Without the read the same system never brings it: the limit ends the wait, nothing is handed over.
        struct NoRead(FakeFinder);
        impl Finder for NoRead {
            fn with_copy(&self, file_id: &str, read: &mut dyn FnMut(&Path)) -> bool {
                self.0.with_copy(file_id, read)
            }
            fn presence(&self, file: &Path) -> Presence {
                self.0.presence(file)
            }
            fn request(&self, file_id: &str) -> Result<(), String> {
                self.0.request(file_id)
            }
        }
        let mut inner = FakeFinder::new(Some(place), Presence::Cloud, Some(2));
        inner.read_serves = true;
        let got = make_present(
            &NoRead(inner),
            "item-eps",
            &wait(Duration::from_millis(60), &|| true, &|| None, &|_, _| {}),
            &mut |_| panic!("never handed over"),
        );
        assert_eq!(got, Err(Stop::Stalled { done: 0, total: 384 }));
    }

    #[test]
    fn the_read_goes_first_and_a_file_it_brings_in_its_time_is_never_asked_of_the_system() {
        let mut finder = FakeFinder::new(Some(Path::new("/finder/logo.eps")), Presence::Cloud, Some(1));
        finder.read_serves = true;
        finder.read_first = Duration::from_secs(30);
        let mut opened = 0;
        let got = make_present(
            &finder,
            "item-eps",
            &wait(Duration::from_secs(60), &|| true, &|| None, &|_, _| {}),
            &mut |_| opened += 1,
        );
        assert_eq!((got, opened), (Ok(true), 1));
        assert_eq!(finder.pulled.load(Ordering::SeqCst), 1);
        assert!(finder.requested().is_empty(), "the read brought it: no request");

        // A read that brings no byte for its time: the system is asked too, once, and the wait goes on.
        let mut finder = FakeFinder::new(Some(Path::new("/finder/IMG.MOV")), Presence::Cloud, Some(1));
        finder.read_first = LOOK_EVERY;
        let got = make_present(
            &finder,
            "item-7",
            &wait(Duration::from_secs(5), &|| true, &|| None, &|_, _| {}),
            &mut |_| {},
        );
        assert_eq!(got, Ok(true));
        assert_eq!(finder.requested(), ["item-7"]);
    }

    /// Dm-77, RC12: 80 s went by with no limit and no line. A wait that stands still inside one of its own asks
    /// cannot end itself; the watch tells the caller at the limit, while that ask has still not returned.
    #[test]
    fn a_wait_that_stands_still_inside_an_ask_is_told_to_the_caller_at_the_limit() {
        let finder = FakeFinder::new(Some(Path::new("/finder/logo.eps")), Presence::Cloud, None);
        let told = std::sync::Arc::new(std::sync::Mutex::new(None::<Instant>));
        let stuck: Stuck = {
            let told = told.clone();
            Box::new(move || *told.lock().unwrap() = Some(Instant::now()))
        };
        let asked = Instant::now();
        // The board does not answer for 1.2 s; the limit is 100 ms.
        let board = || {
            std::thread::sleep(Duration::from_millis(1200));
            None
        };
        let got = arrive_watched(
            &finder,
            "item-eps",
            Path::new("/finder/logo.eps"),
            &wait(Duration::from_millis(100), &|| true, &board, &|_, _| {}),
            Some(stuck),
        );
        // Once its ask returned, the wait ends at its own limit.
        assert_eq!(got, Err(Stop::Stalled { done: 0, total: 384 }));
        let at = told.lock().unwrap().expect("the caller was told");
        let after = at.duration_since(asked);
        assert!(
            after >= Duration::from_millis(100) && after < Duration::from_millis(1000),
            "told at the limit, while the ask had not returned: {after:?}"
        );
    }

    #[test]
    fn a_wait_that_ends_in_time_never_tells_the_caller_it_stood_still() {
        let finder = FakeFinder::new(Some(Path::new("/finder/IMG.MOV")), Presence::Cloud, None);
        let told = std::sync::Arc::new(AtomicBool::new(false));
        let stuck: Stuck = {
            let told = told.clone();
            Box::new(move || told.store(true, Ordering::SeqCst))
        };
        let got = arrive_watched(
            &finder,
            "item-7",
            Path::new("/finder/IMG.MOV"),
            &wait(Duration::from_millis(60), &|| true, &|| None, &|_, _| {}),
            Some(stuck),
        );
        assert_eq!(got, Err(Stop::Stalled { done: 0, total: 384 }));
        // Well past the watch's own limit: it ended with the wait and said nothing.
        std::thread::sleep(Duration::from_millis(300));
        assert!(!told.load(Ordering::SeqCst));
    }

    #[test]
    fn a_file_that_is_here_is_handed_over_and_nothing_is_asked() {
        let finder = FakeFinder::new(Some(Path::new("/finder/a.eps")), Presence::Here, None);
        let mut opened = 0;
        let got = make_present(
            &finder,
            "item-1",
            &wait(Duration::from_secs(5), &|| true, &|| None, &|_, _| {}),
            &mut |_| opened += 1,
        );
        assert_eq!((got, opened), (Ok(true), 1));
        assert!(finder.requested().is_empty());
        assert_eq!(finder.pulled.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn no_byte_for_the_limit_stops_the_wait_and_nothing_is_handed_over() {
        let finder = FakeFinder::new(Some(Path::new("/finder/IMG.MOV")), Presence::Cloud, None);
        let mut opened = 0;
        // Every look at the download asks how far it is: counted, so the check holds on a busy machine too.
        let looks = std::cell::Cell::new(0_u32);
        let progress = || -> Option<(u64, u64)> {
            looks.set(looks.get() + 1);
            None
        };
        let got = make_present(
            &finder,
            "item-7",
            &wait(Duration::from_millis(60), &|| true, &progress, &|_, _| {}),
            &mut |_| opened += 1,
        );
        assert_eq!(got, Err(Stop::Stalled { done: 0, total: 384 }));
        assert_eq!(opened, 0);
        // The limit is the limit, also while nothing says how far the download is (RC11 waited three times as long):
        // it ends at the first look after the limit (a look every 60 ms here), not at the fourth.
        assert!(looks.get() <= 2, "the limit fired at the limit: {} looks", looks.get());
    }

    #[test]
    fn a_download_that_moves_is_not_given_up_and_one_that_stops_moving_is() {
        let finder = FakeFinder::new(Some(Path::new("/finder/IMG.MOV")), Presence::Cloud, None);
        let board = AtomicU64::new(0);
        let asked = Instant::now();
        // A byte more at every look for 150 ms, then none.
        let progress = || {
            if asked.elapsed() < Duration::from_millis(150) {
                board.fetch_add(1, Ordering::SeqCst);
            }
            Some((board.load(Ordering::SeqCst), 0))
        };
        let got = make_present(
            &finder,
            "item-7",
            &wait(Duration::from_millis(80), &|| true, &progress, &|_, _| {}),
            &mut |_| {},
        );
        let Err(Stop::Stalled { done, total }) = got else {
            panic!("the wait ends as stalled, got {got:?}");
        };
        assert!(done > 0 && total == 384);
        assert!(
            asked.elapsed() >= Duration::from_millis(150),
            "not given up while it moved"
        );
    }

    #[test]
    fn a_refusal_is_the_systems_answer_and_a_caller_that_left_ends_the_wait() {
        let mut finder = FakeFinder::new(Some(Path::new("/finder/a")), Presence::Cloud, None);
        finder.refuse = Some("NSFileProviderErrorDomain -1005".into());
        let got = make_present(
            &finder,
            "item-1",
            &wait(Duration::from_secs(5), &|| true, &|| None, &|_, _| {}),
            &mut |_| panic!("never handed over"),
        );
        assert_eq!(got, Err(Stop::Refused("NSFileProviderErrorDomain -1005".into())));
        assert_eq!(
            finder.pulled.load(Ordering::SeqCst),
            1,
            "the read went first; the refusal ends the wait"
        );
        assert!(finder.landed.lock().unwrap().is_empty());

        let finder = FakeFinder::new(Some(Path::new("/finder/a")), Presence::Cloud, None);
        let left = AtomicBool::new(false);
        let still = || !left.swap(true, Ordering::SeqCst);
        let got = make_present(
            &finder,
            "item-1",
            &wait(Duration::from_secs(5), &still, &|| None, &|_, _| {}),
            &mut |_| panic!("never handed over"),
        );
        assert_eq!(got, Err(Stop::Left));
    }

    #[test]
    fn an_item_with_no_place_in_finder_is_not_asked_for() {
        let finder = FakeFinder::new(None, Presence::Cloud, None);
        let got = make_present(
            &finder,
            "item-1",
            &wait(Duration::from_secs(5), &|| true, &|| None, &|_, _| {}),
            &mut |_| panic!("never handed over"),
        );
        assert_eq!(got, Ok(false));
        assert!(finder.requested().is_empty());
    }

    #[test]
    fn presence_reads_the_attributes_and_a_missing_file_is_gone() {
        let dir = tempfile::tempdir().expect("a folder");
        let file = dir.path().join("here.txt");
        std::fs::write(&file, b"bytes").expect("written");
        assert_eq!(presence_on_disk(&file), Presence::Here);
        assert_eq!(
            presence_on_disk(dir.path()),
            Presence::Gone,
            "a folder is not a plain file"
        );
        assert_eq!(presence_on_disk(&dir.path().join("none")), Presence::Gone);
    }
}
