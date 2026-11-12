//! Tests for `crate::config`: the persisted `DesktopConfig` shape, the
//! settings mapping, the `HeldConfig` write lock and the account-id writes.
//!
//! Moved out of `config.rs` byte-identically. `super` is still `crate::config`,
//! so every `use super::{…}` here resolves exactly as it did inline.
//! Tests only: no production item lives in this file.

use super::{
    ConfigError, ConfigHost, ConfigMode, DesktopConfig, DesktopSettings, DesktopTheme, HeldConfig, KnownFolderBackup,
    ReleaseChannel, UpdateError, default_sync_root_suggestion,
};
use std::path::{Path, PathBuf};

// ── Task 0800 — multi-account Phase 1: persisted account id ────────────

#[test]
fn account_id_defaults_none_and_is_omitted_when_unset() {
    // A fresh config has no account id, matching a pre-Phase-1 install.
    let cfg = DesktopConfig::default();
    assert!(cfg.account_id.is_none());

    // An older file that predates the key parses to `None` via serde default.
    let cfg: DesktopConfig = toml::from_str("").expect("parse empty toml");
    assert!(cfg.account_id.is_none());
}

#[test]
fn account_id_none_is_byte_identical_to_pre_phase1_serialisation() {
    // `skip_serializing_if = "Option::is_none"` keeps a fresh-install file
    // byte-identical to the pre-Phase-1 layout: the `account_id` key must NOT
    // appear in the serialised TOML until it is actually set. This is the
    // "byte-identical fresh install" guarantee — onboarding still fires off
    // `sync_root.is_none()`, never file-shape changes.
    let cfg = DesktopConfig::default();
    let toml = toml::to_string_pretty(&cfg).expect("serialize default");
    assert!(
        !toml.contains("account_id"),
        "account_id must be omitted from a fresh-install desktop.toml, got:\n{toml}"
    );

    // Once set, the key DOES appear and round-trips losslessly.
    let mut cfg = DesktopConfig::default();
    cfg.account_id = Some("11111111-2222-3333-4444-555555555555".to_string());
    let toml = toml::to_string_pretty(&cfg).expect("serialize with id");
    assert!(toml.contains("account_id"));
    let back: DesktopConfig = toml::from_str(&toml).expect("parse");
    assert_eq!(back.account_id.as_deref(), Some("11111111-2222-3333-4444-555555555555"));
}

#[test]
fn last_signed_in_email_round_trips_and_stays_absent_until_set() {
    // Same byte-identical guarantee as account_id: a config that never
    // signed in must not grow a `last_signed_in_email` key, and older
    // configs missing the key must load as `None` (serde default).
    let cfg = DesktopConfig::default();
    let toml = toml::to_string_pretty(&cfg).expect("serialize default");
    assert!(
        !toml.contains("last_signed_in_email"),
        "last_signed_in_email must be omitted until an email is remembered, got:\n{toml}"
    );

    // A pre-field config (no key) loads as None — backward compatible.
    let legacy: DesktopConfig = toml::from_str("sync_root = \"/tmp/x\"\n").expect("parse legacy");
    assert_eq!(legacy.last_signed_in_email, None);

    // Once set, the key appears and round-trips losslessly.
    let mut cfg = DesktopConfig::default();
    cfg.last_signed_in_email = Some("user@example.com".to_string());
    let toml = toml::to_string_pretty(&cfg).expect("serialize with email");
    assert!(toml.contains("last_signed_in_email"));
    let back: DesktopConfig = toml::from_str(&toml).expect("parse");
    assert_eq!(back.last_signed_in_email.as_deref(), Some("user@example.com"));
}

#[test]
fn ensure_account_id_is_idempotent_when_already_set() {
    // Pre-set id → returned unchanged, never re-minted, and the file is not rewritten
    // (the change says "nothing to save"), on a file in a temp dir.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("desktop.toml");
    DesktopConfig::update_at(&path, |cfg| {
        cfg.account_id = Some("fixed-id".to_string());
        (true, ())
    })
    .expect("seed an id");
    let before = std::fs::read(&path).expect("read seeded config");

    let returned = DesktopConfig::ensure_account_id_at(&path).expect("idempotent path returns Ok");
    assert_eq!(returned, "fixed-id");

    // Calling again still returns the same id, still unchanged on disk.
    let again = DesktopConfig::ensure_account_id_at(&path).expect("second call returns Ok");
    assert_eq!(again, "fixed-id");
    assert_eq!(
        std::fs::read(&path).expect("read again"),
        before,
        "an id already set is not re-saved"
    );
}

