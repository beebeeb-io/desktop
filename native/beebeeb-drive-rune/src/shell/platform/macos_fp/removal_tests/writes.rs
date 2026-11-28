//! Pins for the kept-folder writes sharing one lock with every config save, and for the
//! dismiss outcome the frontend reads.
use super::*;

// ── r5: the kept-folder writes share one lock with every config save ──────

/// Runs `slow` as an `update` that holds the config-write lock for a while (it has changed
/// the config but not yet saved it), and returns once `slow` is inside its change. Whatever
/// the caller does next runs while that update is in flight.
fn with_an_update_in_flight<R>(
    path: &std::path::Path,
    slow: impl FnOnce(&mut beebeeb_engine::config::DesktopConfig) + Send + 'static,
    meanwhile: impl FnOnce() -> R,
) -> R {
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let slow_path = path.to_path_buf();
    let slow_update = std::thread::spawn(move || {
        beebeeb_engine::config::DesktopConfig::update_at(&slow_path, |cfg| {
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
    let on_disk = beebeeb_engine::config::DesktopConfig::load_from(&path).expect("the config reads back");
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
    let on_disk = beebeeb_engine::config::DesktopConfig::load_from(&path).expect("the config reads back");
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
        beebeeb_engine::config::DesktopConfig::load_from(&path)
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
