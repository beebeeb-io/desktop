//! Tests of the first window's size and place from the screen's work area (`shell::work_area`): a small screen gets a
//! window that fits with its frame, and a roomy one keeps the size.

use super::*;
use crate::shell::window_state::{self, SavedWindow};

const MIN: (f32, f32) = (960.0, 640.0);
const MAIN: (f32, f32) = (1280.0, 800.0);

fn display(x: i32, y: i32, w: u32, h: u32, scale: f64, primary: bool) -> Monitor {
    Monitor {
        x,
        y,
        w,
        h,
        scale,
        primary,
    }
}
fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect { x, y, w, h }
}

#[test]
fn a_1280_by_800_screen_gets_a_window_that_fits_with_its_frame() {
    // 1280x800, taskbar 48 px at the bottom: work area 1280x752.
    let d = [display(0, 0, 1280, 800, 1.0, true)];
    let u = usable(&d, &[(rect(0, 0, 1280, 800), rect(0, 0, 1280, 752))]);
    let r = first_place(MAIN, MIN, &u);
    let (w, h) = r.physical_size.unwrap();
    let (x, y) = r.position.unwrap();
    // The outer window (inner + frame) stays inside the work area.
    assert!(w as i32 + 2 <= 1280 && h as i32 + 1 <= 752, "{w}x{h}");
    assert!(
        x >= 0 && y >= 0 && x + w as i32 + 2 <= 1280 && y + h as i32 + 1 <= 752,
        "{x},{y} {w}x{h}"
    );
    // 800 high does not fit 752: the height gives, the width (1278) is the room.
    assert_eq!((w, h), (1278, 751));
}

#[test]
fn the_size_is_kept_when_the_screen_has_room() {
    let d = [display(0, 0, 1920, 1080, 1.0, true)];
    let u = usable(&d, &[(rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1032))]);
    let r = first_place(MAIN, MIN, &u);
    assert_eq!(r.physical_size, Some((1280, 800)));
    assert_eq!(r.position, Some((319, 115)));
}

#[test]
fn a_150_percent_screen_scales_the_size_then_clamps_it() {
    // 1920x1080 at 150% is 1280x720 points: the window cannot be 1280x800 points.
    let d = [display(0, 0, 1920, 1080, 1.5, true)];
    let u = usable(&d, &[(rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1032))]);
    let r = first_place(MAIN, MIN, &u);
    // width 1920 px wanted, room 1920 - 3; height 1200 px wanted, room 1032 - 2.
    assert_eq!(r.physical_size, Some((1917, 1030)));
    // At 150% with plenty of room (2880x1620) it is the scaled size.
    let d = [display(0, 0, 2880, 1620, 1.5, true)];
    let r = first_place(MAIN, MIN, &usable(&d, &[]));
    assert_eq!(r.physical_size, Some((1920, 1200)));
}

#[test]
fn a_taskbar_on_the_left_or_top_moves_the_work_area_origin() {
    let d = [display(0, 0, 1920, 1080, 1.0, true)];
    let left = usable(&d, &[(rect(0, 0, 1920, 1080), rect(60, 0, 1860, 1080))]);
    let r = first_place(MAIN, MIN, &left);
    assert_eq!(r.position, Some((60 + (1858 - 1280) / 2, (1079 - 800) / 2)));
    assert_eq!((left[0].x, left[0].y), (60, 0));
    let top = usable(&d, &[(rect(0, 0, 1920, 1080), rect(0, 48, 1920, 1032))]);
    let r = first_place(MAIN, MIN, &top);
    let (x, y) = r.position.unwrap();
    assert_eq!(x, 319);
    assert!(y >= 48, "{y}");
}

#[test]
fn the_minimum_is_kept_where_the_screen_allows_it() {
    // 1024x768 work area: min 960x640 fits, a want of 700x500 points rises to the minimum.
    let d = [display(0, 0, 1024, 768, 1.0, true)];
    let r = first_place((700.0, 500.0), MIN, &usable(&d, &[]));
    assert_eq!(r.physical_size, Some((960, 640)));
    // A screen smaller than the minimum: the screen wins.
    let d = [display(0, 0, 800, 600, 1.0, true)];
    let r = first_place(MAIN, MIN, &usable(&d, &[]));
    assert_eq!(r.physical_size, Some((798, 599)));
}