#[test]
fn ensure_account_id_mints_persists_and_is_stable_across_relaunch() {
    // The real mint + persist + readback path, against a file in a temp dir. (This used to
    // snapshot, delete and restore the person's real `desktop.toml`; a crash mid-test would
    // have left it deleted. No test may touch the real file, so it runs on a temp path.)
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("desktop.toml");

    // Mint: a fresh config has no id; ensure_account_id mints + persists one.
    let cfg = DesktopConfig::load_from(&path).expect("load (missing → default)");
    assert!(cfg.account_id.is_none(), "fresh config must start without an id");
    let minted = DesktopConfig::ensure_account_id_at(&path).expect("ensure_account_id should mint");
    assert!(uuid::Uuid::parse_str(&minted).is_ok(), "minted id is a valid uuid");
    assert!(path.exists(), "the id is on disk before ensure_account_id returns");

    // Persist: a fresh load from disk (simulating relaunch) sees the SAME id.
    let reloaded = DesktopConfig::load_from(&path).expect("reload after persist");
    assert_eq!(
        reloaded.account_id.as_deref(),
        Some(minted.as_str()),
        "persisted id must survive a reload"
    );

    // Idempotent across relaunch: ensure_account_id returns the same id (no re-mint) and does
    // not rewrite the file.
    let before = std::fs::read(&path).expect("read saved config");
    let again = DesktopConfig::ensure_account_id_at(&path).expect("idempotent on reload");
    assert_eq!(again, minted, "relaunch must keep the same account id");
    assert_eq!(
        std::fs::read(&path).expect("read again"),
        before,
        "an id already set is not re-saved"
    );
}

#[test]
fn known_folder_backup_defaults_empty() {
    // v1 default-OFF: a fresh config opts into nothing.
    let cfg = DesktopConfig::default();
    assert!(cfg.known_folder_backup.is_empty());
    assert!(!cfg.known_folder_enabled("documents"));
}

#[test]
fn known_folder_backup_set_is_idempotent_and_dedupes() {
    let mut cfg = DesktopConfig::default();

    // Enable twice → present exactly once.
    cfg.set_known_folder("documents", true);
    cfg.set_known_folder("documents", true);
    assert!(cfg.known_folder_enabled("documents"));
    assert_eq!(cfg.known_folder_backup.iter().filter(|k| *k == "documents").count(), 1);

    // A second key coexists.
    cfg.set_known_folder("pictures", true);
    assert!(cfg.known_folder_enabled("pictures"));
    assert_eq!(cfg.known_folder_backup.len(), 2);

    // Disable removes only that key; disabling an absent key is a no-op.
    cfg.set_known_folder("documents", false);
    cfg.set_known_folder("documents", false);
    assert!(!cfg.known_folder_enabled("documents"));
    assert!(cfg.known_folder_enabled("pictures"));
    assert_eq!(cfg.known_folder_backup.len(), 1);
}

#[test]
fn known_folder_onboarding_seen_defaults_false_and_round_trips() {
    // Fresh config → prompt not yet seen, so first-run shows it.
    let cfg = DesktopConfig::default();
    assert!(!cfg.known_folder_onboarding_seen);

    // Missing key (older file) → false via #[serde(default)].
    let cfg: DesktopConfig = toml::from_str("").expect("parse empty toml");
    assert!(!cfg.known_folder_onboarding_seen);

    // Once marked, the flag survives a serialise/parse round-trip.
    let mut cfg = DesktopConfig::default();
    cfg.known_folder_onboarding_seen = true;
    let s = toml::to_string_pretty(&cfg).expect("serialize");
    let back: DesktopConfig = toml::from_str(&s).expect("parse");
    assert!(back.known_folder_onboarding_seen);
}

#[test]
fn menu_bar_hint_seen_defaults_false_and_round_trips() {
    // A fresh config, and a file that has no such key (every install before this one): the hint is not yet seen.
    assert!(!DesktopConfig::default().menu_bar_hint_seen);
    let cfg: DesktopConfig = toml::from_str("").expect("parse empty toml");
    assert!(!cfg.menu_bar_hint_seen);

    let mut cfg = DesktopConfig::default();
    cfg.menu_bar_hint_seen = true;
    let back: DesktopConfig = toml::from_str(&toml::to_string_pretty(&cfg).expect("serialize")).expect("parse");
    assert!(back.menu_bar_hint_seen);
}

#[test]
fn a_settings_save_never_reads_or_writes_the_menu_bar_hint() {
    // The hint belongs to Home, not to Settings: the settings DTO has no field for it, so no save can flip it.
    let mut cfg = DesktopConfig::default();
    cfg.menu_bar_hint_seen = true;
    let dto = DesktopSettings::from(&cfg);
    assert!(!serde_json::to_string(&dto).unwrap().contains("menu_bar_hint"));
    let mut other = DesktopConfig::default();
    other.apply_settings(dto);
    assert!(
        !other.menu_bar_hint_seen,
        "applying the settings of a config that had seen it leaves the hint alone"
    );
}

#[test]
fn known_folder_backup_round_trips_through_toml() {
    // Missing key (older file) → empty vec via #[serde(default)].
    let cfg: DesktopConfig = toml::from_str("").expect("parse empty toml");
    assert!(cfg.known_folder_backup.is_empty());

    // A populated list survives a serialise/parse round-trip.
    let mut cfg = DesktopConfig::default();
    cfg.set_known_folder("desktop", true);
    cfg.set_known_folder("pictures", true);
    let s = toml::to_string_pretty(&cfg).expect("serialize");
    let back: DesktopConfig = toml::from_str(&s).expect("parse");
    assert!(back.known_folder_enabled("desktop"));
    assert!(back.known_folder_enabled("pictures"));
    assert_eq!(back.known_folder_backup.len(), 2);
}

