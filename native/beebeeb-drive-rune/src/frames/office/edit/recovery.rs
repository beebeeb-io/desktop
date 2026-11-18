//! The offer to bring back an unsaved editing: the words and banners the editor shows after the helper stopped, when an
//! opened file has a kept blob, and after unlock.
//!
//! - After a helper restart (`EditUi::restored` is set): "The editor stopped. Your changes up to 14:32 are restored.
//!   The undo history is gone."
//! - A second crash within a minute (`EditUi::stopped` after a restore): read-only, with "Save the restored copy" (the
//!   Keep both path: the copy is saved next to the file).
//! - Opening a file that has a blob (the source's `recovery`): "Unsaved changes to <name> from 14:32." with Restore and
//!   Discard (the source's `restore`, `discard_recovery`; `engine_host::viewer_office::recovery` keeps the blobs).
//! - After unlock, one notice for the account's blobs ([`unlocked_words`]); the shell's call is w05-wire-shell's.
//!
//! Must not: show a blob of another account (the engine never opens one), or put a file's name in a log.

use rune::{El, Tx, row};

use crate::State;
use crate::frames::office::edit::EditUi;
use crate::frames::viewers::source::{Keep, RecoveryOffer};
use crate::frames::viewers::{self};
use crate::theme::tokens::{self, Theme};
use crate::ui::kit;

pub const KEEP_BOTH: &str = "Save the restored copy";
pub const RESTORE: &str = "Restore";
pub const DISCARD: &str = "Discard";

/// What shows over the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Banner {
    /// The helper restarted; the document is the snapshot from `at`.
    Restored { at: String },
    /// A second crash: read-only; the last snapshot (from `at`, when there is one) can be saved as a copy.
    ReadOnly { at: Option<String> },
    /// A kept editing of this file from `at`.
    Offer { at: String },
}

/// After a restart.
pub fn restored_words(at: &str) -> String {
    format!("The editor stopped. Your changes up to {at} are restored. The undo history is gone.")
}

/// After a second crash.
pub fn read_only_words() -> &'static str {
    "The editor stopped again. This document is read-only now."
}

/// Opening a file that has a blob.
pub fn offer_words(name: &str, at: &str) -> String {
    format!("Unsaved changes to {name} from {at}.")
}

/// After unlock: how many files have a kept editing, and the one's name when it is one (`None`: it is not known).
pub fn unlocked_words(count: usize, name: Option<&str>) -> Option<String> {
    match (count, name) {
        (0, _) => None,
        (1, Some(name)) => Some(format!("Unsaved changes to {name} were kept on this PC.")),
        (1, None) => Some("Unsaved changes to a file were kept on this PC.".to_string()),
        (n, _) => Some(format!("Unsaved changes to {n} files were kept on this PC.")),
    }
}

/// What to show: the editor's own state first (a stopped editor is more urgent than an offer).
pub fn banner(edit: Option<&EditUi>, offer: Option<&RecoveryOffer>) -> Option<Banner> {
    if let Some(edit) = edit {
        if edit.stopped {
            return Some(Banner::ReadOnly {
                at: edit.restored.clone(),
            });
        }
        if let Some(at) = &edit.restored {
            return Some(Banner::Restored { at: at.clone() });
        }
    }
    offer.map(|offer| Banner::Offer { at: offer.at.clone() })
}

/// The banner's sentence. `name` is the open file's.
pub fn words(banner: &Banner, name: &str) -> String {
    match banner {
        Banner::Restored { at } => restored_words(at),
        Banner::ReadOnly { at: Some(at) } => format!("{} Your changes up to {at} can be kept as a copy.", read_only_words()),
        Banner::ReadOnly { at: None } => read_only_words().to_string(),
        Banner::Offer { at } => offer_words(name, at),
    }
}

/// The banner's row. Keys: `recovery`, `recovery-text`, and the buttons `recovery-keep` / `recovery-restore` /
/// `recovery-discard`.
pub fn view(banner: &Banner, name: &str, t: &Theme) -> El {
    let button = |key: &str, label: &str, press: fn(&mut State)| {
        row()
            .key(key.to_string())
            .role("button")
            .label(label)
            .h(22.0)
            .px(12.0)
            .center()
            .radius(6.0)
            .fill(t.paper)
            .border(1.0, t.card_border)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .child(kit::label(format!("{key}-label"), label, tokens::type_::CAPTION, t.text))
            .on_press(press)
    };
    let mut b = row()
        .key("recovery")
        .role("status")
        .mx(14.0)
        .mt(10.0)
        .px(12.0)
        .py(9.0)
        .gap(10.0)
        .center_y()
        .radius(10.0)
        .fill(t.ground)
        .child(
            kit::label("recovery-text", words(banner, name), tokens::type_::CAPTION, t.text_secondary)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        );
    match banner {
        Banner::Restored { .. } => {}
        Banner::ReadOnly { at: Some(_) } => b = b.child(button("recovery-keep", KEEP_BOTH, keep_both)),
        Banner::ReadOnly { at: None } => {}
        Banner::Offer { .. } => {
            b = b
                .child(button("recovery-restore", RESTORE, restore_press))
                .child(button("recovery-discard", DISCARD, discard_press));
        }
    }
    b
}

/// What the source kept for the open file.
pub fn offer(s: &mut State) -> Option<RecoveryOffer> {
    let ui = viewers::ui(s)?;
    let (source, id) = ui.edit_source();
    let id = id.to_string();
    source.recovery(&id)
}

/// Restore: the source opens the kept editing (answered `EditAnswer::Restored`, which `edit::poll` takes).
pub fn restore(s: &mut State, _tx: &mut Tx) -> bool {
    restore_now(s)
}

fn restore_press(s: &mut State) {
    restore_now(s);
}

fn restore_now(s: &mut State) -> bool {
    let Some(ui) = viewers::ui(s) else { return false };
    let (source, id) = ui.edit_source();
    let id = id.to_string();
    source.restore(&id)
}

/// Discard: the kept editing is deleted.
pub fn discard(s: &mut State, _tx: &mut Tx) {
    discard_press(s);
}

fn discard_press(s: &mut State) {
    if let Some(ui) = viewers::ui(s) {
        let (source, id) = ui.edit_source();
        let id = id.to_string();
        source.discard_recovery(&id);
    }
}

/// Save the restored copy: the Keep both path.
fn keep_both(s: &mut State) {
    if let Some(ui) = viewers::ui(s) {
        ui.resolve(Keep::Both);
    }
}

/// The session unlocked: offer what this PC kept. The list of the account's blobs is the source's
/// (`engine_host::viewer_office::recovery::Recovery::pending`) and the shell hands it over (w05-wire-shell): until then
/// there is nothing to list here, so nothing shows.
pub fn on_unlocked(_s: &mut State, _tx: &mut Tx) {}
