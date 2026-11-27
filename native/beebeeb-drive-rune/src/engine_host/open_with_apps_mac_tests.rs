//! Dm-73: Open with on a Mac, on a platform that remembers what it was asked. No test here reaches the File Provider,
//! the chooser or `NSWorkspace`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::mac::*;
use crate::frames::viewers::unsupported::{self, Note};

const FINDER: &str = "/fake/CloudStorage/Beebeeb/Dit is een test.docx";
const PAGES: &str = "/Applications/Pages.app";
const WORD: &str = "/Applications/Microsoft Word.app";

/// A Mac that remembers what it was asked, in order.
#[derive(Default)]
struct Fake {
    asked: Mutex<Vec<String>>,
    /// Finder's copy is on this Mac from the start.
    here: AtomicBool,
    /// How many looks after the download was asked for until the copy is no longer dataless (`usize::MAX`: never).
    arrives_after: AtomicUsize,
    looks: AtomicUsize,
    /// The system serves a read of the file, and never the request it "took" (RC12).
    only_a_read_serves: AtomicBool,
    /// The no-byte limit, in ms (0: the 1500 ms every arriving file fits in).
    stall_ms: AtomicU64,
    /// How long the transfer board takes to answer, in ms.
    board_waits_ms: AtomicU64,
    download_refused: AtomicBool,
    no_place: AtomicBool,
    no_default: AtomicBool,
    open_refused: AtomicBool,
    picked: Mutex<Option<PathBuf>>,
    /// Dm-78: how Finder setup stands at each look, in order; the last one stays. Empty: ready.
    setup: Mutex<Vec<Setup>>,
    setup_looks: AtomicUsize,
    /// How long setup is waited for, in ms (0: 1500 ms).
    setup_wait_ms: AtomicU64,
    /// The hand-over's own checks do not pass once Finder is ready.
    may_not_open: AtomicBool,
    checked: AtomicUsize,
}

impl Fake {
    fn note(&self, what: impl Into<String>) {
        self.asked.lock().unwrap().push(what.into());
    }
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
    /// What was asked, without the looks at the file (their number is the wait's own).
    fn steps(&self) -> Vec<String> {
        self.asked().into_iter().filter(|a| a != "dataless?").collect()
    }
}

impl Platform for Fake {
    fn setup(&self) -> Setup {
        self.setup_looks.fetch_add(1, SeqCst);
        let mut setup = self.setup.lock().unwrap();
        match setup.len() {
            0 => Setup::Ready,
            1 => setup[0],
            _ => setup.remove(0),
        }
    }
    fn setup_wait(&self) -> Duration {
        match self.setup_wait_ms.load(SeqCst) {
            0 => Duration::from_millis(1500),
            ms => Duration::from_millis(ms),
        }
    }
    fn setup_grace(&self) -> Duration {
        Duration::from_millis(250)
    }
    fn may_open(&self, _item_id: &str) -> bool {
        self.checked.fetch_add(1, SeqCst);
        !self.may_not_open.load(SeqCst)
    }
    fn presence(&self, _file: &Path) -> Presence {
        self.note("dataless?");
        if self.here.load(SeqCst) {
            return Presence::Here;
        }
        // The system brings the file down only once it was asked, by the ask it serves.
        let serves = if self.only_a_read_serves.load(SeqCst) {
            "read "
        } else {
            "requestDownload"
        };
        let asked = self.asked().iter().any(|a| a.starts_with(serves));
        if asked && self.looks.fetch_add(1, SeqCst) >= self.arrives_after.load(SeqCst) {
            self.here.store(true, SeqCst);
            return Presence::Here;
        }
        Presence::Cloud
    }
    fn materialise(&self, item_id: &str) -> Result<(), SystemError> {
        self.note(format!("requestDownload {item_id}"));
        if self.download_refused.load(SeqCst) {
            Err(SystemError::new("NSFileProviderErrorDomain", -1004))
        } else {
            Ok(())
        }
    }
    fn pull(&self, file: &Path) {
        self.note(format!("read {}", file.display()));
    }
    fn read_first(&self) -> Duration {
        Duration::ZERO
    }
    fn landed(&self, item_id: &str) {
        self.note(format!("on this Mac {item_id}"));
    }
    fn progress(&self, _item_id: &str) -> Option<(u64, u64)> {
        std::thread::sleep(Duration::from_millis(self.board_waits_ms.load(SeqCst)));
        None
    }
    fn stall(&self) -> Duration {
        // Longer than the looks an arriving file takes; a file that never arrives is given up after it.
        match self.stall_ms.load(SeqCst) {
            0 => Duration::from_millis(1500),
            ms => Duration::from_millis(ms),
        }
    }
    fn choose_app(&self) -> Option<PathBuf> {
        self.note("choose app");
        self.picked.lock().unwrap().clone()
    }
    fn finder_copy(&self, item_id: &str, with: &mut dyn FnMut(&Path)) -> bool {
        self.note(format!("finder copy {item_id}"));
        if self.no_place.load(SeqCst) {
            return false;
        }
        with(Path::new(FINDER));
        true
    }
    fn default_app(&self, file: &Path) -> Option<PathBuf> {
        self.note(format!("default app of {}", file.display()));
        (!self.no_default.load(SeqCst)).then(|| PathBuf::from(PAGES))
    }
    fn open_with(&self, app: &Path, file: &Path) -> Result<(), SystemError> {
        self.note(format!("NSWorkspace open {} with {}", file.display(), app.display()));
        if self.open_refused.load(SeqCst) {
            Err(SystemError::new("NSOSStatusErrorDomain", -10814))
        } else {
            Ok(())
        }
    }
}

