//! One process opened as the TEST identity: every consumer of a per-app name hands back the test build's name,
//! and the real app's names are out of its reach. The suites elsewhere run as the real identity (today's names,
//! in sandboxed dirs), so this binary is the one place a consumer that ignores its identity goes red.
//!
//! Everything here is a path computation or a write inside the test sandbox and a temp dir; nothing touches the
//! OS credential store, a registration or the network. The Windows-only consumers (Cloud Files provider name,
//! sync-root id, GUID and class ids) are pinned in `windows_cf::registration`'s tests, which compile there; the
//! credential item names (`<service>/<account>/<leaf>`, a crate-private enum) in `keychain::tests`
//! (`a_store_names_its_items_under_its_identitys_service`), which runs on every OS against that OS's store type.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use beebeeb_engine::app::{CredentialStores, Engine, EngineConfig};
use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::engine_bridge::FinderStagingBases;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::EngineIdentity;
use beebeeb_engine::identity::{REAL, TEST};
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::{local_state, state_paths};

fn has_component(path: &Path, name: &str) -> bool {
    path.components().any(|part| part.as_os_str() == name)
}

/// The base dir a host would pass, and the state dir the engine made of it for the TEST identity (once).
fn opened_as_test() -> &'static (PathBuf, PathBuf) {
    static OPENED: OnceLock<(tempfile::TempDir, (PathBuf, PathBuf))> = OnceLock::new();
    &OPENED
        .get_or_init(|| {
            let base = tempfile::tempdir().expect("a base dir");
            state_paths::init_from_app_local_data_dir(&TEST, base.path()).expect("the first init names the identity");
            let state =
                state_paths::beebeeb_state_dir_for(&TEST).expect("the state dir of the identity it was made for");
            let dirs = (base.path().to_path_buf(), state);
            (base, dirs)
        })
        .1
}

#[test]
fn the_state_folder_is_the_identitys_and_no_other_identity_gets_it() {
    let (base, state) = opened_as_test();
    assert_eq!(
        state,
        &base.join("beebeeb-test"),
        "the host names a base; the engine joins the identity's name"
    );
    assert!(
        state_paths::beebeeb_state_dir_for(&REAL).is_err(),
        "a caller with another identity gets no state dir, not this one's"
    );
    assert!(
        state_paths::init_from_app_local_data_dir(&REAL, base).is_err(),
        "a second init with another identity is an error, never a silent first-wins"
    );
    assert!(
        state_paths::init_from_app_local_data_dir(&TEST, &base.join("elsewhere")).is_err(),
        "nor one that moves the folder"
    );
    assert!(
        state_paths::init_from_app_local_data_dir(&TEST, base).is_ok(),
        "the same answer twice is fine"
    );
}

#[test]
fn the_settings_file_is_the_identitys_and_a_write_does_not_reach_the_real_one() {
    let test_path = DesktopConfig::path_for(&TEST).unwrap();
    let real_path = DesktopConfig::path_for(&REAL).unwrap();
    assert!(
        test_path.ends_with(Path::new("beebeeb-test").join("desktop.toml")),
        "{test_path:?}"
    );
    assert!(
        real_path.ends_with(Path::new("beebeeb").join("desktop.toml")),
        "{real_path:?}"
    );
    assert_ne!(test_path.parent(), real_path.parent());

    assert!(!DesktopConfig::load_for(&REAL).unwrap().pause_sync);
    DesktopConfig::update_for(&TEST, |cfg| {
        cfg.pause_sync = true;
        (true, ())
    })
    .unwrap();
    assert!(DesktopConfig::load_for(&TEST).unwrap().pause_sync);
    assert!(
        !DesktopConfig::load_for(&REAL).unwrap().pause_sync,
        "the pause landed in the real app's settings"
    );
    assert!(!real_path.exists(), "the test build wrote the real app's settings file");

    let account = DesktopConfig::ensure_account_id_for(&TEST).unwrap();
    assert_eq!(
        DesktopConfig::load_for(&TEST).unwrap().account_id.as_deref(),
        Some(account.as_str())
    );
    assert!(!real_path.exists());
}

#[cfg(not(target_os = "macos"))]
#[test]
fn the_suggested_sync_folder_is_the_identitys() {
    let test = beebeeb_engine::config::default_sync_root_suggestion(&TEST);
    let real = beebeeb_engine::config::default_sync_root_suggestion(&REAL);
    assert_eq!(test.file_name().and_then(|n| n.to_str()), Some("Beebeeb Test"));
    assert_eq!(real.file_name().and_then(|n| n.to_str()), Some("Beebeeb"));
    assert_eq!(test.parent(), real.parent(), "beside the real folder, never it");
}

