//! `EngineViewerSource` against an engine of the test's own (`Fake`): what is asked and when, what each answer
//! becomes in the view, and whose answer counts. A download waits at a door the test holds, so "while it is on its
//! way" is a state the test is in, not a race it hopes for. The engine's side (the download path, the handle, the
//! hand-over) is tested in the engine crate; the two together, in a window, in `viewer_window_tests.rs`. Every
//! name, id and byte is invented.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::viewer_api::OpenedLocalCopy;
use zeroize::Zeroizing;

use super::*;
use crate::frames::viewers::source::{Ink, Run};

const NOTE: &str = "f-note";
const NOTE_NAME: &str = "Invented harbour note.txt";
const NOTE_TEXT: &str = "first invented line\n\nthird invented line\n";

struct Stored {
    path: String,
    bytes: Vec<u8>,
    /// The size the local state names (the content's own, unless a test says otherwise).
    size: u64,
    /// What the local state says about the file's bytes on this PC.
    here: Option<OnThisPc>,
    /// The copy on this PC, when it really is wholly there (it may differ from what Beebeeb has).
    copy: Option<Vec<u8>>,
    /// The local state says the copy on this PC is not settled with Beebeeb's version.
    unsettled: bool,
}

struct Inner {
    turn: u64,
    signed_in: bool,
    account: Option<String>,
    files: HashMap<String, Stored>,
    /// What the next downloads answer instead of content.
    error: Option<OpenFileError>,
    /// The generation the next download says it decrypted for (`None`: the one it began in).
    answers_for: Option<u64>,
    panics: bool,
    facts_calls: usize,
    /// The local state answers for the session before this one.
    facts_are_old: bool,
    fetches: Vec<String>,
    /// How many downloads reached the door.
    at_door: usize,
    /// Downloads that found themselves cancelled when the door opened.
    cancelled: usize,
    wakes: usize,
    elsewhere_answers: Elsewhere,
    elsewhere_calls: Vec<String>,
    /// The next downloads hand over content even when they were cancelled (a cancel that came too late for the
    /// engine).
    late: bool,
    /// Downloads that returned.
    returned: usize,
    /// Sync runs for the account's folder.
    syncing: bool,
    /// Reads of the copy on this PC that were asked for.
    local_calls: Vec<String>,
    /// What the next reads of the copy on this PC answer instead of content.
    local_error: Option<OpenFileError>,
    /// What the next reads of the facts answer instead of facts, and how many of them (`usize::MAX`: all).
    facts_error: Option<(OpenFileError, usize)>,
    /// The session ends while the facts are read (the read that is refused finds none after it).
    session_ends_at_facts: bool,
}

struct Fake {
    s: Mutex<Inner>,
    /// Held by a test: a download waits here after its first progress report.
    door: Mutex<()>,
}

impl Fake {
    fn new() -> Arc<Self> {
        let fake = Self {
            s: Mutex::new(Inner {
                turn: 1,
                signed_in: true,
                account: Some("a@example.eu".into()),
                files: HashMap::new(),
                error: None,
                answers_for: None,
                panics: false,
                facts_calls: 0,
                facts_are_old: false,
                fetches: Vec::new(),
                at_door: 0,
                cancelled: 0,
                wakes: 0,
                elsewhere_answers: Elsewhere::Opened,
                elsewhere_calls: Vec::new(),
                late: false,
                returned: 0,
                syncing: true,
                local_calls: Vec::new(),
                local_error: None,
                facts_error: None,
                session_ends_at_facts: false,
            }),
            door: Mutex::new(()),
        };
        fake.put(NOTE, &format!("Invented folder/{NOTE_NAME}"), NOTE_TEXT.as_bytes());
        Arc::new(fake)
    }

    fn s(&self) -> MutexGuard<'_, Inner> {
        // A download that panics on purpose (`panics`) must not take the fake with it.
        self.s.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn put(&self, id: &str, path: &str, bytes: &[u8]) {
        self.s().files.insert(
            id.to_string(),
            Stored {
                path: path.to_string(),
                bytes: bytes.to_vec(),
                size: bytes.len() as u64,
                here: None,
                copy: None,
                unsettled: false,
            },
        );
    }

    /// The local state says `id` is on this PC (`here`), and `copy` is what is really there (`None`: not wholly).
    fn keep(&self, id: &str, here: OnThisPc, copy: Option<&[u8]>) {
        let mut s = self.s();
        let stored = s.files.get_mut(id).expect("a stored file");
        stored.here = Some(here);
        stored.copy = copy.map(<[u8]>::to_vec);
    }

