//! Slice OF4 (plan40 batch 1): the Office viewer's licence notice, DW-1557 (a panel from About in the status bar).
//! DW-1558 (New document from a template) is a 0.9.1 frame (the editor, decision 2539): its row stays a placeholder.
//!
//! The panel is drawn by [`licence_panel`] from the words below. The words sit in ONE text node (`NOTICE`) so that the
//! naming decision D-OV-4 is one edit. Nothing here claims the panel is wired: until the viewer's status bar hosts it
//! (see BRANCH-NOTES, "Integration gap"), the DW-1557 row stays on `placeholder::scene`, which the progress counter
//! reads as "not built".

use rune::{El, TextStyle, col, el, row};

use crate::State;
use crate::frames::placeholder;
use crate::theme::tokens::Theme;
use crate::ui::kit;
use crate::ui::settings_dialog::{BUTTON_MD, DIALOG_TITLE};

/// 12px on 15px (`.t-sub`).
const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);

/// The notice, as the board's DW-1557 words it (viewer contract section 6.2). One text node.
pub const NOTICE: &str = "This viewer is built on WordCraft, GridCraft and DeckCraft, open-source projects \
(MIT OR Apache-2.0), with fonts under the SIL Open Font License 1.1. It runs entirely on this device.";
pub const TITLE: &str = "Open-source licences";
/// The two links, opened in the browser.
pub const LINKS: [&str; 2] = ["Source: the Office engine's public repository", "Third-party licences"];

fn link(index: usize, words: &str, t: &Theme) -> El {
    let key = format!("lic-link-{index}");
    row()
        .key(key.clone())
        .role("link")
        .label(words)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .child(kit::label(format!("{key}-text"), words, BUTTON_MD, t.accent_deep))
}

/// The panel: a title, the notice, the two links. No image and no mark: names appear only inside the notice text.
pub fn licence_panel(t: &Theme) -> El {
    col()
        .key("lic-panel")
        .role("dialog")
        .label(TITLE)
        .w(380.0)
        .p(20.0)
        .gap(10.0)
        .fill(t.card)
        .radius(8.0)
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
            el().key("lic-links")
                .w_full()
                .child(link(0, LINKS[0], t))
                .child(link(1, LINKS[1], t)),
        )
}

/// The group's board frames and the scene that shows each (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] =
    &[("DW-1557", placeholder::scene), ("DW-1558", placeholder::scene)];

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
        assert!(w.contains("SIL Open Font License 1.1"));
        for banned in ["LibreOffice", "Qt", "MPL"] {
            assert!(!w.contains(banned), "{banned} must not appear");
        }
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
        // DW-1557 and DW-1558 keep their rows; DW-1558 is a 0.9.1 placeholder by decision.
        let ids: Vec<&str> = FRAMES.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, ["DW-1557", "DW-1558"]);
        let mut s = crate::init();
        for (_, scene) in FRAMES {
            scene(&mut s);
        }
    }
}