#[test]
fn staged_and_kept_copies_go_under_the_identitys_folder() {
    let bases = FinderStagingBases::current(&TEST);
    let candidates = bases.candidates();
    assert!(!candidates.is_empty());
    for root in candidates.iter().chain([&bases.cache_root(), &bases.temp_root()]) {
        assert!(
            root.ends_with(Path::new("beebeeb-test").join("finder-writes")),
            "{root:?}"
        );
        assert!(!has_component(root, "beebeeb"), "{root:?}");
    }
    assert_eq!(
        bases
            .durable_root()
            .map(|root| root.ends_with(Path::new("beebeeb-test").join("finder-writes"))),
        Some(true)
    );
    assert!(
        FinderStagingBases::current(&REAL)
            .cache_root()
            .ends_with(Path::new("beebeeb").join("finder-writes"))
    );

    let data = Path::new(if cfg!(windows) { r"C:\data" } else { "/data" });
    let kept = beebeeb_engine::sign_out::kept_names::kept_root;
    assert_eq!(
        kept(None, Some(data), &TEST),
        Some(data.join("beebeeb-test").join("Changes not sent"))
    );
    assert_eq!(
        kept(None, Some(data), &REAL),
        Some(data.join("beebeeb").join("Changes not sent"))
    );
    // The real app's sandbox container is not the test build's.
    let real_container = data
        .join("Library")
        .join("Containers")
        .join("io.beebeeb.app")
        .join("Data")
        .join("Documents");
    assert_eq!(
        kept(Some(&real_container), Some(data), &REAL),
        Some(real_container.join("Changes not sent"))
    );
    assert_eq!(
        kept(Some(&real_container), Some(data), &TEST),
        Some(data.join("beebeeb-test").join("Changes not sent"))
    );
}

#[test]
fn the_purge_deletes_only_inside_the_identitys_folders() {
    let temp = std::env::temp_dir();
    let roots = local_state::disposable_cache_roots(&TEST);
    assert!(!roots.is_empty());
    for root in &roots {
        assert_ne!(root, &temp, "the whole temp dir is not the test build's to purge");
        assert!(
            has_component(root, "beebeeb-test") || root.to_string_lossy().contains("io.beebeeb.app.test"),
            "{root:?}"
        );
    }
    assert!(
        local_state::disposable_cache_roots(&REAL).contains(&temp),
        "the real app's roots are unchanged"
    );

    // A file straight in the temp dir (where the real app's Linux mount caches `bb_<id>`): the real app may
    // delete it, the test build may not. A file in the test build's own folder: the test build may.
    let theirs = tempfile::Builder::new().prefix("bb_").tempfile_in(&temp).unwrap();
    assert!(local_state::is_disposable_cache_path(&REAL, theirs.path()));
    assert!(!local_state::is_disposable_cache_path(&TEST, theirs.path()));
    let own_dir = TEST.scratch_dir_in(&temp);
    std::fs::create_dir_all(&own_dir).unwrap();
    let mine = tempfile::Builder::new().prefix("bb_").tempfile_in(&own_dir).unwrap();
    assert!(local_state::is_disposable_cache_path(&TEST, mine.path()));

    // The lifecycle log folder that no purge touches is the identity's own.
    let logs = |name: &str| Path::new("/Users/a/Library/Logs").join(name).join("lifecycle.log");
    assert!(local_state::is_lifecycle_log_path(&TEST, &logs("Beebeeb Test")));
    assert!(!local_state::is_lifecycle_log_path(&TEST, &logs("Beebeeb")));
    assert!(local_state::is_lifecycle_log_path(&REAL, &logs("Beebeeb")));
    assert!(!local_state::is_lifecycle_log_path(&REAL, &logs("Beebeeb Test")));

    let sync_root = tempfile::tempdir().unwrap();
    let allowed = beebeeb_engine::free_up_space::free_up_space_allowed_roots(&TEST, sync_root.path());
    if let Some(cache) = dirs::cache_dir() {
        assert!(allowed.iter().any(|root| root.ends_with("beebeeb-test")), "{allowed:?}");
        assert!(!allowed.contains(&cache.join("beebeeb")), "{allowed:?}");
    }
}

