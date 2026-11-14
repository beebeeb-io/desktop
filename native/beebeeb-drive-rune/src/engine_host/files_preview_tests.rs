//! The Preview panel's read on the engine seam (`Previews`), against a double of the engine with hooks: what is
//! previewed and what is not, what is downloaded and when, and the rule that an answer belongs to the session and
//! the account it was asked under, to the file asked, and to a panel that is still open. Every name and byte is
//! invented.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::time::Duration;

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::viewer_api::{LocalCopy, OpenFileError, OpenFileFacts, OpenedFile};
use zeroize::Zeroizing;

use super::*;
use crate::frames::viewers::source::Elsewhere;

const WAIT: Duration = Duration::from_secs(30);

struct Inner {
    turn: u64,
    signed_in: bool,
    account: Option<String>,
    content: Vec<u8>,
    error: Option<OpenFileError>,
    fetches: usize,
    locals: usize,
    /// The copy on this PC answers instead of a download.
    on_this_pc: bool,
}

struct Fake(Mutex<Inner>);

impl Fake {
    fn new(content: &[u8]) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Inner {
            turn: 1,
            signed_in: true,
            account: Some("a@example.eu".into()),
            content: content.to_vec(),
            error: None,
            fetches: 0,
            locals: 0,
            on_this_pc: false,
        })))
    }
    fn s(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap()
    }
}

