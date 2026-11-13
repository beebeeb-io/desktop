//! The probe in a real window, for what a headless run cannot show: keys from the OS, a real input method, the
//! OS clipboard, and what the platform was told about the candidate window.
//!
//! `RUNE_INPUT_CHECK_LOG=<file>`: every event the probe hears is appended to that file, one per line. On Windows
//! each change of the caret also writes an `imm` line: the composition and candidate forms read back from the
//! window's input context (`ImmGetCompositionWindow`, `ImmGetCandidateWindow`), which is what an input method
//! reads to place its popup.

use std::io::Write;
use std::sync::Mutex;
use std::time::Duration;

use rune::Tx;
use rune_input_check::{self as probe, Probe};

/// Lines of `Probe::log` already written, and the last platform line (written again only when it changes).
static WRITTEN: Mutex<(usize, String)> = Mutex::new((0, String::new()));

fn poll(s: &mut Probe, t: f64, tx: &mut Tx) -> bool {
    let changed = probe::poll(s, t, tx);
    let mut written = WRITTEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(line) = platform::input_context(s.caret_drawn).filter(|line| *line != written.1) {
        s.log.push(line.clone());
        written.1 = line;
    }
    if let Some(path) = std::env::var_os("RUNE_INPUT_CHECK_LOG") {
        if written.0 < s.log.len() {
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                for line in &s.log[written.0..] {
                    let _ = writeln!(file, "{line}");
                }
                written.0 = s.log.len();
            }
        }
    }
    // Rune polls at the start of a frame, and a frame needs a reason: without this wake the log (and the focus
    // the probe reads in `poll`) would lag until the next input.
    tx.wake_after(Duration::from_millis(50));
    changed
}

fn main() {
    let mut spec = probe::spec();
    spec.poll = Some(poll);
    probe::app(spec).run();
}

#[cfg(windows)]
mod platform {
    //! Read-only calls into the window's input context. Declared here rather than through a bindings crate so
    //! the probe's dependency list stays the one line it checks.
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Default)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct CandidateForm {
        index: u32,
        style: u32,
        pos: Point,
        area: Rect,
    }
    #[repr(C)]
    #[derive(Default)]
    struct CompositionForm {
        style: u32,
        pos: Point,
        area: Rect,
    }

    #[link(name = "imm32")]
    unsafe extern "system" {
        fn ImmGetContext(hwnd: *mut c_void) -> *mut c_void;
        fn ImmReleaseContext(hwnd: *mut c_void, himc: *mut c_void) -> i32;
        fn ImmGetCandidateWindow(himc: *mut c_void, index: u32, form: *mut CandidateForm) -> i32;
        fn ImmGetCompositionWindow(himc: *mut c_void, form: *mut CompositionForm) -> i32;
        fn ImmGetOpenStatus(himc: *mut c_void) -> i32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetFocus() -> *mut c_void;
        fn GetDpiForWindow(hwnd: *mut c_void) -> u32;
        fn GetKeyboardLayout(thread: u32) -> *mut c_void;
    }

    /// One line: the caret the app drew (window points) beside the forms the platform holds (client pixels).
    pub fn input_context(caret: Option<[f32; 4]>) -> Option<String> {
        // SAFETY: plain Win32 getters on this thread's focused window; every out-pointer is a live local of the
        // exact struct the call fills, and the context is released before returning.
        unsafe {
            let hwnd = GetFocus();
            if hwnd.is_null() {
                return None;
            }
            let himc = ImmGetContext(hwnd);
            let (mut candidate, mut composition) = (CandidateForm::default(), CompositionForm::default());
            let (got_candidate, got_composition, open) = if himc.is_null() {
                (0, 0, 0)
            } else {
                (
                    ImmGetCandidateWindow(himc, 0, &mut candidate),
                    ImmGetCompositionWindow(himc, &mut composition),
                    ImmGetOpenStatus(himc),
                )
            };
            if !himc.is_null() {
                ImmReleaseContext(hwnd, himc);
            }
            Some(format!(
                "imm caret={caret:?} dpi={} context={} open={open} layout={:#x} candidate ok={got_candidate} style={:#x} pos=({},{}) area=({},{},{},{}) composition ok={got_composition} style={:#x} pos=({},{}) area=({},{},{},{})",
                GetDpiForWindow(hwnd),
                !himc.is_null(),
                GetKeyboardLayout(0) as usize & 0xffff,
                candidate.style,
                candidate.pos.x,
                candidate.pos.y,
                candidate.area.left,
                candidate.area.top,
                candidate.area.right,
                candidate.area.bottom,
                composition.style,
                composition.pos.x,
                composition.pos.y,
                composition.area.left,
                composition.area.top,
                composition.area.right,
                composition.area.bottom,
            ))
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn input_context(_: Option<[f32; 4]>) -> Option<String> {
        None
    }
}
