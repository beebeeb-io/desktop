//! ONE function shows an open file in its folder: [`show`]. The viewer's toolbar and the Photos viewer both call it;
//! it asks the Files source's "Show in File Explorer" (the same reveal step the Files menu and `open_file` step 3
//! use: `explorer.exe /select,<path>` on Windows, `open -R` on macOS). The path is the file's sync-root path, the
//! Cloud Files placeholder as it stands: nothing here opens, reads or hydrates the file.

use crate::State;
use crate::frames::files::source::ActionError;
use crate::source::DriveItem;

/// The button's words on `os` (`std::env::consts::OS`).
pub fn label_for(os: &str) -> &'static str {
    match os {
        "windows" => "Show in File Explorer",
        "macos" => "Show in Finder",
        _ => "Show in folder",
    }
}

/// The button's words on this system.
pub fn label() -> &'static str {
    label_for(std::env::consts::OS)
}

/// w31: the system's own app for a photo, in the words of "Open in ...".
pub fn os_app_label() -> &'static str {
    match std::env::consts::OS {
        "windows" => "Photos",
        "macos" => "Preview",
        _ => "default app",
    }
}

/// What is said when the file has no place in the folder on this PC (not an error of the app: it is just not here).
pub const NOT_CONNECTED: &str = "Couldn\u{2019}t show it \u{00B7} nothing is connected right now.";

/// As above, for a file with no place in the folder.
pub const NOT_HERE: &str = "This file isn\u{2019}t in your Beebeeb folder on this PC yet.";

/// Show the file in its folder, selected. `Err`: nothing was shown, and the words for it ([`NOT_HERE`] unless the
/// source says it is not connected).
pub fn show(s: &mut State, id: &str, name: &str) -> Result<(), &'static str> {
    let item = crate::frames::open_file::item_named(id, name);
    s.actions
        .source
        .show_in_file_explorer(std::slice::from_ref(&item))
        .map_err(refusal)
}

fn refusal(why: ActionError) -> &'static str {
    match why {
        ActionError::Unavailable => NOT_CONNECTED,
        _ => NOT_HERE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::files::source::{Call, FilesFixture};
    use crate::frames::open_file::testing::state;

    #[test]
    fn the_label_is_the_systems_own_word_for_its_file_manager() {
        assert_eq!(label_for("windows"), "Show in File Explorer");
        assert_eq!(label_for("macos"), "Show in Finder");
        assert_eq!(label_for("linux"), "Show in folder");
    }

    #[test]
    fn show_asks_the_reveal_step_for_that_file_and_never_opens_or_reads_it() {
        let (mut s, calls) = state(true);
        assert_eq!(show(&mut s, "f-9", "a.jpg"), Ok(()));
        // Only the reveal: no Open, no OpenDefault, no keep/download (nothing that would hydrate the placeholder).
        assert_eq!(*calls.lock().unwrap(), [Call::ShowInFileExplorer(vec!["f-9".into()])]);
    }

    #[test]
    fn a_file_with_no_place_here_is_a_line_not_a_failure_of_the_app() {
        let mut s = crate::init();
        s.actions.source = Box::new(crate::frames::files::source::NoEngine);
        assert!(show(&mut s, "f-9", "a.jpg").is_err());
        let _ = FilesFixture::default();
    }
}
