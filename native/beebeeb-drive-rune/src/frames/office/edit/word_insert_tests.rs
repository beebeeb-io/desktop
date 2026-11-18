//! Tests of the Insert tab (DW-1550): the bounds and the words of Table, Link and Image, the picture's way from a file
//! to the source, and that the tools do their work only while the editor is live.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use beebeeb_office_protocol::check;
use beebeeb_office_protocol::office::{EditKind, EditOp};
use rune::{Env, Key, KeyEvent, Mods, Runtime};
use zeroize::Zeroizing;

use super::*;
use crate::frames::office::edit::EditUi;
use crate::frames::viewers::office_over::Over;
use crate::frames::viewers::source::{Body, FixtureViewerSource, OpenError, OpenFile, ViewerSource, board};
use crate::frames::viewers::{Ui, open_from};
use crate::{State, init, view};

const PNG: [u8; 12] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 1];
const JPEG: [u8; 6] = [0xFF, 0xD8, 0xFF, 0xE0, 0, 16];

/// A file of `bytes` in a folder of its own that goes with the test.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("bb-word-insert-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        Scratch(dir)
    }
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).expect("a scratch file");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_table_is_asked_for_in_whole_numbers_inside_the_bounds() {
    assert_eq!(table_op("2", "5"), Ok(EditOp::Table { rows: 2, cols: 5 }));
    assert_eq!(table_op(" 3 ", "3"), Ok(EditOp::Table { rows: 3, cols: 3 }));
    assert_eq!(
        table_op("100", "20"),
        Ok(EditOp::Table {
            rows: MAX_ROWS,
            cols: MAX_COLS
        })
    );
    for bad in ["", "0", "-1", "abc", "2.5", "101", "99999999999"] {
        let (field, words) = table_op(bad, "3").expect_err(bad);
        assert_eq!(field, 0, "{bad}");
        assert_eq!(words, "Rows: a whole number from 1 to 100.");
    }
    let (field, words) = table_op("3", "21").expect_err("21 columns");
    assert_eq!((field, words.as_str()), (1, "Columns: a whole number from 1 to 20."));
    assert_eq!(table_op("0", "0").expect_err("both").0, 0, "the first wrong one is named");
}

#[test]
fn a_link_needs_an_address_and_is_never_judged_by_where_it_points() {
    assert_eq!(
        link_op("Our site", " https://beebeeb.io "),
        Ok(EditOp::Link {
            text: "Our site".into(),
            url: "https://beebeeb.io".into()
        })
    );
    // No words: the address stands in.
    assert_eq!(
        link_op("  ", "mailto:hello@beebeeb.io"),
        Ok(EditOp::Link {
            text: "mailto:hello@beebeeb.io".into(),
            url: "mailto:hello@beebeeb.io".into()
        })
    );
    assert_eq!(link_op("x", "   "), Err((1, NO_ADDRESS.into())));
    assert_eq!(link_op("x", "a\nb"), Err((1, BAD_ADDRESS.into())));
    assert_eq!(link_op("a\u{0}b", "https://x"), Err((0, BAD_WORDS.into())));
    let long = "a".repeat(check::MAX_OP_TEXT_BYTES + 1);
    assert_eq!(link_op("x", &long), Err((1, TOO_LONG.into())));
    assert_eq!(link_op(&long, "https://x"), Err((0, TOO_LONG.into())));
    let most = "a".repeat(check::MAX_OP_TEXT_BYTES);
    assert!(link_op(&most, &most).is_ok());
}

#[test]
fn a_picture_is_png_or_jpeg_by_its_bytes_and_nothing_else() {
    assert_eq!(sniff(&PNG), Some(true));
    assert_eq!(sniff(&JPEG), Some(false));
    assert_eq!(sniff(b"GIF89a......"), None);
    assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), None);
    assert_eq!(sniff(b"BM\0\0\0\0\0\0\0\0"), None);
    assert_eq!(sniff(&PNG[..5]), None, "a cut signature");
    assert_eq!(sniff(&[]), None);
}