#[test]
fn advanced_settings_defaults_and_round_trips() {
    // Missing keys from older desktop.toml files pick the new Advanced
    // defaults: follow the OS theme and keep local cache Unlimited.
    let cfg: DesktopConfig = toml::from_str("").expect("parse empty toml");
    assert_eq!(cfg.theme, DesktopTheme::System);
    assert_eq!(cfg.local_cache_limit_bytes, 0);
    assert_eq!(cfg.local_cache_limit_for_eviction(), None);
    assert_eq!(DesktopConfig::default().local_cache_limit_bytes, 0);

    let mut settings = DesktopSettings::from(&DesktopConfig::default());
    settings.theme = Some(DesktopTheme::Dark);
    settings.local_cache_limit_bytes = Some(0);

    let mut cfg = DesktopConfig::default();
    cfg.apply_settings(settings);
    assert_eq!(cfg.theme, DesktopTheme::Dark);
    assert_eq!(cfg.local_cache_limit_bytes, 0);
    assert_eq!(cfg.local_cache_limit_for_eviction(), None);

    let toml = toml::to_string_pretty(&cfg).expect("serialize advanced settings");
    let back: DesktopConfig = toml::from_str(&toml).expect("parse advanced settings");
    assert_eq!(back.theme, DesktopTheme::Dark);
    assert_eq!(back.local_cache_limit_bytes, 0);
}

#[test]
fn installed_release_channel_defaults_absent_and_round_trips_separately_from_configured_channel() {
    let cfg: DesktopConfig = toml::from_str("release_channel = \"alpha\"").expect("parse legacy channel config");
    assert_eq!(cfg.release_channel, ReleaseChannel::Alpha);
    assert_eq!(cfg.installed_release_channel, None);

    let mut cfg = DesktopConfig {
        release_channel: ReleaseChannel::Alpha,
        installed_release_channel: Some(ReleaseChannel::Beta),
        ..DesktopConfig::default()
    };
    cfg.apply_settings(DesktopSettings {
        release_channel: Some(ReleaseChannel::Stable),
        ..DesktopSettings::from(&cfg)
    });

    assert_eq!(cfg.release_channel, ReleaseChannel::Stable);
    assert_eq!(cfg.installed_release_channel, Some(ReleaseChannel::Beta));

    let toml = toml::to_string_pretty(&cfg).expect("serialize installed release channel");
    let back: DesktopConfig = toml::from_str(&toml).expect("parse installed release channel");
    assert_eq!(back.release_channel, ReleaseChannel::Stable);
    assert_eq!(back.installed_release_channel, Some(ReleaseChannel::Beta));
}

#[test]
#[cfg(target_os = "macos")]
fn default_sync_root_uses_private_file_provider_state_folder() {
    let path = default_sync_root_suggestion();

    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("file-provider-state")
    );
    assert!(path.to_string_lossy().contains(crate::identity::config_dir_name()));
}

#[test]
#[cfg(not(target_os = "macos"))]
fn default_sync_root_uses_user_facing_beebeeb_folder() {
    let path = default_sync_root_suggestion();

    assert_eq!(path.file_name().and_then(|name| name.to_str()), Some("Beebeeb"));
}

// ── Task 1882 r5 — one lock around every load-modify-save ───────────────

#[test]
fn test_1882_r5_two_updates_at_once_both_land_and_an_unsaved_change_writes_nothing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("desktop.toml");
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let slow_path = path.clone();
    let slow = std::thread::spawn(move || {
        DesktopConfig::update_at(&slow_path, |cfg| {
            cfg.kept_unsynced_folder = Some("/Users/someone/Kept".to_string());
            entered_tx.send(()).expect("the test is waiting");
            std::thread::sleep(std::time::Duration::from_millis(200));
            (true, "slow")
        })
    });
    entered_rx.recv().expect("the slow update started");
    // This one starts while the slow one is between its load and its save.
    let fast = DesktopConfig::update_at(&path, |cfg| {
        cfg.last_signed_in_email = Some("someone@beebeeb.io".to_string());
        (true, "fast")
    });
    assert_eq!(fast, Ok("fast"));
    assert_eq!(slow.join().expect("no panic"), Ok("slow"));
    let on_disk = DesktopConfig::load_from(&path).expect("reads back");
    assert_eq!(on_disk.kept_unsynced_folder.as_deref(), Some("/Users/someone/Kept"));
    assert_eq!(on_disk.last_signed_in_email.as_deref(), Some("someone@beebeeb.io"));

    // A change that says "nothing to save" leaves the file byte for byte as it was.
    let written = std::fs::read(&path).expect("exists");
    let untouched = DesktopConfig::update_at(&path, |cfg| {
        cfg.kept_unsynced_folder = None;
        (false, 7)
    });
    assert_eq!(untouched, Ok(7));
    assert_eq!(std::fs::read(&path).expect("exists"), written);
}

#[test]
fn test_1882_r5_an_update_of_a_corrupt_config_fails_and_does_not_overwrite_it() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("desktop.toml");
    std::fs::write(&path, "this is = not [valid toml").expect("write");
    let result = DesktopConfig::update_at(&path, |_| (true, ()));
    assert!(result.is_err(), "a corrupt config is an error, never overwritten");
    assert_eq!(
        std::fs::read_to_string(&path).expect("exists"),
        "this is = not [valid toml"
    );
}

