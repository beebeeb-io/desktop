//! Dm-73: Open with on a Mac, on a platform that remembers what it was asked. No test here reaches the File Provider,
//! the chooser or `NSWorkspace`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
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
    /// The copy is on this Mac from the start.
    here: AtomicBool,
    /// How many looks after the download was asked for until the copy is here (`usize::MAX`: never).
    arrives_after: AtomicUsize,
    looks: AtomicUsize,
    /// The system cannot list what is downloaded.
    no_list: AtomicBool,
    download_refused: AtomicBool,
    no_place: AtomicBool,
    no_default: AtomicBool,
    open_refused: AtomicBool,
    picked: Mutex<Option<PathBuf>>,
    waited: Mutex<Duration>,
}

impl Fake {
    fn note(&self, what: impl Into<String>) {
        self.asked.lock().unwrap().push(what.into());
    }
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl Platform for Fake {
    fn present(&self, item_id: &str) -> Result<bool, String> {
        self.note(format!("present {item_id}"));
        if self.no_list.load(SeqCst) {
            return Err("not listed".into());
        }
        if self.here.load(SeqCst) {
            return Ok(true);
        }
        let materialising = self.asked().iter().any(|a| a.starts_with("materialise"));
        Ok(materialising && self.looks.fetch_add(1, SeqCst) >= self.arrives_after.load(SeqCst))
    }
    fn materialise(&self, item_id: &str) -> Result<(), SystemError> {
        self.note(format!("materialise {item_id}"));
        if self.download_refused.load(SeqCst) {
            Err(SystemError::new("NSFileProviderErrorDomain", -1004))
        } else {
            Ok(())
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
    fn wait(&self, time: Duration) {
        *self.waited.lock().unwrap() += time;
    }
}

/// One press on `fake`: what it answered and what the page was told meanwhile.
fn press(fake: &Fake, pick: Pick) -> (Result<(), Failure>, Vec<Note>) {
    let said = Mutex::new(Vec::new());
    let done = run(fake, "f-docx", pick, &|| true, &|note| said.lock().unwrap().push(note));
    (done, said.into_inner().unwrap())
}

#[test]
fn open_with_default_on_a_cloud_only_file_asks_for_materialise_then_opens_finders_url() {
    let fake = Fake::default();
    fake.arrives_after.store(2, SeqCst);
    let (done, said) = press(&fake, Pick::Default);
    assert_eq!(done, Ok(()));
    assert_eq!(
        fake.asked(),
        [
            "present f-docx",
            // Cloud-only: the File Provider is asked for it, as Finder's download does, and it is waited for.
            "materialise f-docx",
            "present f-docx",
            "present f-docx",
            "present f-docx",
            // Here now: the place the File Provider gives, opened through NSWorkspace with the kind's app.
            "finder copy f-docx",
            &format!("default app of {FINDER}"),
            &format!("NSWorkspace open {FINDER} with {PAGES}"),
        ]
    );
    // The page said it was downloading, then that it was opening.
    assert_eq!(said, [Note::Downloading, Note::Opening]);
    assert_eq!(*fake.waited.lock().unwrap(), LOOK_EVERY * 2);
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
            "present f-docx",
            "finder copy f-docx",
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
            &["the download was refused", "the system answered (NSFileProviderErrorDomain)", "-1004"],
        ),
        (
            |f| f.arrives_after.store(usize::MAX, SeqCst),
            Failure::DownloadSlow,
            STILL_DOWNLOADING,
            &["the download was not done in time"],
        ),
        (
            |f| f.no_place.store(true, SeqCst),
            Failure::NotInFinder,
            NOT_IN_FINDER,
            &["it has no place in Finder"],
        ),
        (|f| f.no_default.store(true, SeqCst), Failure::NoApp, NO_APP, &["no app for its kind"]),
        (
            |f| f.open_refused.store(true, SeqCst),
            Failure::Open(SystemError::new("NSOSStatusErrorDomain", -10814)),
            APP_REFUSED,
            &["NSWorkspace refused", "the system answered (NSOSStatusErrorDomain)", "-10814"],
        ),
    ];
    for (breaks, failure, words, reasons) in cases {
        let fake = Fake::default();
        breaks(&fake);
        let (done, _) = press(&fake, Pick::Default);
        assert_eq!(done, Err(failure.clone()));
        assert_eq!(failure.words(), Some(words));
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
    assert!(line.contains("the system answered (another domain)") && !line.contains("Marit"), "{line}");
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
        !fake.asked().iter().any(|a| a.starts_with("finder copy")),
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

#[test]
fn open_with_goes_on_to_the_app_when_the_system_cannot_list_what_is_downloaded() {
    // The download is asked for all the same; the app that opens the file waits for the system, as after Finder's open.
    let fake = Fake::default();
    fake.no_list.store(true, SeqCst);
    let (done, said) = press(&fake, Pick::Default);
    assert_eq!(done, Ok(()));
    assert_eq!(
        fake.asked(),
        [
            "present f-docx",
            "materialise f-docx",
            "finder copy f-docx",
            &format!("default app of {FINDER}"),
            &format!("NSWorkspace open {FINDER} with {PAGES}"),
        ]
    );
    assert_eq!(said, [Note::Opening]);
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
    assert_eq!(start("f-73-test", Pick::Default, || true, || {}), Err(()));
}
