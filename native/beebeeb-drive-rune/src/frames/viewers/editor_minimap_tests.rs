//! The minimap's budget (ux-21): built once in 20 ms, redrawn in 2 ms; and what the picture and a press mean.

use std::time::Instant;

use super::*;
use crate::theme::tokens;

fn big() -> String {
    (0..10_000)
        .map(|i| format!("    let value_{i} = compute(\"text {i}\", {i}) + 1; // note\n"))
        .collect()
}

#[ignore = "timing: wall-clock build/redraw budget; runs in the non-blocking Timing tests job (scripts/ci-timing-tests.txt)"]
#[test]
fn budget_ten_thousand_lines() {
    let t = tokens::theme(false);
    let text = big();
    let start = Instant::now();
    let (rows, buf) = build(&text, "main.rs", &t);
    let built = start.elapsed();
    assert_eq!(rows, MAX_ROWS);
    assert_eq!(buf.len(), rows * WIDTH as usize * 4);
    let edit = rune::TextEdit::new(text);
    let _ = minimap(&edit, "main.rs", 0.0, &t);
    let start = Instant::now();
    for _ in 0..20 {
        let _ = minimap(&edit, "main.rs", 100.0, &t);
    }
    let redraw = start.elapsed() / 20;
    eprintln!("minimap build {built:?}, redraw {redraw:?}");
    // The budget is for an optimised build. Tests always run with debug assertions (the engine needs them), and a
    // plain `cargo test` is unoptimised, so those runs get a documented 10x allowance; the picture is still checked.
    let k: u128 = if cfg!(debug_assertions) { 10 } else { 1 };
    assert!(built.as_millis() <= 20 * k, "build took {built:?}");
    assert!(redraw.as_micros() <= 2_000 * k, "redraw took {redraw:?}");
}

#[test]
fn three_lines_three_rows() {
    let t = tokens::theme(false);
    let (rows, buf) = build("fn a() {}\n\n  let x = 1;", "a.rs", &t);
    assert_eq!(rows, 3);
    let row = |r: usize| &buf[r * WIDTH as usize * 4..(r + 1) * WIDTH as usize * 4];
    assert!(row(0).chunks(4).any(|p| p[3] > 0), "code is coloured");
    assert!(row(1).iter().all(|b| *b == 0), "an empty line stays clear");
    assert_eq!(row(2)[..8], [0u8; 8], "leading space stays clear");
    assert!(row(2).chunks(4).any(|p| p[3] > 0));
}

#[test]
fn pressing_row_two_of_a_hundred_lines() {
    assert_eq!(line_at(2.5, 100.0, 100), 2);
    assert_eq!(scroll_for(line_at(2.5, 100.0, 100)), 42.0);
    assert_eq!(line_at(1e9, 100.0, 100), 99);
    assert_eq!(line_at(-5.0, 100.0, 100), 0);
}