// ── Task 1889 — every load-modify-save runs as one serialized update ─────────────────

const KEPT: &str = "/Users/someone/Kept";

/// Runs a slow `update_at` that sets the kept folder and then stays between its load and its save for 200 ms, and,
/// once it is inside, `fast` on the same file. Returns after both finished, with `fast`'s answer.
fn overlapped_with_a_slow_update<R>(path: &Path, fast: impl FnOnce(&Path) -> R) -> R {
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let slow_path = path.to_path_buf();
    let slow = std::thread::spawn(move || {
        DesktopConfig::update_at(&slow_path, |cfg| {
            cfg.kept_unsynced_folder = Some(KEPT.to_string());
            entered_tx.send(()).expect("the test is waiting");
            std::thread::sleep(std::time::Duration::from_millis(200));
            (true, ())
        })
    });
    entered_rx.recv().expect("the slow update started");
    // `fast` starts while the slow one is between its load and its save.
    let answer = fast(path);
    assert_eq!(slow.join().expect("no panic"), Ok(()));
    answer
}

/// The point of the task: whichever helper a writer uses, a change made while another writer is between its load and
/// its save lands next to that writer's, and neither replaces the other. One field each, so a lost update shows as
/// the missing field. Without the lock the fast helper's field is gone: the slow update saves the copy it loaded.
#[test]
fn test_1889_two_updates_of_different_fields_both_survive_through_every_helper() {
    type Fast = fn(&Path);
    type Landed = fn(&DesktopConfig) -> bool;
    let helpers: [(&str, Fast, Landed); 4] = [
        (
            "update_at",
            |path| {
                DesktopConfig::update_at(path, |cfg| {
                    cfg.last_signed_in_email = Some("someone@beebeeb.io".to_string());
                    (true, ())
                })
                .expect("fast update");
            },
            |cfg| cfg.last_signed_in_email.as_deref() == Some("someone@beebeeb.io"),
        ),
        (
            "update_held_at",
            |path| {
                // The copy was loaded before the slow update began, as an install flow's is.
                let mut held = HeldConfig::from(DesktopConfig::default());
                held.update_held_at(path, |held| {
                    held.pause_sync = true;
                    (true, ())
                })
                .expect("fast held update");
            },
            |cfg| cfg.pause_sync,
        ),
        (
            "ensure_account_id_at",
            |path| {
                DesktopConfig::ensure_account_id_at(path).expect("fast mint");
            },
            |cfg| cfg.account_id.is_some(),
        ),
        (
            "try_update_at",
            |path| {
                DesktopConfig::try_update_at(path, |cfg| {
                    cfg.sync_mode = Some("smart".to_string());
                    (true, ())
                })
                .expect("fast try-update");
            },
            |cfg| cfg.sync_mode.as_deref() == Some("smart"),
        ),
    ];
    let mut lost = Vec::new();
    for (name, fast, landed) in helpers {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        overlapped_with_a_slow_update(&path, fast);
        let on_disk = DesktopConfig::load_from(&path).expect("reads back");
        if on_disk.kept_unsynced_folder.as_deref() != Some(KEPT) {
            lost.push(format!("{name}: the slow update's change is gone"));
        }
        if !landed(&on_disk) {
            lost.push(format!("{name}: the overlapping update's change is gone"));
        }
    }
    assert!(lost.is_empty(), "changes lost to a stale save: {lost:?}");
}

/// The exact bug of the ticket (1882 round 5): an install or Repair flow loads its copy at the start, the app-start
/// sweep saves a kept-folder record meanwhile, and the flow then writes its own result. The flow's write used to put
/// back the copy it loaded, and the record was gone.
#[test]
fn test_1889_a_held_copy_never_puts_back_what_another_writer_saved_since_it_loaded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("desktop.toml");
    let mut held = HeldConfig::from(DesktopConfig::load_from(&path).expect("the flow loads its copy first"));
    assert_eq!(held.kept_unsynced_folder, None);

    // The sweep saves the kept folder while the flow waits on the File Provider.
    DesktopConfig::update_at(&path, |cfg| {
        cfg.kept_unsynced_folder = Some(KEPT.to_string());
        (true, ())
    })
    .expect("the sweep saves");

    // The flow writes its result. Its copy is older than the file.
    let state = held
        .update_held_at(&path, |held| {
            held.finder_install_status = Some("error".to_string());
            held.finder_install_reason_category = Some("timeout".to_string());
            (true, held.finder_install_status.clone())
        })
        .expect("the flow's write");
    assert_eq!(state.as_deref(), Some("error"), "the closure's answer comes back");

    let on_disk = DesktopConfig::load_from(&path).expect("reads back");
    assert_eq!(
        on_disk.kept_unsynced_folder.as_deref(),
        Some(KEPT),
        "the sweep's record survived"
    );
    assert_eq!(on_disk.finder_install_status.as_deref(), Some("error"));
    assert_eq!(on_disk.finder_install_reason_category.as_deref(), Some("timeout"));
    assert_eq!(
        held.kept_unsynced_folder.as_deref(),
        Some(KEPT),
        "afterwards the copy reads the file, not the load it started with"
    );
    assert_eq!(held.finder_install_status.as_deref(), Some("error"));
}

