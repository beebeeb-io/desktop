//! The latency gate's harness (office viewer plan, section 2.4): a page of an open Word document, from the confined
//! helper, timed on the app's own path to a page: section, grant, frame, decode, size check, copy out; and a visible
//! range of a workbook's cells, timed from the request to the checked `CellData` in the app's hands.
//!
//! Windows only (the sandbox is). Release build only for the gate; run on the build VM with
//! `cargo test --release --locked --test latency -- --ignored --nocapture --test-threads=1`.
//!
//! Output carries counts, sizes and milliseconds only: never a file name, page text or pixel value.

use std::fs::File;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use beebeeb_office_protocol::check::{self, Limits as Contract};
use beebeeb_office_protocol::frame::{Frame, Ids, Side, read_frame, write_frame};
use beebeeb_office_protocol::office::{
    CellData, CellRange, Cells, DocKind, GrantRef, KIND_CELL_DATA, KIND_CELLS, KIND_OPEN, KIND_OPENED,
    KIND_OPENED_SHEETS, KIND_RENDER, KIND_RENDERED, Open, Opened, OpenedSheets, Rect, Render, Rendered,
};
use beebeeb_office_sandbox::{Access, Child, Limits as SandboxLimits, Sandbox, Section};

const HELPER: &str = env!("CARGO_BIN_EXE_beebeeb-office-render");
const FIXTURE: &[u8] = include_bytes!("fixtures/latency/big-60.docx");
/// 2,000 rows x 20 columns in three number formats, and one chart (`fixtures/latency/SOURCE.md`).
const GRID: &[u8] = include_bytes!("fixtures/latency/grid-2000.xlsx");
/// A visible range of cells: 60 rows x 20 columns.
const VISIBLE_ROWS: u32 = 60;
const VISIBLE_COLS: u32 = 20;
const DPI: f32 = 96.0;
const SAMPLES: usize = 200;
const TILE: u32 = 512;
const GATE_MS: f64 = 16.0;
const OPEN_RUNS: usize = 20;

fn skipped(name: &str) -> bool {
    if cfg!(windows) {
        println!(
            "latency build: {}",
            if cfg!(debug_assertions) { "debug" } else { "release" }
        );
        return false;
    }
    println!("SKIPPED {name}: no sandbox on this OS");
    true
}

/// A confined helper with the document open.
struct Session {
    child: Child,
    to_helper: File,
    answers: Receiver<Result<Frame, String>>,
    ids: Ids,
    // Held so the helper's read-only mapping of the document stays valid for the whole run.
    _doc: Section,
}

impl Session {
    /// Spawns a confined helper and opens the fixture in it.
    fn start() -> (Session, Opened) {
        let (session, frame) = Session::open(DocKind::Docx, FIXTURE, KIND_OPENED);
        let opened = Opened::decode(&frame.payload).expect("the Opened answer decodes");
        check::check_opened(&opened, &Contract::CONTRACT).expect("the Opened answer passes the checks");
        (session, opened)
    }

    /// Spawns a confined helper and opens the grid workbook in it.
    fn start_sheets() -> (Session, OpenedSheets) {
        let (session, frame) = Session::open(DocKind::Xlsx, GRID, KIND_OPENED_SHEETS);
        let sheets = OpenedSheets::decode(&frame.payload).expect("the OpenedSheets answer decodes");
        check::check_sheets(&sheets, &Contract::CONTRACT).expect("the OpenedSheets answer passes the checks");
        (session, sheets)
    }

    /// Spawns a confined helper and opens `bytes` as `kind` in it; the answer must be of `expect_kind`.
    fn open(kind: DocKind, bytes: &[u8], expect_kind: u16) -> (Session, Frame) {
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
        let payload = open.encode().expect("a small request fits a frame");
        let frame = session.ask(KIND_OPEN, &payload, expect_kind);
        (session, frame)
    }

    /// One visible range of cells, as the app gets it: request, frame, decode, the app's check. Returns the span and
    /// the number of cells.
    fn cells(&mut self, asked: Cells) -> (Duration, usize) {
        let started = Instant::now();
        let payload = asked.encode().expect("a small request fits a frame");
        let frame = self.ask(KIND_CELLS, &payload, KIND_CELL_DATA);
        let data = CellData::decode(&frame.payload).expect("the CellData answer decodes");
        check::check_cells(&asked, &data, &Contract::CONTRACT).expect("the CellData answer passes the checks");
        (started.elapsed(), data.cells.len())
    }

