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
