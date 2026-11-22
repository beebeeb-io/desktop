//! The sidebar's places: folder art in its tint (selected or not); Trash keeps its glyph and colour.

use super::*;
use crate::theme::file_icon_colors::tint_color;
use crate::theme::tokens;

fn colours(dark: bool, on: bool) -> Vec<(SidebarSection, Color)> {
    let t = tokens::theme(dark);
    kit::record();
    for &s in SidebarSection::all() {
        let _ = section_item(s, None, on, &t);
    }
    let notes = kit::take();
    SidebarSection::all()
        .iter()
        .map(|&s| (s, notes[&format!("{}-icon", s.label())].color.unwrap()))
        .collect()
}

#[test]
fn sidebar_places_draw_in_their_tints() {
    for dark in [false, true] {
        for on in [false, true] {
            let t = tokens::theme(dark);
            for (s, c) in colours(dark, on) {
                let want = match s {
                    SidebarSection::Files | SidebarSection::Shared | SidebarSection::Backups => {
                        tint_color(Tint::Folder, dark)
                    }
                    SidebarSection::Photos => tint_color(Tint::Image, dark),
                    SidebarSection::Trash => {
                        if on {
                            t.text
                        } else {
                            t.text_tertiary
                        }
                    }
                };
                assert!(c == want, "{} (dark {dark}, on {on})", s.label());
            }
        }
    }
}