#[test]
fn a_saved_size_larger_than_the_screen_is_cut_to_the_work_area() {
    let d = [display(0, 0, 1280, 800, 1.0, true)];
    let u = usable(&d, &[(rect(0, 0, 1280, 800), rect(0, 0, 1280, 752))]);
    let saved = SavedWindow {
        x: 0,
        y: 0,
        w: 2560,
        h: 1600,
        scale: 1.0,
        maximized: false,
    };
    let r = window_state::restore(Some(&saved), &u, MIN);
    assert_eq!(r.physical_size, Some((1278, 751)));
    let (x, y) = r.position.unwrap();
    assert!(x >= 0 && y >= 0);
}

#[test]
fn a_saved_position_on_a_monitor_that_is_gone_comes_back_on_the_primary() {
    let d = [display(0, 0, 1920, 1080, 1.0, true)];
    let u = usable(&d, &[(rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1032))]);
    // Saved on a second display to the right (x 1920..3840) that is not there now.
    let saved = SavedWindow {
        x: 2400,
        y: 200,
        w: 1280,
        h: 800,
        scale: 1.0,
        maximized: false,
    };
    let r = window_state::restore(Some(&saved), &u, MIN);
    assert_eq!(r.physical_size, Some((1280, 800)));
    assert_eq!(r.position, Some((319, 115)));
}

#[test]
fn a_saved_place_under_the_taskbar_is_not_grabbable_and_comes_back() {
    let d = [display(0, 0, 1920, 1080, 1.0, true)];
    let u = usable(&d, &[(rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1032))]);
    // Top edge 20 px above the whole display's bottom: on the display, but under the taskbar.
    let saved = SavedWindow {
        x: 100,
        y: 1060,
        w: 1000,
        h: 700,
        scale: 1.0,
        maximized: false,
    };
    let r = window_state::restore(Some(&saved), &u, MIN);
    let (_, y) = r.position.unwrap();
    assert!(y + 48 <= 1032, "{y}");
}

#[test]
fn a_display_without_a_work_area_keeps_its_whole_rectangle_less_the_frame() {
    let d = [
        display(0, 0, 1920, 1080, 1.0, true),
        display(1920, 0, 1280, 1024, 1.0, false),
    ];
    let u = usable(&d, &[(rect(1920, 0, 1280, 1024), rect(1920, 0, 1280, 976))]);
    assert_eq!((u[0].w, u[0].h), (1918, 1079));
    assert_eq!((u[1].x, u[1].w, u[1].h), (1920, 1278, 975));
}

#[test]
fn no_display_leaves_the_choice_to_the_os() {
    assert_eq!(first_place(MAIN, MIN, &[]), Restored::default());
}

#[test]
fn growing_from_first_run_to_main_stays_inside_a_752_high_work_area() {
    let work = [(rect(0, 0, 1280, 800), rect(0, 0, 1280, 752))];
    // The first-run window (960x640) sits centred at y=56; the main size is 1280x800.
    let ((w, h), (x, y)) = fit_on_change(MAIN, 1.0, (160, 56), &work).unwrap();
    assert_eq!((w, h), (1278.0, 751.0));
    assert!(
        x >= 0 && y >= 0 && x + w as i32 + 2 <= 1280 && y + h as i32 + 1 <= 752,
        "{x},{y} {w}x{h}"
    );
}

#[test]
fn a_main_size_that_fits_keeps_its_place_and_scale_is_honoured() {
    let work = [(rect(0, 0, 3840, 2160), rect(0, 0, 3840, 2112))];
    let ((w, h), at) = fit_on_change(MAIN, 1.5, (400, 300), &work).unwrap();
    assert_eq!(((w, h), at), ((1280.0, 800.0), (400, 300)));
    assert_eq!(fit_on_change(MAIN, 1.0, (0, 0), &[]), None);
}

#[test]
fn the_display_the_window_is_on_decides() {
    let work = [
        (rect(0, 0, 1280, 800), rect(0, 0, 1280, 752)),
        (rect(1280, 0, 1920, 1080), rect(1280, 0, 1920, 1040)),
    ];
    let ((_, h), (x, _)) = fit_on_change(MAIN, 1.0, (1500, 100), &work).unwrap();
    assert_eq!((h, x), (800.0, 1500));
}
