//! Settings' Open targets file-explorer, support-bundle and choose-photo.
//!
//! - `file-explorer` (Sync, DW-0746): File Explorer at the Beebeeb folder, as the toolbar's button does.
//! - `support-bundle` (Diagnostics, DW-0790b): the saved bundle shown in File Explorer.
//! - `choose-photo` (Previews, DW-0745): the photo picker and the measure, on the drive settings' source.

use rune::Tx;

use crate::State;

/// Opens `target`; `false`: not taken.
pub fn open(s: &mut State, tx: &mut Tx, target: &str) -> bool {
    match target {
        "file-explorer" => {
            crate::ui::toolbar::open_beebeeb_location(s, tx);
            true
        }
        "support-bundle" => {
            if !s.settings.drive.reveal_bundle() {
                super::host::log("settings: the saved support bundle was not shown in File Explorer");
            }
            true
        }
        "choose-photo" => {
            s.settings.drive.choose_photo();
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use rune::{Env, Runtime};

    use crate::State;
    use crate::frames::settings::drive::{BoardDrive, DriveSettings};
    use crate::shell::actions::FixtureEngine;
    use crate::shell::host::{Shell, poll};
    use crate::shell::platform::fake::FakePlatform;
    use crate::shell::{registry, state};

    /// A source that counts the hand-offs it was asked for.
    struct Counting(Rc<RefCell<(u32, u32)>>);

    impl DriveSettings for Counting {
        fn export_bundle(&mut self) {}
        fn reveal_bundle(&mut self) -> bool {
            self.0.borrow_mut().0 += 1;
            true
        }
        fn choose_photo(&mut self) {
            self.0.borrow_mut().1 += 1;
        }
        // The rest answers as the board's sample.
        fn storage(&self) -> crate::frames::settings::drive::Storage {
            BoardDrive::new().storage()
        }
        fn set_limit(&mut self, _: crate::frames::settings::drive::Kept, _: u32) {}
        fn clear(&mut self, _: crate::frames::settings::drive::Kept) {}
        fn sync(&self) -> crate::frames::settings::drive::SyncPane {
            BoardDrive::new().sync()
        }
        fn set_status_on_files(&mut self, _: bool) {}
        fn set_open_at_login(&mut self, _: bool) {}
        fn set_paused(&mut self, _: bool) {}
        fn sync_anyway(&mut self) {}
        fn repair(&mut self) {}
        fn turn_off_explorer(&mut self) {}
        fn insights(&self) -> &crate::frames::settings::drive::Insights {
            unreachable!("not drawn in these tests")
        }
        fn reload_insights(&mut self) {}
    }

    #[test]
    fn each_open_target_does_its_thing_and_an_unknown_one_is_not_taken() {
        let counts = Rc::new(RefCell::new((0, 0)));
        let state = state::init_with(Shell::fixture_on(
            Arc::new(FixtureEngine::ready()),
            Arc::new(FakePlatform::default()),
        ));
        let mut rt = Runtime::new(
            state,
            registry::view,
            (1280.0, 800.0),
            Env {
                system_dark: false,
                scale: 2.0,
            },
        );
        rt.poll = Some(poll);
        rt.update(|s: &mut State, tx| {
            s.settings.drive = Box::new(Counting(counts.clone()));
            assert!(super::open(s, tx, "support-bundle"));
            assert!(super::open(s, tx, "choose-photo"));
            assert!(super::open(s, tx, "file-explorer"));
            assert!(!super::open(s, tx, "privacy"));
        });
        assert_eq!(
            *counts.borrow(),
            (1, 1),
            "the bundle was shown once, the picker asked once"
        );
    }
}
