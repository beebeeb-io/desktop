//! The one file tile, rendered in a runtime: each size's box, its keys, its role and label, and
//! what selection, dimming and a badge do.

use rune::{Cx, El, ElementInfo, Env, Runtime, row};

use super::*;
use crate::theme::file_icons::icon_for;
use crate::theme::tokens::theme;

struct Page {
    size: TileSize,
    look: RowLook,
    badge: bool,
}

fn view(p: &Page, _: &mut Cx<Page>) -> El {
    let t = theme(false);
    let badge = p.badge.then(|| el().key("tl-badge").size(12.0, 12.0));
    row().key("page").px(20.0).pt(20.0).child(tile(
        "tl",
        "Report.pdf, 2 MB",
        TilePicture::Icon(icon_for("Report.pdf", false, None)),
        "Report.pdf",
        badge,
        p.look,
        p.size,
        &t,
    ))
}

fn render(size: TileSize, look: RowLook, badge: bool) -> Runtime<Page> {
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(Page { size, look, badge }, view, (400.0, 300.0), env);
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
    rt
}

fn find(rt: &Runtime<Page>, key: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn info(rt: &Runtime<Page>, key: &str) -> ElementInfo {
    find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}"))
}

#[test]
fn each_size_has_its_box_keys_role_and_label() {
    for (size, w, h, role, name_drawn) in [
        (TileSize::Small, 28.0, 28.0, "image", false),
        (TileSize::Medium, grid_tiles::CELL_W, grid_tiles::CELL_H, "option", true),
        (TileSize::Large, 40.0, 40.0, "image", false),
    ] {
        let rt = render(size, RowLook::default(), false);
        let tl = info(&rt, "tl");
        assert_eq!([tl.rect[2], tl.rect[3]], [w, h], "{size:?}: box");
        assert_eq!(tl.role, role, "{size:?}: role");
        assert_eq!(tl.label.as_deref(), Some("Report.pdf, 2 MB"), "{size:?}: label");
        info(&rt, "tl-icon");
        assert_eq!(find(&rt, "tl-name").is_some(), name_drawn, "{size:?}: name");
        assert_eq!(find(&rt, "tl-thumb").is_some(), name_drawn, "{size:?}: picture box");
    }
    let rt = render(TileSize::Medium, RowLook::default(), false);
    assert_eq!(info(&rt, "tl-name").text.as_deref(), Some("Report.pdf"));
    let thumb = info(&rt, "tl-thumb").rect;
    assert_eq!([thumb[2], thumb[3]], [104.0, 96.0]);
}

#[test]
fn selection_fills_the_cell_dimming_fades_it_and_the_badge_sits_in_the_corner() {
    let t = theme(false);
    let plain = render(TileSize::Medium, RowLook::default(), false);
    assert_eq!(info(&plain, "tl").fill, rune::Color::TRANSPARENT);
    let look = RowLook {
        selected: true,
        dimmed: true,
        position: Some((2, 9)),
        ..RowLook::default()
    };
    let rt = render(TileSize::Medium, look, true);
    let tl = info(&rt, "tl");
    assert_eq!(tl.fill, t.selection_row);
    assert!(tl.opacity < 1.0, "dimmed");
    // The badge sits in the picture's bottom-right corner.
    let (thumb, badge) = (info(&rt, "tl-thumb").rect, info(&rt, "tl-badge").rect);
    assert_eq!(badge[0] + badge[2], thumb[0] + thumb[2] - 5.0);
    assert_eq!(badge[1] + badge[3], thumb[1] + thumb[3] - 5.0);
}
