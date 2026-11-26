//! A .docx written by LibreOffice (the web office) opens in the helper, in the viewer and for editing, through the pipe
//! and (on Windows) inside the sandbox. The 11-24 report: such a file showed "The viewer stopped" in the app.
//!
//! On the Windows test machine this file and the app's host (`office_page --edit`) open, render and edit it, so the
//! stop was not the helper's; the host now logs every stop with its check and the helper's exit code
//! (`office_log::helper_stopped`). These tests keep the helper's side of that green.

mod common;

use std::sync::Arc;

use beebeeb_office_protocol::check::{Limits, asked_size, check_opened, check_rendered, render_scale};
use beebeeb_office_protocol::office::{DocKind, EditKind, Message, Open, OpenEdit, Opened, Outline, PageSize, Render};
use beebeeb_office_render::{EXIT_OK, Office};
use common::*;

/// One A4 page, one paragraph, written by LibreOffice (`tests/fixtures/README.md`).
const LIBREOFFICE: &[u8] = include_bytes!("fixtures/libreoffice-1.docx");

fn one_a4_page(answer: Message) -> Opened {
    let Message::Opened(opened) = answer else {
        panic!("Opened was due: {answer:?}");
    };
    check_opened(&opened, &Limits::CONTRACT).expect("the app's check accepts the Opened");
    assert_eq!(opened.pages, 1, "one page");
    assert_eq!(opened.sizes[0], A4, "an A4 page");
    opened
}

/// Page 1 at 100 % on a 96 DPI screen: answered `Rendered` at the size the app asked, accepted by its check.
fn renders_page_one(helper: &mut Running, memory: &Arc<FakeMemory>, id: u32, page: &PageSize) {
    let ask = Render {
        page: 0,
        scale: render_scale(1.0, 96.0),
        region: None,
        out: NO_GRANT,
    };
    let (w, h) = asked_size(&ask, page, &Limits::CONTRACT).expect("the app can size the page");
    let out = memory.zeroed(w as usize * h as usize * 4);
    let ask = Render { out, ..ask };
    helper.send(id, &Message::Render(ask));
    match helper.answer() {
        (got, Message::Rendered(rendered)) if got == id => {
            assert_eq!((rendered.w, rendered.h), (w, h), "the size the app asked");
            check_rendered(&ask, page, out.len, &rendered, &Limits::CONTRACT).expect("the app's check accepts it");
        }
        other => panic!("Rendered for {id} was due: {other:?}"),
    }
}

