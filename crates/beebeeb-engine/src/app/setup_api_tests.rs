//! The first run's settings and the sync-folder check with the engine's own parts: a real settings file in a
//! folder of the test's own (`CONFIG_DIR_OVERRIDE`), a real handle, the real disk. Every name is invented.

use std::path::PathBuf;

use super::super::{Engine, EngineConfig};
use super::decide_first_run;
use crate::config::{CONFIG_DIR_OVERRIDE, DesktopConfig};
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::signin::memory_store::MemoryCredentials;
use crate::sync_folder_rule::SyncFolderRefused;

/// A full path on this OS (the settings drop any other when they are read) that nothing makes.
fn invented(name: &str) -> PathBuf {
    std::env::temp_dir().join("invented-sync").join(name)
}

fn file() -> DesktopConfig {
    DesktopConfig::load_for(&REAL).expect("the settings read")
}

/// A real handle on the process's test state folder (a handle opens on no other), which nothing here writes.
fn engine() -> (Engine<MemoryCredentials>, PathBuf) {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-slot".into(),
        state_dir: state_dir.clone(),
        api_base: "http://127.0.0.1:9".into(),
    };
    let engine = Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens");
    (engine, state_dir)
}

#[test]
fn a_new_pc_has_its_first_run_to_do_and_it_is_decided_once() {
    let dir = tempfile::tempdir().unwrap();
    CONFIG_DIR_OVERRIDE.sync_scope(dir.path().to_path_buf(), || {
        assert_eq!(file().native_setup_done, None, "a file the native app never opened");
        assert_eq!(decide_first_run(&REAL), Ok(true));
        assert_eq!(file().native_setup_done, Some(false));
        // Quitting in the middle of the first run, after the first start kept its folder: the next open still
        // has the first run to do. The settings said so already; a folder that came later does not change it.
        DesktopConfig::update_for(&REAL, |config| {
            config.sync_root = Some(invented("Beebeeb"));
            (true, ())
        })
        .unwrap();
        assert_eq!(decide_first_run(&REAL), Ok(true));
        assert_eq!(file().native_setup_done, Some(false));
    });
}

#[test]
fn an_08x_install_never_sees_the_first_run() {
    let dir = tempfile::tempdir().unwrap();
    CONFIG_DIR_OVERRIDE.sync_scope(dir.path().to_path_buf(), || {
        // What 0.8.x left: a sync folder, and no word about the native first run.
        DesktopConfig::update_for(&REAL, |config| {
            config.sync_root = Some(invented("Beebeeb"));
            (true, ())
        })
        .unwrap();
        assert_eq!(file().native_setup_done, None);
        assert_eq!(decide_first_run(&REAL), Ok(false));
        assert_eq!(file().native_setup_done, Some(true));
        assert_eq!(decide_first_run(&REAL), Ok(false), "and it stays decided");
    });
}

#[test]
fn settings_that_never_met_the_native_app_do_not_carry_the_key() {
    let text = toml::to_string_pretty(&DesktopConfig::default()).unwrap();
    assert!(!text.contains("native_setup_done"), "{text}");
    let old: DesktopConfig = toml::from_str("").unwrap();
    assert_eq!(old.native_setup_done, None);
    let said: DesktopConfig = toml::from_str("native_setup_done = true").unwrap();
    assert_eq!(said.native_setup_done, Some(true));
}

#[test]
fn the_sync_folder_is_kept_once_and_the_first_run_reads_its_flag() {
    let dir = tempfile::tempdir().unwrap();
    CONFIG_DIR_OVERRIDE.sync_scope(dir.path().to_path_buf(), || {
        let (engine, _) = engine();
        assert_eq!(engine.settings_sync_root(), Ok(None));
        assert_eq!(engine.first_run_due(), Ok(true));
        assert_eq!(engine.keep_sync_root(&invented("first")), Ok(true));
        assert_eq!(engine.keep_sync_root(&invented("second")), Ok(false));
        assert_eq!(engine.settings_sync_root(), Ok(Some(invented("first"))));
        // Keeping a folder says nothing about the first run.
        assert_eq!(file().native_setup_done, None);
        assert_eq!(engine.first_run_due(), Ok(true));
        DesktopConfig::update_for(&REAL, |config| {
            config.native_setup_done = Some(true);
            (true, ())
        })
        .unwrap();
        assert_eq!(engine.first_run_due(), Ok(false));
    });
}

// Windows' temp folder is inside the apps' data folder, which the rule refuses: the rule's Windows cases are
// `sync_folder_rule_tests`, on text.
#[cfg(not(target_os = "windows"))]
#[test]
fn the_folder_check_asks_the_real_disk_and_this_computer() {
    let (dir, place) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    CONFIG_DIR_OVERRIDE.sync_scope(dir.path().to_path_buf(), || {
        let (engine, state_dir) = engine();
        let check = |folder: &std::path::Path| engine.check_sync_folder(folder);

        let folder = place.path().join("Beebeeb");
        assert_eq!(check(&folder), Ok(()), "a folder that is not there yet");
        assert!(!folder.exists(), "the check made nothing");
        std::fs::create_dir(&folder).unwrap();
        assert_eq!(check(&folder), Ok(()), "a folder that is there");

        let a_file = place.path().join("notes.txt");
        std::fs::write(&a_file, b"x").unwrap();
        assert_eq!(check(&a_file), Err(SyncFolderRefused::NotAFolder));
        let a_link = place.path().join("link");
        std::os::unix::fs::symlink(&folder, &a_link).unwrap();
        assert_eq!(check(&a_link), Err(SyncFolderRefused::NotAFolder));

        assert_eq!(check(&state_dir), Err(SyncFolderRefused::OwnData));
        assert_eq!(check(&state_dir.join("cache")), Err(SyncFolderRefused::OwnData));
        let holds_it = state_dir.parent().expect("the state folder is in a folder");
        assert_eq!(check(holds_it), Err(SyncFolderRefused::OwnData));
        assert_eq!(
            check(dir.path()),
            Err(SyncFolderRefused::OwnData),
            "the settings' folder"
        );

        assert_eq!(
            check(std::path::Path::new("Beebeeb")),
            Err(SyncFolderRefused::NotAFullPath)
        );
        assert_eq!(check(std::path::Path::new("/")), Err(SyncFolderRefused::System));
        assert_eq!(
            check(std::path::Path::new("/usr/share/sync")),
            Err(SyncFolderRefused::System)
        );
        if let Some(home) = dirs::home_dir() {
            assert_eq!(check(&home), Err(SyncFolderRefused::Home));
            // The real app never syncs into the test build's folder.
            assert_eq!(check(&home.join("Beebeeb Test")), Err(SyncFolderRefused::AnotherApps));
        }
    });
}
