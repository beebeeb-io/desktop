use super::*;

const ENTRIES: [Entry; 6] = [
    Entry::Head("Sort by"),
    Entry::check("name", "Name", true),
    Entry::item("size", "Size").dimmed(true),
    Entry::Sep,
    Entry::keyed("open", "Open", "Enter"),
    Entry::danger("trash", "Move to Trash", "Del"),
];

#[test]
fn step_skips_heading_sep_and_dimmed_and_wraps() {
    assert_eq!(step(&ENTRIES, None, 1), Some(1));
    assert_eq!(step(&ENTRIES, Some(1), 1), Some(4));
    assert_eq!(step(&ENTRIES, Some(5), 1), Some(1));
    assert_eq!(step(&ENTRIES, Some(1), -1), Some(5));
    assert_eq!(step(&ENTRIES, None, -1), Some(5));
}

#[test]
fn item_keys_skip_separators_only() {
    assert_eq!(item_key("menu", &ENTRIES, 0), None);
    assert_eq!(item_key("menu", &ENTRIES, 4).as_deref(), Some("menu-item-3"));
}

#[test]
fn height_counts_each_kind() {
    assert_eq!(height(&ENTRIES), 8.0 + 26.0 + 4.0 * 30.0 + 8.0 + kit::HAIR);
}

#[test]
fn place_at_stays_in_window() {
    assert_eq!(place_at((10.0, 20.0), (100.0, 50.0), (800.0, 600.0)), (10.0, 20.0));
    assert_eq!(place_at((750.0, 580.0), (100.0, 50.0), (800.0, 600.0)), (650.0, 530.0));
    assert_eq!(place_at((50.0, 50.0), (900.0, 700.0), (800.0, 600.0)), (0.0, 0.0));
}

#[test]
fn place_under_flips_up_when_no_room() {
    let w = (800.0, 600.0);
    assert_eq!(place_under((10.0, 10.0, 80.0, 30.0), (100.0, 100.0), w, 4.0), (10.0, 44.0));
    assert_eq!(place_under((780.0, 560.0, 20.0, 30.0), (100.0, 100.0), w, 4.0), (700.0, 456.0));
}

#[test]
fn sized_height_uses_the_size_row_and_default_is_unchanged() {
    let e = [Entry::item("a", "A"), Entry::Head("H"), Entry::Sep, Entry::item("b", "B")];
    assert_eq!(height(&e), 2.0 * PAD + 30.0 + HEAD_H + 2.0 * PAD + kit::HAIR + 30.0);
    assert_eq!(height_sized(&e, Size::Sm), 2.0 * PAD + 22.0 + HEAD_H + 2.0 * PAD + kit::HAIR + 22.0);
    assert_eq!(item_h(Size::Md), 28.0);
}
