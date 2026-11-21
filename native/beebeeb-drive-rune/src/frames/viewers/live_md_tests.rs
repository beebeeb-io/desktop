use super::{PARSES, blocks_of, body};
use crate::frames::viewers::markdown::Block;
use crate::theme::tokens;

#[test]
fn renders_heading_and_paragraph() {
    let blocks = blocks_of("# Title\n\nSome *text*");
    assert!(matches!(blocks[0], Block::Heading(1, _)) && matches!(blocks[1], Block::Para(_)));
    let _ = body("# Title\n\nSome *text*", &tokens::theme(false));
}

#[test]
fn parses_once_per_source() {
    let t = tokens::theme(false);
    let before = PARSES.with(|n| n.get());
    body("# Cache probe one", &t);
    body("# Cache probe one", &t);
    assert_eq!(PARSES.with(|n| n.get()) - before, 1);
    body("# Cache probe one!", &t);
    assert_eq!(PARSES.with(|n| n.get()) - before, 2);
}
