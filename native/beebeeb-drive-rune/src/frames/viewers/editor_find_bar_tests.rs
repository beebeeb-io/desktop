//! The find bar's steps and replaces on a bare field. Every text is invented.

use super::*;

fn setup(text: &str) -> (EditorView, rune::TextEdit) {
    let mut v = EditorView::default();
    let mut e = rune::TextEdit::new(text);
    open(&mut v, &mut e, true);
    (v, e)
}

#[test]
fn find_bar_steps_through_matches() {
    let (mut v, mut e) = setup("foo bar foo");
    v.find.needle.replace_text("foo");
    step(&mut v, &mut e, false);
    assert_eq!(e.range(), (0, 3));
    assert_eq!(status(&v), "1 of 2");
    step(&mut v, &mut e, false);
    assert_eq!(e.range(), (8, 11));
    assert_eq!(status(&v), "2 of 2");
}

#[test]
fn find_bar_replace_all_marks_unsaved() {
    let (mut v, mut e) = setup("foo bar foo");
    v.find.needle.replace_text("foo");
    v.find.replacement.replace_text("x");
    assert_eq!(replace_all(&mut v, &mut e), "Replaced 2");
    assert_eq!(e.text, "x bar x");
    assert!(!e.is_pristine());
}

#[test]
fn find_bar_whole_word_skips_longer_words() {
    let (mut v, mut e) = setup("food foo");
    v.find.needle.replace_text("foo");
    v.find.query.whole_word = true;
    step(&mut v, &mut e, false);
    assert_eq!(e.range(), (5, 8));
    assert_eq!(status(&v), "1 of 1");
}

#[test]
fn find_bar_replace_current_then_steps() {
    let (mut v, mut e) = setup("foo bar foo");
    v.find.needle.replace_text("foo");
    v.find.replacement.replace_text("x");
    step(&mut v, &mut e, false);
    replace_current(&mut v, &mut e);
    assert_eq!(e.text, "x bar foo");
    assert_eq!(e.range(), (6, 9));
}
