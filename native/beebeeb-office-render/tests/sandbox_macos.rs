//! macOS: the real helper, started confined through the sandbox crate, opens a small Word document and renders a
//! page of it; and the helper confines itself whoever starts it.
//!
//! The confinement's own claims (files, network, processes, the read-only section, the memory cap) are held by the
//! sandbox crate's tests with its probe; here the claim is that the Office engine WORKS under that confinement.
//!
//! Output carries counts and sizes only: never a file name, page text or pixel value.
#![cfg(target_os = "macos")]

use std::ffi::{c_char, c_int};
use std::fs::File;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use beebeeb_office_protocol::check::{self, Limits as Contract};
use beebeeb_office_protocol::frame::{Frame, Ids, Side, read_frame, write_frame};
use beebeeb_office_protocol::office::{
    DocKind, Error, ErrorCode, GrantRef, KIND_ERROR, KIND_OPEN, KIND_OPENED, KIND_RENDER, KIND_RENDERED, Open, Opened,
    PictureKind, Render, Rendered,
};
use beebeeb_office_sandbox::{Access, Child, Limits as SandboxLimits, Sandbox, Section};

const HELPER: &str = env!("CARGO_BIN_EXE_beebeeb-office-render");
const LETTER: &[u8] = include_bytes!("fixtures/letter-1.docx");
/// Names a font of the system: the helper reads it from the system's font folder, the one place it may read.
const SYSTEM_FONT: &[u8] = include_bytes!("fixtures/system-font.docx");
/// A 64 x 32 HEIC, left half red, right half blue (made with `sips` from a bitmap; drive-macos-92).
const PHOTO: &[u8] = include_bytes!("fixtures/photo-64x32.heic");

unsafe extern "C" {
    /// The sandbox library's question to the kernel about a process (no public header; as the sandbox crate uses it).
    fn sandbox_check(pid: c_int, operation: *const c_char, filter: c_int, ...) -> c_int;
}

/// A confined helper with a document open.
struct Session {
    child: Child,
    to_helper: File,
    answers: Receiver<Result<Frame, String>>,
    ids: Ids,
    // Held so the helper's read-only mapping of the document stays valid.
    _doc: Section,
}

impl Session {
    fn open(bytes: &[u8]) -> (Session, Opened) {
        Session::open_as(DocKind::Docx, bytes)
    }

    fn open_as(kind: DocKind, bytes: &[u8]) -> (Session, Opened) {
        let (session, frame) = Session::start_as(kind, bytes, &[KIND_OPENED]);
        let opened = Opened::decode(&frame.payload).expect("the Opened answer decodes");
        check::check_opened(&opened, &Contract::CONTRACT).expect("the Opened answer passes the checks");
        (session, opened)
    }

