//! The 16 ms frame budget of a sheet held at its cell cap: the clone of the view (the host hands the body over by clone
//! each turn) and a frame drawn from it. The cap is `MAX_DRAWN_CELLS` (65,536) held cells, with text and values.
//!
//! Debug builds are slower than the shipped one, so the bounds are generous (several times the measured figure) but
//! still catch an accidental per-cell cost in the frame; the best of several runs is taken so a busy machine does not
//! fail it.

use std::time::Duration;

use super::*;
use crate::frames::office::docs::live_tests::{find, live_runtime};

const CELLS: u32 = MAX_DRAWN_CELLS as u32;
const COLS: u32 = 256;

fn full() -> SheetView {
    let mut view = sheets_fixture::board();
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
    // A selection over every cell: the status line sums them all each frame.
    view.selection = Some((Cell { row: 0, col: 0 }, Cell { row: CELLS / COLS - 1, col: COLS - 1 }));
    view
}

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

#[test]
fn office_sheets_budget_clone_and_frame_at_the_cell_cap_fit_a_frame() {
    let view = full();
    let clone = best(5, || {
        std::hint::black_box(view.clone());
    });
    let stat = best(5, || {
        std::hint::black_box(stats(&view));
    });
    let mut empty = view.clone();
    empty.cells.clear();
    let (mut rt0, _l0) = live_runtime(Body::Sheet(empty), 1.0, None);
    let bare = best(5, || {
        rt0.update(|_, _| {});
        rt0.frame(1.0 / 60.0);
    });
    let (mut rt, _log) = live_runtime(Body::Sheet(view), 1.0, None);
    assert!(find(&rt, "sh-cell-0-0").is_some(), "the sheet is drawn");
    let frame = best(5, || {
        rt.update(|_, _| {});
        rt.frame(1.0 / 60.0);
    });
    eprintln!("sheet budget: bare {bare:?} clone {clone:?}, stats {stat:?}, frame {frame:?}");
    let ms = Duration::from_millis;
    assert!(clone < ms(16), "cloning {CELLS} cells took {clone:?}");
    assert!(stat < ms(16), "the figures over {CELLS} cells took {stat:?}");
    assert!(frame < ms(100), "a frame with {CELLS} cells held took {frame:?}");
}