#[test]
fn viewer_a_libreoffice_docx_opens_and_renders() {
    let memory = FakeMemory::new();
    let mut helper = Running::start(Arc::clone(&memory), Arc::new(Office));
    let doc = memory.section(LIBREOFFICE.to_vec());
    helper.send(
        0,
        &Message::Open(Open {
            kind: DocKind::Docx,
            doc,
        }),
    );
    let (id, answer) = helper.answer();
    assert_eq!(id, 0, "the answer carries the Open's id");
    let opened = one_a4_page(answer);
    renders_page_one(&mut helper, &memory, 2, &opened.sizes[0]);
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// The app opens a writable .docx for editing (`OpenEdit`), and the window asks page 1 and the outline.
#[test]
fn viewer_a_libreoffice_docx_opens_for_editing() {
    let memory = FakeMemory::new();
    let mut helper = Running::start(Arc::clone(&memory), Arc::new(Office));
    let doc = memory.section(LIBREOFFICE.to_vec());
    helper.send(
        0,
        &Message::OpenEdit(OpenEdit {
            doc,
            kind: EditKind::Docx,
        }),
    );
    let (id, answer) = helper.answer();
    assert_eq!(id, 0, "the answer carries the OpenEdit's id");
    let opened = one_a4_page(answer);
    renders_page_one(&mut helper, &memory, 2, &opened.sizes[0]);
    helper.send(4, &Message::Outline(Outline { from: 0 }));
    match helper.answer() {
        (4, Message::OutlineData(outline)) => assert_eq!(outline.next, None, "the whole outline in one answer"),
        other => panic!("OutlineData for 4 was due: {other:?}"),
    }
    helper.close();
    assert_eq!(helper.exit_code(), EXIT_OK);
}

/// The same, with the real helper inside the sandbox: both modes open and render, and the helper is still running.
#[cfg(windows)]
#[test]
fn viewer_a_libreoffice_docx_opens_in_the_sandbox() {
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::Duration;

    use beebeeb_office_protocol::check::REQUEST_TIMEOUT;
    use beebeeb_office_protocol::frame::{Ids, Side, read_frame, write_frame};
    use beebeeb_office_protocol::office::GrantRef;
    use beebeeb_office_sandbox::{Access, Limits as SandboxLimits, Sandbox, Section};

    const HELPER: &str = env!("CARGO_BIN_EXE_beebeeb-office-render");

    for edit in [false, true] {
        let sandbox = Sandbox::new(SandboxLimits::default()).expect("the sandbox starts");
        let mut child = sandbox.spawn(Path::new(HELPER)).expect("the confined helper starts");
        let pipes = child.take_pipes().expect("the pipes are there the first time");
        let (tx, answers) = mpsc::channel();
        let mut from_helper = pipes.from_helper;
        std::thread::spawn(move || {
            while let Ok(frame) = read_frame(&mut from_helper) {
                if tx.send(frame).is_err() {
                    break;
                }
            }
        });
        let mut to_helper = pipes.to_helper;
        let mut ids = Ids::new(Side::App);
        let mut ask = |message: &Message| {
            let id = ids.next_id();
            write_frame(&mut to_helper, message.kind(), id, &message.encode().expect("fits")).expect("written");
            let frame = answers.recv_timeout(REQUEST_TIMEOUT).unwrap_or_else(|_| {
                panic!(
                    "edit={edit}: no answer; the helper ended with {:?}",
                    child.wait_timeout(Duration::from_millis(500))
                )
            });
            assert_eq!(frame.id, id, "the answer's id is the request's");
            Message::decode(frame.kind, &frame.payload).expect("the answer decodes")
        };

        let mut doc = Section::new(LIBREOFFICE.len()).expect("a section for the document");
        doc.as_mut_slice().copy_from_slice(LIBREOFFICE);
        let granted = child.grant(&doc, Access::Read).expect("the document is granted");
        let doc_ref = GrantRef {
            wire: granted.wire,
            len: granted.len,
        };
        let open = if edit {
            Message::OpenEdit(OpenEdit {
                doc: doc_ref,
                kind: EditKind::Docx,
            })
        } else {
            Message::Open(Open {
                kind: DocKind::Docx,
                doc: doc_ref,
            })
        };
        let opened = one_a4_page(ask(&open));

        let render = Render {
            page: 0,
            scale: render_scale(1.0, 96.0),
            region: None,
            out: NO_GRANT,
        };
        let (w, h) = asked_size(&render, &opened.sizes[0], &Limits::CONTRACT).expect("the app can size the page");
        let out = Section::new(w as usize * h as usize * 4).expect("a section for the page");
        let granted = child.grant(&out, Access::ReadWrite).expect("the page is granted");
        let render = Render {
            out: GrantRef {
                wire: granted.wire,
                len: granted.len,
            },
            ..render
        };
        match ask(&Message::Render(render)) {
            Message::Rendered(rendered) => assert_eq!((rendered.w, rendered.h), (w, h), "edit={edit}: the asked size"),
            other => panic!("edit={edit}: Rendered was due: {other:?}"),
        }
        assert!(
            child
                .wait_timeout(Duration::from_millis(100))
                .expect("the helper can be waited on")
                .is_none(),
            "edit={edit}: the helper is still running"
        );
        let _ = child.kill();
    }
}
