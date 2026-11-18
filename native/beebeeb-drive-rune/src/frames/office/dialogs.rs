//! Slice OF4 (plan40 batch 1): the Office viewer's licence notice, DW-1557 (a panel from About in the status bar).
//! DW-1558 (New document from a template) is a 0.9.1 frame (the editor, decision 2539): its row stays a placeholder.
//!
//! The panel is drawn by [`licence_panel`] from the words below. The words sit in ONE text node (`NOTICE`) so that the
//! naming decision D-OV-4 is one edit. Nothing here claims the panel is wired: until the viewer's status bar hosts it
//! (see BRANCH-NOTES, "Integration gap"), the DW-1557 row stays on `placeholder::scene`, which the progress counter
//! reads as "not built".

use rune::{El, TextStyle, col, el, row};

use crate::State;
use crate::frames::viewers;
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;
use crate::ui::settings_dialog::DIALOG_TITLE;

/// 12px on 15px (`.t-sub`).
const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);

/// The notice, as the board's DW-1557 words it (viewer contract section 6.2). One text node.
pub const NOTICE: &str = "This viewer is built on WordCraft, GridCraft and DeckCraft, open-source projects \
(MIT OR Apache-2.0), with fonts under the SIL Open Font License 1.1 and the Ubuntu Font Licence 1.0, and PDF data \
under BSD-3-Clause. Beebeeb is under the AGPL-3.0-or-later, and some media libraries are under MPL-2.0. It runs \
entirely on this device.";
pub const TITLE: &str = "About this viewer";
/// The two links, opened in the browser. The first is a label and an address (the address is in the machine font).
pub const LINKS: [&str; 2] = [
    "Source code github.com/beebeeb-io/desktop-office",
    "Third-party licences",
];
/// The first link's label, and the address after it.
const SOURCE_LABEL: &str = "Source code ";
const SOURCE_ADDRESS: &str = "github.com/beebeeb-io/desktop-office";
/// 11px mono: the address (`.t-cap` in the machine font).
const ADDRESS: TextStyle = TextStyle::new(11.0, 400.0).family(rune::tok::Family::Mono).lh(14.0);

/// A neutral, underlined link on its own row (never Amber: Amber is for encryption state and primary actions).
fn link(index: usize, words: &str, t: &Theme) -> El {
    let key = format!("lic-link-{index}");
    let ink = t.text_tertiary;
    let text = if index == 0 {
        row()
            .key(format!("{key}-line"))
            .child(kit::label(format!("{key}-text"), SOURCE_LABEL, type_::CAPTION, ink))
            .child(kit::label(format!("{key}-address"), SOURCE_ADDRESS, ADDRESS, ink))
    } else {
        row()
            .key(format!("{key}-line"))
            .child(kit::label(format!("{key}-text"), words, type_::CAPTION, ink))
    };
    let link = row()
        .key(key.clone())
        .role("link")
        .label(words)
        .focusable(true)
        .cursor(rune::Cursor::Pointer);
    // "Third-party licences" opens the installed licence lines (Settings' open-source-licences target). The source
    // link has no address this build may open yet, so it stays as it is.
    let link = if index == 1 {
        link.on_press(|s: &mut State| {
            if let Err(why) = crate::frames::settings::about::open_licences() {
                s.settings.toast = Some(why);
            }
        })
    } else {
        link
    };
    link
        .child(
            col()
                .key(format!("{key}-col"))
                .child(text)
                .child(el().key(format!("{key}-rule")).w_full().h(1.0).fill(ink)),
        )
}

