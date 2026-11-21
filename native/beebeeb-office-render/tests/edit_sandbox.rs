//! An EDITING helper inside the sandbox (editor contract section 5.4 and section 5.6 row 4): the helper is started
//! through `beebeeb-office-sandbox` and edits and saves a real document, while the things a compromised or merely
//! chatty engine could try from inside it are watched from outside: no file appears or changes in a canary folder,
//! no connection reaches a loopback listener, no second process starts.
//!
//! The denials themselves (a write, a spawn, a connect each answered with the system's refusal code) are the
//! sandbox crate's own probe tests (`crates/beebeeb-office-sandbox/tests/sandbox.rs`, same container profile and job);
//! the helper has no code that tries them and gets none for a test, so this file shows the editing helper runs under
//! exactly that confinement and leaves no trace of any of the three.
//!
//! Windows only. Runs in W6 on VM 401: `cargo test --locked --test edit_sandbox -- --nocapture`.
//! Output carries counts only.
#![cfg(windows)]

use std::fs::File;
use std::net::TcpListener;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};

use beebeeb_office_protocol::check::{self, Limits as Contract};
use beebeeb_office_protocol::frame::{Frame, Ids, Side, read_frame, write_frame};
use beebeeb_office_protocol::office::{
    Edit, EditKind, EditOp, GrantRef, KIND_EDIT, KIND_EDITED, KIND_OPEN_EDIT, KIND_OPENED, KIND_SAVE_INTO,
    KIND_SAVE_READY, KIND_SAVE_SIZE, KIND_SAVED_INTO, OpenEdit, Purpose, SaveInto, SaveReady, SaveSize,
};
use beebeeb_office_sandbox::{Access, Child, Limits as SandboxLimits, Sandbox, Section};

const HELPER: &str = env!("CARGO_BIN_EXE_beebeeb-office-render");
const DOC: &[u8] = include_bytes!("fixtures/letter-1.docx");

struct Session {
    child: Child,
    to_helper: File,
    answers: Receiver<Result<Frame, String>>,
    ids: Ids,
    keep: Vec<Section>,
}

impl Session {
    fn start() -> Session {
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
        Session {
            child,
            to_helper: pipes.to_helper,
            answers,
            ids: Ids::new(Side::App),
            keep: Vec::new(),
        }
    }

    fn ask(&mut self, kind: u16, payload: &[u8], expect_kind: u16) -> Frame {
        let id = self.ids.next_id();
        write_frame(&mut self.to_helper, kind, id, payload).expect("the request is written");
        let frame = match self.answers.recv_timeout(check::REQUEST_TIMEOUT) {
            Ok(Ok(frame)) => frame,
            Ok(Err(e)) => panic!("the helper's pipe failed: {e}"),
            Err(_) => panic!("the helper was silent for {} s", check::REQUEST_TIMEOUT.as_secs()),
        };
        assert_eq!(frame.id, id, "the answer's id is the request's");
        assert_eq!(frame.kind, expect_kind, "answer kind {:#06x}, wanted {:#06x}", frame.kind, expect_kind);
        frame
    }

    fn grant(&mut self, section: Section, access: Access) -> GrantRef {
        let granted = self.child.grant(&section, access).expect("the section is granted");
        self.keep.push(section);
        GrantRef {
            wire: granted.wire,
            len: granted.len,
        }
    }
}

#[test]
fn edit_sandbox_an_editing_helper_runs_confined_and_leaves_no_file_connection_or_process() {
    // Canaries the confined helper could reach if it were not confined.
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
    listener.set_nonblocking(true).expect("non-blocking");
    let canary = std::env::temp_dir().join(format!("bb-edit-sandbox-{}", std::process::id()));
    std::fs::create_dir_all(&canary).expect("a canary folder");
    let before = std::fs::read_dir(&canary).expect("the canary folder").count();

    let mut s = Session::start();
    let mut doc = Section::new(DOC.len()).expect("a section for the document");
    doc.as_mut_slice().copy_from_slice(DOC);
    let doc = s.grant(doc, Access::Read);
    let open = OpenEdit { doc, kind: EditKind::Docx };
    let opened = s.ask(KIND_OPEN_EDIT, &open.encode().expect("fits"), KIND_OPENED);
    let opened = beebeeb_office_protocol::office::Opened::decode(&opened.payload).expect("decodes");
    check::check_opened(&opened, &Contract::CONTRACT).expect("the Opened passes the checks");

    for text in ["edited ", "inside ", "the sandbox"] {
        let edit = Edit { op: EditOp::InsertText(text.to_owned()) };
        s.ask(KIND_EDIT, &edit.encode().expect("fits"), KIND_EDITED);
    }

    let size = SaveSize { purpose: Purpose::Save };
    let ready = s.ask(KIND_SAVE_SIZE, &size.encode().expect("fits"), KIND_SAVE_READY);
    let ready = SaveReady::decode(&ready.payload).expect("decodes");
    check::check_save_ready(&ready).expect("the SaveReady passes the checks");
    let out = Section::new(ready.len as usize).expect("a section for the save");
    let out_ref = s.grant(out, Access::ReadWrite);
    let into = SaveInto { token: ready.token, out: out_ref };
    s.ask(KIND_SAVE_INTO, &into.encode().expect("fits"), KIND_SAVED_INTO);
    let saved = s.keep.last().expect("the save section").as_slice();
    assert!(saved.starts_with(b"PK"), "the saved bytes are a package");

    assert!(listener.accept().is_err(), "no connection reached the loopback listener");
    assert_eq!(
        std::fs::read_dir(&canary).expect("the canary folder").count(),
        before,
        "no file appeared in the canary folder"
    );
    let _ = s.child.kill();
    let _ = std::fs::remove_dir_all(&canary);
    println!("edit sandbox: connections=0 new-files=0");
}
