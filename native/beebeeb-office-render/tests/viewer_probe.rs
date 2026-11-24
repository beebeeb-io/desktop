//! PROBE (temporary): the 11-24 'Dit is een test.docx' through the real engine.
mod common;
use std::sync::Arc;
use beebeeb_office_protocol::check::{Limits, asked_size, check_opened, render_scale};
use beebeeb_office_protocol::office::{DocKind, Message, Open, Render};
use beebeeb_office_render::{Engine, Office};
use common::*;

const PROBE: &[u8] = include_bytes!("fixtures/probe-lo.docx");

#[test]
fn viewer_probe_engine_direct() {
    let r = std::panic::catch_unwind(|| Office.open(DocKind::Docx, PROBE).map(|d| format!("{:?}", d.opened())).map_err(|e| format!("{e:?}")));
    eprintln!("PROBE direct: {r:?}");
}

#[test]
fn viewer_probe_through_serve() {
    let memory = FakeMemory::new();
    let mut helper = Running::start(Arc::clone(&memory), Arc::new(Office));
    let doc = memory.section(PROBE.to_vec());
    helper.send(0, &Message::Open(Open { kind: DocKind::Docx, doc }));
    let (id, answer) = helper.answer();
    eprintln!("PROBE open answer {id}: {answer:?}");
    if let Message::Opened(opened) = answer {
        eprintln!("PROBE check_opened: {:?}", check_opened(&opened, &Limits::CONTRACT));
        let ask = Render { page: 0, scale: render_scale(1.0, 96.0), region: None, out: NO_GRANT };
        let (w, h) = asked_size(&ask, &opened.sizes[0], &Limits::CONTRACT).expect("size");
        let out = memory.zeroed(w as usize * h as usize * 4);
        helper.send(2, &Message::Render(Render { out, ..ask }));
        let a = helper.answer();
        eprintln!("PROBE render: {:?}", a);
    }
    helper.close();
    eprintln!("PROBE exit {}", helper.exit_code());
}


#[test]
fn viewer_probe_open_edit_through_serve() {
    use beebeeb_office_protocol::office::{EditKind, OpenEdit, Outline};
    let memory = FakeMemory::new();
    let mut helper = Running::start(Arc::clone(&memory), Arc::new(Office));
    let doc = memory.section(PROBE.to_vec());
    helper.send(0, &Message::OpenEdit(OpenEdit { doc, kind: EditKind::Docx }));
    let (id, answer) = helper.answer();
    eprintln!("PROBE-EDIT open answer {id}: {answer:?}");
    if let Message::Opened(opened) = &answer {
        eprintln!("PROBE-EDIT check_opened: {:?}", check_opened(opened, &Limits::CONTRACT));
        let ask = Render { page: 0, scale: render_scale(1.0, 96.0), region: None, out: NO_GRANT };
        let (w, h) = asked_size(&ask, &opened.sizes[0], &Limits::CONTRACT).expect("size");
        let out = memory.zeroed(w as usize * h as usize * 4);
        helper.send(2, &Message::Render(Render { out, ..ask }));
        eprintln!("PROBE-EDIT render: {:?}", helper.answer());
        helper.send(4, &Message::Outline(Outline { from: 0 }));
        eprintln!("PROBE-EDIT outline: {:?}", helper.answer());
    }
    helper.close();
    eprintln!("PROBE-EDIT exit {}", helper.exit_code());
}

#[cfg(windows)]
mod win {
    use std::fs::File;
    use std::path::Path;
    use std::sync::mpsc::{self, Receiver};
    use std::time::Duration;
    use beebeeb_office_protocol::check::{self, Limits as Contract};
    use beebeeb_office_protocol::frame::{Frame, Ids, Side, read_frame, write_frame};
    use beebeeb_office_protocol::office::{DocKind, GrantRef, Message, Open, Render, KIND_OPEN, KIND_RENDER};
    use beebeeb_office_sandbox::{Access, Child, Limits as SandboxLimits, Sandbox, Section};

    const HELPER: &str = env!("CARGO_BIN_EXE_beebeeb-office-render");

    fn run(name: &str, bytes: &[u8], edit: bool) {
        let sandbox = Sandbox::new(SandboxLimits::default()).expect("the sandbox starts");
        let mut child: Child = sandbox.spawn(Path::new(HELPER)).expect("the confined helper starts");
        let pipes = child.take_pipes().expect("pipes");
        let (tx, rx): (_, Receiver<Result<Frame, String>>) = mpsc::channel();
        let mut from = pipes.from_helper;
        std::thread::spawn(move || loop {
            match read_frame(&mut from) {
                Ok(f) => { if tx.send(Ok(f)).is_err() { break; } }
                Err(e) => { let _ = tx.send(Err(e.to_string())); break; }
            }
        });
        let mut to: File = pipes.to_helper;
        let mut ids = Ids::new(Side::App);
        let mut doc = Section::new(bytes.len()).expect("section");
        doc.as_mut_slice().copy_from_slice(bytes);
        let g = child.grant(&doc, Access::Read).expect("grant");
        let gr = GrantRef { wire: g.wire, len: g.len };
        let (open, kind) = if edit { (Message::OpenEdit(beebeeb_office_protocol::office::OpenEdit { doc: gr, kind: beebeeb_office_protocol::office::EditKind::Docx }), beebeeb_office_protocol::office::KIND_OPEN_EDIT) } else { (Message::Open(Open { kind: DocKind::Docx, doc: gr }), KIND_OPEN) };
        let id = ids.next_id();
        write_frame(&mut to, kind, id, &open.encode().expect("fits")).expect("write");
        let a = rx.recv_timeout(Duration::from_secs(60));
        let ans = match &a { Ok(Ok(f)) => format!("kind={:#06x} {:?}", f.kind, Message::decode(f.kind, &f.payload)), other => format!("{other:?}") };
        eprintln!("PROBE-WIN {name} open: {ans}");
        if let Ok(Ok(f)) = a {
            if let Ok(Message::Opened(opened)) = Message::decode(f.kind, &f.payload) {
                let ask = Render { page: 0, scale: check::render_scale(1.0, 96.0), region: None, out: GrantRef { wire: 0, len: 0 } };
                let (w, h) = check::asked_size(&ask, &opened.sizes[0], &Contract::CONTRACT).expect("size");
                let out = Section::new(w as usize * h as usize * 4).expect("out");
                let og = child.grant(&out, Access::ReadWrite).expect("grant out");
                let r = Message::Render(Render { out: GrantRef { wire: og.wire, len: og.len }, ..ask });
                let id = ids.next_id();
                write_frame(&mut to, KIND_RENDER, id, &r.encode().expect("fits")).expect("write");
                let a = rx.recv_timeout(Duration::from_secs(60));
                let ans = match &a { Ok(Ok(f)) => format!("kind={:#06x} {:?}", f.kind, Message::decode(f.kind, &f.payload)), other => format!("{other:?}") };
                eprintln!("PROBE-WIN {name} render: {ans}");
            }
        }
        eprintln!("PROBE-WIN {name} exit: {:?}", child.wait_timeout(Duration::from_millis(500)));
        let _ = child.kill();
    }

    #[test]
    fn viewer_probe_windows_sandbox() {
        for edit in [false, true] {
            eprintln!("PROBE-WIN edit={edit}");
            run("letter", include_bytes!("fixtures/letter-1.docx"), edit);
            run("probe-lo", super::PROBE, edit);
        }
    }
}
