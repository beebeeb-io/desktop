use super::*;

fn disp(x: f64, y: f64, w: f64, h: f64, bar: f64, primary: bool) -> Display {
    Display {
        frame: Frame::new(x, y, w, h),
        visible: Frame::new(x, y + bar, w, h - bar),
        primary,
    }
}
fn main_display() -> Display {
    disp(0.0, 0.0, 1440.0, 900.0, 25.0, true)
}
fn icon_at(cx: f64) -> Option<Frame> {
    Some(Frame::new(cx - 11.0, 0.0, 22.0, 24.0))
}
fn pop(icon: Option<Frame>, d: &[Display]) -> Placement {
    place_popover(icon, d).expect("placement")
}

#[test]
fn centres_under_the_icon_with_the_gap() {
    let p = pop(icon_at(700.0), &[main_display()]);
    assert_eq!((p.x, p.y, p.w, p.h), (700.0 - 186.0, 24.0 + 6.0, 372.0, 488.0));
}
#[test]
fn clamps_on_the_left_with_the_margin() {
    assert_eq!(pop(icon_at(20.0), &[main_display()]).x, 8.0);
}
#[test]
fn clamps_on_the_right_with_the_margin() {
    assert_eq!(pop(icon_at(1435.0), &[main_display()]).x, 1440.0 - 8.0 - 372.0);
}
#[test]
fn clamps_at_the_bottom_with_the_margin() {
    let d = disp(0.0, 0.0, 1440.0, 560.0, 25.0, true);
    let p = pop(Some(Frame::new(689.0, 100.0, 22.0, 24.0)), &[d]);
    assert_eq!(p.y + p.h, 560.0 - 8.0);
}
#[test]
fn clamps_at_the_top_to_the_visible_frame_under_the_notch() {
    // An icon whose bottom is above the visible top (a notch bar of 38): the top is the visible frame itself.
    let d = disp(0.0, 0.0, 1512.0, 982.0, 38.0, true);
    let p = pop(Some(Frame::new(700.0, 0.0, 22.0, 20.0)), &[d]);
    assert_eq!(p.y, 38.0);
}
#[test]
fn height_is_capped_at_the_visible_height_minus_two_margins() {
    let d = disp(0.0, 0.0, 1440.0, 400.0, 25.0, true);
    assert_eq!(pop(icon_at(700.0), &[d]).h, 375.0 - 16.0);
}
#[test]
fn a_screen_narrower_than_the_popover_keeps_the_top_left_on_screen() {
    let d = disp(0.0, 0.0, 300.0, 900.0, 25.0, true);
    let p = pop(icon_at(150.0), &[d]);
    assert_eq!((p.x, p.w), (8.0, 284.0));
}
#[test]
fn icon_is_matched_against_the_full_frame_not_the_visible_frame() {
    // The icon sits in the menu bar: inside the full frame, above the visible frame.
    let d = main_display();
    assert!(icon_display(icon_at(700.0).unwrap(), &[d]).is_some());
    assert!(!d.visible.contains(700.0, 12.0));
}
#[test]
fn ambiguous_two_display_case_picks_the_external_display() {
    let main = main_display();
    let ext = disp(1440.0, -200.0, 2560.0, 1440.0, 25.0, false);
    // Centre on the external display; the naive "icon x within the primary's width" is wrong.
    let icon = Frame::new(1500.0, -200.0, 22.0, 24.0);
    let p = pop(Some(icon), &[main, ext]);
    assert!(p.x >= 1440.0 && p.y == -200.0 + 24.0 + 6.0);
}
#[test]
fn a_centre_exactly_on_the_shared_edge_belongs_to_the_right_display() {
    let main = main_display();
    let ext = disp(1440.0, 0.0, 1920.0, 1080.0, 25.0, false);
    let icon = Frame::new(1429.0, 0.0, 22.0, 24.0); // centre 1440
    assert_eq!(icon_display(icon, &[main, ext]).unwrap().primary, false);
}
#[test]
fn an_icon_straddling_two_displays_goes_with_its_centre() {
    let main = main_display();
    let ext = disp(1440.0, 0.0, 1920.0, 1080.0, 25.0, false);
    let icon = Frame::new(1425.0, 0.0, 22.0, 24.0); // centre 1436
    assert!(icon_display(icon, &[main, ext]).unwrap().primary);
}
#[test]
fn a_display_left_of_the_primary_has_a_negative_origin() {
    let left = disp(-1920.0, 0.0, 1920.0, 1080.0, 25.0, false);
    let p = pop(icon_at(-960.0), &[main_display(), left]);
    assert_eq!(p.x, -960.0 - 186.0);
    assert!(p.x < 0.0);
}
#[test]
fn a_centre_off_every_display_uses_the_largest_overlap() {
    let main = main_display();
    let ext = disp(1440.0, 0.0, 1920.0, 1080.0, 25.0, false);
    // Centre below both frames (y 910 > 900), but the icon overlaps the main display most.
    let icon = Frame::new(700.0, 880.0, 22.0, 60.0);
    assert!(icon_display(icon, &[main, ext]).unwrap().primary);
}
#[test]
fn no_icon_falls_back_to_the_top_right_of_the_primary_display() {
    let ext = disp(1440.0, 0.0, 1920.0, 1080.0, 25.0, false);
    let p = pop(None, &[ext, main_display()]);
    assert_eq!((p.x, p.y), (1440.0 - 8.0 - 372.0, 25.0));
}
#[test]
fn an_icon_on_no_display_falls_back_to_the_primary_not_the_centre() {
    let p = pop(Some(Frame::new(9000.0, 9000.0, 22.0, 24.0)), &[main_display()]);
    assert_eq!((p.x, p.y), (1440.0 - 8.0 - 372.0, 25.0));
}
#[test]
fn with_no_primary_flag_the_first_display_is_the_fallback() {
    let a = disp(0.0, 0.0, 1000.0, 800.0, 25.0, false);
    let b = disp(1000.0, 0.0, 2000.0, 800.0, 25.0, false);
    let p = pop(None, &[a, b]);
    assert_eq!(p.x, 1000.0 - 8.0 - 372.0);
}
#[test]
fn no_displays_means_no_placement() {
    assert_eq!(place_popover(icon_at(100.0), &[]), None);
    assert_eq!(place_popover(None, &[]), None);
}
#[test]
fn settings_opens_top_right_under_the_menu_bar_on_the_icons_display() {
    let ext = disp(1440.0, 0.0, 1920.0, 1080.0, 25.0, false);
    let s = place_settings(Some(Frame::new(1500.0, 0.0, 22.0, 24.0)), &[main_display(), ext], 540.0).unwrap();
    assert_eq!((s.x, s.y, s.w, s.h), (3360.0 - 8.0 - 560.0, 25.0, 560.0, 540.0));
}
#[test]
fn settings_top_edge_lines_up_with_the_popover() {
    let d = [main_display()];
    let s = place_settings(icon_at(1435.0), &d, 540.0).unwrap();
    let p = pop(icon_at(1435.0), &d);
    assert_eq!(s.x + s.w, p.x + p.w);
    assert_eq!(s.y, 25.0);
}
#[test]
fn settings_without_an_icon_display_falls_back_to_the_primary_top_right() {
    let ext = disp(1440.0, 0.0, 1920.0, 1080.0, 25.0, false);
    let s = place_settings(None, &[ext, main_display()], 540.0).unwrap();
    assert_eq!((s.x, s.y), (1440.0 - 8.0 - 560.0, 25.0));
}
#[test]
fn settings_height_is_capped_to_the_visible_frame() {
    let d = disp(0.0, 0.0, 1440.0, 400.0, 25.0, true);
    assert_eq!(place_settings(None, &[d], 900.0).unwrap().h, 375.0 - 16.0);
}
#[test]
fn sweep_every_placement_stays_inside_the_margins_and_never_centres() {
    let main = main_display();
    let ext = disp(1440.0, -100.0, 2560.0, 1440.0, 25.0, false);
    let ds = [main, ext];
    let mut cx = -200.0;
    while cx < 4200.0 {
        let p = pop(icon_at(cx), &ds);
        let known = icon_display(icon_at(cx).unwrap(), &ds)
            .map(|d| d.visible)
            .unwrap_or(main.visible);
        assert!(
            p.x >= known.x + 8.0 - 1e-9 && p.x + p.w <= known.right() + 1e-9 - 8.0,
            "cx {cx}"
        );
        assert!(p.y >= known.y && p.y + p.h <= known.bottom() - 8.0 + 1e-9, "cx {cx}");
        if icon_display(icon_at(cx).unwrap(), &ds).is_none() {
            assert_eq!(
                p.x,
                main.visible.right() - 8.0 - 372.0,
                "off-screen icon is never centred"
            );
        }
        cx += 37.0;
    }
}

