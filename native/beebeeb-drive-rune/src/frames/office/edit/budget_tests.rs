//! The 16 ms frame budget of the editing surfaces: a Word frame with a caret move and a dirty strip, and a sheet
//! commit (one `SetCell` and the frame after it) at the cell cap.
//!
//! The Word frame holds 16 ms in debug too. The sheet commit gate of 16 ms is taken in a release build; a debug build is several times slower, so its bound is generous
//! (still far under a per-cell or per-page cost creeping into the frame) and the numbers are printed. The best of
//! several runs is taken so a busy machine does not fail it.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use beebeeb_office_protocol::office::{
    Dirty, EditKind, EditOp, Edited, HitMode, ListKind, Marks, PageRect, ParaStyle, RectPt, TextAlign,
};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

use crate::frames::office::docs::docs_fixture;
use crate::frames::office::docs::live_tests::{doc_of, find};
use crate::frames::office::edit::{self, EditUi};
use crate::frames::office::sheets::{self, Cell, CellText, SheetView};
use crate::frames::viewers::source::{Body, EditAnswer, OpenError, OpenFile, ViewerSource};
use crate::frames::viewers;
use crate::{State, init, view};

const GATE: Duration = Duration::from_millis(16);
/// A debug build runs several times slower than the shipped one.
const DEBUG_BOUND: Duration = Duration::from_millis(250);

/// The calling thread's CPU time (not wall time: a loaded rig stretches the wall clock, not the work). Where the
/// OS clock is not bound here, the wall clock stands in.
#[cfg(all(unix, target_pointer_width = "64", any(target_os = "linux", target_os = "macos")))]
fn cpu_now() -> Duration {
    #[repr(C)]
    struct Timespec {
        sec: i64,
        nsec: i64,
    }
    unsafe extern "C" {
        fn clock_gettime(clock: i32, ts: *mut Timespec) -> i32;
    }
    #[cfg(target_os = "linux")]
    const THREAD_CPU: i32 = 3;
    #[cfg(target_os = "macos")]
    const THREAD_CPU: i32 = 16;
    let mut ts = Timespec { sec: 0, nsec: 0 };
    // SAFETY: `ts` is a valid out-pointer of the C `timespec` layout on 64-bit Linux and macOS.
    let rc = unsafe { clock_gettime(THREAD_CPU, &mut ts) };
    assert_eq!(rc, 0, "the thread CPU clock");
    Duration::new(ts.sec as u64, ts.nsec as u32)
}

#[cfg(not(all(unix, target_pointer_width = "64", any(target_os = "linux", target_os = "macos"))))]
fn cpu_now() -> Duration {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed()
}

fn bound() -> Duration {
    if cfg!(debug_assertions) { DEBUG_BOUND } else { GATE }
}

fn best(runs: usize, mut f: impl FnMut()) -> Duration {
    (0..runs)
        .map(|_| {
            let t = cpu_now();
            f();
            cpu_now() - t
        })
        .min()
        .expect("a run")
}

#[derive(Default)]
struct Shared {
    answers: RefCell<VecDeque<EditAnswer>>,
    sets: RefCell<usize>,
}

struct Source {
    file: OpenFile,
    shared: Rc<Shared>,
}

impl ViewerSource for Source {
    fn open(&mut self, _id: &str) -> Result<OpenFile, OpenError> {
        Ok(self.file.clone())
    }
    fn poll(&mut self, _id: &str) -> Option<Body> {
        None
    }
    fn cancel(&mut self, _id: &str) {}
    fn retry(&mut self, _id: &str) -> Result<OpenFile, OpenError> {
        Ok(self.file.clone())
    }
    fn office_live(&self) -> bool {
        true
    }
    fn can_edit(&self, _id: &str) -> bool {
        true
    }
    fn edit(&mut self, _id: &str, op: EditOp) -> bool {
        if matches!(op, EditOp::SetCell { .. }) {
            *self.shared.sets.borrow_mut() += 1;
        }
        true
    }
    fn hit(&mut self, _id: &str, _page: u32, _x: f32, _y: f32, _mode: HitMode) -> bool {
        true
    }
    fn poll_edit(&mut self, _id: &str) -> Option<EditAnswer> {
        self.shared.answers.borrow_mut().pop_front()
    }
}

