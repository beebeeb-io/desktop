//! The first run's settings and the sync-folder check with the engine's own parts: a real settings file in a
//! folder of the test's own (`CONFIG_DIR_OVERRIDE`), a real handle, the real disk. Every name is invented.

use std::path::PathBuf;

use super::super::{Engine, EngineConfig};
use super::{Places, decide_first_run, final_path};
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

// Windows' temp folder is inside the apps' data folder, which the rule refuses: the Windows cases are the three
// `on_windows_…` tests below, which pass their own places.
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
            // The real app never syncs into the test build's folder (wherever this OS puts it: on a Mac it is
            // inside the folder apps keep their data in, which the rule says first).
            let others = crate::config::default_sync_root_suggestion(REAL.other_build());
            let refused = check(&others);
            assert!(
                matches!(refused, Err(SyncFolderRefused::AnotherApps | SyncFolderRefused::Home)),
                "{refused:?}"
            );
            #[cfg(target_os = "linux")]
            assert_eq!(refused, Err(SyncFolderRefused::AnotherApps));
        }
    });
}

#[test]
fn a_folder_that_is_not_there_yet_is_resolved_from_the_deepest_folder_that_is() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().canonicalize().unwrap();
    let missing = dir.path().join("a").join("b").join("c");
    assert_eq!(final_path(&missing), Some(real.join("a").join("b").join("c")));
    assert!(!dir.path().join("a").exists(), "resolving made nothing");
    assert_eq!(final_path(dir.path()), Some(real));
}

/// A handle with its own settings folder, run `body` with the test's places.
fn among(body: impl FnOnce(&Engine<MemoryCredentials>)) {
    let settings = tempfile::tempdir().unwrap();
    CONFIG_DIR_OVERRIDE.sync_scope(settings.path().to_path_buf(), || {
        let (engine, _) = engine();
        body(&engine);
    });
}

#[cfg(unix)]
#[test]
fn a_link_in_the_path_is_judged_by_where_it_leads() {
    let dir = tempfile::tempdir().unwrap();
    let t = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(t.join("real").join("own")).unwrap();
    std::fs::create_dir_all(t.join("else").join("plain")).unwrap();
    std::os::unix::fs::symlink(t.join("real"), t.join("else").join("door")).unwrap();
    let places = Places {
        own: vec![t.join("real").join("own")],
        ..Places::default()
    };
    among(|engine| {
        let check = |folder: PathBuf| engine.check_sync_folder_among(&folder, &places);
        assert_eq!(
            check(t.join("else/door/own/sub")),
            Err(SyncFolderRefused::OwnData),
            "through the link"
        );
        assert_eq!(check(t.join("else/plain/sub")), Ok(()), "control: no link");
    });
}

#[cfg(target_os = "windows")]
mod on_windows {
    use super::*;