#[cfg(unix)]
#[test]
fn the_daemon_socket_is_the_identitys() {
    let test = beebeeb_engine::ipc_socket::ipc_socket_path(&TEST);
    let real = beebeeb_engine::ipc_socket::ipc_socket_path(&REAL);
    assert_ne!(
        test, real,
        "a test engine would bind, and remove, the real daemon's socket"
    );
    #[cfg(not(target_os = "macos"))]
    {
        assert_eq!(
            test.file_name().and_then(|n| n.to_str()),
            Some("beebeeb-test-daemon.sock")
        );
        assert_eq!(real.file_name().and_then(|n| n.to_str()), Some("beebeeb-daemon.sock"));
    }
    #[cfg(target_os = "macos")]
    {
        assert!(
            has_component(&test, "R8352WDJJR.io.beebeeb.app.test.fileprovider"),
            "{test:?}"
        );
        assert!(
            has_component(&real, "R8352WDJJR.io.beebeeb.app.fileprovider"),
            "{real:?}"
        );
    }
}

#[test]
fn the_backup_device_is_the_test_builds_own() {
    let device = beebeeb_engine::backup::device::device_self;
    // A machine without a usable name or machine id has no backup device under either identity.
    if let (Some(test), Some(real)) = (device(&TEST), device(&REAL)) {
        assert_ne!(test.device_id, real.device_id);
        assert_eq!(test.name, format!("{} (test)", real.name));
    }
}

/// The handle: opened with the identity the state dir was resolved for, and with no other. On a Mac the test
/// build is refused outright.
#[test]
fn the_engine_handle_opens_for_this_identity_only() {
    let (_, state) = opened_as_test();
    let open = |identity| {
        let (host, _messages) = ChannelHost::new(|| {});
        let config = EngineConfig {
            account_id: "acct-identity".to_string(),
            state_dir: state.clone(),
            api_base: "http://127.0.0.1:9".to_string(),
        };
        Engine::open(identity, config, host, MemoryCredentials::default()).map(|_| ())
    };
    let as_test = open(&TEST);
    if cfg!(target_os = "macos") {
        assert!(as_test.is_err_and(|words| words.contains("macOS")));
    } else {
        assert_eq!(as_test, Ok(()));
    }
    assert!(
        open(&REAL).is_err(),
        "a handle for the real app over the test build's state dir"
    );
}

/// Credential stores that write down which app each store was asked for.
#[derive(Default)]
struct AskedFor {
    inner: MemoryCredentials,
    identities: std::sync::Arc<std::sync::Mutex<Vec<&'static EngineIdentity>>>,
}

impl CredentialStores for AskedFor {
    type Store = <MemoryCredentials as CredentialStores>::Store;
    fn for_account(&self, identity: &'static EngineIdentity, account_id: &str) -> Self::Store {
        self.identities.lock().unwrap().push(identity);
        self.inner.for_account(identity, account_id)
    }
}

/// Review of #309, P2-1: the handle took its identity twice (`Engine::open(&TEST, …, PlatformCredentials(&REAL))`
/// was accepted and read the real app's credentials). Now the stores carry none: every store the handle asks for
/// is asked for with the handle's own identity, and the platform stores cannot be built with one.
#[test]
#[cfg(not(target_os = "macos"))] // the test build does not open on a Mac (the test above)
fn the_handle_names_its_own_identity_to_the_credential_stores() {
    let (_, state) = opened_as_test();
    let stores = AskedFor::default();
    let asked = stores.identities.clone();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "acct-stores".to_string(),
        state_dir: state.clone(),
        api_base: "http://127.0.0.1:9".to_string(),
    };
    let engine = Engine::open(&TEST, config, host, stores).expect("opens as the test build");
    let _ = engine.restore();
    let _ = engine.gate();
    let asked = asked.lock().unwrap();
    assert!(!asked.is_empty(), "restore reads the credential store");
    assert!(
        asked.iter().all(|identity| std::ptr::eq(*identity, &TEST)),
        "a store was asked for with another identity than the handle's"
    );
}

/// The platform stores are a unit value: there is no identity to hand them, so none can differ from the handle's.
#[test]
fn the_platform_credential_stores_carry_no_identity() {
    let app = include_str!("../src/app.rs").replace("\r\n", "\n");
    assert!(
        app.contains("\npub struct PlatformCredentials;\n"),
        "PlatformCredentials grew a field"
    );
    assert!(
        app.contains("fn for_account(&self, identity: &'static EngineIdentity, account_id: &str) -> Self::Store;"),
        "the trait no longer takes the handle's identity per call"
    );
    assert!(
        app.contains("AuthVault::new(self.stores.for_account(self.identity, &self.config.account_id))"),
        "the handle's one vault constructor passes its own identity"
    );
    assert_eq!(app.matches(".for_account(").count(), 1, "one place asks the stores");
    let _unit: beebeeb_engine::app::PlatformCredentials = beebeeb_engine::app::PlatformCredentials;
}