#[test]
fn a_picture_file_is_refused_in_words_before_it_is_read_whole() {
    let dir = Scratch::new("read");
    let png = dir.file("a.bin", &PNG);
    let image = read_image_capped(&png, 64).expect("a PNG");
    assert!(image.png);
    assert_eq!(image.bytes.as_slice(), &PNG);
    let jpeg = dir.file("a.jpg.png", &JPEG);
    assert!(!read_image_capped(&jpeg, 64).expect("a JPEG by its bytes, not its name").png);
    // A renamed WebP or GIF is not a picture of the two kinds.
    let gif = dir.file("b.png", b"GIF89a......");
    assert_eq!(read_image_capped(&gif, 64).err().as_deref(), Some(NOT_AN_IMAGE));
    assert_eq!(read_image_capped(&dir.file("empty.png", b""), 64).err().as_deref(), Some(EMPTY_FILE));
    // Over the cap: refused from the size, with the cap in the words.
    let big = dir.file("big.png", &[PNG.as_slice(), &[0u8; 100]].concat());
    assert_eq!(read_image_capped(&big, 64).err(), Some(too_large(64)));
    assert_eq!(read_image_capped(&dir.0.join("missing.png"), 64).err().as_deref(), Some(UNREADABLE));
    assert_eq!(read_image_capped(&dir.0, 64).err().as_deref(), Some(UNREADABLE), "a folder");
    assert_eq!(
        too_large(check::MAX_INPUT_BYTES),
        "That picture is too large to add: at most 100 MB."
    );
}

fn wait(picking: &Picking) -> Answer {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if let Looked::Got(answer) = picking.look() {
            return answer;
        }
        assert!(Instant::now() < until, "the worker never answered");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_pickers_worker_reads_the_picked_file_and_says_why_when_it_cannot() {
    let dir = Scratch::new("pick");
    let png = dir.file("p.png", &PNG);
    let picking = Picking::start(move || Ok(Some(png))).expect("a worker");
    assert!(matches!(wait(&picking), Answer::Image(image) if image.png && image.bytes.len() == PNG.len()));
    let picking = Picking::start(|| Ok(None)).expect("a worker");
    assert!(matches!(wait(&picking), Answer::Closed));
    let gif = dir.file("g.gif", b"GIF89a......");
    let picking = Picking::start(move || Ok(Some(gif))).expect("a worker");
    assert!(matches!(wait(&picking), Answer::Refused(words) if words == NOT_AN_IMAGE));
    let picking = Picking::start(|| Err(PlatformError::NotBuilt("the photo picker"))).expect("a worker");
    assert!(matches!(wait(&picking), Answer::Refused(words) if words == NO_PICKER));
}

/// What the source was asked.
#[derive(Default)]
struct Asked {
    edits: Vec<EditOp>,
    images: Vec<(usize, bool)>,
}

/// The fixture with an editor behind it that records, and that can be told to refuse.
struct Recording {
    inner: FixtureViewerSource,
    asked: Rc<RefCell<Asked>>,
    takes: bool,
}

impl ViewerSource for Recording {
    fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.inner.open(id)
    }
    fn poll(&mut self, id: &str) -> Option<Body> {
        self.inner.poll(id)
    }
    fn cancel(&mut self, _: &str) {}
    fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
        self.inner.retry(id)
    }
    fn can_edit(&self, _: &str) -> bool {
        true
    }
    fn edit(&mut self, _: &str, op: EditOp) -> bool {
        self.asked.borrow_mut().edits.push(op);
        self.takes
    }
    fn insert_image(&mut self, _: &str, bytes: Zeroizing<Vec<u8>>, png: bool) -> bool {
        self.asked.borrow_mut().images.push((bytes.len(), png));
        self.takes
    }
}

fn source(takes: bool) -> (Box<Recording>, Rc<RefCell<Asked>>) {
    let asked = Rc::new(RefCell::new(Asked::default()));
    let source = Recording {
        inner: FixtureViewerSource::new(),
        asked: asked.clone(),
        takes,
    };
    (Box::new(source), asked)
}

fn live_ui(takes: bool) -> (Ui, Rc<RefCell<Asked>>) {
    let (source, asked) = source(takes);
    let mut ui = Ui::open(source, board::DOCX).expect("the fixture opens it");
    ui.office.edit = Some(EditUi::new(EditKind::Docx));
    (ui, asked)
}

fn type_into(edit: &mut rune::TextEdit, text: &str) {
    edit.replace_text(text);
}

