//! The Files frame group (DW-0401 and the frames of `frames/files/FRAMES.md`): the registry's line
//! for `FrameGroup::Section(Section::Files)`. Its tests are `ui/tests.rs` and `frames/files/tests.rs`.

use rune::{Cx, El, KeyEvent, Tx};

use crate::State;

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    crate::frames::search::over_files(s, cx).unwrap_or_else(|| crate::frames::files::view(s, cx))
}

/// The Files window's keys (arrows, Enter, back, Ctrl+F). The shell's own shortcuts ran first
/// (`host::on_key`).
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    // The protected-folder message is modal: no key reaches search or the list under it.
    if let Some(handled) = crate::frames::files::protected::key(s, ev, tx) {
        return handled;
    }
    crate::frames::search::files_key(s, ev, tx).unwrap_or_else(|| crate::frames::files::keys::on_key(s, ev, tx))
}

/// The tests `main.rs` held on `v2-native` (PRs 280 and 286), re-homed with the entry point.
#[cfg(test)]
mod tests {
    use rune::{Env, Runtime};

    use crate::shell::state::{init, load};
    use crate::source::{self, BoardFixtureSource, DriveSource, LargeFixtureSource, Listing};

    /// A folder that cannot be read draws the reason, and no rows: it never looks like an empty folder.
    #[test]
    fn an_unreadable_folder_draws_its_reason_instead_of_rows() {
        struct Broken;
        impl DriveSource for Broken {
            fn list(&self, _: Option<&str>) -> Result<Listing, source::SourceError> {
                Err(source::SourceError::Unreadable)
            }
            fn storage(&self) -> Option<source::StorageSummary> {
                None
            }
        }
        let env = Env {
            system_dark: false,
            scale: 1.0,
        };
        let mut state = init();
        (state.listing, state.list_error) = load(&Broken, None);
        state.source = Box::new(Broken);
        state.selected = None;
        assert_eq!(state.list_error, Some(source::SourceError::Unreadable));
        let mut rt = Runtime::new(state, crate::shell::registry::view, (1280.0, 800.0), env);
        rt.frame(0.0);
        assert!(rt.find("file-list-error-text").is_some(), "the reason is drawn");
        assert!(rt.find("file-list").is_none(), "and no (empty) list is");
        assert!(
            rt.find("detail-pane").is_none() || rt.find("detail-name").is_none(),
            "nothing is selected"
        );
    }

    #[test]
    fn board_fixture_has_11_items() {
        let source = BoardFixtureSource::new();
        assert_eq!(
            source.list(None).unwrap().items.len(),
            11,
            "board DW-0401 has exactly 11 items"
        );
    }

    #[test]
    fn large_fixture_5000_rows() {
        assert_eq!(LargeFixtureSource::new(5000).list(None).unwrap().items.len(), 5000);
    }
}
