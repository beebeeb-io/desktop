use super::*;

const MIN: (f32, f32) = (960.0, 640.0);

fn primary() -> Monitor {
    Monitor {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
        scale: 1.0,
        primary: true,
    }
}
fn right_4k() -> Monitor {
    Monitor {
        x: 1920,
        y: 0,
        w: 3840,
        h: 2160,
        scale: 2.0,
        primary: false,
    }
}
fn saved(x: i32, y: i32, w: u32, h: u32, scale: f64) -> SavedWindow {
    SavedWindow {
        x,
        y,
        w,
        h,
        scale,
        maximized: false,
    }
}

#[test]
fn nothing_saved_leaves_the_defaults() {
    assert_eq!(restore(None, &[primary()], MIN), Restored::default());
}

#[test]
fn a_window_on_a_display_that_is_still_there_goes_back_exactly() {
    let s = saved(300, 120, 1280, 800, 1.0);
    let r = restore(Some(&s), &[primary(), right_4k()], MIN);
    assert_eq!(
        r,
        Restored {
            position: Some((300, 120)),
            physical_size: Some((1280, 800)),
            maximized: false
        }
    );
    let s = saved(2400, 200, 2560, 1600, 2.0);
    let r = restore(Some(&s), &[primary(), right_4k()], MIN);
    assert_eq!(r.position, Some((2400, 200)));
    assert_eq!(r.physical_size, Some((2560, 1600)));
}

#[test]
fn a_window_on_a_display_that_is_gone_is_centred_on_the_primary() {
    // Saved on the 4K display at 2x; only the primary (1x) is left.
    let s = saved(2400, 200, 2560, 1600, 2.0);
    let r = restore(Some(&s), &[primary()], MIN);
    // 1280x800 points at 1x, centred in 1920x1080.
    assert_eq!(r.physical_size, Some((1280, 800)));
    assert_eq!(r.position, Some((320, 140)));
}

#[test]
fn a_display_to_the_left_that_is_gone_also_falls_back() {
    let s = saved(-1700, 100, 1280, 800, 1.0);
    let r = restore(Some(&s), &[primary()], MIN);
    assert_eq!(r.position, Some((320, 140)));
}

#[test]
fn the_primary_is_chosen_by_its_flag_not_its_order() {
    let left = Monitor {
        x: -1920,
        y: 0,
        w: 1920,
        h: 1080,
        scale: 1.0,
        primary: false,
    };
    let s = saved(9000, 9000, 1280, 800, 1.0);
    let r = restore(Some(&s), &[left, primary()], MIN);
    assert_eq!(r.position, Some((320, 140)));
    // No display says it is primary: the first usable one.
    let r = restore(Some(&s), &[left], MIN);
    assert_eq!(r.position, Some((-1920 + 320, 140)));
}

#[test]
fn a_scale_change_on_the_same_display_keeps_the_size_in_points() {
    // Saved at 100%; the display is now at 150%.
    let s = saved(100, 100, 1280, 800, 1.0);
    let now = Monitor {
        x: 0,
        y: 0,
        w: 2880,
        h: 1620,
        scale: 1.5,
        primary: true,
    };
    let r = restore(Some(&s), &[now], MIN);
    assert_eq!(r.physical_size, Some((1920, 1200)));
    assert_eq!(r.position, Some((100, 100)));
}

#[test]
fn a_window_larger_than_its_display_is_cut_to_it() {
    let s = saved(0, 0, 3000, 2000, 1.0);
    let r = restore(Some(&s), &[primary()], MIN);
    assert_eq!(r.physical_size, Some((1920, 1080)));
}

#[test]
fn a_saved_size_under_the_minimum_grows_to_it() {
    let s = saved(10, 10, 400, 300, 1.0);
    let r = restore(Some(&s), &[primary()], MIN);
    assert_eq!(r.physical_size, Some((960, 640)));
}

#[test]
fn a_window_mostly_off_the_edge_is_out_of_reach_and_recentred() {
    // Only 50 px of the top strip is on the display: not enough to grab.
    let s = saved(1870, 100, 1280, 800, 1.0);
    let r = restore(Some(&s), &[primary()], MIN);
    assert_eq!(r.position, Some((320, 140)));
    // Above the top edge: the title strip is off screen.
    let s = saved(100, -60, 1280, 800, 1.0);
    assert_eq!(restore(Some(&s), &[primary()], MIN).position, Some((320, 140)));
    // 200 px visible at the right: still reachable, left where it was.
    let s = saved(1720, 100, 1280, 800, 1.0);
    assert_eq!(restore(Some(&s), &[primary()], MIN).position, Some((1720, 100)));
}

