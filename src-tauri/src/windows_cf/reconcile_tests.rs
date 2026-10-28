//! Task 2240 slice 2: the placeholder reconcile through folder listings, on real Windows.
//!
//! Spec: `docs/superpowers/specs/2026-10-28-2240-reconcile-from-listing-design.md` (workspace repo), T10–T16, and
//! the source pins on the wiring.
use super::*;
use crate::source_pin::squeeze;

/// The text of `fn <name>(` up to the end of its body, with LF line endings on every checkout.
fn fn_text(source: &str, name: &str) -> String {
    let start = source
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("fn {name} is missing"));
    let end = source[start..].find("\n}\n").expect("fn body end");
    source[start..start + end].to_string()
}

#[test]
fn task_2240_reconcile_reads_no_attributes_per_row() {
    let source = include_str!("mod.rs").replace("\r\n", "\n");
    let with = squeeze(&fn_text(&source, "reconcile_placeholder_state_with"));
    assert!(
        with.contains("plan_reconcile("),
        "the pass must be decided by os_state_reconcile"
    );
    assert!(
        with.contains("placeholders::list_dir_attributes("),
        "attributes come from one listing per folder"
    );
    assert_eq!(
        with.matches("read_os_placeholder_state").count(),
        1,
        "the direct read appears once: in the read_direct closure"
    );
    assert!(with.contains("|rel|placeholders::read_os_placeholder_state("));
    let public = squeeze(&fn_text(&source, "reconcile_placeholder_state"));
    assert!(
        !public.contains("read_os_placeholder_state"),
        "the per-row loop is gone from the public wrapper"
    );
}
