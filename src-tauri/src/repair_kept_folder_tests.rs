    use super::{DesktopConfig, HeldConfig, finder_removal, finish_repair_after_removal};

    const FOLDER: &str = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";

    /// What the production surface would raise: `surface_kept_folder` saves the record then shows
    /// `kept_folder_alert`'s text. The text is the part that must still be produced.
    type Alert = Option<(&'static str, String)>;

    #[test]
    fn test_1882_r3_a_repair_whose_save_fails_after_the_removal_still_names_the_kept_folder() {
        let mut cfg = HeldConfig::from(DesktopConfig::default());
        let mut alert: Alert = None;
        let mut raised = 0;
        let result = finish_repair_after_removal(
            &mut cfg,
            Some(FOLDER),
            |_| Err("No space left on device".to_string()),
            |location| {
                raised += 1;
                alert = finder_removal::kept_folder_alert(location);
            },
        );
        let error = result.expect_err("the save failed");
        // Rebase re-review Minor 4: the save's own error goes back. The r4 code that marked it as after the removal
        // is gone: under spec A no platform can fail after a removal that worked.
        assert_eq!(
            error, "No space left on device",
            "the save error is the error, unmarked"
        );
        assert_eq!(raised, 1, "the folder is shown once");
        assert_eq!(
            alert,
            Some((
                finder_removal::PRESERVED_FILES_TITLE,
                format!("{}\n\n{FOLDER}", finder_removal::PRESERVED_FILES_SENTENCE)
            )),
            "the alert text names the folder before the error returns"
        );
    }

    #[test]
    fn test_1882_r3_a_repair_whose_save_fails_with_nothing_kept_shows_nothing() {
        let mut cfg = HeldConfig::from(DesktopConfig::default());
        let mut alert: Alert = None;
        let result = finish_repair_after_removal(
            &mut cfg,
            None,
            |_| Err("No space left on device".to_string()),
            |location| alert = finder_removal::kept_folder_alert(location),
        );
        assert_eq!(result, Err("No space left on device".to_string()));
        assert_eq!(alert, None, "nothing kept → no alert, even when the save fails");
    }

    #[test]
    fn test_1882_r3_a_repair_that_saves_shows_no_extra_alert() {
        // The Repair result carries the folder to the Sync tab's row; a second alert would repeat it.
        let mut cfg = HeldConfig::from(DesktopConfig::default());
        let mut raised = 0;
        let result = finish_repair_after_removal(&mut cfg, Some(FOLDER), |_| Ok(()), |_| raised += 1);
        assert_eq!(result, Ok(()));
        assert_eq!(raised, 0);
    }

    /// Fix round 1 (M2) replaces `test_1882_r3_the_save_is_given_the_config_the_repair_recorded_into`, which recorded
    /// into the held copy and handed the same copy to the save: the pattern that loses data now that the save writes
    /// onto the file as it is. A Repair's record is written onto the file first; the result is written next, through the
    /// copy's own update; the copy the save is given already shows the record; and the file holds both.
    #[test]
    fn test_1889_the_repairs_result_write_lands_next_to_the_kept_folder_record_that_came_before_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        let mut cfg = HeldConfig::from(DesktopConfig::load_from(&path).expect("the Repair loads its copy first"));
        // The app-start sweep saves a setting while the Repair waits on the removal.
        DesktopConfig::update_at(&path, |on_file| {
            on_file.last_signed_in_email = Some("someone@beebeeb.io".to_string());
            (true, ())
        })
        .expect("the sweep saves");

        assert_eq!(
            finder_removal::remember_kept_folder_held_at(&mut cfg, &path, FOLDER),
            Ok(true),
            "the record is written onto the file as it is now"
        );
        let mut saw = None;
        let result = finish_repair_after_removal(
            &mut cfg,
            Some(FOLDER),
            |held| {
                saw = held.kept_unsynced_folder.clone();
                held.update_held_at(&path, |on_file| {
                    on_file.finder_install_status = Some("missing".to_string());
                    (true, ())
                })
            },
            |_| {},
        );
        assert_eq!(result, Ok(()));
        assert_eq!(
            saw.as_deref(),
            Some(FOLDER),
            "the copy the save is given already shows the record"
        );
        let on_file = DesktopConfig::load_from(&path).expect("reads back");
        assert_eq!(
            on_file.kept_unsynced_folder.as_deref(),
            Some(FOLDER),
            "the record is on the file"
        );
        assert_eq!(
            on_file.finder_install_status.as_deref(),
            Some("missing"),
            "and so is the result"
        );
        assert_eq!(
            on_file.last_signed_in_email.as_deref(),
            Some("someone@beebeeb.io"),
            "and the setting saved meanwhile"
        );
    }