#[test]
fn a_window_spanning_two_displays_belongs_to_the_one_holding_most_of_it() {
    let s = saved(1500, 100, 1280, 800, 1.0);
    let r = restore(Some(&s), &[primary(), right_4k()], MIN);
    // 860 px are on the 4K display, 420 on the primary: laid out for 2x there.
    assert_eq!(r.physical_size, Some((2560, 1600)));
    assert_eq!(r.position, Some((1500, 100)));
}

#[test]
fn maximized_is_carried_and_no_displays_means_defaults() {
    let s = SavedWindow {
        maximized: true,
        ..saved(100, 100, 1280, 800, 1.0)
    };
    assert!(restore(Some(&s), &[primary()], MIN).maximized);
    assert_eq!(
        restore(Some(&s), &[], MIN),
        Restored {
            position: None,
            physical_size: None,
            maximized: true
        }
    );
}

#[test]
fn nonsense_is_ignored() {
    for s in [
        saved(0, 0, 0, 800, 1.0),
        saved(0, 0, 1280, 800, 0.0),
        saved(0, 0, 1280, 800, f64::NAN),
        saved(0, 0, 1280, 800, 99.0),
    ] {
        assert_eq!(restore(Some(&s), &[primary()], MIN), Restored::default());
    }
    let broken = Monitor {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
        scale: 1.0,
        primary: true,
    };
    let s = saved(100, 100, 1280, 800, 1.0);
    assert_eq!(restore(Some(&s), &[broken], MIN), Restored::default());
}

fn observed(x: i32, y: i32, w: u32, h: u32) -> Observed {
    Observed {
        x,
        y,
        w,
        h,
        scale: 1.0,
        maximized: false,
        minimized: false,
        full_screen: false,
    }
}

#[test]
fn saving_keeps_the_normal_rectangle_while_maximized_or_minimized() {
    let before = saved(300, 120, 1280, 800, 1.0);
    let max = Observed {
        maximized: true,
        ..observed(-8, -8, 1936, 1056)
    };
    assert_eq!(
        merge_for_save(Some(before), max),
        Some(SavedWindow {
            maximized: true,
            ..before
        })
    );
    let min = Observed {
        minimized: true,
        ..observed(-32000, -32000, 160, 28)
    };
    assert_eq!(merge_for_save(Some(before), min), Some(before));
    assert_eq!(merge_for_save(None, min), None);
    // First ever close, maximized: the only rectangle there is.
    assert_eq!(merge_for_save(None, max).map(|s| s.maximized), Some(true));
    let moved = observed(40, 50, 1000, 700);
    assert_eq!(
        merge_for_save(
            Some(SavedWindow {
                maximized: true,
                ..before
            }),
            moved
        ),
        Some(saved(40, 50, 1000, 700, 1.0))
    );
}

#[test]
fn the_file_round_trips_and_survives_damage() {
    let mut windows = SavedWindows::new();
    windows.insert(
        "main".into(),
        SavedWindow {
            maximized: true,
            ..saved(-1200, 40, 1280, 800, 1.25)
        },
    );
    let text = to_json(&windows);
    assert_eq!(from_json(&text), windows);
    assert!(from_json("").is_empty());
    assert!(from_json("{\"version\":2,\"windows\":{}}").is_empty());
    assert!(from_json("[1,2,3]").is_empty());
    // One bad entry does not take the good one with it.
    let mixed = r#"{"version":1,"windows":{"main":{"x":1,"y":2,"w":900,"h":700,"scale":1.0},"x":{"x":"no"},"y":{"x":1,"y":1,"w":0,"h":0,"scale":1.0}}}"#;
    let read = from_json(mixed);
    assert_eq!(read.len(), 1);
    assert_eq!(read["main"], saved(1, 2, 900, 700, 1.0));
}

