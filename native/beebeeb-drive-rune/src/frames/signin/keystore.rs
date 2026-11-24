//! The credential store did not answer (`GateState::KeyStoreUnavailable`): the window cannot read the keys it
//! needs, and nothing syncs until Windows answers. The board has no frame for it; this card is styled like the
//! other gate cards (`before_files.rs`) and says what the engine's gate and the tray say
//! (`frames/tray/source.rs`: "Can't read your keys", "Windows didn't answer. Nothing syncs until it does.").
//!
//! Buttons: Try again (the engine reads its gate once more; the frame leaves when the store answers), Get help
//! (the website's support page), Quit. There is no Sign out here: it needs the store that is not answering.

use super::{button, head, open_page, recheck};
use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::scale::space;
use crate::theme::tokens::Theme;
use crate::ui::button::{Size, Variant};
use rune::{El, col};

pub const TRY_AGAIN_KEY: &str = "gate-btn-0";
pub const GET_HELP_KEY: &str = "gate-btn-1";
pub const QUIT_KEY: &str = "gate-btn-2";

/// The block in the middle of the window.
pub fn block(t: &Theme) -> El {
    let mut block = col().key("gate").w(super::NO_PLAN_W).items_center().gap(space::S8);
    for child in head(
        &ic::PADLOCK,
        "Can't read your keys",
        "Windows didn't answer. Nothing syncs until it does.",
        true,
        t,
    ) {
        block = block.child(child);
    }
    let wide = |key: &str, label: &str, look| button(key, label, None, look, Size::Lg, t).w_full();
    block.child(
        col()
            .key("gate-actions")
            .w_full()
            .gap(space::S8)
            .mt(12.0)
            .child(
                wide(TRY_AGAIN_KEY, "Try again", Variant::Primary)
                    .default_button()
                    .autofocus()
                    .on_press(recheck),
            )
            .child(wide(GET_HELP_KEY, "Get help", Variant::Secondary).on_press(|s: &mut State| open_page(s, "support")))
            .child(wide(QUIT_KEY, "Quit", Variant::Ghost).on_press_tx(|_: &mut State, tx: &mut rune::Tx| tx.quit())),
    )
}
