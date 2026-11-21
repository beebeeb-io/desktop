use super::*;

#[test]
fn starts() {
    assert_eq!(line_starts("a\nb\n"), vec![0, 2, 4]);
    assert_eq!(line_starts(""), vec![0]);
}

#[test]
fn line_of_edges() {
    let t = "a\nb\n";
    let s = line_starts(t);
    assert_eq!(line_of(&s, 0), 0);
    assert_eq!(line_of(&s, 1), 0);
    assert_eq!(line_of(&s, 2), 1);
    assert_eq!(line_of(&s, 4), 2);
    assert_eq!(line_of(&s, t.len()), 2);
}

#[test]
fn gutter() {
    assert_eq!(gutter_digits(0), 2);
    assert_eq!(gutter_digits(99), 2);
    assert_eq!(gutter_digits(100), 3);
}