/// One press on `fake`: what it answered and what the page was told meanwhile.
fn press(fake: &Fake, pick: Pick) -> (Result<(), Failure>, Vec<Note>) {
    let said = Mutex::new(Vec::new());
    let done = run(fake, "f-docx", pick, &|| true, &|note| said.lock().unwrap().push(note));
    (done, said.into_inner().unwrap())
}

#[test]
fn open_with_default_on_a_dataless_file_asks_requestdownload_and_opens_only_once_it_is_no_longer_dataless() {
    let fake = Fake::default();
    fake.arrives_after.store(2, SeqCst);
    let (done, said) = press(&fake, Pick::Default);
    assert_eq!(done, Ok(()));
    assert_eq!(
        fake.steps(),
        [
            // Finder's own place for the item, held for the whole press.
            "finder copy f-docx",
            // Dataless there: the file is read so it comes now, and the system is asked for it by its identifier.
            &format!("read {FINDER}"),
            "requestDownload f-docx",
            // No longer dataless: the marks are told, and that very file is opened through NSWorkspace with the
            // kind's app.
            "on this Mac f-docx",
            &format!("default app of {FINDER}"),
            &format!("NSWorkspace open {FINDER} with {PAGES}"),
        ]
    );
    // The app got the file only after a look that found its bytes here.
    let asked = fake.asked();
    let opened = asked.iter().position(|a| a.starts_with("NSWorkspace open")).unwrap();
    let looks_before = asked[..opened].iter().filter(|a| *a == "dataless?").count();
    assert!(looks_before >= 4, "looked until it was here: {asked:?}");
    assert!(fake.here.load(SeqCst));
    // The page said it was downloading, then that it was opening.
    assert_eq!(said, [Note::Downloading, Note::Opening]);
}

#[test]
fn open_with_a_file_already_here_downloads_nothing_and_the_app_picked_gets_finders_url() {
    let fake = Fake::default();
    fake.here.store(true, SeqCst);
    *fake.picked.lock().unwrap() = Some(PathBuf::from(WORD));
    let (done, said) = press(&fake, Pick::Choose);
    assert_eq!(done, Ok(()));
    assert_eq!(
        fake.asked(),
        [
            "choose app",
            "finder copy f-docx",
            "dataless?",
            "dataless?",
            "on this Mac f-docx",
            // The app picked, not the kind's own: it is not asked for.
            &format!("NSWorkspace open {FINDER} with {WORD}"),
        ]
    );
    assert_eq!(said, [Note::Opening]);
}