/// The panel: a title, the notice, the two links. No image and no mark: names appear only inside the notice text.
pub fn licence_panel(t: &Theme) -> El {
    col()
        .key("lic-panel")
        .role("dialog")
        .label(TITLE)
        .w(360.0)
        .padding(16.0)
        .gap(8.0)
        .fill(t.card)
        .radius(12.0)
        .box_shadow(kit::drop(24.0, 60.0, rune::Color::BLACK.alpha(0.28)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            kit::label("lic-title", TITLE, DIALOG_TITLE, t.text)
                .role("heading")
                .level(2)
                .w_full(),
        )
        .child(kit::label("lic-notice", NOTICE, SUB, t.text_tertiary).wrap().w_full())
        .child(
            col()
                .key("lic-links")
                .w_full()
                .gap(4.0)
                .child(link(0, LINKS[0], t))
                .child(link(1, LINKS[1], t)),
        )
}

/// DW-1558: the Files window as the board draws it under New (Contract draft.pdf selected), New document over it on
/// Docs, Blank chosen.
pub fn scene_new_document(s: &mut State) {
    s.select_only(6);
    let folder = s.listing.folder.id.clone();
    super::edit::new_document::begin(s, Some(crate::frames::viewers::new_file::Kind::Word), folder);
}

/// The group's board frames and the scene that shows each (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] = &[
    ("DW-1557", viewers::scene_licence),
    ("DW-1558", scene_new_document),
    ("DW-1562", super::open_with::scene_open_with),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn all_words() -> String {
        format!("{NOTICE} {TITLE} {} {}", LINKS[0], LINKS[1])
    }

    #[test]
    fn office_licence_names_the_licences() {
        let w = all_words();
        assert!(w.contains("MIT OR Apache-2.0"));
        for named in [
            "SIL Open Font License 1.1",
            "Ubuntu Font Licence 1.0",
            "BSD-3-Clause",
            "MPL-2.0",
            "AGPL-3.0-or-later",
        ] {
            assert!(w.contains(named), "{named} must be named (the shipped notice carries all of them)");
        }
        // Stale spec text: the Office engine is not LibreOffice, and no Qt or LGPL code ships.
        for banned in ["LibreOffice", "Qt", "LGPL", "ffmpeg", "GPL-2"] {
            assert!(!w.contains(banned), "{banned} must not appear");
        }
    }

    #[test]
    fn office_licence_title_is_the_boards() {
        assert_eq!(TITLE, "About this viewer");
    }

    #[test]
    fn office_licence_links_are_neutral_and_on_their_own_lines() {
        use rune::{Env, Runtime};
        let t = crate::theme::tokens::theme(false);
        let mut state = crate::init();
        (FRAMES[0].1)(&mut state);
        let mut rt = Runtime::new(
            state,
            crate::view,
            (1278.0, 798.0),
            Env {
                system_dark: false,
                scale: 2.0,
            },
        );
        rt.frame(0.0);
        let find = |key: &str| {
            rt.elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some(key))
                .unwrap_or_else(|| panic!("{key} is drawn"))
        };
        let (a, b) = (find("lic-link-0-text"), find("lic-link-1-text"));
        for e in [&a, &b] {
            assert_ne!(e.text_color, Some(t.accent), "a link is never Amber");
            assert_ne!(e.text_color, Some(t.accent_deep), "a link is never Amber");
        }
        assert!(b.rect[1] - a.rect[1] >= 14.0, "the links are on two lines");
    }

    #[test]
    fn third_party_licences_opens_the_licence_lines_and_the_source_link_opens_nothing() {
        use rune::{Env, Runtime};
        let mut state = crate::init();
        (FRAMES[0].1)(&mut state);
        let mut rt = Runtime::new(
            state,
            crate::view,
            (1278.0, 798.0),
            Env {
                system_dark: false,
                scale: 2.0,
            },
        );
        rt.frame(0.0);
        let click = |rt: &mut Runtime<State>, key: &str| {
            let r = rt
                .elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some(key))
                .unwrap_or_else(|| panic!("{key} is drawn"))
                .rect;
            let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
        };
        rt.state.settings.toast = None;
        click(&mut rt, "lic-link-0");
        assert_eq!(rt.state.settings.toast, None, "the source link opens nothing here");
        // No licence file sits beside a test build, so the press reaching the licence target says so.
        click(&mut rt, "lic-link-1");
        assert_eq!(
            rt.state.settings.toast,
            Some(crate::frames::settings::about::LICENCES_MISSING),
            "the press went to the open-source-licences target"
        );
    }

    #[test]
    fn office_dialogs_no_logo_or_brand_mark() {
        // Names appear only inside the one notice text node; no other text carries one.
        for other in [TITLE, LINKS[0], LINKS[1]] {
            for name in ["WordCraft", "GridCraft", "DeckCraft"] {
                assert!(!other.contains(name), "{name} outside the notice");
            }
        }
        for name in ["WordCraft", "GridCraft", "DeckCraft"] {
            assert!(NOTICE.contains(name));
        }
    }

    #[test]
    fn office_dialogs_scenes_set_their_state() {
        // DW-1557, DW-1558 (New document) and DW-1562 (Open With, D-OV-14, drawn by the Files window) keep their rows.
        let ids: Vec<&str> = FRAMES.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, ["DW-1557", "DW-1558", "DW-1562"]);
        let mut s = crate::init();
        for (_, scene) in FRAMES {
            scene(&mut s);
        }
    }
}