#[test]
fn table_sends_the_rows_and_columns_and_a_wrong_number_stays_in_the_dialog() {
    let (mut ui, asked) = live_ui(true);
    ui.office_insert(Insert::Table);
    assert!(matches!(&ui.office.over, Some(Over::Table(form)) if form.rows.text == "3" && form.cols.text == "3"));
    let Some(Over::Table(form)) = &mut ui.office.over else { unreachable!() };
    type_into(&mut form.rows, "0");
    ui.office_table_ok();
    assert!(asked.borrow().edits.is_empty(), "nothing was sent");
    let Some(Over::Table(form)) = &mut ui.office.over else { panic!("the dialog stays") };
    assert_eq!(form.error.as_ref().map(|(at, _)| *at), Some(0));
    type_into(&mut form.rows, "4");
    type_into(&mut form.cols, "2");
    ui.office_table_ok();
    assert!(ui.office.over.is_none());
    assert_eq!(asked.borrow().edits, [EditOp::Table { rows: 4, cols: 2 }]);
}

#[test]
fn link_sends_text_and_address_as_text_only() {
    let (mut ui, asked) = live_ui(true);
    ui.office_insert(Insert::Link);
    ui.office_link_ok();
    assert!(matches!(&ui.office.over, Some(Over::Link(form)) if form.error.as_ref().is_some_and(|(at, _)| *at == 1)));
    assert!(asked.borrow().edits.is_empty());
    let Some(Over::Link(form)) = &mut ui.office.over else { unreachable!() };
    type_into(&mut form.text, "Terms");
    type_into(&mut form.url, "https://beebeeb.io/terms");
    ui.office_link_ok();
    assert!(ui.office.over.is_none());
    assert_eq!(
        asked.borrow().edits,
        [EditOp::Link {
            text: "Terms".into(),
            url: "https://beebeeb.io/terms".into()
        }]
    );
    // Never opened from here.
    assert!(ui.take_intents().is_empty());
}

#[test]
fn page_break_is_one_edit_and_a_source_that_does_not_take_it_says_so() {
    let (mut ui, asked) = live_ui(true);
    ui.office_insert(Insert::PageBreak);
    assert_eq!(asked.borrow().edits, [EditOp::PageBreak]);
    assert!(ui.toast.is_none());
    let (mut ui, _) = live_ui(false);
    ui.office_insert(Insert::PageBreak);
    assert_eq!(ui.toast.as_deref(), Some(NOT_DONE));
    ui.toast = None;
    ui.office_insert(Insert::Table);
    let Some(Over::Table(_)) = &ui.office.over else { panic!("a table dialog") };
    ui.office_table_ok();
    assert_eq!(ui.toast.as_deref(), Some(NOT_DONE), "a refused table is not silent");
}

#[test]
fn a_picked_picture_goes_to_the_source_and_a_refusal_is_said() {
    let dir = Scratch::new("ui");
    let (mut ui, asked) = live_ui(true);
    let png = dir.file("a.png", &PNG);
    assert!(ui.office_pick_image(move || Ok(Some(png))));
    assert!(ui.office_image_waiting());
    let until = Instant::now() + Duration::from_secs(10);
    while !ui.office_image_poll() {
        assert!(Instant::now() < until, "no answer");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!ui.office_image_waiting() && ui.office.over.is_none());
    assert_eq!(asked.borrow().images, [(PNG.len(), true)]);
    assert!(ui.toast.is_none());

    // WebP and GIF are refused in words and nothing is sent.
    ui.office.over = Some(Over::Image(Picking::answered(Answer::Refused(NOT_AN_IMAGE.into()))));
    assert!(ui.office_image_poll());
    assert_eq!(ui.toast.as_deref(), Some(NOT_AN_IMAGE));
    assert_eq!(asked.borrow().images.len(), 1);

    // Closed without a file: nothing at all.
    ui.toast = None;
    ui.office.over = Some(Over::Image(Picking::answered(Answer::Closed)));
    assert!(ui.office_image_poll());
    assert!(ui.toast.is_none() && ui.office.over.is_none());

    // A source that does not take the picture says so.
    let (mut ui, asked) = live_ui(false);
    let image = read_image(&dir.file("b.jpg", &JPEG)).expect("a JPEG");
    ui.office_image_answer(Answer::Image(image));
    assert_eq!(asked.borrow().images, [(JPEG.len(), false)]);
    assert_eq!(ui.toast.as_deref(), Some(NOT_ADDED));
}