#[test]
fn load_and_save_use_one_file_and_refuse_an_oversized_one() {
    let dir = std::env::temp_dir().join(format!("bb-shell-ws-{}", std::process::id()));
    let path = dir.join(FILE_NAME);
    assert!(load(&path).is_empty());
    let mut windows = SavedWindows::new();
    windows.insert("main".into(), saved(10, 20, 1280, 800, 1.0));
    save(&path, &windows).unwrap();
    assert_eq!(load(&path), windows);
    assert!(!path.with_extension("json.tmp").exists());
    std::fs::write(&path, vec![b' '; 70 * 1024]).unwrap();
    assert!(load(&path).is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Review test 10: the first save, with nothing saved before, of a maximized window.
#[test]
fn a_first_save_while_maximized_keeps_the_maximized_flag_and_a_sane_rectangle() {
    let now = Observed {
        maximized: true,
        ..observed(0, 0, 2560, 1400)
    };
    let saved = merge_for_save(None, now).unwrap();
    assert!(saved.maximized);
    assert_eq!((saved.w, saved.h), (2560, 1400));
    // And the next normal rectangle replaces it.
    let later = merge_for_save(Some(saved), observed(100, 100, 1280, 800)).unwrap();
    assert_eq!(
        (later.x, later.y, later.w, later.h, later.maximized),
        (100, 100, 1280, 800, false)
    );
    // Minimized with nothing saved: still nothing.
    let minimized = Observed {
        minimized: true,
        ..observed(-32000, -32000, 160, 28)
    };
    assert_eq!(merge_for_save(None, minimized), None);
}

#[test]
fn the_old_apps_frame_is_read_in_points_and_becomes_pixels_on_its_display() {
    let monitors = [primary(), right_4k()];
    let text = r#"{"x":100.0,"y":80.0,"w":1280.0,"h":800.0,"maximized":false}"#;
    let on_primary = import_legacy_frame(text, &monitors).unwrap();
    let p = primary();
    assert_eq!(on_primary.scale, p.scale);
    assert_eq!(on_primary.x, (100.0 * p.scale).round() as i32);
    assert_eq!(on_primary.w, (1280.0 * p.scale).round() as u32);
    assert!(!on_primary.maximized);
    // A frame whose centre is on the second display takes that display's scale.
    let r = right_4k();
    // (In this format each display is its pixels divided by its own scale, so the second
    // display's points overlap the primary's; a point past the primary's width is only on it.)
    let (lx, ly) = (2000.0_f64, 50.0_f64);
    assert!(lx + 500.0 < f64::from(r.x + r.w as i32) / r.scale);
    let text = format!(r#"{{"x":{lx},"y":{ly},"w":1000.0,"h":700.0,"maximized":true}}"#);
    let on_right = import_legacy_frame(&text, &monitors).unwrap();
    assert_eq!(on_right.scale, r.scale);
    assert_eq!(on_right.x, (lx * r.scale).round() as i32);
    assert!(on_right.maximized);
    // Off every display: the primary's scale; `restore` recentres it.
    let lost = import_legacy_frame(r#"{"x":-90000.0,"y":5.0,"w":1280.0,"h":800.0}"#, &monitors).unwrap();
    assert_eq!(lost.scale, p.scale);
    for bad in [
        "",
        "not json",
        "{}",
        "[]",
        r#"{"x":1,"y":2,"w":0,"h":5}"#,
        r#"{"x":1,"y":2,"w":5}"#,
        r#"{"x":"1","y":2,"w":5,"h":5}"#,
        r#"{"x":1e300,"y":2,"w":5,"h":5}"#,
    ] {
        assert_eq!(import_legacy_frame(bad, &monitors), None, "{bad}");
    }
    assert!(import_legacy_frame(&" ".repeat(70_000), &monitors).is_none());
}

#[test]
fn the_old_frame_file_is_read_only_while_this_app_has_no_file_and_is_never_written() {
    let dir = std::env::temp_dir().join(format!("beebeeb-shell-legacy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (own, legacy) = (dir.join(FILE_NAME), dir.join(LEGACY_FRAME_FILE_NAME));
    assert_eq!(read_legacy_once(&own, &legacy), None, "no old file");
    let text = r#"{"x":10.0,"y":20.0,"w":1280.0,"h":800.0,"maximized":false}"#;
    std::fs::write(&legacy, text).unwrap();
    assert_eq!(read_legacy_once(&own, &legacy).as_deref(), Some(text));
    // Once this app has saved its own file, the old one is never read again.
    save(&own, &SavedWindows::new()).unwrap();
    assert_eq!(read_legacy_once(&own, &legacy), None);
    assert_eq!(
        std::fs::read_to_string(&legacy).unwrap(),
        text,
        "the old file is untouched"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
