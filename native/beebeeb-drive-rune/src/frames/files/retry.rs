//! Try again, on the message of a failure (DW-0109): which action is repeated, and how. Moved
//! out of `mod.rs` unchanged when that file passed the size limit.

use super::Toast;
use super::source::Action;
use crate::State;
use rune::Tx;

impl State {
    /// Try again, on the message of a failure: the same action, on what is selected now.
    pub fn retry(&mut self, tx: &mut Tx) {
        let Some(Toast::Failed(action, _)) = self.actions.toast.take() else {
            return;
        };
        match action {
            Action::Trash => {
                self.trash_selection(tx);
            }
            Action::Restore => {
                self.undo(tx);
            }
            Action::Duplicate => {
                self.duplicate_selected(tx);
            }
            Action::KeepOffline | Action::RemoveOffline => {
                self.toggle_keep_offline(tx);
            }
            Action::CopyLink => {
                self.copy_link(tx);
            }
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
            Action::FreeUpSpace => {
                self.free_up_space(tx);
            }
            Action::OpenInFileExplorer => {
                self.open_in_file_explorer(tx);
            }
            // These are asked again from their own dialog: the message goes, nothing is repeated blind.
            // Said without Try again (`window::toast_of`): nothing to repeat.
            Action::Share
            | Action::RestoreVersion
            | Action::OpenWith
            | Action::KeepVersion
            | Action::Invite
            | Action::PauseUploads
            | Action::CancelUpload => {}
        }
    }
}
