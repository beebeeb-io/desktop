//! Tests: syntax highlighting.

use super::*;

#[test]
fn rust_gets_coloured_spans() {
    let t = "fn main() {}";
    let s = spans("main.rs", t, true);
    assert!(s.iter().any(|s| s.color.is_some()));
    assert!(s.iter().all(|s| (s.end as usize) <= t.len()));
}

#[test]
fn unknown_extension_gives_none() {
    assert!(spans("a.xyz", "fn main() {}", false).is_empty());
    assert_eq!(language_for("a.xyz"), None);
}
