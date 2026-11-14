//! Try again, on the message of a failure (DW-0109): which action is repeated, and how.

use super::Toast;
use super::source::{Action, ActionError};
use crate::State;
use rune::Tx;

impl State {
    /// Try again, on the message of a failure: the same action; where the failure held what it was for (an
    /// upload's files, a share's items, the items a change of what this PC holds left), on exactly that.
    pub fn retry(&mut self, tx: &mut Tx) {
        if self.needs_session(tx) {
            return;
        }
        let Some(Toast::Failed(action, why)) = self.actions.toast else {
            return;
        };
        // Not drawn, so not done either (a key that reaches here).
        let offered = self.offers_try_again(action, why);
        self.actions.toast = None;
        if !offered {
            return;
        }
        match action {
            // Exactly the items that were left (`to_trash.rs`).
            Action::Trash => self.trash_again(tx),
            Action::Restore => {
                self.undo(tx);
            }
            Action::Duplicate => {
                self.duplicate_selected(tx);
            }
            // Exactly the items that were left, in the same direction (`local.rs`).
            Action::KeepOffline | Action::RemoveOffline | Action::FreeUpSpace => self.local_again(action, tx),
            // Exactly the items the link was asked for (`link.rs`).
            Action::CopyLink => self.copy_link_again(tx),
            // The Share dialog again, with the items and the choices it had. No link is made
            // here: a plain link would drop the password, the access and the end date.
            Action::ShareLink => self.share_again(tx),
            Action::Open => {
                self.open_selection(tx);
            }
            Action::ShowInFileExplorer => {
                self.show_in_file_explorer(tx);
            }
            Action::Upload => {
                if let Some((folder, files)) = self.actions.retry_upload.take() {
                    self.upload(folder, &files, tx);
                }
            }
            Action::Rename => {
                self.begin_rename(tx);
            }
            Action::NewFolder => {
                self.begin_new_folder();
            }
            Action::Move => {
                self.begin_move(tx);
            }
            Action::OpenInFileExplorer => {
                self.open_in_file_explorer(tx);
            }
            // These are asked again from their own dialog: the message goes, nothing is repeated blind.
            // Said without Try again ([`offers_try_again`]): nothing is repeated blind.
            Action::Share
            | Action::RestoreVersion
            | Action::OpenWith
            | Action::KeepVersion
            | Action::Invite
            | Action::PauseUploads
            | Action::RetryUpload
            | Action::CancelUpload => {}
        }
    }

    /// Does the message of this failure offer Try again, as the window is now? [`offers_try_again`], and
    /// something to repeat: not what this system cannot do at all or what asking again cannot change, not an
    /// upload with every file in, not a change of what this PC holds with no item left to ask for.
    pub fn offers_try_again(&self, action: Action, why: ActionError) -> bool {
        offers_try_again(action)
            && !matches!(
                why,
                ActionError::NotOnThisSystem | ActionError::HoldsKept | ActionError::NotYet | ActionError::HasLink
            )
            && match action {
                Action::Upload => !matches!(&self.actions.retry_upload, Some((_, files)) if files.is_empty()),
                Action::KeepOffline | Action::RemoveOffline | Action::FreeUpSpace => self.local_can_retry(action),
                Action::Trash => self.trash_can_retry(),
                Action::Restore => self.undo_can_retry(),
                // "Nothing can do it" (no session, no key here, no such file): asking again changes nothing.
                Action::CopyLink => why != ActionError::Unavailable && self.link_can_retry(),
                // The dialog is where a link with choices is asked again; not while another is on its way.
                Action::ShareLink => !matches!(why, ActionError::Unavailable | ActionError::InFlight),
                _ => true,
            }
    }
}

/// Does the message of this failure offer Try again? Only where [`State::retry`] repeats
/// something: a Try again that does nothing is not drawn. These are asked again from where they
/// were asked (their dialog, the Uploads panel), which is still there.
pub fn offers_try_again(action: Action) -> bool {
    !matches!(
        action,
        Action::Share
            | Action::RestoreVersion
            | Action::OpenWith
            | Action::KeepVersion
            | Action::Invite
            | Action::PauseUploads
            | Action::RetryUpload
            | Action::CancelUpload
    )
}