#[test]
fn test_1889_a_held_update_that_says_nothing_changed_writes_nothing_and_a_corrupt_file_is_never_overwritten() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("desktop.toml");
    DesktopConfig::update_at(&path, |cfg| {
        cfg.pause_sync = true;
        (true, ())
    })
    .expect("seed");
    let written = std::fs::read(&path).expect("exists");
    let mut held = HeldConfig::from(DesktopConfig::default());
    let answer = held
        .update_held_at(&path, |held| {
            held.pause_sync = false; // changed in memory, but the closure says: do not save
            (false, 7)
        })
        .expect("no write, no error");
    assert_eq!(answer, 7);
    assert_eq!(
        std::fs::read(&path).expect("exists"),
        written,
        "the file is byte for byte as it was"
    );
    // Fix round 1 (M3): the copy is what the file holds, not the change that was not saved.
    assert!(
        held.pause_sync,
        "the copy reads the file (pause_sync = true), not the unsaved change"
    );
    assert_eq!(
        toml::to_string_pretty(&*held).expect("serialize the copy"),
        toml::to_string_pretty(&DesktopConfig::load_from(&path).expect("reads back")).expect("serialize the file"),
        "after a no-save update the copy equals what a load of the file returns"
    );

    // A file that cannot be read is an error and is left alone, and the held copy is not touched.
    std::fs::write(&path, "this is = not [valid toml").expect("write");
    let mut held = HeldConfig::from(DesktopConfig {
        last_signed_in_email: Some("kept@beebeeb.io".to_string()),
        ..DesktopConfig::default()
    });
    let result = held.update_held_at(&path, |held| {
        held.pause_sync = true;
        (true, ())
    });
    assert!(result.is_err(), "a corrupt config is an error");
    assert_eq!(
        std::fs::read_to_string(&path).expect("exists"),
        "this is = not [valid toml"
    );
    assert_eq!(held.last_signed_in_email.as_deref(), Some("kept@beebeeb.io"));
    assert!(!held.pause_sync, "a failed update leaves the copy as it was");
}

/// The sign-out of an account switch logs a read that failed and a write that failed apart.
#[test]
fn test_1889_try_update_at_says_whether_the_read_or_the_write_failed() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("desktop.toml");
    std::fs::write(&path, "this is = not [valid toml").expect("write");
    let read = DesktopConfig::try_update_at(&path, |_| (true, ()));
    assert!(matches!(read, Err(UpdateError::Read(_))), "{read:?}");

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("desktop.toml");
    // A save writes `desktop.toml.tmp` first: a directory in its place fails the write.
    std::fs::create_dir(path.with_extension("toml.tmp")).expect("mkdir");
    let write = DesktopConfig::try_update_at(&path, |cfg| {
        cfg.pause_sync = true;
        (true, ())
    });
    assert!(matches!(write, Err(UpdateError::Write(_))), "{write:?}");
    assert!(!path.exists(), "nothing was written");
    // `update_at` says the same thing as a string.
    let as_string = DesktopConfig::update_at(&path, |_| (true, ()));
    assert_eq!(as_string, Err(write.expect_err("failed").into_error().to_string()));
}

// ── Task 1889 fix round 1 (M5) — a settings write carries only the fields that changed ──────────────

