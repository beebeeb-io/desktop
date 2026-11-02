//! Pins for the kept-folder writes sharing one lock with every config save, and for the
//! dismiss outcome the frontend reads.
use super::*;

    // ── r5: the kept-folder writes share one lock with every config save ──────

    /// Runs `slow` as an `update` that holds the config-write lock for a while (it has changed
    /// the config but not yet saved it), and returns once `slow` is inside its change. Whatever
    /// the caller does next runs while that update is in flight.
    fn with_an_update_in_flight<R>(
        path: &std::path::Path,
        slow: impl FnOnce(&mut crate::config::DesktopConfig) + Send + 'static,
        meanwhile: impl FnOnce() -> R,
    ) -> R {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let slow_path = path.to_path_buf();
        let slow_update = std::thread::spawn(move || {
            crate::config::DesktopConfig::update_at(&slow_path, |cfg| {
                slow(cfg);
                entered_tx.send(()).expect("the test is waiting");
                std::thread::sleep(std::time::Duration::from_millis(200));
                (true, ())
            })
        });
        entered_rx.recv().expect("the slow update started");
        let result = meanwhile();
        slow_update
            .join()
            .expect("the slow update did not panic")
            .expect("the slow update saved");
        result
    }

    #[test]
    fn test_1882_r5_the_sweeps_kept_folder_save_does_not_lose_a_concurrent_settings_save() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";
        let changed = with_an_update_in_flight(
            &path,
            |cfg| cfg.last_signed_in_email = Some("someone@beebeeb.io".to_string()),
            || remember_kept_folder_at(&path, folder).expect("the kept folder was saved"),
        );
        assert!(changed);
        let on_disk = crate::config::DesktopConfig::load_from(&path).expect("the config reads back");
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(folder),
            "the kept folder survived"
        );
        assert_eq!(
            on_disk.last_signed_in_email.as_deref(),
            Some("someone@beebeeb.io"),
            "and so did the settings change that was saving at the same moment"
        );
    }

    /// Task 1889: the Windows and Linux flows that hold their own copy of the config (Add to Finder, Repair) record a kept
    /// folder onto the file as it is now. The copy was loaded before a settings save that is still in flight, so saving
    /// the copy would have put the settings back; the record waits for that save and lands next to it, and the copy then
    /// reads both.
    #[test]
    fn test_1889_a_flow_with_its_own_copy_records_the_kept_folder_without_losing_a_concurrent_save() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        let folder = "/Users/someone/Kept";
        let mut held = crate::config::HeldConfig::from(
            crate::config::DesktopConfig::load_from(&path).expect("the flow loads its copy first"),
        );
        let changed = with_an_update_in_flight(
            &path,
            |cfg| cfg.last_signed_in_email = Some("someone@beebeeb.io".to_string()),
            || remember_kept_folder_held_at(&mut held, &path, folder).expect("the kept folder was saved"),
        );
        assert!(changed);
        let on_disk = crate::config::DesktopConfig::load_from(&path).expect("the config reads back");
        assert_eq!(on_disk.kept_unsynced_folder.as_deref(), Some(folder));
        assert_eq!(
            on_disk.last_signed_in_email.as_deref(),
            Some("someone@beebeeb.io"),
            "the settings save that was in flight survived the record"
        );
        assert_eq!(
            held.kept_unsynced_folder.as_deref(),
            Some(folder),
            "the copy has the record"
        );
        assert_eq!(
            held.last_signed_in_email.as_deref(),
            Some("someone@beebeeb.io"),
            "and what was saved meanwhile"
        );
        // The same folder again changes nothing and writes nothing.
        let written = std::fs::read(&path).expect("the file exists");
        assert!(!remember_kept_folder_held_at(&mut held, &path, folder).expect("same folder"));
        assert_eq!(std::fs::read(&path).expect("still there"), written);
    }

    /// Fix round 1 (M4). On Windows and Linux the record is its own write, ahead of the flow's result write. A record whose
    /// write failed (a scanner holding `desktop.toml.tmp` for a moment) must not be lost while the result write goes
    /// through: the alert names the folder, and the row's record rides the flow's next write, so the two land together or
    /// the flow reports the write that failed (the live 1882 rule: "A failed save ... Repair then returns an error and the
    /// app's alert names the folder", re-review D5, P1).
    #[test]
    fn test_1889_a_record_whose_write_failed_rides_the_flows_next_write() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        let folder = "/Users/someone/Kept";
        let mut held = crate::config::HeldConfig::from(
            crate::config::DesktopConfig::load_from(&path).expect("the flow loads its copy first"),
        );
        // A directory where the temp file goes fails the record's write.
        let tmp = path.with_extension("toml.tmp");
        std::fs::create_dir(&tmp).expect("mkdir");
        assert!(
            remember_kept_folder_held_at(&mut held, &path, folder).is_err(),
            "the record's write failed"
        );
        assert!(!path.exists(), "nothing was written");

        // The flow's result write fails the same way: the flow gets the error, and the record is still owed.
        let failed = held.update_held_at(&path, |cfg| {
            cfg.finder_install_status = Some("error".to_string());
            (true, ())
        });
        assert!(failed.is_err(), "the flow's own write reports the failure");
        assert!(!path.exists(), "still nothing written");

        // The scanner lets go; the next write carries the record and the result together.
        std::fs::remove_dir(&tmp).expect("rmdir");
        held.update_held_at(&path, |cfg| {
            cfg.finder_install_status = Some("error".to_string());
            (true, ())
        })
        .expect("the flow's write");
        let on_disk = crate::config::DesktopConfig::load_from(&path).expect("reads back");
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(folder),
            "the record landed"
        );
        assert_eq!(
            on_disk.finder_install_status.as_deref(),
            Some("error"),
            "with the result"
        );
        assert_eq!(
            held.kept_unsynced_folder.as_deref(),
            Some(folder),
            "and the copy reads it"
        );

        // Once landed it is not owed again: a later write that says nothing changed writes nothing.
        let written = std::fs::read(&path).expect("exists");
        held.update_held_at(&path, |_| (false, ())).expect("no-op");
        assert_eq!(std::fs::read(&path).expect("exists"), written);
    }

    #[test]
    fn test_1882_r5_dismissing_the_row_does_not_lose_a_concurrent_settings_save() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";
        assert!(remember_kept_folder_at(&path, folder).expect("saved"));
        let cleared = with_an_update_in_flight(
            &path,
            |cfg| cfg.last_signed_in_email = Some("someone@beebeeb.io".to_string()),
            || dismiss_saved_kept_folder_at(&path, folder).expect("the dismiss was saved"),
        );
        assert_eq!(
            cleared,
            DismissOutcome {
                cleared: true,
                current: None
            }
        );
        let on_disk = crate::config::DesktopConfig::load_from(&path).expect("the config reads back");
        assert_eq!(on_disk.kept_unsynced_folder, None, "the row is dismissed");
        assert_eq!(
            on_disk.last_signed_in_email.as_deref(),
            Some("someone@beebeeb.io"),
            "and the settings change that was saving at the same moment survived"
        );
    }

    #[test]
    fn test_1882_r5_the_saved_folder_write_reports_unchanged_and_leaves_the_file_alone() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        assert!(remember_kept_folder_at(&path, "/Users/someone/Kept").expect("saved"));
        let written = std::fs::read(&path).expect("the file exists");
        assert!(
            !remember_kept_folder_at(&path, "/Users/someone/Kept").expect("same folder"),
            "the same folder again changes nothing"
        );
        assert_eq!(
            std::fs::read(&path).expect("still there"),
            written,
            "and writes nothing"
        );
        assert!(
            !dismiss_saved_kept_folder_at(&path, "/Users/someone/Other")
                .expect("stale row")
                .cleared,
            "a row showing another folder dismisses nothing"
        );
        assert_eq!(
            std::fs::read(&path).expect("still there"),
            written,
            "and writes nothing"
        );
        assert!(!path.with_extension("toml.tmp").exists(), "no temp file is left behind");
    }

    // ── r5: Dismiss says whether it cleared the row, and what is saved now ────

    #[test]
    fn test_1882_r5_a_dismiss_with_a_stale_path_leaves_the_newer_folder_and_reports_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        let older = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";
        let newer = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 11:20)";
        assert!(remember_kept_folder_at(&path, older).expect("saved"));
        assert!(remember_kept_folder_at(&path, newer).expect("a newer folder replaces it"));
        let written = std::fs::read(&path).expect("the file exists");

        // The row still showed the older folder when the person pressed Dismiss.
        let outcome = dismiss_saved_kept_folder_at(&path, older).expect("the command ran");
        assert_eq!(
            outcome,
            DismissOutcome {
                cleared: false,
                current: Some(newer.to_string())
            },
            "it clears nothing, says so, and names the newer folder"
        );
        assert_eq!(
            crate::config::DesktopConfig::load_from(&path)
                .expect("reads back")
                .kept_unsynced_folder
                .as_deref(),
            Some(newer),
            "the newer folder is still saved"
        );
        assert_eq!(
            std::fs::read(&path).expect("exists"),
            written,
            "and nothing was written"
        );

        // Dismissing the folder that is saved clears it and reports that.
        assert_eq!(
            dismiss_saved_kept_folder_at(&path, newer).expect("the command ran"),
            DismissOutcome {
                cleared: true,
                current: None
            }
        );
        // Nothing saved at all (dismissed elsewhere): not cleared by this call, and nothing to show.
        assert_eq!(
            dismiss_saved_kept_folder_at(&path, newer).expect("the command ran"),
            DismissOutcome {
                cleared: false,
                current: None
            }
        );
    }

    /// The frontend reads exactly these two keys (`DismissKeptFolderResult` in
    /// `src/macSettingsModel.ts`).
    #[test]
    fn test_1882_r5_the_dismiss_outcome_is_the_json_the_frontend_reads() {
        assert_eq!(
            serde_json::to_value(DismissOutcome {
                cleared: false,
                current: Some("/Users/someone/Kept".to_string())
            })
            .expect("serializes"),
            serde_json::json!({ "cleared": false, "current": "/Users/someone/Kept" })
        );
        assert_eq!(
            serde_json::to_value(DismissOutcome {
                cleared: true,
                current: None
            })
            .expect("serializes"),
            serde_json::json!({ "cleared": true, "current": null })
        );
    }