    /// Writes one request and waits for its answer, which must carry the request's id and the expected kind.
    fn ask(&mut self, kind: u16, payload: &[u8], expect_kind: u16) -> Frame {
        let id = self.ids.next_id();
        write_frame(&mut self.to_helper, kind, id, payload).expect("the request is written");
        let frame = match self.answers.recv_timeout(check::REQUEST_TIMEOUT) {
            Ok(Ok(frame)) => frame,
            Ok(Err(e)) => panic!("the helper's pipe failed: {e}"),
            Err(_) => panic!("the helper was silent for {} s", check::REQUEST_TIMEOUT.as_secs()),
        };
        assert_eq!(frame.id, id, "the answer's id is the request's");
        assert_eq!(
            frame.kind, expect_kind,
            "answer kind {:#06x}, wanted {:#06x}",
            frame.kind, expect_kind
        );
        frame
    }

    /// One page (or one region of it) at `scale`, as the app gets it. Returns (full span, render-only span, pixels,
    /// width, height).
    fn render(
        &mut self,
        opened: &Opened,
        page: u32,
        scale: f32,
        region: Option<Rect>,
    ) -> (Duration, Duration, Vec<u8>, u32, u32) {
        let started = Instant::now();
        let mut ask = Render {
            page,
            scale,
            region,
            out: GrantRef { wire: 0, len: 0 },
        };
        let size = &opened.sizes[page as usize];
        let (w, h) = check::asked_size(&ask, size, &Contract::CONTRACT).expect("the ask can be sized");
        let len = w as usize * h as usize * 4;
        let out = Section::new(len).expect("a section for the pixels");
        let granted = self
            .child
            .grant(&out, Access::ReadWrite)
            .expect("the pixel section is granted");
        ask.out = GrantRef {
            wire: granted.wire,
            len: granted.len,
        };
        let payload = ask.encode().expect("a small request fits a frame");

        let render_started = Instant::now();
        let frame = self.ask(KIND_RENDER, &payload, KIND_RENDERED);
        let got = Rendered::decode(&frame.payload).expect("the Rendered answer decodes");
        check::check_rendered(&ask, size, out.len() as u64, &got, &Contract::CONTRACT)
            .expect("the Rendered answer passes the checks");
        let render_only = render_started.elapsed();

        let pixels = out.as_slice()[..got.w as usize * got.h as usize * 4].to_vec();
        let full = started.elapsed();
        // Not timed: the sandbox zeroizes the section as it drops, at the end of this function (after `full`).
        (full, render_only, pixels, got.w, got.h)
    }
}

fn not_flat(pixels: &[u8]) -> bool {
    let (px, _) = pixels.as_chunks::<4>();
    px.len() >= 2 && px.iter().any(|p| p != &px[0])
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

struct Stats {
    median: f64,
    p95: f64,
    max: f64,
}

/// Nearest rank: the sample at index `ceil(p * n) - 1` of the sorted samples.
fn stats(samples: &[f64]) -> Stats {
    assert!(!samples.is_empty(), "no samples");
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("a duration is a number"));
    let n = sorted.len();
    let rank = |p: f64| sorted[((p * n as f64).ceil() as usize).clamp(1, n) - 1];
    Stats {
        median: rank(0.5),
        p95: rank(0.95),
        max: sorted[n - 1],
    }
}

/// A fixed-seed shuffle (linear congruential generator, Fisher-Yates), so every run visits the same order.
fn shuffled<T>(mut items: Vec<T>, seed: u64) -> Vec<T> {
    let mut state = seed;
    let mut next = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state >> 33
    };
    for i in (1..items.len()).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        items.swap(i, j);
    }
    items
}

struct RunResult {
    n: usize,
    full: Stats,
    render_only: Stats,
    w: u32,
    h: u32,
}