/// The Settings windows used to send back the whole object they read when they opened, so changing a speed after a
/// pause from the tray wrote the old `pause_sync` over the pause. A write now carries only what changed; every field
/// that is left out keeps the value on the file.
#[test]
fn test_1889_m5_a_settings_write_with_one_field_changes_only_that_field() {
    let patch: DesktopSettings =
        serde_json::from_str(r#"{ "download_kbps_limit": 5000 }"#).expect("one field is a write");
    let mut cfg = DesktopConfig {
        upload_kbps_limit: 111,
        pause_sync: true,
        notify_conflicts: false,
        notify_sync_complete: true,
        notify_quota_warnings: false,
        ..DesktopConfig::default()
    };
    cfg.apply_settings(patch);
    assert_eq!(cfg.download_kbps_limit, 5000, "the field that was sent");
    assert_eq!(cfg.upload_kbps_limit, 111);
    assert!(cfg.pause_sync, "a pause made elsewhere is not undone");
    assert!(!cfg.notify_conflicts && cfg.notify_sync_complete && !cfg.notify_quota_warnings);
}

#[test]
fn test_1889_m5_a_whole_settings_object_still_writes_every_field() {
    let wanted = DesktopConfig {
        upload_kbps_limit: 7,
        download_kbps_limit: 8,
        pause_sync: true,
        notify_conflicts: false,
        notify_sync_complete: true,
        notify_quota_warnings: false,
        ..DesktopConfig::default()
    };
    let json = serde_json::to_string(&DesktopSettings::from(&wanted)).expect("serialize");
    let back: DesktopSettings = serde_json::from_str(&json).expect("a whole object parses");
    let mut cfg = DesktopConfig::default();
    cfg.apply_settings(back);
    assert_eq!(
        (cfg.upload_kbps_limit, cfg.download_kbps_limit, cfg.pause_sync),
        (7, 8, true)
    );
    assert!(!cfg.notify_conflicts && cfg.notify_sync_complete && !cfg.notify_quota_warnings);
}

// ── Spec 2026-10-06 (macOS Finder setup reconciler) §9 ─────────────────

#[test]
fn a_0_8_11_desktop_toml_with_the_old_finder_keys_still_loads() {
    let cfg: DesktopConfig = toml::from_str(include_str!("../../../../src-tauri/tests/fixtures/desktop-0.8.11.toml"))
        .expect("a 0.8.11 config parses");
    // The old keys still load: Windows and Linux keep using them; macOS ignores them.
    assert_eq!(cfg.finder_install_status.as_deref(), Some("error"));
    assert_eq!(cfg.finder_install_reason_category.as_deref(), Some("unknown"));
    assert_eq!(cfg.finder_last_failure, None);
    assert!(!cfg.finder_signed_out_by_choice);
    let back = toml::to_string_pretty(&cfg).expect("serialize");
    assert!(!back.contains("finder_last_failure"), "None is not written:\n{back}");
    assert!(
        !back.contains("finder_signed_out_by_choice"),
        "false is not written:\n{back}"
    );
}

#[test]
fn the_new_finder_keys_round_trip() {
    use crate::finder_setup::error::FailureRecord;
    use crate::surfaces::phase::FinderFailureReason;
    // A struct literal, not field assignments after `default()`: clippy's
    // `field_reassign_with_default` would add a warning over the baseline.
    let cfg = DesktopConfig {
        finder_signed_out_by_choice: true,
        finder_last_failure: Some(FailureRecord {
            reason: FinderFailureReason::FolderTaken,
            domain: "NSCocoaErrorDomain".into(),
            code: 516,
            at: 1_791_291_909,
        }),
        ..DesktopConfig::default()
    };
    let text = toml::to_string_pretty(&cfg).expect("serialize");
    assert!(text.contains("reason = \"folder_taken\""), "{text}");
    let back: DesktopConfig = toml::from_str(&text).expect("parse");
    assert_eq!(back.finder_last_failure, cfg.finder_last_failure);
    assert!(back.finder_signed_out_by_choice);
}

/// Forward compatibility (area A M3): a later build can add a failure reason and write it, and a downgrade
/// (`install_channel_downgrade`) then runs this build on that file. An unknown reason reads as `Unknown`; it never
/// makes the whole `desktop.toml` unreadable for every caller.
#[test]
fn a_failure_reason_from_a_later_build_reads_as_unknown_and_the_rest_still_loads() {
    use crate::surfaces::phase::FinderFailureReason;
    let text = r#"
sync_root = "/tmp/bb-later-build"
finder_signed_out_by_choice = true

[finder_last_failure]
reason = "something_new"
domain = "NSFileProviderErrorDomain"
code = -2099
at = 1791291909
"#;
    let cfg: DesktopConfig = toml::from_str(text).expect("a later build's reason never breaks the file");
    let failure = cfg.finder_last_failure.expect("the record is kept");
    assert_eq!(failure.reason, FinderFailureReason::Unknown);
    assert_eq!(
        (failure.domain.as_str(), failure.code),
        ("NSFileProviderErrorDomain", -2099)
    );
    assert!(cfg.finder_signed_out_by_choice, "and the rest of the file still loads");
}

// ── Lead ruling T4-3: the old, unredacted Finder error is scrubbed on macOS ─────────
//
// A 0.8.11 `desktop.toml` can hold `finder_install_last_error` = the OS's own message, which
// can name a file or folder. macOS no longer writes it, so loading drops it and the next save
// removes it from disk. Windows/Linux still read and write it. The platform is a parameter of
// `scrub_legacy_finder_error` so both branches run on any host; the two file tests below are
// `cfg`-split and run the real load/save against a temp dir on whichever host builds them.
// None of these tests calls `DesktopConfig::load`/`save`/`path`, so none can reach the
// person's real `desktop.toml`.

const FIXTURE_ERROR: &str =
    "The file couldn\u{2019}t be saved because a file with the same name already exists. (NSCocoaErrorDomain 516)";
const PATH_ERROR: &str =
    "The file \u{201c}/Users/sam/Secret Folder/tax.pdf\u{201d} couldn\u{2019}t be saved. (NSCocoaErrorDomain 516)";

/// The 0.8.11 fixture with a path in the old error key (the fixture itself carries none).
fn fixture_with_a_path_in_the_old_error() -> String {
    let fixture = include_str!("../../../../src-tauri/tests/fixtures/desktop-0.8.11.toml");
    let with_path = fixture.replace(FIXTURE_ERROR, PATH_ERROR);
    assert_ne!(
        with_path, fixture,
        "the fixture's error line changed: update FIXTURE_ERROR"
    );
    with_path
}

#[test]
fn the_legacy_finder_error_is_scrubbed_on_macos_and_kept_elsewhere() {
    let text = fixture_with_a_path_in_the_old_error();
    let on_macos: DesktopConfig = toml::from_str(&text).expect("parse");
    assert_eq!(
        on_macos.finder_install_last_error.as_deref(),
        Some(PATH_ERROR),
        "precondition"
    );
    let mut elsewhere = on_macos.clone();
    let mut on_macos = on_macos;

    on_macos.scrub_legacy_finder_error(ConfigHost::MacOs);
    elsewhere.scrub_legacy_finder_error(ConfigHost::Other);

    assert_eq!(on_macos.finder_install_last_error, None);
    assert_eq!(elsewhere.finder_install_last_error.as_deref(), Some(PATH_ERROR));
    // Only the one field that can carry a path goes; the other old keys still load.
    assert_eq!(on_macos.finder_install_status.as_deref(), Some("error"));
    assert_eq!(on_macos.finder_install_last_attempt_at, Some(1_791_291_909));
    assert_eq!(on_macos.finder_install_reason_category.as_deref(), Some("unknown"));
    assert_eq!(on_macos.sync_root, elsewhere.sync_root);
}

#[cfg(target_os = "macos")]
#[test]
fn on_macos_loading_a_0_8_11_file_drops_the_old_error_and_the_next_save_removes_it_from_disk() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("desktop.toml");
    std::fs::write(&path, fixture_with_a_path_in_the_old_error()).expect("write fixture");

    let cfg = DesktopConfig::load_from(&path).expect("a 0.8.11 file loads");
    assert_eq!(cfg.finder_install_last_error, None);
    assert_eq!(
        cfg.finder_install_status.as_deref(),
        Some("error"),
        "the other old keys still load"
    );
    // Loading never rewrites the file (it runs from many threads): the path is on disk until a save.
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("/Users/sam/Secret Folder")
    );

    cfg.save_to(&path).expect("save");
    let saved = std::fs::read_to_string(&path).unwrap();
    for leaked in ["/Users/sam/Secret", "tax.pdf", "finder_install_last_error"] {
        assert!(
            !saved.contains(leaked),
            "the saved file still holds {leaked:?}:\n{saved}"
        );
    }
    assert!(
        saved.contains("finder_install_status"),
        "only the error key is dropped:\n{saved}"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn off_macos_a_0_8_11_file_with_the_old_error_round_trips_unchanged() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("desktop.toml");
    std::fs::write(&path, fixture_with_a_path_in_the_old_error()).expect("write fixture");

    let cfg = DesktopConfig::load_from(&path).expect("a 0.8.11 file loads");
    assert_eq!(cfg.finder_install_last_error.as_deref(), Some(PATH_ERROR));

    cfg.save_to(&path).expect("save");
    let back = DesktopConfig::load_from(&path).expect("reload");
    assert_eq!(back.finder_install_last_error.as_deref(), Some(PATH_ERROR));
    assert_eq!(back.finder_install_status.as_deref(), Some("error"));
}

