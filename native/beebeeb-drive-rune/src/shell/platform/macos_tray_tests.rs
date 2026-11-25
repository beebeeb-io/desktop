use super::*;
use crate::shell::tray::{tray_model, TrayInputs};

fn r(x: f64, y: f64, w: f64, h: f64) -> NsRect {
    NsRect { x, y, w, h }
}

#[test]
fn anchor_flips_y_on_the_primary_display() {
    // Primary 1440x900; the item sits in the 24 pt menu bar at the top right.
    let a = anchor_from_frames(
        r(1300.0, 876.0, 30.0, 24.0),
        r(0.0, 0.0, 1440.0, 900.0),
        r(0.0, 0.0, 1440.0, 876.0),
        900.0,
        2.0,
    );
    assert_eq!(
        a.icon,
        Some(Rect {
            x: 1300,
            y: 0,
            w: 30,
            h: 24
        })
    );
    assert_eq!(
        a.monitor,
        Rect {
            x: 0,
            y: 0,
            w: 1440,
            h: 900
        }
    );
    assert_eq!(
        a.work,
        Rect {
            x: 0,
            y: 24,
            w: 1440,
            h: 876
        }
    );
    assert_eq!(a.scale, 2.0);
}

#[test]
fn anchor_on_a_display_above_and_one_beside_the_primary() {
    // A 1920x1080 display stacked above the primary: AppKit y 900..1980, so top-left y is -1080.
    let above = anchor_from_frames(
        r(500.0, 1956.0, 30.0, 24.0),
        r(0.0, 900.0, 1920.0, 1080.0),
        r(0.0, 900.0, 1920.0, 1056.0),
        900.0,
        1.0,
    );
    assert_eq!(
        above.monitor,
        Rect {
            x: 0,
            y: -1080,
            w: 1920,
            h: 1080
        }
    );
    assert_eq!(
        above.work,
        Rect {
            x: 0,
            y: -1080 + 24,
            w: 1920,
            h: 1056
        }
    );
    assert_eq!(
        above.icon,
        Some(Rect {
            x: 500,
            y: -1080,
            w: 30,
            h: 24
        })
    );
    // A display to the left, same height: x negative, y as on the primary.
    let left = anchor_from_frames(
        r(-400.0, 876.0, 30.0, 24.0),
        r(-1440.0, 0.0, 1440.0, 900.0),
        r(-1440.0, 0.0, 1440.0, 876.0),
        900.0,
        2.0,
    );
    assert_eq!(
        left.icon,
        Some(Rect {
            x: -400,
            y: 0,
            w: 30,
            h: 24
        })
    );
    assert_eq!(left.monitor.x, -1440);
}

#[test]
fn appearance_names_pick_the_theme() {
    assert_eq!(theme_from_appearance("NSAppearanceNameDarkAqua"), TaskbarTheme::Dark);
    assert_eq!(theme_from_appearance("NSAppearanceNameVibrantDark"), TaskbarTheme::Dark);
    assert_eq!(theme_from_appearance("NSAppearanceNameAqua"), TaskbarTheme::Light);
}

#[test]
fn model_gives_tooltip_and_menu_items() {
    let model = tray_model(&TrayInputs::default());
    assert!(model.tooltip.starts_with("Beebeeb"));
    let spec = menu_spec(&model);
    assert_eq!(spec.len(), model.menu.len());
    for (line, entry) in spec.iter().zip(&model.menu) {
        match (line, entry) {
            (MenuSpec::Separator, MenuEntry::Separator) => {}
            (
                MenuSpec::Item { action, title, enabled },
                MenuEntry::Item {
                    action: a,
                    label,
                    enabled: e,
                },
            ) => assert_eq!((action, title, enabled), (a, label, e)),
            _ => panic!("line and entry differ"),
        }
    }
    assert!(spec.iter().any(|l| matches!(
        l,
        MenuSpec::Item {
            action: MenuAction::Quit,
            ..
        }
    )));
}

#[test]
fn right_and_ctrl_clicks_open_the_menu() {
    assert!(is_menu_click(4, 0));
    assert!(is_menu_click(2, 1 << 18));
    assert!(!is_menu_click(2, 0));
}