/// What the app's log file holds after `body` ran with it as the thread's log: the real filter, which masks every
/// word it does not know.
fn logged(body: impl FnOnce()) -> String {
    use crate::log_file::{AppLog, FILE_NAME, MAX_FILE_BYTES};
    let dir = tempfile::tempdir().unwrap();
    let log = AppLog::open(dir.path(), MAX_FILE_BYTES).expect("the log opens");
    tracing::subscriber::with_default(log, body);
    std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap_or_default()
}

#[test]
fn open_with_says_every_failure_in_words_and_logs_its_reason() {
    type Break = fn(&Fake);
    let cases: [(Break, Failure, &str, &[&str]); 5] = [
        (
            |f| f.download_refused.store(true, SeqCst),
            Failure::Download(SystemError::new("NSFileProviderErrorDomain", -1004)),
            NOT_DOWNLOADED,
            &[
                "the download was refused",
                "the system answered (NSFileProviderErrorDomain)",
                "-1004",
            ],
        ),
        (
            // Dm-75: no byte for the limit. The page says so, and the reason is in the log.
            |f| f.arrives_after.store(usize::MAX, SeqCst),
            Failure::DownloadStalled,
            NOT_MOVING,
            &["the download stopped moving"],
        ),
        (
            |f| f.no_place.store(true, SeqCst),
            Failure::NotInFinder,
            NOT_IN_FINDER,
            &["it has no place in Finder"],
        ),
        (
            |f| f.no_default.store(true, SeqCst),
            Failure::NoApp,
            NO_APP,
            &["no app for its kind"],
        ),
        (
            |f| f.open_refused.store(true, SeqCst),
            Failure::Open(SystemError::new("NSOSStatusErrorDomain", -10814)),
            APP_REFUSED,
            &[
                "NSWorkspace refused",
                "the system answered (NSOSStatusErrorDomain)",
                "-10814",
            ],
        ),
    ];
    for (breaks, failure, words, reasons) in cases {
        let fake = Fake::default();
        breaks(&fake);
        let (done, _) = press(&fake, Pick::Default);
        assert_eq!(done, Err(failure.clone()));
        assert_eq!(failure.words().as_deref(), Some(words));
        // The log's own filter keeps every word of the line: the reason is readable there, domain and code included.
        let line = logged(|| failure.log());
        assert!(line.contains("open with: the file did not open"), "{line}");
        for reason in reasons {
            assert!(line.contains(reason), "{failure:?} without {reason:?}: {line}");
        }
        assert!(!line.contains("<\u{2026}>"), "no word of the line is masked: {line}");
        // Nothing opened unless NSWorkspace itself was the one that refused.
        let opened = fake.asked().iter().any(|a| a.starts_with("NSWorkspace open"));
        assert_eq!(opened, matches!(failure, Failure::Open(_)), "{:?}", fake.asked());
        // Neither the words nor the line holds the file's name or its place.
        assert!(!words.contains("docx") && !line.contains("docx") && !line.contains("/fake"));
    }
    // A domain this app has no name for is not written (the system's own words never are); its code is.
    let line = logged(|| Failure::Open(SystemError::new("An Invented Domain Of Marit Bakker", 7)).log());
    assert!(
        line.contains("the system answered (another domain)") && !line.contains("Marit"),
        "{line}"
    );
    // A closed chooser and a press that is over are not failures: no line.
    assert_eq!(logged(|| Failure::Cancelled.log()), "");
    assert_eq!(logged(|| Failure::Moved.log()), "");
}

