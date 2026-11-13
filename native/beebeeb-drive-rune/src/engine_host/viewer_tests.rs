//! `EngineViewerSource` against an engine of the test's own (`Fake`): what is asked and when, what each answer
//! becomes in the view, and whose answer counts. A download waits at a door the test holds, so "while it is on its
//! way" is a state the test is in, not a race it hopes for. The engine's side (the download path, the handle, the
//! hand-over) is tested in the engine crate; the two together, in a window, in `viewer_window_tests.rs`. Every
//! name, id and byte is invented.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
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
    fetches: Vec<String>,
    /// How many downloads reached the door.
    at_door: usize,
    /// Downloads that found themselves cancelled when the door opened.
    cancelled: usize,
    wakes: usize,
    elsewhere_answers: bool,
    elsewhere_calls: Vec<String>,
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
                fetches: Vec::new(),
                at_door: 0,
                cancelled: 0,
                wakes: 0,
                elsewhere_answers: true,
                elsewhere_calls: Vec::new(),
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
            },
        );
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
        let session = SessionGeneration::for_test(s.turn);
        let stored = s.files.get(file_id).ok_or(OpenFileError::NotFound)?;
        let mut folders: Vec<String> = stored.path.split('/').map(str::to_string).collect();
        let name = folders.pop().unwrap();
        Ok(OpenFileFacts {
            session,
            name,
            folders,
            size: stored.size,
        })
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
            if cancel.load(Ordering::SeqCst) {
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

    fn open_elsewhere(&self, file_id: &str) -> bool {
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
fn png_bytes(width: u32, height: u32) -> Vec<u8> {
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
                total: lines as u32
            })
        );
        assert_eq!(doc.plain(), line.repeat(SHOWN_LINES));
    }
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
        ("Invented plan.pdf", "PDF"),
        ("Invented take.m4a", "Audio"),
        ("Invented export.zip", "Archive"),
        ("Invented shot.arw", "Picture"),
        ("Invented shot.heic", "Picture"),
        ("Invented letter.docx", "Document"),
        ("Invented sheet.xlsx", "Document"),
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
    }
    assert!(fake.s().fetches.is_empty(), "nothing was downloaded");
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
    assert_eq!(source.open(NOTE).err(), Some(OpenError::NotFound));
    assert_eq!(source.retry(NOTE).err(), Some(OpenError::NotFound));
    assert_eq!(
        fake.s().facts_calls,
        facts_before,
        "signed out, the local state is not asked"
    );
    assert!(fake.s().fetches.is_empty());
    assert_eq!(source.poll(NOTE), None);
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
fn a_state_that_holds_nothing_stays_when_the_session_moves_on() {
    let fake = Fake::new();
    fake.put("f-pdf", "Invented plan.pdf", b"%PDF invented");
    let (mut source, _, body) = opened(&fake, "f-pdf");
    assert_eq!(body, Body::NotPreviewable(NoPreview::NoViewer));
    fake.new_session("a@example.eu");
    assert_eq!(source.poll("f-pdf"), None, "there was no content to take");
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
    fake.s().elsewhere_answers = false;
    assert_eq!(source.open_elsewhere(NOTE), Elsewhere::Refused);
    // A file this source does not have open is not opened anywhere.
    assert_eq!(source.open_elsewhere("another"), Elsewhere::Refused);
    assert_eq!(fake.s().elsewhere_calls.len(), 2);
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