impl ViewerEngine for Fake {
    fn session_turn(&self) -> SessionGeneration {
        SessionGeneration::for_test(self.s().turn)
    }
    fn has_session(&self) -> bool {
        self.s().signed_in
    }
    fn account(&self) -> Option<String> {
        self.s().account.clone()
    }
    fn facts(&self, _: &str) -> Result<OpenFileFacts, OpenFileError> {
        Err(OpenFileError::Failed)
    }
    fn fetch(
        &self,
        _: &str,
        _: u64,
        _: &(dyn Fn(u64, u64) + Send + Sync + 'static),
        _: &AtomicBool,
    ) -> Result<OpenedFile, OpenFileError> {
        let mut s = self.s();
        s.fetches += 1;
        if let Some(why) = s.error {
            return Err(why);
        }
        Ok(OpenedFile {
            session: SessionGeneration::for_test(s.turn),
            bytes: Zeroizing::new(s.content.clone()),
        })
    }
    fn local(&self, _: &str, _: u64, _: &AtomicBool) -> Result<LocalCopy, OpenFileError> {
        let mut s = self.s();
        s.locals += 1;
        if !s.on_this_pc {
            return Ok(LocalCopy::NotHere);
        }
        Ok(LocalCopy::Opened(beebeeb_engine::app::viewer_api::OpenedLocalCopy {
            session: SessionGeneration::for_test(s.turn),
            bytes: Zeroizing::new(s.content.clone()),
            copy: beebeeb_engine::app::viewer_api::OnThisPc::InSync,
        }))
    }
    fn is_syncing(&self) -> bool {
        true
    }
    fn open_elsewhere(&self, _: &str) -> Elsewhere {
        Elsewhere::Refused
    }
    fn wake(&self) {}
}

fn file(id: &str, name: &str, size: i64) -> DriveItem {
    DriveItem {
        id: id.to_string(),
        parent_id: None,
        name: name.to_string(),
        is_folder: false,
        size: Some(size),
        child_count: None,
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    }
}

fn notes(size: i64) -> DriveItem {
    file("n1", "invented notes.txt", size)
}

fn hold_next_call(previews: &mut Previews<Fake>) -> (mpsc::Sender<()>, mpsc::Receiver<()>) {
    let ((release, hold), (ended, has_ended)) = (mpsc::channel(), mpsc::channel());
    previews.next_call = Some((hold, ended));
    (release, has_ended)
}

fn watch_next_call(previews: &mut Previews<Fake>) -> mpsc::Receiver<()> {
    let (release, ended) = hold_next_call(previews);
    release.send(()).expect("the hook is set");
    ended
}

/// Start the preview of `item`, wait for the answer to be in the inbox, and ask where it stands.
fn preview_now(previews: &mut Previews<Fake>, item: &DriveItem) -> Previewed {
    let ended = watch_next_call(previews);
    previews.start(item);
    ended.recv_timeout(WAIT).expect("the read ended");
    previews.later(item)
}

fn calls(fake: &Fake) -> (usize, usize) {
    let s = fake.s();
    (s.locals, s.fetches)
}

fn lines_of(text: &str) -> usize {
    text.lines().count()
}

#[test]
fn a_text_file_previews_its_first_lines() {
    let body: String = (1..=100).map(|n| format!("line number {n}\n")).collect();
    let fake = Fake::new(body.as_bytes());
    let mut previews = Previews::new(fake.clone());
    let item = notes(body.len() as i64);
    let Previewed::Shown(shown) = preview_now(&mut previews, &item) else {
        panic!("not shown")
    };
    assert_eq!(shown.place, "The first lines");
    assert_eq!(shown.page.title, "invented notes.txt");
    assert_eq!(
        shown.page.byline,
        format!("Text \u{00B7} {}", format_size(body.len() as i64))
    );
    let text = &shown.page.sections[0].1;
    assert_eq!(lines_of(text), 60);
    assert!(text.starts_with("line number 1\n") && text.ends_with("line number 60"));
    // A short file is shown whole.
    let fake = Fake::new(b"one\ntwo\n");
    let mut previews = Previews::new(fake);
    let Previewed::Shown(whole) = preview_now(&mut previews, &notes(8)) else {
        panic!("not shown")
    };
    assert_eq!(whole.place, "The whole file");
    assert_eq!(whole.page.sections[0].1, "one\ntwo");
    // Long lines: at most 6,000 bytes, cut at a character.
    let wide: String = "\u{00E9}".repeat(5_000);
    let fake = Fake::new(wide.as_bytes());
    let mut previews = Previews::new(fake);
    let Previewed::Shown(cut) = preview_now(&mut previews, &notes(10_000)) else {
        panic!("not shown")
    };
    assert_eq!(cut.place, "The first lines");
    assert!(cut.page.sections[0].1.len() <= 6_000);
    assert!(cut.page.sections[0].1.chars().all(|c| c == '\u{00E9}'));
}

#[test]
fn a_copy_on_this_pc_opens_without_a_download() {
    let fake = Fake::new(b"here\n");
    fake.s().on_this_pc = true;
    let mut previews = Previews::new(fake.clone());
    assert!(matches!(preview_now(&mut previews, &notes(5)), Previewed::Shown(_)));
    assert_eq!(calls(&fake), (1, 0));
}

#[test]
fn kinds_without_a_preview_download_nothing() {
    let fake = Fake::new(b"bytes");
    let mut previews = Previews::new(fake.clone());
    let cases = [
        (file("z", "invented backup.zip", 10), Previewed::NoPreview),
        (file("p", "invented paper.pdf", 10), Previewed::NotYet),
        (file("i", "invented picture.png", 10), Previewed::NotYet),
        (file("b", "invented big.txt", 2_000_001), Previewed::TooLarge),
        (file("m", "invented big.md", 3_000_000), Previewed::TooLarge),
    ];
    for (item, expected) in cases {
        // Nothing is started: a hook set now stays unused.
        let (_release, _ended) = hold_next_call(&mut previews);
        previews.start(&item);
        assert_eq!(previews.later(&item), expected, "{}", item.name);
        assert!(previews.next_call.is_some(), "a worker was started for {}", item.name);
    }
    assert_eq!(calls(&fake), (0, 0), "nothing was read or downloaded");
}

#[test]
fn the_panel_hears_loading_until_the_read_lands() {
    let fake = Fake::new(b"text\n");
    let mut previews = Previews::new(fake);
    let item = notes(5);
    let (release, ended) = hold_next_call(&mut previews);
    previews.start(&item);
    assert_eq!(previews.later(&item), Previewed::Loading);
    assert_eq!(previews.later(&item), Previewed::Loading);
    release.send(()).unwrap();
    ended.recv_timeout(WAIT).unwrap();
    assert!(matches!(previews.later(&item), Previewed::Shown(_)));
}

#[test]
fn errors_are_said_by_their_kind() {
    for (error, expected) in [
        (OpenFileError::Offline, Previewed::Failed(ActionError::Offline)),
        (OpenFileError::TooLarge { size: 3 }, Previewed::TooLarge),
        (OpenFileError::OverLimit, Previewed::TooLarge),
        (OpenFileError::NotSignedIn, Previewed::Failed(ActionError::Unavailable)),
        (OpenFileError::Failed, Previewed::Failed(ActionError::Failed)),
        (OpenFileError::NotFound, Previewed::Failed(ActionError::Failed)),
    ] {
        let fake = Fake::new(b"x");
        fake.s().error = Some(error);
        let mut previews = Previews::new(fake);
        assert_eq!(preview_now(&mut previews, &notes(1)), expected, "{error:?}");
    }
}

#[test]
fn closing_the_panel_cancels_and_wipes() {
    let fake = Fake::new(b"secret lines\n");
    let mut previews = Previews::new(fake);
    let item = notes(13);
    let (release, ended) = hold_next_call(&mut previews);
    previews.start(&item);
    let cancel = previews.ask.as_ref().expect("an ask").cancel.clone();
    assert!(!cancel.load(Ordering::SeqCst));
    previews.cancel();
    assert!(cancel.load(Ordering::SeqCst), "the read was told to stop");
    assert!(previews.ask.is_none(), "nothing is held");
    release.send(()).unwrap();
    ended.recv_timeout(WAIT).unwrap();
    // The answer that lands later is dropped: nothing shown.
    assert_eq!(previews.later(&item), Previewed::Failed(ActionError::Unavailable));
    assert!(previews.ask.is_none());
}

#[test]
fn a_preview_of_an_earlier_session_is_never_handed_over() {
    let fake = Fake::new(b"first account's text\n");
    let mut previews = Previews::new(fake.clone());
    let item = notes(21);
    let (release, ended) = hold_next_call(&mut previews);
    previews.start(&item);
    // A lock and an unlock: the session moved on while the worker waited.
    fake.s().turn += 1;
    release.send(()).unwrap();
    ended.recv_timeout(WAIT).unwrap();
    assert!(!matches!(previews.later(&item), Previewed::Shown(_)));
    assert!(previews.ask.is_none(), "nothing of the earlier session is held");
    // Another account in the same turn of a later session is as little shown.
    let (release, ended) = hold_next_call(&mut previews);
    previews.start(&item);
    fake.s().account = Some("b@example.eu".into());
    release.send(()).unwrap();
    ended.recv_timeout(WAIT).unwrap();
    assert!(!matches!(previews.later(&item), Previewed::Shown(_)));
    // Signed out: nothing is handed over at all.
    let (release, ended) = hold_next_call(&mut previews);
    previews.start(&item);
    fake.s().signed_in = false;
    release.send(()).unwrap();
    ended.recv_timeout(WAIT).unwrap();
    assert!(!matches!(previews.later(&item), Previewed::Shown(_)));
}

#[test]
fn stepping_to_the_next_file_never_shows_the_first_files_text() {
    let fake = Fake::new(b"the first file's text\n");
    let mut previews = Previews::new(fake);
    let (first, second) = (file("f1", "first.txt", 22), file("f2", "second.txt", 22));
    let (release, ended) = hold_next_call(&mut previews);
    previews.start(&first);
    // The panel stepped to the next file, whose read is held back; the first read lands meanwhile.
    previews.cancel();
    let (release_second, ended_second) = hold_next_call(&mut previews);
    previews.start(&second);
    release.send(()).unwrap();
    ended.recv_timeout(WAIT).unwrap();
    assert_eq!(
        previews.later(&second),
        Previewed::Loading,
        "the first file's text is not the second's"
    );
    assert_eq!(
        previews.later(&first),
        Previewed::Failed(ActionError::Unavailable),
        "and nothing is handed out for a file that is not the panel's"
    );
    release_second.send(()).unwrap();
    ended_second.recv_timeout(WAIT).unwrap();
    let Previewed::Shown(shown) = previews.later(&second) else {
        panic!("not shown")
    };
    assert_eq!(shown.page.title, "second.txt");
}

#[test]
fn an_answer_for_another_file_than_the_one_asked_is_dropped() {
    // The same ask, answered for an id that is not its own (the pump's own check, without the cancel in front).
    let fake = Fake::new(b"text\n");
    let mut previews = Previews::new(fake);
    let item = notes(5);
    let (release, ended) = hold_next_call(&mut previews);
    previews.start(&item);
    let now = stamp_of(&*previews.engine);
    previews
        .answers
        .send(Answer {
            id: "another file".into(),
            pressed: now.clone(),
            session: Some(now.0),
            outcome: Outcome::Text {
                text: Zeroizing::new("not this file's text".into()),
                cut: false,
            },
        })
        .unwrap();
    assert_eq!(
        previews.later(&item),
        Previewed::Loading,
        "what the other file sent is not shown"
    );
    release.send(()).unwrap();
    ended.recv_timeout(WAIT).unwrap();
    let Previewed::Shown(shown) = previews.later(&item) else {
        panic!("not shown")
    };
    assert!(!shown.page.sections[0].1.contains("not this file"));
}

#[test]
fn shown_is_handed_over_once() {
    let fake = Fake::new(b"once\n");
    let mut previews = Previews::new(fake);
    let item = notes(5);
    assert!(matches!(preview_now(&mut previews, &item), Previewed::Shown(_)));
    let again = previews.later(&item);
    assert!(!matches!(again, Previewed::Shown(_)), "the second call holds no text");
    assert!(matches!(previews.ask.as_ref().map(|ask| &ask.held), Some(Held::Gone)));
}

#[test]
fn bytes_that_are_not_text_have_no_preview() {
    for bytes in [&b"a\xff\xfe broken"[..], &b"nul\0inside"[..]] {
        let fake = Fake::new(bytes);
        let mut previews = Previews::new(fake);
        assert_eq!(preview_now(&mut previews, &notes(10)), Previewed::NoPreview);
    }
}

#[test]
fn nothing_is_asked_without_a_session() {
    let fake = Fake::new(b"x");
    fake.s().signed_in = false;
    let mut previews = Previews::new(fake.clone());
    let item = notes(1);
    previews.start(&item);
    assert_eq!(previews.later(&item), Previewed::Failed(ActionError::Unavailable));
    assert_eq!(calls(&fake), (0, 0));
}