    /// Starts the helper, sends `Open` and returns its answer, which must be of `expect_kind`.
    fn start_as(kind: DocKind, bytes: &[u8], expect_kinds: &[u16]) -> (Session, Frame) {
        let sandbox = Sandbox::new(SandboxLimits::default()).expect("the sandbox starts");
        let mut child = sandbox.spawn(Path::new(HELPER)).expect("the confined helper starts");
        let pipes = child.take_pipes().expect("the pipes are there the first time");
        let (tx, answers) = mpsc::channel();
        let mut from_helper = pipes.from_helper;
        std::thread::spawn(move || {
            loop {
                match read_frame(&mut from_helper) {
                    Ok(frame) => {
                        if tx.send(Ok(frame)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        break;
                    }
                }
            }
        });
        let mut doc = Section::new(bytes.len()).expect("a section for the document");
        doc.as_mut_slice().copy_from_slice(bytes);
        let granted = child
            .grant(&doc, Access::Read)
            .expect("the document is granted read-only");
        let mut session = Session {
            child,
            to_helper: pipes.to_helper,
            answers,
            ids: Ids::new(Side::App),
            _doc: doc,
        };
        let open = Open {
            kind,
            doc: GrantRef {
                wire: granted.wire,
                len: granted.len,
            },
        };
        let frame = session.ask_any(KIND_OPEN, &open.encode().expect("a small request"), expect_kinds);
        (session, frame)
    }

    fn ask(&mut self, kind: u16, payload: &[u8], expect_kind: u16) -> Frame {
        self.ask_any(kind, payload, &[expect_kind])
    }

    /// [`Session::ask`] when the answer may be one of `expect_kinds`.
    fn ask_any(&mut self, kind: u16, payload: &[u8], expect_kinds: &[u16]) -> Frame {
        let id = self.ids.next_id();
        write_frame(&mut self.to_helper, kind, id, payload).expect("the request is written");
        let frame = match self.answers.recv_timeout(check::REQUEST_TIMEOUT) {
            Ok(Ok(frame)) => frame,
            Ok(Err(_)) => panic!("the helper's pipe failed"),
            Err(_) => panic!("the helper was silent for {} s", check::REQUEST_TIMEOUT.as_secs()),
        };
        assert_eq!(frame.id, id, "the answer's id is the request's");
        assert!(expect_kinds.contains(&frame.kind), "answer kind {:#06x}", frame.kind);
        frame
    }

    /// Page `page` at 96 dpi, as the app gets it: a read-write section, the request, the checked answer, the pixels.
    fn render(&mut self, opened: &Opened, page: u32) -> (Vec<u8>, u32, u32) {
        let mut ask = Render {
            page,
            scale: 1.0,
            region: None,
            out: GrantRef { wire: 0, len: 0 },
        };
        let size = &opened.sizes[page as usize];
        let (w, h) = check::asked_size(&ask, size, &Contract::CONTRACT).expect("the ask can be sized");
        let out = Section::new(w as usize * h as usize * 4).expect("a section for the pixels");
        let granted = self
            .child
            .grant(&out, Access::ReadWrite)
            .expect("the pixel section is granted");
        ask.out = GrantRef {
            wire: granted.wire,
            len: granted.len,
        };
        let frame = self.ask(KIND_RENDER, &ask.encode().expect("a small request"), KIND_RENDERED);
        let got = Rendered::decode(&frame.payload).expect("the Rendered answer decodes");
        check::check_rendered(&ask, size, out.len() as u64, &got, &Contract::CONTRACT)
            .expect("the Rendered answer passes the checks");
        let mut pixels = vec![0u8; out.len()];
        out.copy_to(&mut pixels).expect("the pixels are copied out");
        pixels.truncate(got.w as usize * got.h as usize * 4);
        (pixels, got.w, got.h)
    }
}

/// How many pixels differ from the first one: a page that was drawn is not one flat colour.
fn drawn(pixels: &[u8]) -> usize {
    let (px, _) = pixels.as_chunks::<4>();
    px.iter().filter(|p| *p != &px[0]).count()
}

#[test]
fn the_confined_helper_opens_a_small_document_and_renders_a_page() {
    let mut rendered = 0;
    for (which, bytes) in [LETTER, SYSTEM_FONT].into_iter().enumerate() {
        let started = Instant::now();
        let (mut session, opened) = Session::open(bytes);
        assert!(opened.pages >= 1, "document {which} has a page");
        let (pixels, w, h) = session.render(&opened, 0);
        assert!(w > 100 && h > 100, "document {which}: a page of {w} x {h}");
        assert_eq!(pixels.len(), w as usize * h as usize * 4);
        let ink = drawn(&pixels);
        assert!(ink > 0, "document {which}: the page is not flat");
        println!(
            "document {which}: {} page(s), page 1 is {w} x {h}, {ink} drawn pixels, {} ms from start",
            opened.pages,
            started.elapsed().as_millis()
        );
        assert!(!session.child.killed_for_memory());
        assert_eq!(
            session.child.try_wait().expect("the helper can be asked"),
            None,
            "it still runs"
        );
        rendered += 1;
    }
    assert_eq!(rendered, 2);
}

/// Drive-macos-92: a HEIC goes to the confined helper (the app never decodes one), and the helper answers without
/// crashing. In the app the helper is the XPC service, in an App Sandbox, where ImageIO decodes it: one page of the
/// picture's size in pixels, page 0 at scale 1 the picture (that decode is held by the unit tests of `picture.rs`;
/// ImageIO under a plain App Sandbox was measured on 2026-11-27 with an ad-hoc signed probe: 64 x 32, red and blue).
/// Under the deny-default profile a helper puts on itself outside a bundle, macOS's HEVC decoder is denied and draws
/// nothing (measured): the helper then says `Damaged`, never a blank picture. Either answer passes; a crash, a blank
/// page or wrong colours do not.
#[test]
fn the_confined_helper_decodes_a_heic_photo_or_says_it_cannot() {
    let (mut session, frame) =
        Session::start_as(DocKind::Picture(PictureKind::Heic), PHOTO, &[KIND_OPENED, KIND_ERROR]);
    if frame.kind == KIND_ERROR {
        let error = Error::decode(&frame.payload).expect("the Error answer decodes");
        assert_eq!(error.code, ErrorCode::Damaged);
        println!("the confined helper cannot decode HEVC under this profile: Damaged");
        return;
    }
    let opened = Opened::decode(&frame.payload).expect("the Opened answer decodes");
    check::check_opened(&opened, &Contract::CONTRACT).expect("the Opened answer passes the checks");
    assert_eq!(opened.pages, 1);
    let (pixels, w, h) = session.render(&opened, 0);
    assert_eq!((w, h), (64, 32));
    let at = |x: usize, y: usize| &pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
    let (left, right) = (at(2, 16), at(61, 16));
    assert!(left[0] > left[2] + 60 && left[3] == 255, "left is red: {left:?}");
    assert!(right[2] > right[0] + 60 && right[3] == 255, "right is blue: {right:?}");
    assert!(!session.child.killed_for_memory());
}

/// The helper is never unconfined: started by hand, with no sandbox crate in front of it, it has put the same
/// profile on itself by the time it waits for its first frame. The kernel is asked, not the helper.
#[test]
fn the_helper_started_by_hand_confines_itself() {
    let mut helper = Command::new(HELPER)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the helper starts");
    let pid = helper.id() as c_int;
    // SAFETY (all four calls below): a null operation asks whether the process has a sandbox; the others pass a
    // terminated operation name and no object (filter 0). Nothing is changed in either process.
    let sandboxed = || unsafe { sandbox_check(pid, std::ptr::null(), 0) } == 1;
    let started = Instant::now();
    while !sandboxed() && started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(sandboxed(), "the kernel reports a sandbox on the helper");
    let denied = [c"network-outbound", c"network-inbound", c"process-fork"]
        .iter()
        .filter(|operation| unsafe { sandbox_check(pid, operation.as_ptr(), 0) } == 1)
        .count();
    assert_eq!(denied, 3, "and denies it the network and new processes");
    // The instrument can fail: this test process has none.
    assert_eq!(
        unsafe { sandbox_check(std::process::id() as c_int, std::ptr::null(), 0) },
        0
    );

    // The app closing the pipe ends it, as ever.
    drop(helper.stdin.take());
    let status = helper.wait().expect("the helper ends");
    assert_eq!(status.code(), Some(0));
}