/// 1 warm-up (timed, printed, left out) and then `SAMPLES` requests, cycling through `work`.
fn run(
    session: &mut Session,
    opened: &Opened,
    kind: &str,
    zoom_percent: u32,
    scale: f32,
    work: &[(u32, Option<Rect>)],
) -> RunResult {
    assert!(!work.is_empty(), "no {kind} to ask for at zoom {zoom_percent}%");
    let (warm_full, warm_render, _, _, _) = session.render(opened, work[0].0, scale, work[0].1);
    println!(
        "latency warm-up {kind} zoom={zoom_percent}% dpi={DPI} scale={scale}: full={:.2}ms render-only={:.2}ms (not counted)",
        ms(warm_full),
        ms(warm_render)
    );
    let mut full = Vec::with_capacity(SAMPLES);
    let mut render_only = Vec::with_capacity(SAMPLES);
    let (mut w, mut h) = (0, 0);
    for i in 0..SAMPLES {
        let (page, region) = work[(i + 1) % work.len()];
        let (f, r, _pixels, pw, ph) = session.render(opened, page, scale, region);
        full.push(ms(f));
        render_only.push(ms(r));
        (w, h) = (pw, ph);
    }
    let result = RunResult {
        n: full.len(),
        full: stats(&full),
        render_only: stats(&render_only),
        w,
        h,
    };
    println!(
        "latency {kind} zoom={zoom_percent}% dpi={DPI} scale={scale} px={}x{}: n={} full median={:.2} p95={:.2} max={:.2} | render-only median={:.2} p95={:.2}",
        result.w,
        result.h,
        result.n,
        result.full.median,
        result.full.p95,
        result.full.max,
        result.render_only.median,
        result.render_only.p95
    );
    result
}

#[test]
#[ignore = "latency: VM 401, release build, --ignored"]
fn latency_open_to_first_page() {
    if skipped("latency_open_to_first_page") {
        return;
    }
    let scale = check::render_scale(1.0, DPI);
    let mut samples = Vec::with_capacity(OPEN_RUNS);
    for _ in 0..OPEN_RUNS {
        // A new helper each time, from before the spawn.
        let started = Instant::now();
        let (mut session, opened) = Session::start();
        let (_, _, pixels, _, _) = session.render(&opened, 0, scale, None);
        samples.push(ms(started.elapsed()));
        assert!(not_flat(&pixels), "page 0 is flat: one pixel value");
        let _ = session.child.kill();
    }
    assert_eq!(samples.len(), OPEN_RUNS, "20 of 20 opens succeeded");
    let s = stats(&samples);
    println!(
        "latency open->first-page: n={} median={:.2} p95={:.2} max={:.2} target<=500ms (reported, not a gate)",
        samples.len(),
        s.median,
        s.p95,
        s.max
    );
}

#[test]
#[ignore = "latency: VM 401, release build, --ignored"]
fn latency_page_p95_within_16ms() {
    if skipped("latency_page_p95_within_16ms") {
        return;
    }
    let (mut session, opened) = Session::start();
    let first = opened.sizes[0];
    println!(
        "latency fixture: pages={} size_pt={}x{}",
        opened.pages, first.w_pt, first.h_pt
    );
    assert!(opened.pages >= 2, "the fixture has at least 2 pages");

    let mut gates = Vec::new();
    for zoom_percent in [100u32, 150] {
        // One scale value per run: the same f32 goes into the size check and on the wire.
        let scale = check::render_scale(zoom_percent as f32 / 100.0, DPI);

        let pages = shuffled((0..opened.pages).map(|p| (p, None)).collect::<Vec<_>>(), 0x00BE_EBEE);
        let page_run = run(&mut session, &opened, "page", zoom_percent, scale, &pages);

        // 512 x 512 regions on a fixed grid, only those that lie wholly inside the page at this scale.
        let mut grid = Vec::new();
        for p in 0..opened.pages {
            let (pw, ph) = check::page_pixels(&opened.sizes[p as usize], scale).expect("the page can be sized");
            for ty in 0..ph / TILE {
                for tx in 0..pw / TILE {
                    grid.push((
                        p,
                        Some(Rect {
                            x: tx * TILE,
                            y: ty * TILE,
                            w: TILE,
                            h: TILE,
                        }),
                    ));
                }
            }
        }
        let tiles = shuffled(grid, 0x00BE_EBEE);
        let tile_run = run(&mut session, &opened, "tile", zoom_percent, scale, &tiles);

        let page_pass = page_run.full.p95 <= GATE_MS;
        let tile_pass = tile_run.full.p95 <= GATE_MS;
        let verdict = if page_pass {
            "PASS (page)"
        } else if tile_pass {
            "PASS (tile)"
        } else {
            "FAIL"
        };
        println!(
            "latency gate zoom={zoom_percent}%: page p95={:.2} tile p95={:.2} -> {verdict}",
            page_run.full.p95, tile_run.full.p95
        );
        assert_eq!(page_run.n, SAMPLES, "200 page samples at zoom {zoom_percent}%");
        assert_eq!(tile_run.n, SAMPLES, "200 tile samples at zoom {zoom_percent}%");
        gates.push((
            zoom_percent,
            page_pass || tile_pass,
            page_run.full.p95,
            tile_run.full.p95,
        ));
    }
    let _ = session.child.kill();

    if cfg!(debug_assertions) {
        println!("latency gate: not taken in a debug build");
        return;
    }
    for (zoom_percent, pass, page_p95, tile_p95) in gates {
        assert!(
            pass,
            "latency gate failed at zoom {zoom_percent}%: page p95={page_p95:.2} ms, tile p95={tile_p95:.2} ms, limit {GATE_MS:.2} ms"
        );
    }
}