#[test]
fn open_with_a_closed_chooser_or_an_ended_session_opens_nothing_and_says_nothing() {
    // The chooser closed without an app: nothing is downloaded either.
    let fake = Fake::default();
    let (done, said) = press(&fake, Pick::Choose);
    assert_eq!(done, Err(Failure::Cancelled));
    assert_eq!(fake.asked(), ["choose app"]);
    assert_eq!((said, Failure::Cancelled.words()), (vec![], None));

    // The session of the press ends while the copy comes down: the wait ends, and nothing is opened.
    let fake = Fake::default();
    fake.arrives_after.store(usize::MAX, SeqCst);
    let looks = AtomicUsize::new(0);
    let done = run(
        &fake,
        "f-docx",
        Pick::Default,
        &|| looks.fetch_add(1, SeqCst) < 3,
        &|_| {},
    );
    assert_eq!(done, Err(Failure::Moved));
    assert_eq!(Failure::Moved.words(), None);
    assert!(
        !fake
            .asked()
            .iter()
            .any(|a| a.starts_with("NSWorkspace open") || a.starts_with("default app")),
        "{:?}",
        fake.asked()
    );

    // It ends right before the app would get the file: the app does not get it.
    let fake = Fake::default();
    fake.here.store(true, SeqCst);
    let looks = AtomicUsize::new(0);
    let done = run(
        &fake,
        "f-docx",
        Pick::Default,
        &|| looks.fetch_add(1, SeqCst) < 2,
        &|_| {},
    );
    assert_eq!(done, Err(Failure::Moved));
    assert!(
        !fake.asked().iter().any(|a| a.starts_with("NSWorkspace open")),
        "{:?}",
        fake.asked()
    );
}

/// Dm-77, RC12: the system "took" the request for a small file and never served it; 80 s on nothing had opened.
#[test]
fn open_with_reads_a_file_whose_request_was_taken_but_never_served_and_opens_only_once_it_is_no_longer_dataless() {
    let fake = Fake::default();
    fake.only_a_read_serves.store(true, SeqCst);
    fake.arrives_after.store(2, SeqCst);
    let (done, said) = press(&fake, Pick::Default);
    assert_eq!(done, Ok(()));
    assert_eq!(
        fake.steps(),
        [
            "finder copy f-docx",
            &format!("read {FINDER}"),
            // Taken, and nothing follows from it.
            "requestDownload f-docx",
            "on this Mac f-docx",
            &format!("default app of {FINDER}"),
            &format!("NSWorkspace open {FINDER} with {PAGES}"),
        ]
    );
    // NSWorkspace got the file only after the looks that found it dataless, and after the one that did not.
    let asked = fake.asked();
    let opened = asked.iter().position(|a| a.starts_with("NSWorkspace open")).unwrap();
    let landed = asked.iter().position(|a| a.starts_with("on this Mac")).unwrap();
    assert!(
        landed < opened && asked[..landed].iter().filter(|a| *a == "dataless?").count() >= 4,
        "{asked:?}"
    );
    assert!(fake.here.load(SeqCst));
    assert_eq!(said, [Note::Downloading, Note::Opening]);

    // A system that serves neither: the limit ends the press, nothing is opened, and the marks are told nothing.
    let fake = Fake::default();
    fake.arrives_after.store(usize::MAX, SeqCst);
    fake.stall_ms.store(300, SeqCst);
    let (done, said) = press(&fake, Pick::Default);
    assert_eq!(done, Err(Failure::DownloadStalled));
    assert_eq!(said, [Note::Downloading]);
    assert!(
        !fake
            .asked()
            .iter()
            .any(|a| a.starts_with("NSWorkspace open") || a.starts_with("on this Mac")),
        "{:?}",
        fake.asked()
    );
}

/// The presses of [`start_on`] share the number of the newest press: the tests that make one take turns.
static PRESSES: Mutex<()> = Mutex::new(());