// ── No unit test may reach the person's real desktop.toml ────────────────────────────

#[test]
fn unit_tests_resolve_the_config_path_in_a_sandbox_never_the_real_one() {
    // `DesktopConfig::path()` is what `load()`, `update()` and `ensure_account_id()` resolve.
    // Under `cfg(test)` it must point inside a per-process sandbox next to the test binary,
    // so a test that calls `load()`/`update()` (directly, or through a command it exercises)
    // cannot read, delete or rewrite the real `desktop.toml` in the person's config dir.
    // Only paths are computed here; nothing is read.
    let sandboxed = DesktopConfig::path().expect("config path resolves");
    let real_dir = dirs::config_dir().expect("a user config dir exists on a dev machine or CI runner");
    let real = real_dir
        .join(super::paths::app_config_dir())
        .join(super::paths::CONFIG_FILENAME);

    assert_ne!(sandboxed, real, "unit tests resolved the REAL desktop.toml");
    // The specific real `beebeeb` config folder, not the whole config dir: a `CARGO_TARGET_DIR`
    // under `~/Library/Application Support` (or `~/.config`) puts the sandbox inside the config
    // dir legitimately.
    let real_app_dir = real_dir.join(super::paths::app_config_dir());
    assert!(
        !sandboxed.starts_with(&real_app_dir),
        "{sandboxed:?} is inside the real config folder {real_app_dir:?}"
    );
    let exe_dir = std::env::current_exe()
        .expect("test binary path")
        .parent()
        .expect("exe dir")
        .to_path_buf();
    assert!(
        sandboxed.starts_with(&exe_dir),
        "{sandboxed:?} is not inside the test binary's directory {exe_dir:?}"
    );
}

// ── D-P2-6 — typed error, sync mode and the two boolean flags ──────────

