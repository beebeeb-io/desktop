//! Tests: the file list widget.

use super::*;
use crate::theme::file_icon_colors::tint_color;
use crate::theme::file_icons::Tint;
use crate::theme::tokens;

fn item(name: &str, folder: bool, protected: bool, mime: Option<&str>) -> DriveItem {
    DriveItem {
        id: name.into(),
        parent_id: None,
        name: name.into(),
        is_folder: folder,
        size: None,
        child_count: None,
        modified_at: String::new(),
        mime: mime.map(String::from),
        has_thumbnail: false,
        shared_type: None,
        shared_badges: vec![],
        pc_status: None,
        is_protected: protected,
    }
}

#[test]
fn rows_draw_the_registry_icon_in_its_tint() {
    let t = tokens::theme(false);
    let items = [
        (item("Photos", true, false, None), Tint::Image),
        (item("Backups", true, true, None), Tint::Folder),
        (item("a.pdf", false, false, None), Tint::Pdf),
        (item("main.rs", false, false, None), Tint::Code),
    ];
    kit::record();
    for (i, (it, _)) in items.iter().enumerate() {
        let _ = file_row(i, it, false, &t);
    }
    let notes = kit::take();
    for (i, (it, tint)) in items.iter().enumerate() {
        let n = &notes[&format!("row-{i}-icon")];
        assert_eq!(n.color, Some(tint_color(*tint, false)), "{}", it.name);
        assert!(std::ptr::eq(n.icon.unwrap(), file_type_icon(it)), "{}", it.name);
    }
}

#[test]
fn board_stamp_keeps_yesterday_and_the_weekday_on_any_day() {
    use chrono::{Duration, NaiveDate};
    let first = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
    for n in 0..60 {
        let today = first + Duration::days(n);
        let now = today.and_hms_opt(12, 0, 0).unwrap();
        let fmt = |iso: &str| {
            let s = board_stamp_at(iso, today);
            let dt = chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S").unwrap();
            format_date_at(dt, now)
        };
        assert_eq!(fmt("2026-11-05T09:12:00Z"), "Today, 09:12", "{today}");
        assert_eq!(fmt("2026-11-04T16:42:00Z"), "Yesterday, 16:42", "{today}");
        let harbour = fmt("2026-11-01T14:00:00Z");
        assert!(harbour == "Sunday" || harbour.contains("20"), "{today}: {harbour}");
    }
    assert_eq!(board_stamp_at("2026-10-01T00:00:00Z", first), "2026-10-01T00:00:00Z");
}