#[test]
fn cancel_drops_the_picker_and_a_late_answer_goes_nowhere() {
    let (mut ui, asked) = live_ui(true);
    ui.office_pick_image(|| {
        std::thread::sleep(Duration::from_millis(50));
        Ok(None)
    });
    ui.office_close_over();
    assert!(ui.office.over.is_none());
    assert!(!ui.office_image_poll());
    assert!(asked.borrow().images.is_empty());
}

fn runtime(source: Box<dyn ViewerSource>, live: bool) -> Runtime<State> {
    let mut state = init();
    assert!(open_from(&mut state, source, board::DOCX));
    let ui = state.viewer.ui.as_mut().expect("a file is open");
    ui.office.tab = crate::frames::viewers::office::Tab::Insert;
    if live {
        ui.office.edit = Some(EditUi::new(EditKind::Docx));
    }
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    rt
}

fn key(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    rt.frame(0.0);
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

/// Tab to a control and press it with Space.
fn operate(rt: &mut Runtime<State>, to: &str) {
    for _ in 0..80 {
        if focused(rt).as_deref() == Some(to) {
            key(rt, Key::Text(" ".into()));
            return;
        }
        key(rt, Key::Tab);
    }
    panic!("Tab never reached {to}; focus is on {:?}", focused(rt));
}

fn ui_of(rt: &Runtime<State>) -> &Ui {
    rt.state.viewer.ui.as_ref().expect("a file is open")
}

#[test]
fn the_tools_press_through_the_window_and_stay_on_the_boards_keys() {
    let (source, asked) = source(true);
    let mut rt = runtime(source, true);
    for key in ["rt-0", "rt-1", "rt-2", "rsep-0", "rt-3"] {
        assert!(has(&rt, key), "{key}");
    }
    operate(&mut rt, "rt-3");
    assert_eq!(asked.borrow().edits, [EditOp::PageBreak]);
    operate(&mut rt, "rt-0");
    assert!(has(&rt, "dlg") && has(&rt, ROWS_KEY) && has(&rt, COLS_KEY), "the table dialog");
    // Esc leaves it without a word to the editor.
    key(&mut rt, Key::Escape);
    assert!(!has(&rt, "dlg"));
    assert_eq!(asked.borrow().edits.len(), 1);
    operate(&mut rt, "rt-0");
    // Enter in a field presses the default button.
    key(&mut rt, Key::Enter);
    assert!(!has(&rt, "dlg"), "inserted");
    assert_eq!(asked.borrow().edits[1], EditOp::Table { rows: 3, cols: 3 });
    operate(&mut rt, "rt-2");
    assert!(has(&rt, LINK_URL_KEY));
    operate(&mut rt, "btn-0");
    assert!(has(&rt, "dlg"), "no address: the dialog stays");
    assert_eq!(asked.borrow().edits.len(), 2);
}

#[test]
fn without_a_live_editor_the_tools_say_not_built_yet_and_change_nothing() {
    let (source, asked) = source(true);
    let mut rt = runtime(source, false);
    operate(&mut rt, "rt-0");
    assert!(!has(&rt, "dlg"));
    assert!(ui_of(&rt).toast.as_deref().is_some_and(|t| t.contains("Table")));
    operate(&mut rt, "rt-3");
    assert!(asked.borrow().edits.is_empty() && asked.borrow().images.is_empty());
}

const ROWS_KEY: &str = crate::frames::viewers::office_over::ROWS;
const COLS_KEY: &str = crate::frames::viewers::office_over::COLS;
const LINK_URL_KEY: &str = crate::frames::viewers::office_over::LINK_URL;

#[test]
fn the_fields_of_the_dialogs_are_not_the_editing_surfaces() {
    for key in [ROWS_KEY, COLS_KEY, LINK_URL_KEY, crate::frames::viewers::office_over::LINK_TEXT] {
        assert!(!key.starts_with(crate::frames::office::edit::FOCUS_PREFIX), "{key}");
    }
}