/// The message of every variant is the line main produced: lib.rs logs it,
/// and a command's String error is that same line.
#[test]
fn config_error_display_keeps_the_legacy_message_of_every_variant() {
    let io = |text: &str| std::io::Error::new(std::io::ErrorKind::Other, text);
    let path = Path::new("/tmp/desktop.toml");

    let read = io("boom");
    let expected = format!("read /tmp/desktop.toml: {read}");
    let error = ConfigError::ReadFile {
        path: path.to_path_buf(),
        source: read,
    };
    assert_eq!(error.to_string(), expected);
    assert_eq!(String::from(error), expected, "a command sees the same line");

    let create_dir = io("boom");
    assert_eq!(
        ConfigError::CreateConfigDirectory(create_dir).to_string(),
        "create config dir: boom"
    );
    assert_eq!(
        ConfigError::ConfigDirectoryUnknown.to_string(),
        "could not determine user config directory"
    );

    let written = io("boom");
    let expected = format!("write /tmp/desktop.toml: {written}");
    assert_eq!(
        ConfigError::WriteFile {
            path: path.to_path_buf(),
            source: written,
        }
        .to_string(),
        expected
    );

    let chmod = io("boom");
    let expected = format!("chmod /tmp/desktop.toml: {chmod}");
    assert_eq!(
        ConfigError::SetPermissions {
            path: path.to_path_buf(),
            source: chmod,
        }
        .to_string(),
        expected
    );

    let rename = io("boom");
    let expected = format!("rename /tmp/a.toml → /tmp/b.toml: {rename}");
    assert_eq!(
        ConfigError::Rename {
            from: PathBuf::from("/tmp/a.toml"),
            to: PathBuf::from("/tmp/b.toml"),
            source: rename,
        }
        .to_string(),
        expected
    );

    let flush = io("boom");
    let expected = format!("flush /tmp/desktop.toml: {flush}");
    assert_eq!(
        ConfigError::Flush {
            path: path.to_path_buf(),
            source: flush,
        }
        .to_string(),
        expected
    );

    assert_eq!(
        ConfigError::MissingParentDirectory.to_string(),
        "the config file has no directory"
    );
    assert_eq!(
        ConfigError::NotADirectory(PathBuf::from("/tmp/x")).to_string(),
        "/tmp/x exists but is not a directory"
    );

    let make = io("boom");
    let expected = format!("create directory /tmp/x: {make}");
    assert_eq!(
        ConfigError::CreateDirectory {
            path: PathBuf::from("/tmp/x"),
            source: make,
        }
        .to_string(),
        expected
    );

    let parse = toml::from_str::<DesktopConfig>("= this is not toml").expect_err("invalid toml");
    let expected = format!("parse /tmp/desktop.toml: {parse}");
    assert_eq!(
        ConfigError::ParseFile {
            path: path.to_path_buf(),
            source: parse,
        }
        .to_string(),
        expected
    );

    // A value TOML cannot serialize stands in for the write path's serializer failure.
    let unrepresentable = std::collections::BTreeMap::from([(vec![1_u8], 2_u8)]);
    let serialize = toml::to_string_pretty(&unrepresentable).expect_err("a sequence key is not TOML");
    assert!(
        ConfigError::Serialize(serialize).to_string().starts_with("serialize: "),
        "the serializer failure keeps its prefix"
    );
}

/// The four stored mode strings parse to the enum, and any other value is
/// an error — never a silent default.
#[test]
fn config_mode_parses_the_four_stored_modes_and_refuses_an_unknown_one() {
    for (raw, mode) in [
        ("everything", ConfigMode::Everything),
        ("smart", ConfigMode::Smart),
        ("custom", ConfigMode::Custom),
        ("online_only", ConfigMode::OnlineOnly),
    ] {
        assert_eq!(ConfigMode::parse(raw).expect("a known mode"), mode);
    }
    assert!(matches!(
        ConfigMode::parse("turbo"),
        Err(ConfigError::UnknownSyncMode(value)) if value == "turbo"
    ));

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("desktop.toml");
    std::fs::write(&path, "sync_mode = \"turbo\"\n").expect("write");
    assert!(
        matches!(
            DesktopConfig::load_from(&path),
            Err(ConfigError::UnknownSyncMode(value)) if value == "turbo"
        ),
        "an unknown mode in the file is an error, not a default"
    );
}

/// The known-folder setter reads as a sentence; the frozen lib.rs command
/// still sends its boolean, which means the same two states.
#[test]
fn known_folder_backup_names_the_two_states_and_reads_a_command_boolean() {
    assert_eq!(KnownFolderBackup::from(true), KnownFolderBackup::Enabled);
    assert_eq!(KnownFolderBackup::from(false), KnownFolderBackup::Disabled);

    let mut cfg = DesktopConfig::default();
    cfg.set_known_folder("documents", KnownFolderBackup::Enabled);
    assert!(cfg.known_folder_enabled("documents"));
    cfg.set_known_folder("documents", KnownFolderBackup::Disabled);
    assert!(!cfg.known_folder_enabled("documents"));
    cfg.set_known_folder("pictures", true);
    assert!(cfg.known_folder_enabled("pictures"));
}

/// The scrub is the macOS-only migration; naming the host makes the branch
/// explicit and still runs both sides on any build host.
#[test]
fn config_host_says_whether_the_legacy_finder_error_is_scrubbed() {
    let secret = Some("/Users/someone/Library/secret".to_string());
    let mut on_macos = DesktopConfig {
        finder_install_last_error: secret.clone(),
        ..DesktopConfig::default()
    };
    on_macos.scrub_legacy_finder_error(ConfigHost::MacOs);
    assert_eq!(on_macos.finder_install_last_error, None);

    let mut elsewhere = DesktopConfig {
        finder_install_last_error: secret.clone(),
        ..DesktopConfig::default()
    };
    elsewhere.scrub_legacy_finder_error(ConfigHost::Other);
    assert_eq!(elsewhere.finder_install_last_error, secret);
}