/// Dm-77: no byte for the limit. The page of the file says the download stopped (the reason is in the log:
/// `open_with_says_every_failure_in_words_and_logs_its_reason`), and nothing is opened.
#[test]
fn open_with_start_shows_the_page_with_the_reason_when_no_byte_came_for_the_limit() {
    let _turn = PRESSES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let fake = Arc::new(Fake::default());
    fake.arrives_after.store(usize::MAX, SeqCst);
    fake.stall_ms.store(300, SeqCst);
    assert_eq!(
        start_on(Some(fake.clone()), "f-77-stalled", Pick::Default, || true, || {}),
        Ok(())
    );
    note_becomes("f-77-stalled", Some(Note::Failed(NOT_MOVING.to_string())));
    assert!(
        !fake.asked().iter().any(|a| a.starts_with("NSWorkspace open")),
        "{:?}",
        fake.asked()
    );

    // The limit is real even when the wait itself stands still (RC12: no limit fired in 80 s). The board does not
    // answer for 2 s; the limit is 300 ms. The page says so well before the board answered, in the log's words too.
    let fake = Arc::new(Fake::default());
    fake.arrives_after.store(usize::MAX, SeqCst);
    fake.stall_ms.store(300, SeqCst);
    fake.board_waits_ms.store(2000, SeqCst);
    let woken = Arc::new(AtomicUsize::new(0));
    let wake = {
        let woken = woken.clone();
        move || {
            woken.fetch_add(1, SeqCst);
        }
    };
    let pressed = std::time::Instant::now();
    assert_eq!(
        start_on(Some(fake.clone()), "f-77-stuck", Pick::Default, || true, wake),
        Ok(())
    );
    note_becomes("f-77-stuck", Some(Note::Failed(NOT_MOVING.to_string())));
    assert!(
        pressed.elapsed() < Duration::from_millis(1500),
        "said at the limit, not when the board answered"
    );
    assert!(
        woken.load(SeqCst) >= 2,
        "the window drew the downloading note, then the failure"
    );
    // The board answers at last: that press is over. It opens nothing and takes nothing off the page.
    std::thread::sleep(Duration::from_millis(2300).saturating_sub(pressed.elapsed()));
    assert_eq!(
        unsupported::note_of("f-77-stuck"),
        Some(Note::Failed(NOT_MOVING.to_string()))
    );
    assert!(
        !fake.asked().iter().any(|a| a.starts_with("NSWorkspace open")),
        "{:?}",
        fake.asked()
    );
}

