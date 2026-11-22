//! Task 2530: the question a problem's way out asks before anything changes (Sync history › Problems, DW-0428's row):
//! the two-versions answers the engine listed for the row (Keep both first, the default; Keep the other; Keep this
//! PC's), or Remove from the list's one confirmation. NOT ON THE BOARD (design deviation, flagged in `FRAMES.md`): it is
//! drawn from the kit as the Files two-versions card (DW-0429) and the Remove-all ask are, with words of its own.
//!
//! Nothing changes until an answer is pressed; Cancel, Esc and a press outside change nothing. Each answer is one
//! [`super::Ui::settle`]: the source asks the engine on its worker, for the session of the press, and says what it did.

use rune::{El, el};

use super::Asking;
use super::source::Exit;
use crate::State;
use crate::theme::scale::space;
use crate::theme::tokens::{Theme, type_};
use crate::ui::settings_dialog::{self, Weight};
use crate::ui::{banner, button, dialog, kit};

/// Remove from the list's question (the contract's words).
pub const REMOVE_TITLE: &str = "Remove this change?";
pub const REMOVE_WORDS: &str = "This PC no longer has its bytes. The file on Beebeeb stays as it is.";

/// The two-versions question's title.
pub const KEEP_TITLE: &str = "Two versions";

/// What each answer does, said before it is pressed: one line per answer the row offers.
pub fn keep_line(exit: Exit) -> Option<&'static str> {
    Some(match exit {
        Exit::KeepBoth => {
            "Keep both saves this PC's version next to the original, with this PC's name and the time added."
        }
        Exit::KeepOther => {
            "Keeping the other one keeps the version in Beebeeb and puts this PC's version in Changes not sent, in Beebeeb's app data."
        }
        Exit::KeepThisPcs => "Keeping this PC's sends it as the newest version; the other stays in the file's history.",
        Exit::TryAgain | Exit::RemoveFromList => return None,
    })
}

/// An answer's words on its button.
pub fn label(exit: Exit) -> &'static str {
    match exit {
        Exit::KeepBoth => "Keep both",
        Exit::KeepOther => "Keep the other",
        Exit::KeepThisPcs => "Keep this PC's",
        Exit::RemoveFromList => "Remove from the list",
        Exit::TryAgain => "Try again",
    }
}

fn press(exit: Exit) -> banner::Handler {
    match exit {
        Exit::KeepBoth => |s: &mut State, _| s.this_pc.settle(Exit::KeepBoth),
        Exit::KeepOther => |s: &mut State, _| s.this_pc.settle(Exit::KeepOther),
        Exit::KeepThisPcs => |s: &mut State, _| s.this_pc.settle(Exit::KeepThisPcs),
        Exit::RemoveFromList => |s: &mut State, _| s.this_pc.settle(Exit::RemoveFromList),
        Exit::TryAgain => |s: &mut State, _| {
            s.this_pc.ask_cancelled();
        },
    }
}

/// The card, over the window. Keys: `dialog-settle`, the answers `settle-<n>` in the order drawn, `settle-cancel`.
pub fn ask(asking: &Asking, t: &Theme) -> El {
    let remove = asking.exits == [Exit::RemoveFromList];
    let (name, title, body) = if remove {
        ("Remove this change", REMOVE_TITLE, Vec::new())
    } else {
        let lines = asking
            .exits
            .iter()
            .filter_map(|exit| keep_line(*exit))
            .enumerate()
            .map(|(i, line)| {
                kit::label(format!("settle-text-{i}"), line, type_::CAPTION, t.text_tertiary)
                    .wrap()
                    .w_full()
            })
            .collect();
        ("Two versions", KEEP_TITLE, lines)
    };
    let mut foot = rune::col().key("settle-foot").w_full().gap(space::S8);
    for (i, exit) in asking.exits.iter().enumerate() {
        let words = label(*exit);
        // The first answer is the default (Keep both when listed), as the two-versions question draws it; the others
        // are outlined. Remove from the list is the only answer of its question, and never the default: a stray Enter
        // removes nothing.
        let b = if i == 0 {
            let b = settings_dialog::button(i, words, Weight::Primary, t).w_full();
            if remove { b } else { b.default_button() }
        } else {
            button::outline_button(&format!("btn-{i}"), words, None, t).w_full()
        };
        foot = foot.child(banner::pressable(b.key(format!("settle-{i}")), press(*exit)));
    }
    foot = foot.child(banner::pressable(
        settings_dialog::button(asking.exits.len(), "Cancel", Weight::Quiet, t)
            .w_full()
            .key("settle-cancel"),
        |s: &mut State, _| {
            s.this_pc.ask_cancelled();
        },
    ));
    let sub = if remove {
        REMOVE_WORDS.to_string()
    } else {
        asking.name.clone()
    };
    let card = dialog::card(
        name,
        if remove { 400.0 } else { 460.0 },
        title,
        Some(if remove {
            dialog::Sub::Wrap(&sub)
        } else {
            dialog::Sub::Truncate(&sub)
        }),
        body,
        foot,
        t,
    );
    // A modal's keys are heard on the modal itself (Esc says no), not on the window under it.
    el().key("dialog-settle")
        .pin(Some(0.0), Some(32.0), Some(0.0), Some(0.0))
        .child(dialog::scrim(0.0, card, t))
        .on_key(super::on_key)
        // A press outside the card does nothing; the way out is Cancel or Esc.
        .modal(false, |s: &mut State| {
            s.this_pc.ask_cancelled();
        })
}
