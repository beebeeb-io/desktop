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