/// Waits for the page's note of `id` to be `want`.
fn note_becomes(id: &str, want: Option<Note>) {
    for _ in 0..500 {
        if unsupported::note_of(id) == want {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the page says {:?}, not {want:?}", unsupported::note_of(id));
}

#[test]
fn open_with_start_puts_the_failure_on_the_files_page_and_takes_its_words_away_once_it_opened() {
    let _turn = PRESSES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // A refusal: the page of that file says it, and the window is woken to draw it.
    let fake = Arc::new(Fake::default());
    fake.here.store(true, SeqCst);
    fake.open_refused.store(true, SeqCst);
    let woken = Arc::new(AtomicUsize::new(0));
    let wake = {
        let woken = woken.clone();
        move || {
            woken.fetch_add(1, SeqCst);
        }
    };
    assert_eq!(
        start_on(Some(fake.clone()), "f-73-refused", Pick::Default, || true, wake),
        Ok(())
    );
    note_becomes("f-73-refused", Some(Note::Failed(APP_REFUSED.to_string())));
    assert!(woken.load(SeqCst) >= 1);

    // It opened: nothing is left on the page.
    let fake = Arc::new(Fake::default());
    fake.here.store(true, SeqCst);
    unsupported::set_note("f-73-opened", Some(Note::Failed("an older press".into())));
    assert_eq!(
        start_on(Some(fake.clone()), "f-73-opened", Pick::Default, || true, || {}),
        Ok(())
    );
    note_becomes("f-73-opened", None);
    assert!(fake.asked().iter().any(|a| a.starts_with("NSWorkspace open")));

    // No platform (a build without one): refused at once, said on the page.
    assert_eq!(start_on(None, "f-73-none", Pick::Default, || true, || {}), Err(()));
    assert_eq!(
        unsupported::note_of("f-73-none"),
        Some(Note::Failed(NOT_STARTED.to_string()))
    );
    // And the test build's own platform is none: no test reaches the File Provider or NSWorkspace.
    assert_eq!(
        start("f-73-test", Pick::Default, || true, || {}, |_| None, |_| true),
        Err(())
    );
}

/// A fake whose Finder setup stands as `setup` says, look after look.
fn setting_up_fake(setup: &[Setup]) -> Fake {
    let fake = Fake::default();
    *fake.setup.lock().unwrap() = setup.to_vec();
    fake
}

const TIMED_OUT: &str = "macOS didn\u{2019}t finish adding Beebeeb to Finder.";

/// Dm-78, RC13: Finder setup was slow to start ("adding", trying again), and "Open with default app" answered at once
/// with the words of a sync folder. The press waits for setup while the page says so, then goes dm-77's way.
#[test]
fn open_with_while_finder_setup_is_adding_waits_and_opens_once_it_is_ready() {
    assert_eq!(
        setting_up().words(),
        "Finder is still being set up \u{b7} opens when it is ready"
    );
    let fake = setting_up_fake(&[Setup::Adding, Setup::Adding, Setup::Adding, Setup::Adding, Setup::Ready]);
    fake.arrives_after.store(1, SeqCst);
    let said = Mutex::new(Vec::new());
    let done = run(&fake, "f-eps", Pick::Default, &|| true, &|note| {
        if note == setting_up() {
            // Nothing is asked of Finder while it is not set up.
            assert_eq!(fake.asked(), Vec::<String>::new());
        }
        said.lock().unwrap().push(note);
    });
    assert_eq!(done, Ok(()));
    // The page said it once, then what every press says.
    assert_eq!(
        said.into_inner().unwrap(),
        [setting_up(), Note::Downloading, Note::Opening]
    );
    assert!(
        fake.setup_looks.load(SeqCst) >= 5,
        "setup was looked at until it was ready"
    );
    assert_eq!(
        fake.checked.load(SeqCst),
        1,
        "the hand-over's checks, once Finder was ready"
    );
    // Dm-77's path, whole: Finder's place, the read first, the file no longer dataless, then the app.
    let steps = fake.steps();
    assert_eq!(steps[0], "finder copy f-eps");
    assert_eq!(steps[1], format!("read {FINDER}"));
    assert_eq!(
        steps.last().map(String::as_str),
        Some(format!("NSWorkspace open {FINDER} with {PAGES}").as_str())
    );
    assert!(steps.contains(&"on this Mac f-eps".to_string()), "{steps:?}");

    // Setup that is ready is not waited for, and nothing is said of it.
    let fake = Fake::default();
    fake.here.store(true, SeqCst);
    let (done, said) = press(&fake, Pick::Default);
    assert_eq!((done, said), (Ok(()), vec![Note::Opening]));

    // Setup that had not begun yet at the press (the first moments of a run) and then does: the same wait.
    let fake = setting_up_fake(&[Setup::NotBegun, Setup::Adding, Setup::Ready]);
    fake.here.store(true, SeqCst);
    let (done, said) = press(&fake, Pick::Default);
    assert_eq!((done, said), (Ok(()), vec![setting_up(), Note::Opening]));
}

/// The viewer's "…" menu "Open with…" is `Pick::Choose` on the same call: dm-73's chooser first (never minutes after
/// the press), then the wait for setup, then dm-77's materialise and the app picked.
#[test]
fn open_with_from_the_more_menu_is_the_same_path_the_chooser_first_then_the_wait_then_the_file() {
    let fake = setting_up_fake(&[Setup::Adding, Setup::Adding, Setup::Adding, Setup::Ready]);
    *fake.picked.lock().unwrap() = Some(PathBuf::from(WORD));
    fake.arrives_after.store(1, SeqCst);
    let said = Mutex::new(Vec::new());
    let done = run(&fake, "f-md", Pick::Choose, &|| true, &|note| {
        if note == setting_up() {
            assert_eq!(fake.asked(), ["choose app"], "the chooser came before the wait");
        }
        said.lock().unwrap().push(note);
    });
    assert_eq!(done, Ok(()));
    assert_eq!(
        said.into_inner().unwrap(),
        [setting_up(), Note::Downloading, Note::Opening]
    );
    let steps = fake.steps();
    assert_eq!(
        steps[..3],
        [
            "choose app".to_string(),
            "finder copy f-md".into(),
            format!("read {FINDER}")
        ]
    );
    assert_eq!(
        steps.last().map(String::as_str),
        Some(format!("NSWorkspace open {FINDER} with {WORD}").as_str())
    );
    assert!(!steps.iter().any(|a| a.starts_with("default app")), "{steps:?}");
}

/// Setup that ended without Finder: the page says its reason in the words Settings uses, never a sync folder's.
#[test]
fn open_with_after_a_finder_setup_that_failed_says_its_reason_in_mac_words_and_asks_nothing() {
    use beebeeb_engine::finder_setup::driver::FinderSetupView;
    use beebeeb_engine::finder_setup::launch_location::LaunchLocation;
    use beebeeb_engine::surfaces::phase::{FinderFailureReason, FinderSetup};
    let view = |setup, reason| {
        let mut view = FinderSetupView::initial(LaunchLocation::Applications);
        (view.setup, view.reason) = (setup, reason);
        view
    };
    // What `finder_setup_state` says, as the press reads it.
    assert_eq!(setup_of(&view(FinderSetup::Ready, None)), Setup::Ready);
    assert_eq!(setup_of(&view(FinderSetup::Adding, None)), Setup::Adding);
    assert_eq!(setup_of(&view(FinderSetup::Missing, None)), Setup::NotBegun);
    let failed = setup_of(&view(FinderSetup::Failed, Some(FinderFailureReason::Timeout)));
    assert_eq!(failed, Setup::Ended(TIMED_OUT));
    for (setup, reason) in [
        (FinderSetup::Failed, None),
        (FinderSetup::Failed, Some(FinderFailureReason::Signing)),
        (FinderSetup::UserDisabled, Some(FinderFailureReason::UserDisabled)),
        (FinderSetup::UserDisabled, None),
        (FinderSetup::Missing, Some(FinderFailureReason::NotInApplications)),
    ] {
        let Setup::Ended(why) = setup_of(&view(setup, reason)) else {
            panic!("{setup:?} {reason:?} is waited for");
        };
        assert!(!why.is_empty());
    }

    // It failed before the press: said at once, and neither the chooser nor Finder is asked anything.
    for pick in [Pick::Default, Pick::Choose] {
        let fake = setting_up_fake(&[failed]);
        *fake.picked.lock().unwrap() = Some(PathBuf::from(WORD));
        let (done, said) = press(&fake, pick);
        assert_eq!(done, Err(Failure::Setup(TIMED_OUT)));
        assert_eq!((fake.asked(), said), (Vec::<String>::new(), vec![]));
    }
    // It failed while the press waited: the wait ends with the reason.
    let fake = setting_up_fake(&[Setup::Adding, Setup::Adding, Setup::Adding, failed]);
    let (done, said) = press(&fake, Pick::Default);
    assert_eq!(done, Err(Failure::Setup(TIMED_OUT)));
    assert_eq!((fake.asked(), said), (Vec::<String>::new(), vec![setting_up()]));

    // Setup still adding at the limit, and setup that never began (Beebeeb is turned off in Finder here).
    let fake = setting_up_fake(&[Setup::Adding]);
    fake.setup_wait_ms.store(300, SeqCst);
    let pressed = std::time::Instant::now();
    let (done, said) = press(&fake, Pick::Default);
    assert_eq!((done, said), (Err(Failure::SetupSlow), vec![setting_up()]));
    assert!(pressed.elapsed() >= Duration::from_millis(300) && fake.asked().is_empty());
    let fake = setting_up_fake(&[Setup::NotBegun]);
    let (done, _) = press(&fake, Pick::Default);
    assert_eq!(done, Err(Failure::Setup(NOT_BEGUN)));
    assert!(fake.asked().is_empty());

    // The session of the press ends while it waits: nothing more is said, nothing opened.
    let fake = setting_up_fake(&[Setup::Adding]);
    let looks = AtomicUsize::new(0);
    let done = run(
        &fake,
        "f-eps",
        Pick::Default,
        &|| looks.fetch_add(1, SeqCst) < 3,
        &|_| {},
    );
    assert_eq!((done, fake.asked()), (Err(Failure::Moved), Vec::<String>::new()));

    // Finder got ready, and the hand-over's own checks do not pass then (sync did not start): nothing is asked.
    let fake = setting_up_fake(&[Setup::Adding, Setup::Ready]);
    fake.may_not_open.store(true, SeqCst);
    let (done, _) = press(&fake, Pick::Default);
    assert_eq!((done, fake.asked()), (Err(Failure::Refused), Vec::<String>::new()));

    // The words: a Mac's, with the reason; and each has its line in the log.
    let words = Failure::Setup(TIMED_OUT).words().expect("said");
    assert_eq!(
        words,
        "macOS didn\u{2019}t finish adding Beebeeb to Finder. This file opens from Finder, so it didn\u{2019}t open."
    );
    for failure in [
        Failure::Setup(TIMED_OUT),
        Failure::Setup(NOT_BEGUN),
        Failure::SetupSlow,
        Failure::Refused,
    ] {
        let words = failure.words().expect("said");
        for old in ["sync", "folder", "PC", "Windows"] {
            assert!(!words.contains(old), "{failure:?}: {words}");
        }
        let line = logged(|| failure.log());
        assert!(line.contains("open with: the file did not open"), "{line}");
    }
    assert!(
        Failure::Setup(NOT_BEGUN)
            .words()
            .unwrap()
            .starts_with("Beebeeb isn\u{2019}t in Finder on ")
    );
}

/// The whole press on its own thread: the page says Finder is still being set up, then the reason when setup failed;
/// and after a wait that ended ready, nothing is left on the page.
#[test]
fn open_with_start_says_the_wait_for_finder_on_the_page_and_then_what_became_of_it() {
    let _turn = PRESSES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let failed = Setup::Ended(TIMED_OUT);
    let mut script = vec![Setup::Adding; 5];
    script.push(failed);
    let fake = Arc::new(setting_up_fake(&script));
    fake.setup_wait_ms.store(60_000, SeqCst);
    let woken = Arc::new(AtomicUsize::new(0));
    let wake = {
        let woken = woken.clone();
        move || {
            woken.fetch_add(1, SeqCst);
        }
    };
    assert_eq!(
        start_on(Some(fake.clone()), "f-78-failed", Pick::Default, || true, wake),
        Ok(())
    );
    note_becomes("f-78-failed", Some(setting_up()));
    note_becomes(
        "f-78-failed",
        Some(Note::Failed(Failure::Setup(TIMED_OUT).words().unwrap())),
    );
    assert!(woken.load(SeqCst) >= 2, "the window drew the wait, then the reason");
    assert!(fake.asked().is_empty(), "{:?}", fake.asked());

    let mut script = vec![Setup::Adding; 3];
    script.push(Setup::Ready);
    let fake = Arc::new(setting_up_fake(&script));
    fake.here.store(true, SeqCst);
    assert_eq!(
        start_on(Some(fake.clone()), "f-78-ready", Pick::Default, || true, || {}),
        Ok(())
    );
    for _ in 0..500 {
        if fake.asked().iter().any(|a| a.starts_with("NSWorkspace open")) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        fake.asked().iter().any(|a| a.starts_with("NSWorkspace open")),
        "{:?}",
        fake.asked()
    );
    note_becomes("f-78-ready", None);
}
