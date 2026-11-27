//! Dm-75: the ONE "make Finder's copy present" call. The viewer (dm-74), Open with (dm-73), the Files list's Open
//! and keep all come here; none of them downloads a copy of its own on a Mac.
//!
//! What it does, at Finder's own place for the item (the scoped URL `bridge::with_finder_copy` hands out):
//! 1. looks at the file's attributes ([`presence_on_disk`]: `SF_DATALESS`), which downloads nothing;
//! 2. a file that is online only is asked of the system: `NSFileProviderManager
//!    requestDownloadForItemWithIdentifier:requestedRange:completionHandler:` with the whole file as the range
//!    ([`Finder::request`]), and the file is read at its first byte ([`Finder::pull`]). The request alone is a
//!    scheduling hint ("at the earliest convenient time", `NSFileProviderManager.h`): RC11 asked and the file stayed
//!    online only for 80 s. A read is what Finder's own open makes, and the system fetches for it at once. Both end
//!    in the extension's `fetchContents`, the one download;
//! 3. waits until the file is no longer dataless. How far it is comes from the transfer board (the caller's
//!    `progress`). No byte more for [`Wait::stall`] ends it ([`Stop::Stalled`]).
//!
//! Every step is logged, by its kind: never a name, an id, a path or the system's own text.

use std::path::Path;
use std::time::{Duration, Instant};

/// How long a download may go without a byte more before it is given up.
pub(crate) const STALL: Duration = Duration::from_secs(60);
/// How often the download is looked at.
pub(crate) const LOOK_EVERY: Duration = Duration::from_millis(200);

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
    /// now, whatever became of the request. Never waited for. Default: nothing (a test's finder has no system).
    fn pull(&self, _file: &Path) {}
}

/// The one place the app asks the system to download an item (`beebeeb_fp_request_download`:
/// `requestDownloadForItemWithIdentifier:requestedRange:completionHandler:`, the whole file). The viewer, Open with,
/// the list's Open and keep all ask here. `Ok`: the system took the request, or did not answer within the bridge's
/// limit (it may still have taken it; the wait for the file tells). `Err`: the system's own answer. Waits up to ten
/// seconds: a worker's. Logged by its kind.
#[cfg(not(test))]
pub(crate) fn request_download(file_id: &str) -> Result<(), beebeeb_engine::finder_setup::error::FpError> {
    use beebeeb_engine::finder_setup::error::{BRIDGE_DOMAIN, FpError, bridge_code};

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

/// Finder's copy at `place` is on this Mac when this answers `Ok(true)`: it was, or the system was asked for it and
/// it is no longer dataless. `Ok(false)`: no plain file at Finder's place. Call it inside [`Finder::with_copy`]'s
/// read, where `place` may be looked at. Blocks: a worker's.
pub(crate) fn arrive(finder: &dyn Finder, file_id: &str, place: &Path, wait: &Wait<'_>) -> Result<bool, Stop> {
    match finder.presence(place) {
        Presence::Here => return Ok(true),
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
    if let Err(why) = finder.request(file_id) {
        return Err(Stop::Refused(why));
    }
    finder.pull(place);
    // When the download was asked for, when it last moved, how far it was then, and whether a byte was seen.
    let asked = Instant::now();
    let (mut moved, mut done, mut seen) = (asked, 0_u64, false);
    loop {
        if !(wait.still)() {
            tracing::info!("materialise: the wait was left before Finder\u{2019}s copy was here");
            return Err(Stop::Left);
        }
        match finder.presence(place) {
            Presence::Here => {
                (wait.report)(wait.size, wait.size);
                tracing::info!(
                    ms = asked.elapsed().as_millis() as u64,
                    "materialise: Finder\u{2019}s copy is on this Mac"
                );
                return Ok(true);
            }
            Presence::Gone => {
                tracing::warn!("materialise: the file left Finder\u{2019}s place while it was downloading");
                return Ok(false);
            }
            Presence::Cloud => {}
        }
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
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// A finder over one place: what is there is a value the test sets, and every ask is remembered.
    pub(crate) struct FakeFinder {
        pub place: Option<PathBuf>,
        pub presence: Mutex<Presence>,
        /// The looks after the request that still answer online only, before the file is here (`None`: it never is).
        pub arrives_after: Option<usize>,
        pub refuse: Option<String>,
        pub requested: Mutex<Vec<String>>,
        pub pulled: AtomicUsize,
        looks: AtomicUsize,
    }

    impl FakeFinder {
        pub fn new(place: Option<&Path>, presence: Presence, arrives_after: Option<usize>) -> Self {
            FakeFinder {
                place: place.map(Path::to_path_buf),
                presence: Mutex::new(presence),
                arrives_after,
                refuse: None,
                requested: Mutex::new(Vec::new()),
                pulled: AtomicUsize::new(0),
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
            // The system brings the file down only once it was asked.
            if *presence == Presence::Cloud && !self.requested.lock().unwrap().is_empty() {
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
        let asked = Instant::now();
        let got = make_present(
            &finder,
            "item-7",
            &wait(Duration::from_millis(60), &|| true, &|| None, &|_, _| {}),
            &mut |_| opened += 1,
        );
        assert_eq!(got, Err(Stop::Stalled { done: 0, total: 384 }));
        assert_eq!(opened, 0);
        // The limit is the limit, also while nothing says how far the download is (RC11 waited three times as long).
        assert!(
            asked.elapsed() < Duration::from_millis(60) * 3,
            "the limit fired at the limit"
        );
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
        assert_eq!(finder.pulled.load(Ordering::SeqCst), 0, "a refused file is not read");

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
