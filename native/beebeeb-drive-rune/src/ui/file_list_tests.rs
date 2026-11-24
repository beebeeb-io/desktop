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
        let _ = file_row(i, it, false, Fit::ALL, &t);
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

/// RC2-10 (w150): at 125 % with the details panel open the list was about 520 pt wide and the name about 9
/// characters ("pho...png"). The fixed columns give way, Shared first, then PC, by the list's own width.
#[test]
fn file_list_rc2_name_keeps_its_floor_at_125_with_details() {
    const W150_LIST_W: f32 = 520.0;
    // The floor is at least 20 characters of the 13 pt name beside its tile.
    const { assert!(NAME_MIN_W >= 22.0 + 8.0 + 20.0 * 7.0) };
    let fit = Fit::of(Some(W150_LIST_W));
    assert!(
        fit.name_w(W150_LIST_W) >= NAME_MIN_W,
        "{fit:?}: {}",
        fit.name_w(W150_LIST_W)
    );
    // Before this item every column stood and the name had less than half the floor.
    assert!(Fit::ALL.name_w(W150_LIST_W) < NAME_MIN_W / 2.0);
    // Shared gives way first: a list just too narrow for all of them keeps PC.
    let only_shared = Fit {
        shared: false,
        pc: true,
    };
    let w = NAME_MIN_W + (Fit::ALL.name_w(0.0)).abs() - 1.0;
    assert_eq!(Fit::of(Some(w)), only_shared);
    assert!(only_shared.name_w(w) >= NAME_MIN_W);
    // A wide list, and one not laid out yet, keep every column (the board's).
    assert_eq!(Fit::of(Some(1000.0)), Fit::ALL);
    assert_eq!(Fit::of(None), Fit::ALL);
    // The headers and the rows drop the same columns: the cells line up.
    let keys: Vec<_> = fit.columns().iter().map(|c| c.key).collect();
    assert_eq!(keys, ["name", "date", "size"]);
    assert_eq!(
        only_shared.columns().iter().map(|c| c.key).collect::<Vec<_>>(),
        ["name", "date", "size", "pc"]
    );
    let t = tokens::theme(false);
    let mut it = item("photo-2026-11-05.png", false, false, None);
    it.pc_status = Some("online_only".into());
    it.shared_type = Some("Link".into());
    kit::record();
    let _ = file_row(0, &it, false, fit, &t);
    let notes = kit::take();
    assert!(notes.contains_key("row-0-date") && notes.contains_key("row-0-size"));
    assert!(!notes.contains_key("row-0-link") && !notes.contains_key("row-0-pc"));
}

/// RC2-15: the regional format's decimal separator and date order; en-GB (the board's) everywhere else.
#[test]
fn file_list_rc2_nl_size_has_a_comma_and_dates_follow_the_region() {
    use crate::ui::region::{self, Region};
    use chrono::NaiveDate;
    const BYTES: i64 = 658_299_699; // 627.8 MB
    assert_eq!(crate::source::format_size(BYTES), "627.8 MB");
    region::pin(Region::NL_NL, || {
        assert_eq!(crate::source::format_size(BYTES), "627,8 MB");
        assert_eq!(crate::source::format_size(1_288_490_189), "1,2 GB");
        assert_eq!(crate::source::format_size(84 * 1024), "84 KB");
        // The list's own rule drops ",0" from 10 up, as it drops ".0".
        assert_eq!(crate::frames::files::long_list::size_words(112 * 1024 * 1024), "112 MB");
        assert_eq!(crate::frames::files::long_list::size_words(4 * 1024 * 1024), "4,0 MB");
    });
    assert_eq!(
        crate::source::format_size(BYTES),
        "627.8 MB",
        "en-GB again after the pin"
    );

    let now = NaiveDate::from_ymd_opt(2026, 11, 22)
        .unwrap()
        .and_hms_opt(18, 0, 0)
        .unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 11, 22)
        .unwrap()
        .and_hms_opt(17, 19, 0)
        .unwrap();
    let old = NaiveDate::from_ymd_opt(2026, 10, 28)
        .unwrap()
        .and_hms_opt(9, 5, 0)
        .unwrap();
    assert_eq!(format_date_at(today, now), "Today, 17:19");
    assert_eq!(format_date_at(old, now), "28 Oct 2026");
    assert_eq!(format_date_in(today, now, Region::NL_NL), "Today, 17:19");
    assert_eq!(format_date_in(old, now, Region::NL_NL), "28 Oct 2026");
    assert_eq!(format_date_in(today, now, Region::EN_US), "Today, 5:19 PM");
    assert_eq!(format_date_in(old, now, Region::EN_US), "Oct 28, 2026");
    region::pin(Region::EN_US, || assert_eq!(format_date_at(old, now), "Oct 28, 2026"));

    // What Windows stores, read into a region.
    let nl = Region::from_values(Some(","), Some("d-M-yyyy"), Some("HH:mm"));
    assert_eq!(nl, Region::NL_NL);
    let us = Region::from_values(Some("."), Some("M/d/yyyy"), Some("h:mm tt"));
    assert_eq!(us, Region::EN_US);
    let ja = Region::from_values(Some("."), Some("yyyy/MM/dd"), Some("H:mm"));
    assert_eq!(ja.date_order, crate::ui::region::DateOrder::Ymd);
    assert_eq!(Region::from_values(None, None, None), Region::EN_GB);
}

/// RC2-10 (w150): on the large account the status line read "483 online only" while the first list was still
/// arriving: it counted the rows listed so far as if they were the folder. It says what it counts.
#[test]
fn file_list_rc2_pc_summary_says_it_counts_a_partly_loaded_listing() {
    use crate::frames::files::window::pc_summary;
    use crate::frames::global::source::board;
    use crate::frames::global::{GlobalFixture, Ui};

    let mut s = crate::init();
    let whole = pc_summary(&s);
    assert!(!whole.starts_with("Listed so far"), "{whole}");
    s.global = Ui::new(Box::new(GlobalFixture::of(board::loading())));
    s.actions.first_load_strip = true;
    s.listing.items.truncate(7);
    let listed = s.listing.items.len();
    let partial = pc_summary(&s);
    let counts = partial
        .strip_prefix("Listed so far: ")
        .unwrap_or_else(|| panic!("{partial}"));
    let sum: usize = counts
        .split(" \u{00B7} ")
        .map(|part| {
            part.split(' ')
                .next()
                .unwrap()
                .replace(',', "")
                .parse::<usize>()
                .unwrap()
        })
        .sum();
    assert!(sum <= listed, "{partial}: {sum} of {listed} rows");
}