#[test]
fn a_click_right_after_a_blur_hide_does_not_reopen() {
    let mut g = BlurGuard::new();
    g.blurred(1000);
    assert_eq!(g.click(false, 1040), Toggle::Ignore);
}
#[test]
fn the_window_is_250_ms_exclusive() {
    let mut g = BlurGuard::new();
    g.blurred(1000);
    assert_eq!(g.click(false, 1249), Toggle::Ignore);
    g.blurred(1000);
    assert_eq!(g.click(false, 1250), Toggle::Show);
}
#[test]
fn with_no_blur_a_click_opens() {
    assert_eq!(BlurGuard::new().click(false, 5), Toggle::Show);
}
#[test]
fn one_blur_suppresses_one_click() {
    let mut g = BlurGuard::new();
    g.blurred(1000);
    assert_eq!(g.click(false, 1010), Toggle::Ignore);
    assert_eq!(g.click(false, 1020), Toggle::Show);
}
#[test]
fn a_click_on_a_visible_popover_hides_it_and_clears_a_stale_blur() {
    let mut g = BlurGuard::new();
    g.blurred(1000);
    assert_eq!(g.click(true, 1010), Toggle::Hide);
    assert_eq!(g.click(false, 1020), Toggle::Show);
}
#[test]
fn a_clock_that_steps_backwards_counts_as_the_same_gesture() {
    let mut g = BlurGuard::new();
    g.blurred(5000);
    assert_eq!(g.click(false, 4000), Toggle::Ignore);
}