    /// A temp folder, its fake home inside it, and the places that name only that home.
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path().canonicalize().unwrap();
        let t = PathBuf::from(t.to_str().unwrap().trim_start_matches(r"\\?\"));
        let home = t.join("Home");
        std::fs::create_dir_all(home.join("AppData")).unwrap();
        (dir, t, home)
    }

    #[test]
    fn on_windows_the_folder_check_asks_the_real_disk_and_this_computer() {
        let (_dir, t, home) = fixture();
        let own = t.join("Own");
        std::fs::create_dir_all(own.join("deeper")).unwrap();
        let places = Places {
            home: Some(home.clone()),
            system: super::super::system_folders().into_iter().map(PathBuf::from).collect(),
            own: vec![own.clone()],
            other_builds: Vec::new(),
        };
        among(|engine| {
            let check = |folder: &std::path::Path| engine.check_sync_folder_among(folder, &places);
            let folder = t.join("Beebeeb");
            assert_eq!(check(&folder), Ok(()), "a folder that is not there yet");
            assert!(!folder.exists(), "the check made nothing");
            std::fs::create_dir(&folder).unwrap();
            // Some Windows installs (a hosted CI runner) have no Cloud Files service: Windows then cannot say whose
            // sync folder an existing folder is, and the rule must refuse it as `Unreadable`, never let it through.
            if crate::sync_root_owner::registered(&folder) == crate::sync_root_owner::Registration::Unreadable {
                println!(
                    "this computer cannot read Cloud Files registrations: an existing folder is judged Unreadable (fails closed), the Ok case for it is not provable here"
                );
                assert_eq!(
                    check(&folder),
                    Err(SyncFolderRefused::Unreadable),
                    "a folder that is there, no registration readable"
                );
            } else {
                assert_eq!(check(&folder), Ok(()), "a folder that is there");
            }
            let a_file = t.join("notes.txt");
            std::fs::write(&a_file, b"x").unwrap();
            assert_eq!(check(&a_file), Err(SyncFolderRefused::NotAFolder));
            let junction = t.join("junction");
            let made = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&junction)
                .arg(&folder)
                .output()
                .unwrap();
            assert!(made.status.success(), "mklink /J failed");
            assert_eq!(check(&junction), Err(SyncFolderRefused::NotAFolder));
            assert_eq!(check(std::path::Path::new("C:\\")), Err(SyncFolderRefused::System));
            let root = std::env::var("SystemRoot").unwrap();
            assert_eq!(check(&PathBuf::from(root).join("Sync")), Err(SyncFolderRefused::System));
            assert_eq!(check(&home), Err(SyncFolderRefused::Home));
            assert_eq!(check(&home.join("AppData").join("x")), Err(SyncFolderRefused::Home));
            assert_eq!(check(&own), Err(SyncFolderRefused::OwnData));
            assert_eq!(check(&own.join("deeper")), Err(SyncFolderRefused::OwnData));
        });
    }

    /// The 8.3 name of `path` (which exists).
    fn short_name(path: &std::path::Path) -> PathBuf {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        use windows::Win32::Storage::FileSystem::GetShortPathNameW;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut out = vec![0u16; 1024];
        // SAFETY: `wide` is NUL-terminated and outlives the call; `out` is a writable buffer of the length given.
        let len = unsafe { GetShortPathNameW(windows::core::PCWSTR(wide.as_ptr()), Some(&mut out)) } as usize;
        assert!(len > 0 && len < out.len(), "GetShortPathNameW failed");
        PathBuf::from(std::ffi::OsString::from_wide(&out[..len]))
    }

    #[test]
    fn on_windows_a_short_name_is_judged_by_the_folder_it_names() {
        let (_dir, t, home) = fixture();
        let own = t.join("Beebeeb Own Data Folder");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::create_dir_all(t.join("Plain")).unwrap();
        let short = short_name(&own);
        if short == own {
            // Volumes can switch 8.3 names off (a hosted CI runner does). The junction test and the any-OS tests
            // still cover the rule; this one proves nothing here, and says so.
            println!("SKIPPED: this volume makes no short names, so a short name cannot be judged here");
            return;
        }
        let places = Places {
            home: Some(home),
            own: vec![own.clone()],
            ..Places::default()
        };
        among(|engine| {
            let check = |folder: PathBuf| engine.check_sync_folder_among(&folder, &places);
            assert_eq!(check(short.join("sub")), Err(SyncFolderRefused::OwnData));
            assert_eq!(check(t.join("Plain").join("sub")), Ok(()), "control: a plain folder");
        });
    }

    #[test]
    fn on_windows_a_junction_in_the_path_is_judged_by_where_it_leads() {
        let (_dir, t, home) = fixture();
        let own = t.join("own");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::create_dir_all(t.join("Plain")).unwrap();
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(t.join("door"))
            .arg(&own)
            .output()
            .unwrap();
        assert!(made.status.success(), "mklink /J failed");
        let places = Places {
            home: Some(home),
            own: vec![own],
            ..Places::default()
        };
        among(|engine| {
            let check = |folder: PathBuf| engine.check_sync_folder_among(&folder, &places);
            assert_eq!(check(t.join("door").join("sub")), Err(SyncFolderRefused::OwnData));
            assert_eq!(check(t.join("Plain").join("sub")), Ok(()), "control: a plain folder");
        });
    }
}