    /// A lock and an unlock, or a sign-out and a sign-in: the generation moves, and the account is `account`.
    fn new_session(&self, account: &str) {
        let mut s = self.s();
        s.turn += 1;
        s.account = Some(account.to_string());
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

    fn facts(&self, file_id: &str) -> Result<OpenFileFacts, OpenFileError> {
        let mut s = self.s();
        s.facts_calls += 1;
        if s.session_ends_at_facts {
            s.signed_in = false;
        }
        if let Some((error, left)) = s.facts_error {
            s.facts_error = (left > 1).then_some((error, left.saturating_sub(usize::from(left != usize::MAX))));
            return Err(error);
        }
        // A read that began before a session change answers for the session before it.
        let session = SessionGeneration::for_test(s.turn - u64::from(s.facts_are_old));
        let stored = s.files.get(file_id).ok_or(OpenFileError::NotFound)?;
        let mut folders: Vec<String> = stored.path.split('/').map(str::to_string).collect();
        let name = folders.pop().unwrap();
        Ok(OpenFileFacts {
            session,
            name,
            folders,
            size: stored.size,
            on_this_pc: stored.here,
            unsettled_here: stored.unsettled,
        })
    }

    fn local(&self, file_id: &str, max_bytes: u64, cancel: &AtomicBool) -> Result<LocalCopy, OpenFileError> {
        let mut s = self.s();
        s.local_calls.push(file_id.to_string());
        if let Some(error) = s.local_error {
            return Err(error);
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(OpenFileError::Cancelled);
        }
        let turn = s.answers_for.unwrap_or(s.turn);
        let stored = s.files.get(file_id).ok_or(OpenFileError::NotFound)?;
        let (Some(here), Some(copy)) = (stored.here, stored.copy.clone()) else {
            return Ok(LocalCopy::NotHere);
        };
        if copy.len() as u64 > max_bytes {
            return Err(OpenFileError::TooLarge {
                size: copy.len() as u64,
            });
        }
        Ok(LocalCopy::Opened(OpenedLocalCopy {
            session: SessionGeneration::for_test(turn),
            bytes: Zeroizing::new(copy),
            copy: here,
        }))
    }

    fn is_syncing(&self) -> bool {
        self.s().syncing
    }

    fn fetch(
        &self,
        file_id: &str,
        max_bytes: u64,
        progress: &(dyn Fn(u64, u64) + Send + Sync + 'static),
        cancel: &AtomicBool,
    ) -> Result<OpenedFile, OpenFileError> {
        let (bytes, began) = {
            let mut s = self.s();
            s.fetches.push(file_id.to_string());
            assert!(!s.panics, "an invented panic in the download");
            (s.files.get(file_id).map(|stored| stored.bytes.clone()), s.turn)
        };
        let bytes = bytes.ok_or(OpenFileError::NotFound)?;
        let total = bytes.len() as u64;
        progress(total / 2, total);
        self.s().at_door += 1;
        drop(self.door.lock().unwrap());
        let answers_for = {
            let mut s = self.s();
            s.returned += 1;
            if cancel.load(Ordering::SeqCst) && !s.late {
                s.cancelled += 1;
                return Err(OpenFileError::Cancelled);
            }
            if let Some(error) = s.error {
                return Err(error);
            }
            s.answers_for.unwrap_or(began)
        };
        if total > max_bytes {
            return Err(OpenFileError::TooLarge { size: total });
        }
        // Not under the fake's lock: a progress report wakes the window, which the fake counts.
        progress(total, total);
        Ok(OpenedFile {
            session: SessionGeneration::for_test(answers_for),
            bytes: Zeroizing::new(bytes),
        })
    }

    fn open_elsewhere(&self, file_id: &str) -> Elsewhere {
        let mut s = self.s();
        s.elsewhere_calls.push(file_id.to_string());
        s.elsewhere_answers
    }

    fn wake(&self) {
        self.s().wakes += 1;
    }
}

type Source = EngineViewerSource<Fake>;

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let began = Instant::now();
    while !done() {
        assert!(began.elapsed() < Duration::from_secs(10), "{what} did not happen");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Poll until the view is handed a body that is not progress.
fn answered(source: &mut Source, id: &str) -> Body {
    let began = Instant::now();
    loop {
        match source.poll(id) {
            Some(Body::Decrypting { .. }) | None => {}
            Some(body) => return body,
        }
        assert!(began.elapsed() < Duration::from_secs(10), "the download did not answer");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Open `id` and take its answer.
fn opened(fake: &Arc<Fake>, id: &str) -> (Source, OpenFile, Body) {
    let mut source = Source::on(fake.clone());
    let file = source.open(id).expect("the file opens");
    let body = if matches!(file.body, Body::Decrypting { .. }) {
        answered(&mut source, id)
    } else {
        file.body.clone()
    };
    (source, file, body)
}

fn words(body: &Body) -> String {
    match body {
        Body::Text(doc) => doc.plain(),
        Body::Markdown(doc) => doc.source.clone(),
        other => panic!("not text: {other:?}"),
    }
}

/// A small PNG, encoded here.
pub(super) fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![180u8; (width * height * 4) as usize])
            .unwrap();
    }
    out
}

#[test]
fn a_text_file_says_decrypting_with_the_engines_progress_and_then_shows_its_lines() {
    let fake = Fake::new();
    let mut source = Source::on(fake.clone());
    // Nothing is read or asked before a file is opened.
    assert_eq!(fake.s().facts_calls, 0);
    assert!(fake.s().fetches.is_empty());

    let door = fake.door.lock().unwrap();
    let file = source.open(NOTE).expect("the file opens");
    let size = NOTE_TEXT.len() as u64;
    assert_eq!(file.name, NOTE_NAME);
    assert_eq!(file.path, ["Files", "Invented folder"]);
    assert_eq!(file.facts, "Text \u{00B7} 41 B");
    assert_eq!(file.body, Body::Decrypting { done: 0, total: size });
    assert_eq!(
        file.details.facts,
        [("Where", "Files \u{203A} Invented folder".to_string(), false)]
    );

    // On its way: the view is handed the engine's progress, once per change, and the window was woken for it.
    until("the download reaches the door", || fake.s().at_door == 1);
    assert_eq!(
        source.poll(NOTE),
        Some(Body::Decrypting {
            done: size / 2,
            total: size
        })
    );
    assert_eq!(source.poll(NOTE), None, "the same progress is not handed over twice");
    assert!(fake.s().wakes >= 1, "progress wakes the window");
    assert!(source.picture(NOTE, Picture::Photo).is_none());

    drop(door);
    let body = answered(&mut source, NOTE);
    assert_eq!(
        body,
        Body::Text(TextDoc {
            lines: plain_lines(NOTE_TEXT),
            truncated: None
        })
    );
    assert_eq!(source.poll(NOTE), None, "one answer, handed over once");
    assert_eq!(fake.s().fetches, [NOTE], "one download");
    assert_eq!(fake.s().facts_calls, 1, "the facts are read once");
    // Another file's id gets nothing of this one.
    assert_eq!(source.poll("another"), None);
}

#[test]
fn markdown_is_a_note_and_a_table_is_shown_as_its_text() {
    let fake = Fake::new();
    fake.put("f-md", "Invented plan.md", b"# Invented plan\n\n- one\n");
    fake.put("f-csv", "Invented rates.csv", b"name,rate\ninvented,4\n");
    fake.put("f-bom", "Invented bom.txt", "\u{FEFF}after the mark\n".as_bytes());
    let (_, file, body) = opened(&fake, "f-md");
    assert_eq!(file.facts, "Markdown \u{00B7} 23 B");
    assert_eq!(
        body,
        Body::Markdown(MdDoc {
            source: "# Invented plan\n\n- one\n".into()
        })
    );
    let (_, file, body) = opened(&fake, "f-csv");
    assert_eq!(file.facts, "Table \u{00B7} 21 B");
    assert_eq!(words(&body), "name,rate\ninvented,4\n");
    // A byte-order mark is not a character of the file.
    assert_eq!(words(&opened(&fake, "f-bom").2), "after the mark\n");
}

#[test]
fn a_text_over_the_readers_limit_shows_its_first_lines_read_only() {
    let fake = Fake::new();
    let line = "an invented line of a long log, forty-eight bytes\n";
    let lines = READ_WHOLE_MAX / line.len() + 10;
    fake.put("f-log", "Invented long.log", line.repeat(lines).as_bytes());
    fake.put("f-long-md", "Invented long.md", line.repeat(lines).as_bytes());
    for id in ["f-log", "f-long-md"] {
        let (_, _, body) = opened(&fake, id);
        let Body::Text(doc) = body else {
            panic!("{id}: a text over the limit is read as text");
        };
        assert_eq!(doc.lines.len(), SHOWN_LINES);
        assert_eq!(
            doc.truncated,
            Some(Truncated {
                shown: SHOWN_LINES as u32,
                total: lines as u32,
                why: Cut::OverTwoMb
            })
        );
        assert_eq!(doc.plain(), line.repeat(SHOWN_LINES));
    }
}

#[test]
fn a_text_the_reader_cannot_hold_whole_is_cut_by_lines_and_by_bytes_and_says_why() {
    let fake = Fake::new();
    // Under 2 MB, too many lines.
    let many = "short\n".repeat(WHOLE_LINES_MAX + 1);
    fake.put("f-many", "Invented many.log", many.as_bytes());
    // At the limit: whole.
    fake.put("f-at", "Invented at.log", "short\n".repeat(WHOLE_LINES_MAX).as_bytes());
    // Under 2 MB, one line over the line limit (and over what is held), after two short ones.
    let long_line = format!("one\ntwo\n{}\nlast\n", "x".repeat(SHOWN_BYTES * 2));
    fake.put("f-long-line", "Invented long line.json", long_line.as_bytes());
    // Over 2 MB with no newline at all, in three-byte characters: cut by bytes, on a whole character.
    let one_line = "\u{20AC}".repeat(READ_WHOLE_MAX / 3 + 10);
    fake.put("f-one-line", "Invented one line.json", one_line.as_bytes());

    let cut = |id: &str| match opened(&fake, id).2 {
        Body::Text(doc) => doc,
        other => panic!("{id}: {other:?}"),
    };
    let doc = cut("f-many");
    assert_eq!(doc.lines.len(), SHOWN_LINES);
    assert_eq!(
        doc.truncated,
        Some(Truncated {
            shown: SHOWN_LINES as u32,
            total: (WHOLE_LINES_MAX + 1) as u32,
            why: Cut::TooLong
        })
    );
    // A line exactly at the line limit is read whole; one byte more is not.
    fake.put(
        "f-line-at",
        "Invented line at.txt",
        "y".repeat(LINE_BYTES_MAX).as_bytes(),
    );
    fake.put(
        "f-line-over",
        "Invented line over.txt",
        "y".repeat(LINE_BYTES_MAX + 1).as_bytes(),
    );
    assert_eq!(cut("f-line-at").truncated, None);
    assert_eq!(cut("f-line-over").truncated.map(|cut| cut.why), Some(Cut::TooLong));
    let whole = cut("f-at");
    assert_eq!((whole.lines.len(), whole.truncated), (WHOLE_LINES_MAX, None));

    let doc = cut("f-long-line");
    assert_eq!(doc.truncated.map(|cut| (cut.total, cut.why)), Some((4, Cut::TooLong)));
    assert_eq!(doc.lines.len(), 4, "every line, the long one cut");
    let held = doc.plain();
    let longest = held.lines().map(str::len).max().unwrap();
    assert_eq!(
        longest, LINE_BYTES_MAX,
        "no line handed to the reader is over the line limit"
    );
    assert!(held.starts_with("one\ntwo\nxxx") && held.trim_end().ends_with("last"));

    // One line of 30,000 bytes (a minified file): shown cut at the line limit, never whole under a "cut" banner.
    fake.put("f-minified", "Invented minified.json", "z".repeat(30_000).as_bytes());
    let doc = cut("f-minified");
    assert_eq!(doc.truncated.map(|cut| cut.why), Some(Cut::TooLong));
    assert_eq!(doc.plain().trim_end_matches('\n').len(), LINE_BYTES_MAX);

    // Many long lines: the byte limit over all of them holds too.
    let wide = format!("{}\n", "w".repeat(LINE_BYTES_MAX + 5)).repeat(10);
    fake.put("f-wide", "Invented wide.log", wide.as_bytes());
    let doc = cut("f-wide");
    let held = doc.plain();
    assert!(held.len() <= SHOWN_BYTES + 1, "{}", held.len());
    assert!(held.lines().all(|line| line.len() <= LINE_BYTES_MAX));
    assert_eq!(
        held.lines().count(),
        4,
        "three whole cut lines and what the byte limit leaves of a fourth"
    );

    let doc = cut("f-one-line");
    assert_eq!(
        doc.truncated.map(|cut| (cut.shown, cut.total, cut.why)),
        Some((1, 1, Cut::OverTwoMb))
    );
    let held = doc.plain();
    let held = held.trim_end_matches('\n');
    assert_eq!(
        held.len(),
        LINE_BYTES_MAX - LINE_BYTES_MAX % 3,
        "cut on a whole character"
    );
    assert!(held.chars().all(|c| c == '\u{20AC}'));
}

#[test]
fn a_long_note_is_shown_as_its_text_not_rendered() {
    let fake = Fake::new();
    let line = format!("{}\n", "x".repeat(999));
    // At the rendered note's limit: a note.
    let at = line.repeat(MARKDOWN_RENDER_MAX / 1_000);
    assert_eq!(at.len(), MARKDOWN_RENDER_MAX);
    fake.put("f-at-md", "Invented at.md", at.as_bytes());
    assert_eq!(opened(&fake, "f-at-md").2, Body::Markdown(MdDoc { source: at }));
    // One line more: its text, whole (it is under every limit of the reader).
    let over = line.repeat(MARKDOWN_RENDER_MAX / 1_000 + 1);
    fake.put("f-over-md", "Invented over.md", over.as_bytes());
    assert_eq!(
        opened(&fake, "f-over-md").2,
        Body::Text(TextDoc {
            lines: plain_lines(&over),
            truncated: None
        })
    );
}

#[test]
fn a_picture_is_decoded_here_and_bytes_that_are_not_what_the_name_says_are_said_so() {
    let fake = Fake::new();
    fake.put("f-png", "Invented harbour.png", &png_bytes(6, 4));
    fake.put("f-not-png", "Invented broken.png", b"these bytes are not a picture");
    fake.put("f-not-text", "Invented binary.txt", &[0xff, 0xfe, 0x00, 0x80, 0x81]);
    fake.put("f-nul", "Invented nul.txt", b"text with a \0 in it");

    let (source, file, body) = opened(&fake, "f-png");
    assert!(file.facts.starts_with("Picture \u{00B7} "), "{}", file.facts);
    assert!(matches!(&body, Body::Photo(doc) if doc.pill.is_none() && doc.strip.is_empty()));
    let pixels = source.picture("f-png", Picture::Photo).expect("the decoded picture");
    assert_eq!(pixels.size(), (6, 4));
    // Only the photo itself: nothing else is bound, and nothing is a sample.
    for other in [Picture::Strip(0), Picture::Artwork, Picture::Page(1)] {
        assert!(source.picture("f-png", other).is_none(), "{other:?}");
    }
    assert!(source.picture("another", Picture::Photo).is_none());

    for id in ["f-not-png", "f-not-text", "f-nul"] {
        let (source, _, body) = opened(&fake, id);
        assert_eq!(body, Body::NotPreviewable(NoPreview::Unreadable), "{id}");
        assert!(source.picture(id, Picture::Photo).is_none(), "{id}");
    }
}

#[test]
fn a_kind_without_a_viewer_here_is_said_so_and_nothing_is_downloaded() {
    let fake = Fake::new();
    let kinds = [
        ("Invented take.m4a", "Audio"),
        ("Invented export.zip", "Archive"),
        ("Invented shot.arw", "Picture"),
        ("Invented shot.heic", "Picture"),
        ("Invented letter.docx", "File"),
        ("Invented sheet.xlsx", "File"),
        ("Invented thing.xyz", "File"),
        ("Invented no extension", "File"),
    ];
    for (i, (name, kind)) in kinds.iter().enumerate() {
        let id = format!("f-kind-{i}");
        fake.put(&id, name, b"some invented bytes");
        let mut source = Source::on(fake.clone());
        let file = source.open(&id).expect("it is a file of the account");
        assert_eq!(file.body, Body::NotPreviewable(NoPreview::NoViewer), "{name}");
        assert_eq!(file.facts, format!("{kind} \u{00B7} 19 B"), "{name}");
        assert_eq!(source.poll(&id), None, "{name}");
        // A list's Open leaves such a file to its own app.
        assert!(!source.previews(name), "{name}");
    }
    assert!(fake.s().fetches.is_empty(), "nothing was downloaded");
    let source = Source::on(fake.clone());
    for shown in ["a.txt", "a.md", "a.csv", "a.PNG", "a.jpeg", "a.gif", "a.webp", "a.log"] {
        assert!(source.previews(shown), "{shown}");
    }
}

#[test]
fn a_file_over_the_preview_limit_is_not_downloaded() {
    let fake = Fake::new();
    // The local state names the size: nothing is asked of Beebeeb.
    fake.s().files.get_mut(NOTE).unwrap().size = IN_MEMORY_MAX + 1;
    let (_, _, body) = opened(&fake, NOTE);
    assert_eq!(
        body,
        Body::NotPreviewable(NoPreview::TooBig {
            size: IN_MEMORY_MAX + 1,
            limit: IN_MEMORY_MAX
        })
    );
    assert!(fake.s().fetches.is_empty());
    // At the limit it is downloaded.
    fake.s().files.get_mut(NOTE).unwrap().size = IN_MEMORY_MAX;
    assert_eq!(words(&opened(&fake, NOTE).2), NOTE_TEXT);

    // The local state named a small size and Beebeeb's version is larger: the engine refuses, and the view says so.
    fake.s().error = Some(OpenFileError::TooLarge { size: 90_000_000 });
    assert_eq!(
        opened(&fake, NOTE).2,
        Body::NotPreviewable(NoPreview::TooBig {
            size: 90_000_000,
            limit: IN_MEMORY_MAX
        })
    );
}

#[test]
fn each_refusal_of_the_engine_is_a_state_the_view_draws_and_trying_again_downloads_again() {
    let fake = Fake::new();
    let cases = [
        (OpenFileError::Offline, Body::Stopped(Stopped::Offline)),
        (OpenFileError::SessionEnded, Body::Stopped(Stopped::SessionEnded)),
        (OpenFileError::NotSignedIn, Body::Stopped(Stopped::SessionChanged)),
        (OpenFileError::NotYours, Body::Stopped(Stopped::SessionChanged)),
        (OpenFileError::NotFound, Body::Stopped(Stopped::Gone)),
        (OpenFileError::Failed, Body::Failed),
    ];
    for (error, shown) in cases {
        fake.s().error = Some(error);
        let (mut source, _, body) = opened(&fake, NOTE);
        assert_eq!(body, shown, "{error:?}");
        assert_eq!(source.poll(NOTE), None, "{error:?}: said once");
        assert!(source.picture(NOTE, Picture::Photo).is_none());

        // Try again: a new download, and this time Beebeeb answers.
        fake.s().error = None;
        let before = fake.s().fetches.len();
        let again = source.retry(NOTE).expect("the retry starts");
        assert!(matches!(again.body, Body::Decrypting { .. }), "{error:?}");
        assert_eq!(words(&answered(&mut source, NOTE)), NOTE_TEXT, "{error:?}");
        assert_eq!(fake.s().fetches.len(), before + 1, "{error:?}");
    }
}

#[test]
fn a_download_that_panics_ends_as_failed_not_as_decrypting_for_ever() {
    let fake = Fake::new();
    fake.s().panics = true;
    assert_eq!(opened(&fake, NOTE).2, Body::Failed);
    assert!(fake.s().wakes >= 1, "the window is woken for the failure");
}

#[test]
fn without_a_session_or_without_the_file_nothing_opens() {
    let fake = Fake::new();
    let mut source = Source::on(fake.clone());
    assert_eq!(source.open("no-such-file").err(), Some(OpenError::NotFound));
    fake.s().signed_in = false;
    let facts_before = fake.s().facts_calls;
    assert_eq!(source.open(NOTE).err(), Some(OpenError::SignedOut));
    assert_eq!(source.retry(NOTE).err(), Some(OpenError::SignedOut));
    assert_eq!(
        fake.s().facts_calls,
        facts_before,
        "signed out, the local state is not asked"
    );
    assert!(fake.s().fetches.is_empty());
    assert_eq!(source.poll(NOTE), None);
}

#[test]
fn facts_read_across_a_session_change_open_nothing() {
    let fake = Fake::new();
    let mut source = Source::on(fake.clone());
    fake.s().facts_are_old = true;
    // Not "no such file": it could not be read for this session right now, after one more try.
    assert_eq!(source.open(NOTE).err(), Some(OpenError::NotNow));
    assert_eq!(
        fake.s().facts_calls,
        2,
        "the read is tried once more before it is refused"
    );
    assert!(
        fake.s().fetches.is_empty(),
        "nothing is downloaded for facts of another session"
    );
    assert_eq!(source.poll(NOTE), None);
    fake.s().facts_are_old = false;
    assert!(source.open(NOTE).is_ok());
}

#[test]
fn an_answer_that_outlived_its_account_is_never_shown() {
    let fake = Fake::new();
    let mut source = Source::on(fake.clone());
    let door = fake.door.lock().unwrap();
    source.open(NOTE).expect("the file opens");
    until("the download reaches the door", || fake.s().at_door == 1);
    // A sign-out and another account's sign-in while the download is on its way.
    fake.new_session("b@example.eu");
    // The window's side, at once, without the worker: the view holds nothing of the file.
    assert_eq!(source.poll(NOTE), Some(Body::Stopped(Stopped::SessionChanged)));
    assert_eq!(source.poll(NOTE), None);
    // The download was told to stop.
    drop(door);
    until("the download saw its cancel", || fake.s().cancelled == 1);
    for _ in 0..20 {
        assert_eq!(source.poll(NOTE), None, "nothing of the first account arrives later");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(source.open_elsewhere(NOTE), Elsewhere::Refused);
    assert!(
        fake.s().elsewhere_calls.is_empty(),
        "the system is not asked for a file of a session that is gone"
    );
}

#[test]
fn a_lock_takes_the_shown_content_and_the_picture_on_the_windows_side() {
    let fake = Fake::new();
    fake.put("f-png", "Invented harbour.png", &png_bytes(4, 4));
    let (mut source, _, body) = opened(&fake, "f-png");
    assert!(matches!(body, Body::Photo(_)));
    assert!(source.picture("f-png", Picture::Photo).is_some());
    // A lock and an unlock of the same account: the generation moved, the account did not.
    fake.new_session("a@example.eu");
    // Before any poll: the accessor already hands out nothing.
    assert!(source.picture("f-png", Picture::Photo).is_none());
    assert_eq!(source.poll("f-png"), Some(Body::Stopped(Stopped::SessionChanged)));
    assert_eq!(source.poll("f-png"), None);
    assert!(source.picture("f-png", Picture::Photo).is_none());
    // Released, not only hidden: decoded pixels cannot be overwritten, so letting go of them is all there is.
    assert!(
        source.open.as_ref().is_some_and(|opening| opening.picture.is_none()),
        "the ended opening still holds the picture"
    );
    // Open it again: the new session's download.
    let again = source.retry("f-png").expect("the retry starts");
    assert!(matches!(again.body, Body::Decrypting { .. }));
    assert!(matches!(answered(&mut source, "f-png"), Body::Photo(_)));
    assert!(source.picture("f-png", Picture::Photo).is_some());
}

#[test]
fn another_account_with_the_same_generation_is_not_the_session_either() {
    let fake = Fake::new();
    let (mut source, _, body) = opened(&fake, NOTE);
    assert_eq!(words(&body), NOTE_TEXT);
    // The generation is what it was; the account is not (the identity is both).
    fake.s().account = Some("b@example.eu".into());
    assert_eq!(source.poll(NOTE), Some(Body::Stopped(Stopped::SessionChanged)));
}

#[test]
fn a_state_that_holds_nothing_stays_and_acts_for_the_same_account_after_a_lock() {
    let fake = Fake::new();
    fake.put("f-pdf", "Invented plan.zip", b"PK invented");
    let (mut source, _, body) = opened(&fake, "f-pdf");
    assert_eq!(body, Body::NotPreviewable(NoPreview::NoViewer));
    // A lock and an unlock of the same account: there was no content to take, the state stays, and its button
    // still opens the file in another app, for the session that is there now.
    fake.new_session("a@example.eu");
    assert_eq!(source.poll("f-pdf"), None);
    assert_eq!(source.open_elsewhere("f-pdf"), Elsewhere::Opened);
    assert_eq!(fake.s().elsewhere_calls, ["f-pdf"]);
    // Another account: nothing is asked for the first account's file, before the poll and after it.
    fake.new_session("b@example.eu");
    assert_eq!(source.open_elsewhere("f-pdf"), Elsewhere::Refused);
    assert_eq!(source.poll("f-pdf"), None);
    assert_eq!(source.open_elsewhere("f-pdf"), Elsewhere::Refused);
    // And not after the first account is back either: this opening is over (the window closed the file).
    fake.new_session("a@example.eu");
    assert_eq!(source.poll("f-pdf"), None);
    assert_eq!(source.open_elsewhere("f-pdf"), Elsewhere::Refused);
    assert_eq!(fake.s().elsewhere_calls.len(), 1);
}

#[test]
fn content_that_arrives_after_its_session_moved_on_is_dropped_by_the_worker_not_queued() {
    let fake = Fake::new();
    // The cancel comes too late for the engine: it hands the worker the decrypted file anyway.
    fake.s().late = true;
    let mut source = Source::on(fake.clone());
    let door = fake.door.lock().unwrap();
    source.open(NOTE).expect("the file opens");
    until("the download reaches the door", || fake.s().at_door == 1);
    fake.new_session("a@example.eu");
    assert_eq!(source.poll(NOTE), Some(Body::Stopped(Stopped::SessionChanged)));
    drop(door);
    until("the download returned", || fake.s().returned == 1);
    // What the worker queues holds no content: it says "cancelled" and nothing else. Waited for on the queue
    // itself: a wake is not the answer (the download's last progress report wakes the window too).
    let mut queued = None;
    until("the worker answered", || {
        queued = source.open.as_ref().unwrap().inbox.try_recv().ok();
        queued.is_some()
    });
    assert!(
        matches!(queued, Some(Err(OpenFileError::Cancelled))),
        "decrypted content was queued for a session that is gone"
    );
    assert_eq!(source.poll(NOTE), None);
}

#[test]
fn content_decrypted_for_another_generation_is_never_shown() {
    let fake = Fake::new();
    // The engine's answer says it was decrypted for a session that is not the one the file was opened for, while
    // the session the window sees did not move.
    fake.s().answers_for = Some(7);
    let (mut source, _, body) = opened(&fake, NOTE);
    assert_eq!(body, Body::Stopped(Stopped::SessionChanged));
    assert_eq!(source.poll(NOTE), None);
}

#[test]
fn cancel_and_closing_stop_the_download() {
    let fake = Fake::new();
    // Cancel: the download is told, and its "cancelled" changes nothing on screen.
    let mut source = Source::on(fake.clone());
    let door = fake.door.lock().unwrap();
    source.open(NOTE).expect("the file opens");
    until("the download reaches the door", || fake.s().at_door == 1);
    source.cancel(NOTE);
    drop(door);
    until("the download saw its cancel", || fake.s().cancelled == 1);
    for _ in 0..20 {
        assert!(!matches!(source.poll(NOTE), Some(Body::Text(_) | Body::Failed)));
        std::thread::sleep(Duration::from_millis(2));
    }

    // Closing the file drops the source: the download is told too.
    let mut source = Source::on(fake.clone());
    let door = fake.door.lock().unwrap();
    source.open(NOTE).expect("the file opens");
    until("the second download reaches the door", || fake.s().at_door == 2);
    drop(source);
    drop(door);
    until("the second download saw its cancel", || fake.s().cancelled == 2);
}

#[test]
fn open_in_another_app_asks_the_engine_and_says_when_it_was_refused() {
    let fake = Fake::new();
    let (mut source, _, _) = opened(&fake, NOTE);
    assert_eq!(source.open_elsewhere(NOTE), Elsewhere::Opened);
    assert_eq!(fake.s().elsewhere_calls, [NOTE]);
    // The engine's refusal keeps its kind: the view says a different sentence for each.
    for refusal in [Elsewhere::NeedsSync, Elsewhere::Refused] {
        fake.s().elsewhere_answers = refusal;
        assert_eq!(source.open_elsewhere(NOTE), refusal);
    }
    // A file this source does not have open is not opened anywhere.
    fake.s().elsewhere_answers = Elsewhere::Opened;
    assert_eq!(source.open_elsewhere("another"), Elsewhere::Refused);
    assert_eq!(fake.s().elsewhere_calls.len(), 3);
    // What is not bound says so to the view.
    assert!(!source.can_save());
    assert!(!source.knows_offline());
    assert!(!source.set_offline(NOTE, true));
}

#[test]
fn an_open_files_text_is_overwritten_when_it_is_let_go_and_never_printed() {
    let fake = Fake::new();
    let (_, mut file, body) = opened(&fake, NOTE);
    file.body = body;
    let printed = format!("{file:?} {:?}", file.body);
    assert!(
        !printed.contains("invented") && !printed.contains("Invented"),
        "{printed}"
    );
    assert!(printed.contains("3 lines"), "{printed}");
    let run = Run {
        text: "an invented secret".into(),
        ink: Ink::Plain,
        bold: false,
        italic: false,
        underline: false,
    };
    assert_eq!(format!("{run:?}"), "Run(Plain, 18 bytes)");
    assert_eq!(
        format!(
            "{:?}",
            MdDoc {
                source: "an invented note".into()
            }
        ),
        "MdDoc(16 bytes)"
    );

    file.wipe();
    let Body::Text(doc) = &file.body else {
        panic!("still a text")
    };
    assert!(
        doc.lines.iter().flatten().all(|run| run.text.is_empty()),
        "every line is overwritten"
    );
    let mut note = Body::Markdown(MdDoc {
        source: "an invented note".into(),
    });
    note.wipe();
    assert_eq!(note, Body::Markdown(MdDoc { source: String::new() }));
}

/// A source whose open file is in the state `body`, and that counts what Try again asks of it.
struct InState {
    body: Body,
    retries: std::rc::Rc<std::cell::Cell<u32>>,
    /// Whether a retry can start.
    there: bool,
}

impl InState {
    fn file(&self, file_id: &str, body: Body) -> OpenFile {
        OpenFile {
            id: file_id.to_string(),
            path: vec!["Files".into()],
            name: "Invented state.txt".into(),
            facts: "Text".into(),
            version: 0,
            body,
            details: Details::default(),
            newer: None,
        }
    }
}

impl ViewerSource for InState {
    fn open(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        Ok(self.file(file_id, self.body.clone()))
    }
    fn poll(&mut self, _: &str) -> Option<Body> {
        None
    }
    fn cancel(&mut self, _: &str) {}
    fn retry(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        self.retries.set(self.retries.get() + 1);
        if self.there {
            Ok(self.file(file_id, Body::Decrypting { done: 0, total: 10 }))
        } else {
            // A refusal the window has no state of its own for.
            Err(OpenError::SignedOut)
        }
    }
}

fn ui_in(body: Body, there: bool) -> (crate::frames::viewers::Ui, std::rc::Rc<std::cell::Cell<u32>>) {
    let retries = std::rc::Rc::new(std::cell::Cell::new(0));
    let source = InState {
        body,
        retries: retries.clone(),
        there,
    };
    let ui = crate::frames::viewers::Ui::open(Box::new(source), "f-state").expect("it opened");
    (ui, retries)
}

#[test]
fn try_again_is_asked_only_from_a_state_it_can_get_past() {
    let can = [
        Body::Failed,
        Body::Stopped(Stopped::Offline),
        Body::Stopped(Stopped::SessionChanged),
    ];
    for body in can {
        let (mut ui, retries) = ui_in(body.clone(), true);
        ui.retry();
        assert_eq!(retries.get(), 1, "{body:?}");
        assert_eq!(ui.file.body, Body::Decrypting { done: 0, total: 10 }, "{body:?}");
    }
    let cannot = [
        Body::Stopped(Stopped::SessionEnded),
        Body::Stopped(Stopped::Gone),
        Body::NotPreviewable(NoPreview::NoViewer),
        Body::Decrypting { done: 1, total: 10 },
    ];
    for body in cannot {
        let (mut ui, retries) = ui_in(body.clone(), true);
        ui.retry();
        assert_eq!(retries.get(), 0, "{body:?}");
        assert_eq!(ui.file.body, body);
    }
}

#[test]
fn try_again_that_cannot_even_start_says_so() {
    let (mut ui, retries) = ui_in(Body::Failed, false);
    ui.retry();
    assert_eq!(retries.get(), 1);
    assert_eq!(ui.toast.as_deref(), Some(crate::frames::viewers::NOT_REOPENED));
    assert_eq!(ui.file.body, Body::Failed, "what was on screen stays");
}

#[test]
fn a_source_that_cannot_save_or_keep_offline_says_so_on_the_press() {
    // The default source (the board's) can: the editor opens.
    struct CannotSave(InState);
    impl ViewerSource for CannotSave {
        fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            self.0.open(id)
        }
        fn poll(&mut self, id: &str) -> Option<Body> {
            self.0.poll(id)
        }
        fn cancel(&mut self, _: &str) {}
        fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            self.0.retry(id)
        }
        fn can_save(&self) -> bool {
            false
        }
        fn knows_offline(&self) -> bool {
            false
        }
    }
    let text = Body::Text(TextDoc {
        lines: plain_lines("an invented line"),
        truncated: None,
    });
    let (mut can, _) = ui_in(text.clone(), true);
    can.edit();
    assert!(can.editor.is_some(), "a source that saves opens the editor");

    let inner = InState {
        body: text,
        retries: Default::default(),
        there: true,
    };
    let mut cannot = crate::frames::viewers::Ui::open(Box::new(CannotSave(inner)), "f-state").expect("it opened");
    cannot.edit();
    assert!(cannot.editor.is_none());
    assert_eq!(cannot.toast.as_deref(), Some("Not built yet: Edit and save from here"));
    cannot.toggle_offline();
    assert!(!cannot.file.details.offline, "nothing was switched");
    assert_eq!(
        cannot.toast.as_deref(),
        Some("Not built yet: Available offline from here")
    );
    assert!(!cannot.knows_offline());
}

#[path = "viewer_round2_tests.rs"]
mod round2;
