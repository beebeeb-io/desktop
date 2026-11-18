//! Typing latency (contract section 6, "keystroke to pixels"): 200 characters typed in the middle of
//! `fixtures/latency/big-60.docx`, each timed from the `Edit` frame to the `Edited` answer and the pixels of the
//! strip the answer calls dirty (the page's dirty rectangle, or the caret's page for `Dirty::All`), through the
//! helper's loop with the real Office engine. p95 must be <= 16 ms.
//!
//! The gate is taken in a release build, like `tests/latency.rs` (a debug engine is several times slower); a debug
//! build runs the same path and prints the numbers. On Linux this runs now; the Windows number comes in W6
//! (`cargo test --release --locked --test edit_latency -- --nocapture` on VM 401).
//!
//! Output carries counts and milliseconds only: never page text or a pixel value.

mod common;

use std::time::{Duration, Instant};

use beebeeb_office_protocol::check::{self, Limits};
use beebeeb_office_protocol::office::{
    Dirty, Edit, EditKind, EditOp, GrantRef, Hit, HitMode, Message, Opened, PageRect, Rect, Render,
};
use common::{FakeMemory, Running, open_real_edit};

const FIXTURE: &[u8] = include_bytes!("fixtures/latency/big-60.docx");
const SAMPLES: usize = 200;
const GATE_MS: f64 = 16.0;
const DPI: f32 = 96.0;
/// A strip is at least this many pixels high and at most the page.
const MIN_STRIP_PX: f32 = 8.0;

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Nearest rank, as `tests/latency.rs`.
fn rank(samples: &[f64], p: f64) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("a duration is a number"));
    let n = sorted.len();
    sorted[((p * n as f64).ceil() as usize).clamp(1, n) - 1]
}

/// The pixel region of `r` on `opened`'s page at `scale`, inside the page.
fn region(opened: &Opened, r: &PageRect, scale: f32) -> Rect {
    let (pw, ph) = check::page_pixels(&opened.sizes[r.page as usize], scale).expect("the page can be sized");
    let y0 = ((r.rect.y * scale).floor().max(0.0) as u32).min(ph.saturating_sub(1));
    let y1 = (((r.rect.y + r.rect.h) * scale).ceil().max(0.0) as u32).clamp(y0 + 1, ph);
    let h = (y1 - y0).max(MIN_STRIP_PX as u32).min(ph - y0);
    Rect { x: 0, y: y0, w: pw, h }
}

fn render(helper: &mut Running, memory: &FakeMemory, id: u32, opened: &Opened, page: u32, scale: f32, region: Option<Rect>) -> Vec<u8> {
    let mut ask = Render {
        page,
        scale,
        region,
        out: GrantRef { wire: 0, len: 0 },
    };
    let (w, h) = check::asked_size(&ask, &opened.sizes[page as usize], &Limits::CONTRACT).expect("the ask can be sized");
    let out = memory.zeroed(w as usize * h as usize * 4);
    ask.out = out;
    helper.send(id, &Message::Render(ask));
    match helper.answer() {
        (got, Message::Rendered(r)) if got == id => {
            assert_eq!((r.w, r.h), (w, h), "the strip is the size asked");
        }
        other => panic!("Rendered for {id} was due: {other:?}"),
    }
    memory.bytes(out)
}

#[test]
fn edit_latency_typing_in_the_middle_of_60_pages_p95_within_16ms() {
    let memory = FakeMemory::new();
    let (mut helper, opened) = open_real_edit(&memory, EditKind::Docx, FIXTURE);
    let opened = match opened {
        Message::Opened(o) => o,
        other => panic!("Opened was due: {other:?}"),
    };
    assert!(opened.pages >= 30, "the fixture is a long document, {} pages", opened.pages);
    let scale = check::render_scale(1.0, DPI);

    // Put the caret in the middle of the middle page.
    let mid = opened.pages / 2;
    let size = opened.sizes[mid as usize];
    let mut id = 2u32;
    helper.send(
        id,
        &Message::Hit(Hit {
            page: mid,
            x_pt: size.w_pt / 2.0,
            y_pt: size.h_pt / 2.0,
            mode: HitMode::Place,
        }),
    );
    match helper.answer() {
        (got, Message::Edited(e)) if got == id => assert!(e.caret.is_some(), "the click put a caret down"),
        other => panic!("Edited for the click was due: {other:?}"),
    }

    let mut samples = Vec::with_capacity(SAMPLES);
    let mut whole = 0usize;
    let mut applies: Vec<f64> = Vec::new();
    let mut strips: Vec<(u32, u32)> = Vec::new();
    // One warm-up character (not counted), then the 200.
    for i in 0..=SAMPLES {
        let ch = char::from(b'a' + (i % 26) as u8).to_string();
        id += 2;
        let started = Instant::now();
        helper.send(id, &Message::Edit(Edit { op: EditOp::InsertText(ch) }));
        let edited = match helper.answer() {
            (got, Message::Edited(e)) if got == id => e,
            other => panic!("Edited for {id} was due: {other:?}"),
        };
        let apply_ms = ms(started.elapsed());
        let caret = edited.caret.expect("the caret is on a page after a character");
        let (page, strip) = match &edited.dirty {
            Dirty::Rects(rects) if !rects.is_empty() => (rects[0].page, Some(region(&opened, &rects[0], scale))),
            _ => {
                whole += 1;
                (caret.page, None)
            }
        };
        id += 2;
        let strip_dbg = strip;
        let pixels = render(&mut helper, &memory, id, &opened, page, scale, strip);
        let elapsed = ms(started.elapsed());
        assert!(pixels.chunks_exact(4).any(|p| p != &pixels[..4]), "the strip is not flat");
        if i > 0 {
            applies.push(apply_ms);
            if let Some(s) = &strip_dbg { strips.push((s.w, s.h)); }
            samples.push(elapsed);
        }
    }
    helper.close();

    assert_eq!(samples.len(), SAMPLES);
    let p95 = rank(&samples, 0.95);
    println!(
        "edit latency ({}): n={} median={:.2} p95={:.2} max={:.2} whole-page-redraws={} limit={GATE_MS:.2}",
        if cfg!(debug_assertions) { "debug" } else { "release" },
        samples.len(),
        rank(&samples, 0.5),
        p95,
        rank(&samples, 1.0),
        whole
    );
    println!("edit apply: median={:.2} p95={:.2}; strip {:?}", rank(&applies, 0.5), rank(&applies, 0.95), strips.first());
    if cfg!(debug_assertions) {
        println!("edit latency gate: not taken in a debug build");
        return;
    }
    assert!(p95 <= GATE_MS, "typing latency p95 {p95:.2} ms is above {GATE_MS:.2} ms");
}