#[test]
#[ignore = "latency: VM 401, release build, --ignored"]
fn latency_what_o_cp2_does_not_measure() {
    if skipped("latency_what_o_cp2_does_not_measure") {
        return;
    }
    println!("latency scroll: not measured at O-CP2 (needs the live docs scene, OF5 + V3-5): O-CP3");
    println!("latency cells: not measured at O-CP2 (sheets API, V3-2): O-CP3, latency_cells_p95_within_16ms");
}

/// The cells gate (PLAN section 2.4): a visible range of 60 x 20 cells, from the request to the checked `CellData` in
/// the app's hands, p95 at most 16 ms. 200 ranges (row starts every 10 rows, in a fixed shuffled order), after one
/// warm-up that is printed and left out. A 20 ms sleep before each answer in the helper turns it red.
#[test]
#[ignore = "latency: VM 401, release build, --ignored"]
fn latency_cells_p95_within_16ms() {
    if skipped("latency_cells_p95_within_16ms") {
        return;
    }
    let (mut session, sheets) = Session::start_sheets();
    let grid = sheets.sheets.first().expect("the grid workbook has a sheet");
    println!(
        "latency cells fixture: sheets={} rows={} cols={}",
        sheets.sheets.len(),
        grid.rows,
        grid.cols
    );
    assert!(
        grid.rows >= 2_000 && grid.cols >= VISIBLE_COLS,
        "the grid sheet is 2,000 x 20: {} x {}",
        grid.rows,
        grid.cols
    );

    let starts: Vec<u32> = (0..=grid.rows - VISIBLE_ROWS).step_by(10).collect();
    let ranges = shuffled(
        starts
            .into_iter()
            .map(|row| Cells {
                sheet: 0,
                range: CellRange {
                    row,
                    col: 0,
                    rows: VISIBLE_ROWS,
                    cols: VISIBLE_COLS,
                },
            })
            .collect::<Vec<_>>(),
        0x00BE_EBEE,
    );
    let (warm, warm_cells) = session.cells(ranges[0]);
    println!(
        "latency warm-up cells {VISIBLE_ROWS}x{VISIBLE_COLS}: {:.2}ms cells={warm_cells} (not counted)",
        ms(warm)
    );
    let mut samples = Vec::with_capacity(SAMPLES);
    let mut least = usize::MAX;
    for i in 0..SAMPLES {
        let (span, n) = session.cells(ranges[(i + 1) % ranges.len()]);
        samples.push(ms(span));
        least = least.min(n);
    }
    let _ = session.child.kill();
    let s = stats(&samples);
    let pass = s.p95 <= GATE_MS;
    println!(
        "latency cells {VISIBLE_ROWS}x{VISIBLE_COLS}: n={} median={:.2} p95={:.2} max={:.2} fewest-cells={least} -> {}",
        samples.len(),
        s.median,
        s.p95,
        s.max,
        if pass { "PASS" } else { "FAIL" }
    );
    assert_eq!(samples.len(), SAMPLES, "200 cell samples");
    assert!(least > 0, "every visible range holds cells");

    if cfg!(debug_assertions) {
        println!("latency cells gate: not taken in a debug build");
        return;
    }
    assert!(
        pass,
        "latency cells gate failed: p95={:.2} ms, limit {GATE_MS:.2} ms",
        s.p95
    );
}