fn rig(body: Body, kind: EditKind) -> (Runtime<State>, Rc<Shared>) {
    let mut file = docs_fixture::file(false);
    file.body = body;
    let shared = Rc::new(Shared::default());
    let mut state = init();
    let id = file.id.clone();
    let source = Source { file, shared: shared.clone() };
    assert!(viewers::open_from(&mut state, Box::new(source), &id));
    viewers::ui(&mut state).expect("open").office.edit = Some(EditUi::new(kind));
    let env = Env { system_dark: false, scale: 1.0 };
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
    rt.on_key = Some(crate::shell::host::on_key);
    rt.poll = Some(crate::shell::host::poll);
    rt.frame(0.0);
    rt.frame(0.0);
    (rt, shared)
}

fn caret_answer(revision: u64, y: f32) -> EditAnswer {
    let strip = PageRect { page: 30, rect: RectPt { x: 0.0, y, w: 595.0, h: 16.0 } };
    EditAnswer::Edited(Edited {
        revision,
        caret: Some(PageRect { page: 30, rect: RectPt { x: 200.0, y, w: 1.0, h: 14.0 } }),
        selection: Vec::new(),
        can_undo: true,
        can_redo: false,
        dirty: Dirty::Rects(vec![strip]),
        pages: None,
        cells: Vec::new(),
        slides: None,
        marks: Marks {
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            style: ParaStyle::Normal,
            align: TextAlign::Left,
            list: ListKind::None,
        },
    })
}

/// A 60-page document: the helper's answer to a keystroke (a caret and a dirty strip) taken in, and the frame drawn.
#[test]
fn office_word_budget_a_caret_move_and_a_dirty_strip_fit_a_frame() {
    let (mut rt, shared) = rig(doc_of(60, 100, 30), EditKind::Docx);
    assert!(find(&rt, "pg-30").is_some() || find(&rt, "pg-29").is_some() || rt.elements().len() > 10, "the stage is drawn");
    let mut revision = 1u64;
    let frame = best(7, || {
        revision += 1;
        shared.answers.borrow_mut().push_back(caret_answer(revision, 100.0 + (revision % 40) as f32 * 14.0));
        rt.update(|s, tx| {
            edit::poll(s, tx);
        });
        rt.frame(1.0 / 60.0);
    });
    eprintln!("word budget: caret move + dirty strip frame {frame:?}");
    assert!(frame < bound(), "a Word frame with a caret move took {frame:?}");
}

fn full() -> SheetView {
    const CELLS: u32 = 65_536;
    const COLS: u32 = 256;
    let mut view = sheets::sheets_fixture::board();
    view.cells = (0..CELLS)
        .map(|i| CellText {
            row: i / COLS,
            col: i % COLS,
            text: format!("{}", 1000 + i),
            right_aligned: true,
            bold: false,
            value: Some(f64::from(i)),
            formula: None,
        })
        .collect();
    view.col_widths = vec![80.0; COLS as usize];
    view.row_heights = vec![24.0; (CELLS / COLS) as usize];
    view.extent = (CELLS / COLS, COLS);
    view.selection = None;
    view.current = Cell { row: 0, col: 0 };
    view
}

/// A sheet at the cell cap: type into a cell and press Enter (one `SetCell`, the editor closing, the current cell
/// moving) and draw the frame after it.
#[test]
fn office_sheets_budget_a_commit_at_the_cell_cap_fits_a_frame() {
    let (mut rt, shared) = rig(Body::Sheet(full()), EditKind::Xlsx);
    for _ in 0..4 {
        rt.frame(1.0 / 60.0);
    }
    let r = find(&rt, "sh-cell-3-3").expect("the cell is drawn").rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(1.0 / 60.0);
    let mut runs = Vec::new();
    for _ in 0..5 {
        rt.key(KeyEvent { key: Key::Text("4".into()), mods: Mods::default() });
        rt.frame(1.0 / 60.0);
        let t = cpu_now();
        rt.key(KeyEvent { key: Key::Enter, mods: Mods::default() });
        rt.frame(1.0 / 60.0);
        runs.push(cpu_now() - t);
    }
    let commit = runs.into_iter().min().expect("a run");
    eprintln!("sheet budget: commit + frame at the cell cap {commit:?}");
    assert_eq!(*shared.sets.borrow(), 5, "each Enter sent one SetCell");
    assert!(commit < bound(), "a commit at the cell cap took {commit:?}");
}
