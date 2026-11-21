use super::{PARSES, view};
use crate::frames::Ui;
use crate::theme::tokens;

fn dump(source: &str) -> String {
    format!("{:?}", view(source, &Ui::default(), &tokens::theme(false)))
}

#[test]
fn renders_heading_and_paragraph_under_the_body_key() {
    let d = dump("# Title\n\nSome *text*");
    assert!(d.contains("live-preview-md-body") && d.contains("md-h1") && d.contains("md-p-0"), "{d}");
}

#[test]
fn parses_once_per_source() {
    let before = PARSES.with(|n| n.get());
    dump("# Cache probe one");
    dump("# Cache probe one");
    assert_eq!(PARSES.with(|n| n.get()) - before, 1);
    dump("# Cache probe one!");
    assert_eq!(PARSES.with(|n| n.get()) - before, 2);
}
