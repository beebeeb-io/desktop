//! Tests: the file row widget.

use super::*;
use crate::theme::file_icons::{FileIcon, IconId, Tint};
use crate::theme::tokens;
use rune::{Env, Runtime, col};

fn draw(view: fn(&(), &mut rune::Cx<()>) -> El) -> Vec<rune::ElementInfo> {
    let mut rt = Runtime::new((), view, (900.0, 300.0), Env { system_dark: false, scale: 1.0 });
    rt.frame(0.0);
    rt.elements().into_iter().filter(|e| !e.exiting).collect()
}

fn info<'a>(all: &'a [rune::ElementInfo], key: &str) -> &'a rune::ElementInfo {
    all.iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

const PDF: FileIcon = FileIcon { id: IconId::Pdf, tint: Tint::Pdf };
const LONG: &str = "Quarterly report for the board of directors, second draft, with appendix 12.pdf";

fn rows(_: &(), _: &mut rune::Cx<()>) -> El {
    let t = tokens::theme(false);
    let cell = |k: &str| el().key(k.to_string()).w(80.0).h(10.0);
    col()
        .w_full()
        .child(row("row-0", "a.pdf", PDF, "a.pdf", vec![cell("c-0")], RowLook::default(), &t))
        .child(row(
            "row-1",
            LONG,
            PDF,
            LONG,
            vec![cell("c-1")],
            RowLook { selected: true, position: Some((2, 212)), ..RowLook::default() },
            &t,
        ))
        .child(row(
            "row-2",
            "b.pdf",
            PDF,
            "b.pdf",
            vec![],
            RowLook { dimmed: true, ..RowLook::default() },
            &t,
        ))
}

#[test]
fn a_row_is_keyed_by_the_caller_named_by_its_label_and_holds_its_cells() {
    let all = draw(rows);
    let r = info(&all, "row-0");
    assert_eq!(r.role, "row");
    assert_eq!(r.label.as_deref(), Some("a.pdf"));
    assert!(r.focusable);
    for k in ["row-0-tile", "row-0-icon", "row-0-name", "c-0"] {
        let e = info(&all, k);
        assert!(e.path.starts_with(&r.path), "{k} is inside row-0");
    }
    assert_eq!(info(&all, "row-0-name").text.as_deref(), Some("a.pdf"));
}

#[test]
fn the_selected_row_has_the_selection_fill_and_the_others_none() {
    let t = tokens::theme(false);
    let all = draw(rows);
    assert_eq!(info(&all, "row-1").fill, t.selection_row);
    assert_eq!(info(&all, "row-0").fill, rune::Color::TRANSPARENT);
}

#[test]
fn a_dimmed_row_shows_less_and_a_long_name_is_cut_in_the_middle() {
    let all = draw(rows);
    assert!(info(&all, "row-2").opacity < 1.0);
    assert_eq!(info(&all, "row-0").opacity, 1.0);
    let name = info(&all, "row-1-name").text.clone().unwrap();
    assert!(name.contains('\u{2026}'), "{name}");
    assert!(name.ends_with("appendix 12.pdf"), "{name}");
    assert_eq!(info(&all, "row-1").label.as_deref(), Some(LONG), "the label stays whole");
}

#[test]
fn shortened_keeps_short_names_whole() {
    assert_eq!(shortened("a.pdf"), None);
    assert_eq!(shortened(&"x".repeat(NAME_MAX)), None);
    let cut = shortened(&"x".repeat(NAME_MAX + 1)).unwrap();
    assert_eq!(cut.chars().count(), 32 + 1 + 23);
}

fn headers(_: &(), _: &mut rune::Cx<()>) -> El {
    let t = tokens::theme(false);
    header(
        &[
            Column::new("name", "Name", Width::Flex(1.0)),
            Column::new("size", "Size", Width::Fixed(84.0)).align(rune::Align::End),
            Column::new("pc", "PC", Width::Fixed(34.0)).spoken("On this PC"),
        ],
        Some((1, false)),
        &t,
    )
}

#[test]
fn the_header_has_a_columnheader_per_column_and_one_sort_arrow() {
    let all = draw(headers);
    let h = info(&all, "column-headers");
    assert_eq!(h.role, "row");
    for (k, label) in [("name", "Name"), ("size", "Size"), ("pc", "On this PC")] {
        let cell = info(&all, &format!("col-{k}-cell"));
        assert_eq!(cell.role, "columnheader", "{k}");
        assert_eq!(cell.label.as_deref(), Some(label), "{k}");
        assert!(info(&all, &format!("col-{k}")).path.starts_with(&cell.path), "{k}");
    }
    assert_eq!(info(&all, "col-size-cell").rect[2], 84.0);
    let arrows: Vec<_> = all.iter().filter(|e| e.key.as_deref() == Some("sort-arrow")).collect();
    assert_eq!(arrows.len(), 1);
    assert!(arrows[0].path.starts_with(&info(&all, "col-size-cell").path));
}
