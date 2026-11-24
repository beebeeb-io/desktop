//! Tests: indentation rules.

use super::*;

#[test]
fn detects() {
    assert_eq!(detect("fn a() {\n\tx;\n\ty;\n}"), IndentUnit::Tab);
    assert_eq!(detect("fn a() {\n  x;\n  y;\n}"), IndentUnit::Spaces(2));
    assert_eq!(detect("plain"), IndentUnit::Spaces(4));
}

#[test]
fn round_trip_keeps_selection_lines() {
    let t = "one\ntwo\nthree\nfour";
    let sel = 2..10; // from inside line 0 to inside line 2
    for u in [IndentUnit::Tab, IndentUnit::Spaces(2)] {
        let (a, s1) = indent_lines(t, sel.clone(), u);
        assert_eq!(a.lines().filter(|l| l.starts_with(unit_str(u).as_str())).count(), 3);
        assert_eq!(&a[s1.start..s1.start + 1], "e");
        let (b, s2) = outdent_lines(&a, s1, u);
        assert_eq!(b, t);
        assert_eq!(s2, sel);
    }
}

#[test]
fn selection_ending_at_line_start_leaves_that_line() {
    let (a, _) = indent_lines("a\nb\n", 0..2, IndentUnit::Spaces(2));
    assert_eq!(a, "  a\nb\n");
}

#[test]
fn outdent_partial_and_empty_lines() {
    let (a, _) = outdent_lines(" a\n\nb", 0..5, IndentUnit::Spaces(4));
    assert_eq!(a, "a\n\nb");
}
